use mesh_crypto::{DeliverySecret, IdentitySigningKey, RandomSource, Scope};
use mesh_object::ObjectPolicy;
use mesh_protocol::{self as protocol, CertificateClaims, SealedMessage, VerifiedRoster};
use mesh_runtime::durable::*;
use mesh_types::durable::*;
use zeroize::Zeroizing;
fn policy() -> ObjectPolicy {
    ObjectPolicy {
        namespace: Namespace::new("synthetic").unwrap(),
        epoch: 1,
        targets: vec![MemberId([2; 32])],
        expires_at: 100,
        hop_limit: 4,
    }
}
fn request(op: u8, payload: &[u8]) -> SendRequest {
    SendRequest::new(
        OperationId([op; 16]),
        policy(),
        Zeroizing::new(payload.to_vec()),
    )
    .unwrap()
}
struct TestRandom(u64);
impl RandomSource for TestRandom {
    fn fill(&mut self, out: &mut [u8]) -> mesh_crypto::Result<()> {
        for byte in out {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *byte = self.0 as u8;
        }
        Ok(())
    }
}
fn sealed(sequence: u64) -> SealedMessage {
    let mut random = TestRandom(20260911);
    let authority = IdentitySigningKey::import(Zeroizing::new([42; 32]));
    let sender = IdentitySigningKey::import(Zeroizing::new([3; 32]));
    let receiver = DeliverySecret::import(Zeroizing::new([23; 32])).unwrap();
    let scope = Scope {
        group: [7; 32],
        epoch: 1,
    };
    let certificates = vec![
        protocol::issue_certificate(
            &authority,
            &CertificateClaims {
                group: scope.group,
                member: MemberId([1; 32]),
                signing_key: sender.public_key(),
                delivery_key: DeliverySecret::import(Zeroizing::new([24; 32]))
                    .unwrap()
                    .public_key(),
                valid_from: 0,
                valid_until: 1000,
                epoch: 1,
                serial: 1,
            },
        )
        .unwrap(),
        protocol::issue_certificate(
            &authority,
            &CertificateClaims {
                group: scope.group,
                member: MemberId([2; 32]),
                signing_key: IdentitySigningKey::import(Zeroizing::new([4; 32])).public_key(),
                delivery_key: receiver.public_key(),
                valid_from: 0,
                valid_until: 1000,
                epoch: 1,
                serial: 2,
            },
        )
        .unwrap(),
    ];
    let roster =
        VerifiedRoster::verify(authority.public_key(), scope, &certificates, &[], 1).unwrap();
    protocol::seal_message(
        protocol::SealRequest {
            origin: MemberId([1; 32]),
            sequence,
            policy: policy(),
            plaintext: b"authenticated",
            now: 2,
        },
        &roster,
        &sender,
        &mut random,
    )
    .unwrap()
}
fn engine() -> DurableSender {
    DurableSender::new(MemberId([1; 32]), 1).unwrap()
}
#[test]
fn accepted_only_after_matching_durable_commit() {
    let mut e = engine();
    let mut t = e.start(request(1, b"hello"), 1).unwrap();
    assert!(t.event.is_none());
    let reserve = t.effect.take().unwrap().token();
    assert!(e.committed(reserve, ObjectId([0; 32])).is_err());
    t = e.reserved(reserve, 7, None, 2).unwrap();
    assert!(t.event.is_none());
    let protect = t.effect.take().unwrap().token();
    assert!(e.reserved(reserve, 7, None, 2).is_err());
    assert!(e.protected(protect, sealed(8), 3).is_err());
    t = e.protected(protect, sealed(7), 3).unwrap();
    assert!(t.event.is_none());
    let commit = t.effect.take().unwrap().token();
    assert!(e.committed(commit, ObjectId([0; 32])).is_err());
    let id = sealed(7).object().manifest().id();
    assert_eq!(
        e.committed(commit, id).unwrap().event,
        Some(SendEvent::Accepted {
            operation: OperationId([1; 16]),
            object: id
        })
    );
    assert!(e.pending().is_empty());
    assert!(e.committed(commit, id).is_err());
    assert!(e.failed(commit, DurableError::StorageUnavailable).is_err());
}
#[test]
fn duplicate_conflict_capacity_and_late_callbacks_are_bounded() {
    let mut e = engine();
    let token = e
        .start(request(1, b"a"), 1)
        .unwrap()
        .effect
        .unwrap()
        .token();
    let duplicate = e.start(request(1, b"a"), 1).unwrap();
    assert!(duplicate.effect.is_none() && duplicate.event.is_none());
    assert!(matches!(
        e.start(request(1, b"changed"), 1),
        Err(DurableError::Conflict)
    ));
    for i in 2..=8 {
        e.start(request(i, b"x"), 1).unwrap();
    }
    assert_eq!(e.pending().len(), 8);
    assert!(matches!(
        e.start(request(9, b"x"), 1),
        Err(DurableError::ResourcePressure)
    ));
    assert_eq!(
        e.failed(token, DurableError::StorageUnavailable)
            .unwrap()
            .event,
        Some(SendEvent::NeedsRetry {
            operation: OperationId([1; 16]),
            error: DurableError::StorageUnavailable
        })
    );
    let new = e
        .start(request(1, b"a"), 1)
        .unwrap()
        .effect
        .unwrap()
        .token();
    assert_ne!(token, new);
    assert!(e.reserved(token, 1, None, 1).is_err());
    let mut restarted = DurableSender::new(MemberId([1; 32]), 2).unwrap();
    restarted.start(request(1, b"a"), 1).unwrap();
    assert!(restarted.reserved(new, 1, None, 1).is_err());
}
#[test]
fn expiry_does_not_overwrite_an_existing_durable_result() {
    let mut e = engine();
    let effect = e.start(request(1, b"a"), 100).unwrap().effect.unwrap();
    assert!(matches!(
        effect,
        SendEffect::Reserve {
            create_if_missing: false,
            ..
        }
    ));
    let token = effect.token();
    let id = ObjectId([7; 32]);
    assert_eq!(
        e.reserved(token, 1, Some(id), 100).unwrap().event,
        Some(SendEvent::Accepted {
            operation: OperationId([1; 16]),
            object: id
        })
    );
    let effect = e.start(request(2, b"a"), 99).unwrap().effect.unwrap();
    let t = e.reserved(effect.token(), 2, None, 100).unwrap();
    assert_eq!(
        t.event,
        Some(SendEvent::NeedsRetry {
            operation: OperationId([2; 16]),
            error: DurableError::Expired
        })
    );
    assert!(t.effect.is_none());
}
#[test]
fn command_digest_binds_payload_and_every_policy_field() {
    fn hashed(mut p: ObjectPolicy, bytes: &[u8]) -> [u8; 32] {
        p.targets.sort();
        let mut e = engine();
        let request =
            SendRequest::new(OperationId([1; 16]), p, Zeroizing::new(bytes.to_vec())).unwrap();
        match e.start(request, 1).unwrap().effect.unwrap() {
            SendEffect::Reserve { command_digest, .. } => command_digest,
            _ => panic!(),
        }
    }
    let base = hashed(policy(), b"a");
    assert_ne!(base, hashed(policy(), b"b"));
    let mut policies = Vec::new();
    macro_rules! change {
        ($field:ident,$value:expr) => {{
            let mut p = policy();
            p.$field = $value;
            policies.push(p);
        }};
    }
    change!(namespace, Namespace::new("different").unwrap());
    change!(epoch, 2);
    change!(targets, vec![MemberId([3; 32])]);
    change!(expires_at, 101);
    change!(hop_limit, 5);
    for p in policies {
        assert_ne!(base, hashed(p, b"a"));
    }
}
