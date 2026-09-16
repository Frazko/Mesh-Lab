//! Rust owns the session and frame bytes; a native adapter only moves them.
use mesh_link::{Error as PortError, Frame, LinkId, PortEvent, TransportPort};
use mesh_protocol::VerifiedRoster;
use mesh_session::{Incoming, Session};

pub struct SessionLink<P> {
    port: P,
    link: LinkId,
}

impl<P: TransportPort> SessionLink<P> {
    pub fn new(port: P, link: LinkId) -> Self {
        Self { port, link }
    }

    pub fn send(
        &mut self,
        session: &mut Session,
        payload: &[u8],
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<(), LinkError> {
        let frame = Frame::new(
            session
                .send(payload, roster, now)
                .map_err(LinkError::Session)?,
        )
        .map_err(LinkError::Port)?;
        self.port.send(self.link, &frame).map_err(LinkError::Port)
    }

    /// Delivers only exact frames from this link. Link events remain native facts;
    /// they never authenticate a peer or change a Rust session by themselves.
    pub fn poll(
        &mut self,
        session: &mut Session,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<Vec<Incoming>, LinkError> {
        let mut incoming = Vec::new();
        for event in self.port.poll().map_err(LinkError::Port)? {
            if let PortEvent::Frame { link, frame } = event {
                if link == self.link {
                    incoming.push(
                        session
                            .receive(frame.bytes(), roster, now)
                            .map_err(LinkError::Session)?,
                    );
                }
            }
        }
        Ok(incoming)
    }

    pub fn port_mut(&mut self) -> &mut P {
        &mut self.port
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    Port(PortError),
    Session(mesh_session::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_crypto::{DeliverySecret, IdentitySigningKey, RandomSource, Scope};
    use mesh_protocol::{issue_certificate, CertificateClaims};
    use mesh_session::{Config, Handshake, Role, SessionSecret};
    use mesh_types::durable::MemberId;
    use std::collections::VecDeque;
    use zeroize::Zeroizing;

    struct Rng(u8);
    impl RandomSource for Rng {
        fn fill(&mut self, bytes: &mut [u8]) -> mesh_crypto::Result<()> {
            self.0 = self.0.wrapping_add(1);
            bytes.fill(self.0);
            Ok(())
        }
    }
    struct QueuePort {
        sent: Vec<Frame>,
        events: VecDeque<PortEvent>,
    }
    impl TransportPort for QueuePort {
        fn send(&mut self, link: LinkId, frame: &Frame) -> mesh_link::Result<()> {
            assert_eq!(link, LinkId(9));
            self.sent.push(frame.clone());
            Ok(())
        }
        fn poll(&mut self) -> mesh_link::Result<Vec<PortEvent>> {
            Ok(self.events.drain(..).collect())
        }
    }
    fn roster() -> (VerifiedRoster, [IdentitySigningKey; 2]) {
        let authority = IdentitySigningKey::import(Zeroizing::new([42; 32]));
        let keys = [
            IdentitySigningKey::import(Zeroizing::new([1; 32])),
            IdentitySigningKey::import(Zeroizing::new([2; 32])),
        ];
        let scope = Scope {
            group: [7; 32],
            epoch: 1,
        };
        let certs = (0..2)
            .map(|i| {
                issue_certificate(
                    &authority,
                    &CertificateClaims {
                        group: scope.group,
                        epoch: 1,
                        member: MemberId([(i + 1) as u8; 32]),
                        signing_key: keys[i].public_key(),
                        delivery_key: DeliverySecret::import(Zeroizing::new([(i + 11) as u8; 32]))
                            .unwrap()
                            .public_key(),
                        valid_from: 0,
                        valid_until: 100,
                        serial: (i + 1) as u64,
                    },
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        (
            VerifiedRoster::verify(authority.public_key(), scope, &certs, &[], 1).unwrap(),
            keys,
        )
    }
    fn session_pair(roster: &VerifiedRoster) -> (Session, Session) {
        let mut rng = Rng(50);
        let mut a = Handshake::start(
            Config {
                role: Role::Initiator,
                local: MemberId([1; 32]),
                expected_peer: Some(MemberId([2; 32])),
                now: 1,
            },
            roster,
            &SessionSecret::import(Zeroizing::new([31; 32])),
            &mut rng,
        )
        .unwrap();
        let mut b = Handshake::start(
            Config {
                role: Role::Responder,
                local: MemberId([2; 32]),
                expected_peer: Some(MemberId([1; 32])),
                now: 1,
            },
            roster,
            &SessionSecret::import(Zeroizing::new([32; 32])),
            &mut rng,
        )
        .unwrap();
        b.read(&a.write(2).unwrap(), 2).unwrap();
        a.read(&b.write(2).unwrap(), 2).unwrap();
        b.read(&a.write(2).unwrap(), 2).unwrap();
        (a.finish(roster, 3).unwrap(), b.finish(roster, 3).unwrap())
    }
    fn transfer(port: &mut SessionLink<QueuePort>, target: &mut SessionLink<QueuePort>) {
        for frame in port.port_mut().sent.drain(..) {
            target.port_mut().events.push_back(PortEvent::Frame {
                link: LinkId(9),
                frame,
            });
        }
    }

    #[test]
    fn authenticated_session_data_crosses_only_the_transport_port() {
        let (roster, keys) = roster();
        let (mut a, mut b) = session_pair(&roster);
        let mut left = SessionLink::new(
            QueuePort {
                sent: vec![],
                events: VecDeque::new(),
            },
            LinkId(9),
        );
        let mut right = SessionLink::new(
            QueuePort {
                sent: vec![],
                events: VecDeque::new(),
            },
            LinkId(9),
        );
        let auth_a = Frame::new(a.authentication(&keys[0], &roster, 4).unwrap()).unwrap();
        left.port_mut().sent.push(auth_a);
        transfer(&mut left, &mut right);
        match right.poll(&mut b, &roster, 4).unwrap().as_slice() {
            [Incoming::Authenticated(member)] => assert_eq!(*member, MemberId([1; 32])),
            _ => panic!("expected authenticated peer"),
        }
        let auth_b = Frame::new(b.authentication(&keys[1], &roster, 4).unwrap()).unwrap();
        right.port_mut().sent.push(auth_b);
        transfer(&mut right, &mut left);
        match left.poll(&mut a, &roster, 4).unwrap().as_slice() {
            [Incoming::Authenticated(member)] => assert_eq!(*member, MemberId([2; 32])),
            _ => panic!("expected authenticated peer"),
        }
        left.send(&mut a, b"hola", &roster, 5).unwrap();
        transfer(&mut left, &mut right);
        let received = right.poll(&mut b, &roster, 5).unwrap();
        match received.as_slice() {
            [Incoming::Data(bytes)] => assert_eq!(&bytes[..], b"hola"),
            _ => panic!("expected application data"),
        }
    }
}
