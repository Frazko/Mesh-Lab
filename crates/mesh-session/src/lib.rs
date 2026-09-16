//! Lab session v1: Noise XX, explicit certified identity confirmation, bounded
//! datagrams and per-direction replay windows. No radio, reconnect IK or key I/O.
use mesh_codec::canonical::{Reader, Writer};
use mesh_crypto::{Domain, IdentitySigningKey, RandomSource, Scope};
use mesh_protocol::VerifiedRoster;
use mesh_types::durable::{MemberId, MAX_LOGICAL_TIME};
use snow::{Builder, HandshakeState, StatelessTransportState};
use zeroize::Zeroizing;
mod provider;
pub const NOISE_NAME: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
pub const MAX_PAYLOAD: usize = 4096;
pub const MAX_FRAME: usize = 1 + 32 + 8 + 1 + MAX_PAYLOAD + 16;
pub const MAX_PACKETS: u64 = 1 << 20;
pub const HANDSHAKE_TIMEOUT: u64 = 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    WrongPhase,
    Authentication,
    Replay,
    Expired,
    RandomUnavailable,
    Closed,
    Exhausted,
}
pub type Result<T> = std::result::Result<T, Error>;
fn auth<T, E>(r: std::result::Result<T, E>) -> Result<T> {
    r.map_err(|_| Error::Authentication)
}
/// Separate from Ed25519 identity and HPKE delivery secrets; host stores this key.
pub struct SessionSecret(Zeroizing<[u8; 32]>);
impl SessionSecret {
    pub fn import(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }
    pub fn generate(rng: &mut dyn RandomSource) -> Result<Self> {
        let mut bytes = Zeroizing::new([0; 32]);
        rng.fill(&mut bytes[..])
            .map_err(|_| Error::RandomUnavailable)?;
        Ok(Self(bytes))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Initiator,
    Responder,
}
impl Role {
    fn code(self) -> u64 {
        match self {
            Self::Initiator => 0,
            Self::Responder => 1,
        }
    }
    fn opposite(self) -> Self {
        match self {
            Self::Initiator => Self::Responder,
            Self::Responder => Self::Initiator,
        }
    }
}
pub struct Config {
    pub role: Role,
    pub local: MemberId,
    pub expected_peer: Option<MemberId>,
    pub now: u64,
}
fn prologue(scope: Scope) -> Vec<u8> {
    let mut w = Writer::default();
    w.array(6);
    w.text("MeshLab/Session/v1");
    w.uint(1);
    w.bytes(&scope.group);
    w.uint(scope.epoch);
    w.uint(1);
    w.uint(MAX_PAYLOAD as u64);
    w.finish()
}
fn proof_body(scope: Scope, session: [u8; 32], member: MemberId, role: Role) -> Vec<u8> {
    let mut w = Writer::default();
    w.array(8);
    w.uint(1);
    w.bytes(&scope.group);
    w.uint(scope.epoch);
    w.bytes(&session);
    w.bytes(&member.0);
    w.uint(role.code());
    w.uint(1);
    w.uint(MAX_PAYLOAD as u64);
    w.finish()
}
/// No Clone/Debug/serialization; dropping on any fatal handshake error consumes
/// the ephemeral state. Retransmission must reuse saved frame bytes, never rewind.
pub struct Handshake {
    state: Option<HandshakeState>,
    config: Config,
    scope: Scope,
    roster: [u8; 32],
    deadline: u64,
}
impl Handshake {
    pub fn start(
        config: Config,
        roster: &VerifiedRoster,
        key: &SessionSecret,
        rng: &mut dyn RandomSource,
    ) -> Result<Self> {
        auth(roster.validate_at(config.now))?;
        auth(roster.signing_key(config.local))?;
        if config.expected_peer == Some(config.local) {
            return Err(Error::InvalidInput);
        }
        if let Some(peer) = config.expected_peer {
            auth(roster.signing_key(peer))?;
        }
        let deadline = config
            .now
            .checked_add(HANDSHAKE_TIMEOUT)
            .filter(|v| *v <= MAX_LOGICAL_TIME)
            .ok_or(Error::InvalidInput)?;
        let mut entropy = Zeroizing::new([0; 32]);
        rng.fill(&mut entropy[..])
            .map_err(|_| Error::RandomUnavailable)?;
        let p = prologue(roster.scope());
        let builder = Builder::with_resolver(
            NOISE_NAME.parse().map_err(|_| Error::InvalidInput)?,
            Box::new(provider::Resolver(std::sync::Mutex::new(Some(entropy)))),
        );
        let builder = auth(builder.local_private_key(&key.0[..]))?;
        let builder = auth(builder.prologue(&p))?;
        let state = auth(match config.role {
            Role::Initiator => builder.build_initiator(),
            Role::Responder => builder.build_responder(),
        })?;
        Ok(Self {
            state: Some(state),
            config,
            scope: roster.scope(),
            roster: roster.digest(),
            deadline,
        })
    }
    fn check(&mut self, now: u64) -> Result<()> {
        if self.state.is_none() {
            return Err(Error::Closed);
        }
        if now < self.config.now || now >= self.deadline {
            self.state = None;
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn write(&mut self, now: u64) -> Result<Vec<u8>> {
        self.check(now)?;
        let s = self.state.as_mut().ok_or(Error::Closed)?;
        if s.is_handshake_finished() || !s.is_my_turn() {
            return Err(Error::WrongPhase);
        }
        let mut bytes = vec![0; 96];
        match s.write_message(&[], &mut bytes) {
            Ok(n) => {
                bytes.truncate(n);
                Ok(bytes)
            }
            Err(_) => {
                self.state = None;
                Err(Error::Authentication)
            }
        }
    }
    pub fn read(&mut self, bytes: &[u8], now: u64) -> Result<()> {
        self.check(now)?;
        let s = self.state.as_mut().ok_or(Error::Closed)?;
        if s.is_handshake_finished() || s.is_my_turn() {
            return Err(Error::WrongPhase);
        }
        if bytes.is_empty() || bytes.len() > 96 {
            self.state = None;
            return Err(Error::InvalidInput);
        }
        let mut payload = Zeroizing::new([0; 96]);
        match s.read_message(bytes, &mut payload[..]) {
            Ok(0) => Ok(()),
            _ => {
                self.state = None;
                Err(Error::Authentication)
            }
        }
    }
    pub fn finish(mut self, roster: &VerifiedRoster, now: u64) -> Result<Session> {
        self.check(now)?;
        auth(roster.validate_at(now))?;
        if roster.digest() != self.roster {
            return Err(Error::Authentication);
        }
        let state = self.state.take().ok_or(Error::Closed)?;
        if !state.is_handshake_finished() {
            return Err(Error::WrongPhase);
        }
        let id = state
            .get_handshake_hash()
            .try_into()
            .map_err(|_| Error::Authentication)?;
        let state = auth(state.into_stateless_transport_mode())?;
        Ok(Session {
            state: Some(state),
            id,
            scope: self.scope,
            roster: self.roster,
            role: self.config.role,
            local: self.config.local,
            expected: self.config.expected_peer,
            peer: None,
            sent_auth: false,
            next: 1,
            replay: Replay::default(),
            started: self.config.now,
            deadline: self.deadline,
        })
    }
}
#[derive(Default)]
struct Replay {
    highest: Option<u64>,
    bits: u64,
}
impl Replay {
    fn check(&self, n: u64) -> Result<()> {
        if let Some(high) = self.highest {
            if n <= high {
                let delta = high - n;
                if delta >= 64 || self.bits & (1 << delta) != 0 {
                    return Err(Error::Replay);
                }
            }
        }
        Ok(())
    }
    fn accept(&mut self, n: u64) {
        match self.highest {
            None => {
                self.highest = Some(n);
                self.bits = 1
            }
            Some(high) if n > high => {
                self.bits = if n - high >= 64 {
                    1
                } else {
                    (self.bits << (n - high)) | 1
                };
                self.highest = Some(n)
            }
            Some(high) => self.bits |= 1 << (high - n),
        }
    }
}
pub struct Session {
    state: Option<StatelessTransportState>,
    id: [u8; 32],
    scope: Scope,
    roster: [u8; 32],
    role: Role,
    local: MemberId,
    expected: Option<MemberId>,
    peer: Option<MemberId>,
    sent_auth: bool,
    next: u64,
    replay: Replay,
    started: u64,
    deadline: u64,
}
pub enum Incoming {
    Authenticated(MemberId),
    Data(Zeroizing<Vec<u8>>),
}
impl Session {
    /// Certified member identity proven by the remote Noise authentication.
    /// Hosts use it for relay `previous_hop` validation, never radio addresses.
    pub fn peer(&self) -> Option<MemberId> {
        self.peer
    }
    pub fn id(&self) -> [u8; 32] {
        self.id
    }
    pub fn is_authenticated(&self) -> bool {
        self.state.is_some() && self.sent_auth && self.peer.is_some()
    }
    pub fn close(&mut self) {
        self.state = None;
        self.peer = None;
    }
    fn check(&mut self, roster: &VerifiedRoster, now: u64) -> Result<()> {
        if self.state.is_none() {
            return Err(Error::Closed);
        }
        if now < self.started || (!self.is_authenticated() && now >= self.deadline) {
            self.close();
            return Err(Error::Expired);
        }
        if roster.digest() != self.roster || roster.validate_at(now).is_err() {
            self.close();
            return Err(Error::Authentication);
        }
        Ok(())
    }
    fn frame(&self, n: u64, clear: &[u8]) -> Result<Vec<u8>> {
        let mut bytes = vec![0; 41 + clear.len() + 16];
        bytes[0] = 1;
        bytes[1..33].copy_from_slice(&self.id);
        bytes[33..41].copy_from_slice(&n.to_be_bytes());
        let size = auth(self.state.as_ref().ok_or(Error::Closed)?.write_message(
            n,
            clear,
            &mut bytes[41..],
        ))?;
        bytes.truncate(41 + size);
        Ok(bytes)
    }
    pub fn authentication(
        &mut self,
        signer: &IdentitySigningKey,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<Vec<u8>> {
        self.check(roster, now)?;
        if self.sent_auth {
            return Err(Error::WrongPhase);
        }
        if auth(roster.signing_key(self.local))? != signer.public_key() {
            return Err(Error::Authentication);
        }
        let body = proof_body(self.scope, self.id, self.local, self.role);
        let signature = auth(signer.sign(self.scope, Domain::Session, &body))?;
        let mut w = Writer::default();
        w.array(2);
        w.bytes(&self.local.0);
        w.bytes(&signature);
        let mut clear = vec![0];
        clear.extend(w.finish());
        let bytes = self.frame(0, &clear)?;
        self.sent_auth = true;
        Ok(bytes)
    }
    pub fn send(&mut self, bytes: &[u8], roster: &VerifiedRoster, now: u64) -> Result<Vec<u8>> {
        self.check(roster, now)?;
        if !self.is_authenticated() {
            return Err(Error::WrongPhase);
        }
        if bytes.is_empty() || bytes.len() > MAX_PAYLOAD {
            return Err(Error::InvalidInput);
        }
        if self.next >= MAX_PACKETS {
            self.close();
            return Err(Error::Exhausted);
        }
        let n = self.next;
        self.next += 1; // consume before encryption; never reuse after an error
        let mut clear = Zeroizing::new(vec![1]);
        clear.extend(bytes);
        self.frame(n, &clear)
    }
    pub fn receive(&mut self, frame: &[u8], roster: &VerifiedRoster, now: u64) -> Result<Incoming> {
        self.check(roster, now)?;
        if frame.len() < 58 || frame.len() > MAX_FRAME || frame[0] != 1 || frame[1..33] != self.id {
            return Err(Error::InvalidInput);
        }
        let n = u64::from_be_bytes(frame[33..41].try_into().map_err(|_| Error::InvalidInput)?);
        if n >= MAX_PACKETS {
            return Err(Error::Exhausted);
        }
        self.replay.check(n)?;
        if n != 0 && !self.is_authenticated() {
            return Err(Error::WrongPhase);
        }
        let mut clear = Zeroizing::new(vec![0; frame.len() - 41]);
        let len = auth(self.state.as_ref().ok_or(Error::Closed)?.read_message(
            n,
            &frame[41..],
            &mut clear,
        ))?;
        clear.truncate(len);
        if n == 0 {
            let result = (|| {
                if clear.first() != Some(&0) {
                    return Err(Error::Authentication);
                }
                let mut r = auth(Reader::new(&clear[1..], 128))?;
                auth(r.array(2))?;
                let member = MemberId(auth(r.fixed())?);
                let signature = auth(r.fixed())?;
                auth(r.end())?;
                if member == self.local || self.expected.is_some_and(|id| id != member) {
                    return Err(Error::Authentication);
                }
                let body = proof_body(self.scope, self.id, member, self.role.opposite());
                auth(mesh_crypto::verify(
                    auth(roster.signing_key(member))?,
                    self.scope,
                    Domain::Session,
                    &body,
                    signature,
                ))?;
                Ok(member)
            })();
            match result {
                Ok(member) => {
                    self.peer = Some(member);
                    self.replay.accept(0);
                    Ok(Incoming::Authenticated(member))
                }
                Err(e) => {
                    self.close();
                    Err(e)
                }
            }
        } else {
            if clear.first() != Some(&1) || clear.len() < 2 || clear.len() > MAX_PAYLOAD + 1 {
                self.close();
                return Err(Error::InvalidInput);
            }
            self.replay.accept(n);
            clear.remove(0);
            Ok(Incoming::Data(clear))
        }
    }
}

#[cfg(test)]
mod tests;
