//! Pure authenticated-receive reducer. Only the trusted native host supplies
//! storage callbacks. A completed chunk transfer is never a delivery callback.
use mesh_protocol::VerifiedDelivery;
use mesh_types::durable::*;
use std::collections::BTreeMap;

pub const MAX_PENDING_RECEIVES: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveToken {
    incarnation: u64,
    sequence: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceivePhase {
    Inspecting,
    Verifying,
    Committing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveEvent {
    Received {
        object: ObjectId,
    },
    NeedsRetry {
        object: ObjectId,
        error: DurableError,
    },
}
pub enum ReceiveEffect {
    /// Look up the authenticated durable receipt first, including after expiry.
    Inspect {
        token: ReceiveToken,
        object: ObjectId,
    },
    Verify {
        token: ReceiveToken,
        object: ObjectId,
    },
    Commit {
        token: ReceiveToken,
        proof: Box<VerifiedDelivery>,
    },
}
impl ReceiveEffect {
    pub fn token(&self) -> ReceiveToken {
        match self {
            Self::Inspect { token, .. }
            | Self::Verify { token, .. }
            | Self::Commit { token, .. } => *token,
        }
    }
}
#[derive(Default)]
pub struct ReceiveTransition {
    pub effect: Option<ReceiveEffect>,
    pub event: Option<ReceiveEvent>,
}
struct Pending {
    token: ReceiveToken,
    phase: ReceivePhase,
}
pub struct DurableReceiver {
    member: MemberId,
    incarnation: u64,
    next: u64,
    pending: BTreeMap<ObjectId, Pending>,
}
impl DurableReceiver {
    /// Incarnations must not repeat while an older host callback can still arrive.
    pub fn new(member: MemberId, incarnation: u64) -> Result<Self> {
        if incarnation == 0 {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self {
            member,
            incarnation,
            next: 1,
            pending: BTreeMap::new(),
        })
    }
    fn token(&mut self) -> Result<ReceiveToken> {
        let sequence = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or(DurableError::ResourcePressure)?;
        Ok(ReceiveToken {
            incarnation: self.incarnation,
            sequence,
        })
    }
    fn locate(&self, token: ReceiveToken, phase: Option<ReceivePhase>) -> Result<ObjectId> {
        self.pending
            .iter()
            .find(|(_, p)| p.token == token && phase.is_none_or(|phase| phase == p.phase))
            .map(|(id, _)| *id)
            .ok_or(DurableError::Conflict)
    }
    pub fn pending(&self) -> Vec<(ObjectId, ReceivePhase)> {
        self.pending.iter().map(|(id, p)| (*id, p.phase)).collect()
    }
    pub fn start(&mut self, object: ObjectId) -> Result<ReceiveTransition> {
        if self.pending.contains_key(&object) {
            return Ok(ReceiveTransition::default());
        }
        if self.pending.len() >= MAX_PENDING_RECEIVES {
            return Err(DurableError::ResourcePressure);
        }
        let token = self.token()?;
        self.pending.insert(
            object,
            Pending {
                token,
                phase: ReceivePhase::Inspecting,
            },
        );
        Ok(ReceiveTransition {
            effect: Some(ReceiveEffect::Inspect { token, object }),
            event: None,
        })
    }
    /// Pass Some(id) only for a committed authenticated receipt in local storage.
    pub fn inspected(
        &mut self,
        token: ReceiveToken,
        committed: Option<ObjectId>,
    ) -> Result<ReceiveTransition> {
        let object = self.locate(token, Some(ReceivePhase::Inspecting))?;
        if let Some(id) = committed {
            if id != object {
                return Err(DurableError::Conflict);
            }
            return Ok(self.received(object));
        }
        let token = self.token()?;
        self.pending.insert(
            object,
            Pending {
                token,
                phase: ReceivePhase::Verifying,
            },
        );
        Ok(ReceiveTransition {
            effect: Some(ReceiveEffect::Verify { token, object }),
            event: None,
        })
    }
    pub fn verified(
        &mut self,
        token: ReceiveToken,
        proof: VerifiedDelivery,
    ) -> Result<ReceiveTransition> {
        let object = self.locate(token, Some(ReceivePhase::Verifying))?;
        if proof.object_id() != object || proof.member() != self.member {
            return Err(DurableError::Conflict);
        }
        let token = self.token()?;
        self.pending.insert(
            object,
            Pending {
                token,
                phase: ReceivePhase::Committing,
            },
        );
        Ok(ReceiveTransition {
            effect: Some(ReceiveEffect::Commit {
                token,
                proof: Box::new(proof),
            }),
            event: None,
        })
    }
    /// The host calls this only AFTER the receipt+delivery transaction commits.
    pub fn committed(
        &mut self,
        token: ReceiveToken,
        committed: ObjectId,
    ) -> Result<ReceiveTransition> {
        let object = self.locate(token, Some(ReceivePhase::Committing))?;
        if object != committed {
            return Err(DurableError::Conflict);
        }
        Ok(self.received(object))
    }
    fn received(&mut self, object: ObjectId) -> ReceiveTransition {
        self.pending.remove(&object);
        ReceiveTransition {
            effect: None,
            event: Some(ReceiveEvent::Received { object }),
        }
    }
    pub fn failed(
        &mut self,
        token: ReceiveToken,
        error: DurableError,
    ) -> Result<ReceiveTransition> {
        let object = self.locate(token, None)?;
        self.pending.remove(&object);
        Ok(ReceiveTransition {
            effect: None,
            event: Some(ReceiveEvent::NeedsRetry { object, error }),
        })
    }
}
