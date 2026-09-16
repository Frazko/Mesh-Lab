//! Pure durable-send reducer. The native host executes effects serially and feeds
//! correlated results back. No file I/O, randomness, signing or radio runs here.
use mesh_object::{digest, ObjectPolicy};
use mesh_protocol::SealedMessage;
use mesh_types::durable::*;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub const MAX_PENDING_SENDS: usize = 8;
pub const MAX_SEND_BYTES: usize = 48 * 1024;
pub struct SendRequest {
    operation: OperationId,
    policy: ObjectPolicy,
    plaintext: Zeroizing<Vec<u8>>,
}
impl SendRequest {
    pub fn new(
        operation: OperationId,
        mut policy: ObjectPolicy,
        plaintext: Zeroizing<Vec<u8>>,
    ) -> Result<Self> {
        if plaintext.is_empty()
            || plaintext.len() > MAX_SEND_BYTES
            || policy.targets.is_empty()
            || policy.targets.len() > MAX_TARGETS
            || policy.epoch == 0
            || policy.epoch > MAX_LOGICAL_TIME
            || policy.expires_at == 0
            || policy.expires_at > MAX_LOGICAL_TIME
            || policy.hop_limit == 0
            || policy.hop_limit > 16
        {
            return Err(DurableError::InvalidInput);
        }
        policy.targets.sort();
        if !policy.targets.windows(2).all(|p| p[0] < p[1]) {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self {
            operation,
            policy,
            plaintext,
        })
    }
    pub fn operation(&self) -> OperationId {
        self.operation
    }
    pub fn policy(&self) -> &ObjectPolicy {
        &self.policy
    }
    pub fn with_plaintext<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(&self.plaintext)
    }
    fn command_digest(&self, origin: MemberId) -> [u8; 32] {
        let mut b = b"MeshLab/SendCommand/v1\0".to_vec();
        b.extend(origin.0);
        b.push(self.policy.namespace.as_str().len() as u8);
        b.extend(self.policy.namespace.as_str().bytes());
        b.extend(self.policy.epoch.to_be_bytes());
        b.push(self.policy.targets.len() as u8);
        for target in &self.policy.targets {
            b.extend(target.0)
        }
        b.extend(self.policy.expires_at.to_be_bytes());
        b.push(self.policy.hop_limit);
        b.extend((self.plaintext.len() as u32).to_be_bytes());
        b.extend(digest(&self.plaintext));
        digest(&b)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectToken {
    incarnation: u64,
    sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendPhase {
    Reserving,
    Protecting,
    Committing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingSend {
    pub operation: OperationId,
    pub phase: SendPhase,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendEvent {
    /// Local durable acceptance only. No remote delivery is implied.
    Accepted {
        operation: OperationId,
        object: ObjectId,
    },
    /// Commit may be ambiguous on I/O failure. Retry MUST reuse the operation ID.
    NeedsRetry {
        operation: OperationId,
        error: DurableError,
    },
}
pub enum SendEffect {
    Reserve {
        token: EffectToken,
        operation: OperationId,
        command_digest: [u8; 32],
        create_if_missing: bool,
    },
    Protect {
        token: EffectToken,
        origin: MemberId,
        origin_sequence: u64,
        request: SendRequest,
    },
    Commit {
        token: EffectToken,
        operation: OperationId,
        command_digest: [u8; 32],
        /// Preserve both the protected object and its signed announcement. The
        /// authenticated store commits them atomically; transport may only send
        /// these exact persisted bytes after that commit succeeds.
        message: Box<SealedMessage>,
    },
}
impl SendEffect {
    pub fn token(&self) -> EffectToken {
        match self {
            Self::Reserve { token, .. }
            | Self::Protect { token, .. }
            | Self::Commit { token, .. } => *token,
        }
    }
}
#[derive(Default)]
pub struct SendTransition {
    pub effect: Option<SendEffect>,
    pub event: Option<SendEvent>,
}
struct Pending {
    token: EffectToken,
    phase: SendPhase,
    command_digest: [u8; 32],
    policy: ObjectPolicy,
    request: Option<SendRequest>,
    origin_sequence: Option<u64>,
    expected_object: Option<ObjectId>,
}
pub struct DurableSender {
    origin: MemberId,
    incarnation: u64,
    next_token: u64,
    pending: BTreeMap<OperationId, Pending>,
}
impl DurableSender {
    /// The host must never reuse an incarnation while callbacks from it can exist.
    pub fn new(origin: MemberId, incarnation: u64) -> Result<Self> {
        if incarnation == 0 {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self {
            origin,
            incarnation,
            next_token: 1,
            pending: BTreeMap::new(),
        })
    }
    fn token(&mut self) -> Result<EffectToken> {
        let sequence = self.next_token;
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or(DurableError::ResourcePressure)?;
        Ok(EffectToken {
            incarnation: self.incarnation,
            sequence,
        })
    }
    fn locate(&self, token: EffectToken, phase: Option<SendPhase>) -> Result<OperationId> {
        self.pending
            .iter()
            .find(|(_, p)| p.token == token && phase.is_none_or(|phase| p.phase == phase))
            .map(|(op, _)| *op)
            .ok_or(DurableError::Conflict)
    }
    pub fn pending(&self) -> Vec<PendingSend> {
        self.pending
            .iter()
            .map(|(operation, p)| PendingSend {
                operation: *operation,
                phase: p.phase,
            })
            .collect()
    }
    pub fn start(&mut self, request: SendRequest, now: u64) -> Result<SendTransition> {
        if now > MAX_LOGICAL_TIME {
            return Err(DurableError::InvalidInput);
        }
        let command_digest = request.command_digest(self.origin);
        let operation = request.operation;
        if let Some(existing) = self.pending.get(&operation) {
            return if existing.command_digest == command_digest {
                Ok(SendTransition::default())
            } else {
                Err(DurableError::Conflict)
            };
        }
        if self.pending.len() >= MAX_PENDING_SENDS {
            return Err(DurableError::ResourcePressure);
        }
        // Always consult durable idempotency first: an old successful request remains
        // successful after expiry. New/pending expired requests stop after reservation.
        let create_if_missing = now < request.policy.expires_at;
        let token = self.token()?;
        self.pending.insert(
            operation,
            Pending {
                token,
                phase: SendPhase::Reserving,
                command_digest,
                policy: request.policy.clone(),
                request: Some(request),
                origin_sequence: None,
                expected_object: None,
            },
        );
        Ok(SendTransition {
            effect: Some(SendEffect::Reserve {
                token,
                operation,
                command_digest,
                create_if_missing,
            }),
            event: None,
        })
    }
    pub fn reserved(
        &mut self,
        token: EffectToken,
        origin_sequence: u64,
        committed_object: Option<ObjectId>,
        now: u64,
    ) -> Result<SendTransition> {
        let operation = self.locate(token, Some(SendPhase::Reserving))?;
        if origin_sequence == 0 || origin_sequence > MAX_LOGICAL_TIME || now > MAX_LOGICAL_TIME {
            return Err(DurableError::InvalidInput);
        }
        if let Some(object) = committed_object {
            self.pending.remove(&operation);
            return Ok(SendTransition {
                effect: None,
                event: Some(SendEvent::Accepted { operation, object }),
            });
        }
        if now >= self.pending[&operation].policy.expires_at {
            return self.failed(token, DurableError::Expired);
        }
        let next = self.token()?;
        let pending = self
            .pending
            .get_mut(&operation)
            .ok_or(DurableError::NotFound)?;
        pending.phase = SendPhase::Protecting;
        pending.token = next;
        pending.origin_sequence = Some(origin_sequence);
        let request = pending.request.take().ok_or(DurableError::Conflict)?;
        Ok(SendTransition {
            effect: Some(SendEffect::Protect {
                token: next,
                origin: self.origin,
                origin_sequence,
                request,
            }),
            event: None,
        })
    }
    /// Completes protection with the complete authenticated message. Accepting a
    /// bare `PreparedObject` here would make it possible for a host to discard
    /// the roster-bound signed announcement before durable commit.
    pub fn protected(
        &mut self,
        token: EffectToken,
        message: SealedMessage,
        now: u64,
    ) -> Result<SendTransition> {
        let operation = self.locate(token, Some(SendPhase::Protecting))?;
        let p = &self.pending[&operation];
        let m = message.object().manifest();
        if now > MAX_LOGICAL_TIME {
            return Err(DurableError::InvalidInput);
        }
        if m.origin() != self.origin
            || Some(m.sequence()) != p.origin_sequence
            || m.namespace() != &p.policy.namespace
            || m.epoch() != p.policy.epoch
            || m.targets() != p.policy.targets
            || m.expires_at() != p.policy.expires_at
            || m.hop_limit() != p.policy.hop_limit
        {
            return Err(DurableError::Conflict);
        }
        if now >= p.policy.expires_at {
            return self.failed(token, DurableError::Expired);
        }
        let next = self.token()?;
        let p = self
            .pending
            .get_mut(&operation)
            .ok_or(DurableError::NotFound)?;
        p.token = next;
        p.phase = SendPhase::Committing;
        p.expected_object = Some(m.id());
        Ok(SendTransition {
            effect: Some(SendEffect::Commit {
                token: next,
                operation,
                command_digest: p.command_digest,
                message: Box::new(message),
            }),
            event: None,
        })
    }
    pub fn committed(&mut self, token: EffectToken, object: ObjectId) -> Result<SendTransition> {
        let operation = self.locate(token, Some(SendPhase::Committing))?;
        if self.pending[&operation].expected_object != Some(object) {
            return Err(DurableError::Conflict);
        }
        self.pending.remove(&operation);
        Ok(SendTransition {
            effect: None,
            event: Some(SendEvent::Accepted { operation, object }),
        })
    }
    pub fn failed(&mut self, token: EffectToken, error: DurableError) -> Result<SendTransition> {
        let operation = self.locate(token, None)?;
        self.pending.remove(&operation);
        Ok(SendTransition {
            effect: None,
            event: Some(SendEvent::NeedsRetry { operation, error }),
        })
    }
}
