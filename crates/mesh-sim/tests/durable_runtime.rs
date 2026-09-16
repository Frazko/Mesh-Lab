//! Host integration harness. Its private fixture is NOT an app API.
//! Exercises the authenticated reducer, envelope, roster and SQLCipher store together.
use mesh_crypto::{DeliverySecret, IdentitySigningKey, OsRandom, Scope};
use mesh_object::ObjectPolicy;
use mesh_protocol::{self as protocol, CertificateClaims, SealedMessage, VerifiedRoster};
use mesh_runtime::durable::*;
use mesh_store::{Limits, Store};
use mesh_types::durable::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use zeroize::Zeroizing;
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "mesh-durable-runtime-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn open(&self) -> Store {
        Store::open(
            &self.0.join("node.db"),
            Zeroizing::new([17; 32]),
            MemberId([1; 32]),
            Limits::default(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn request() -> SendRequest {
    SendRequest::new(
        OperationId([1; 16]),
        ObjectPolicy {
            namespace: Namespace::new("mesh.lab.synthetic.v2").unwrap(),
            epoch: 1,
            targets: vec![MemberId([2; 32])],
            expires_at: 100,
            hop_limit: 4,
        },
        Zeroizing::new(b"SYNTHETIC encrypted message".to_vec()),
    )
    .unwrap()
}
fn engine(incarnation: u64) -> DurableSender {
    DurableSender::new(MemberId([1; 32]), incarnation).unwrap()
}
struct Lab {
    signer: IdentitySigningKey,
    receiver: DeliverySecret,
    roster: VerifiedRoster,
}
impl Lab {
    fn new() -> Self {
        let authority = IdentitySigningKey::import(Zeroizing::new([42; 32]));
        let signer = IdentitySigningKey::import(Zeroizing::new([3; 32]));
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
                    signing_key: signer.public_key(),
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
        Self {
            signer,
            receiver,
            roster,
        }
    }
}
fn protect(effect: SendEffect, lab: &Lab, now: u64) -> (EffectToken, SealedMessage) {
    let SendEffect::Protect {
        token,
        origin,
        origin_sequence,
        request,
    } = effect
    else {
        panic!("expected protection")
    };
    let message = request
        .with_plaintext(|plaintext| {
            protocol::seal_message(
                protocol::SealRequest {
                    origin,
                    sequence: origin_sequence,
                    policy: request.policy().clone(),
                    plaintext,
                    now,
                },
                &lab.roster,
                &lab.signer,
                &mut OsRandom,
            )
        })
        .unwrap();
    (token, message)
}
fn assert_recipient_can_verify(store: &Store, id: ObjectId, lab: &Lab, now: u64) {
    let announcement = store
        .authenticated_announcement(id, &lab.roster, now)
        .unwrap();
    protocol::verify_delivery(
        announcement,
        &store.object_bytes(id).unwrap(),
        &lab.roster,
        MemberId([2; 32]),
        &lab.receiver,
        now,
    )
    .unwrap();
}
fn resolve(e: &mut DurableSender, s: &mut Store, effect: SendEffect, now: u64) -> SendTransition {
    let SendEffect::Reserve {
        token,
        operation,
        command_digest,
        create_if_missing,
    } = effect
    else {
        panic!("expected reserve")
    };
    let result = if create_if_missing {
        s.reserve(operation, command_digest).map(Some)
    } else {
        s.lookup_operation(operation, command_digest)
    };
    match result {
        Ok(Some(r)) => e.reserved(token, r.sequence, r.object_id, now).unwrap(),
        Ok(None) => e.failed(token, DurableError::Expired).unwrap(),
        Err(error) => e.failed(token, error).unwrap(),
    }
}
#[test]
fn protected_send_survives_lost_commit_callback_without_reencrypting() {
    let fixture = Fixture::new();
    let mut s = fixture.open();
    let mut e = engine(1);
    let lab = Lab::new();
    let effect = e.start(request(), 1).unwrap().effect.unwrap();
    let t = resolve(&mut e, &mut s, effect, 1);
    assert!(t.event.is_none());
    let (token, message) = protect(t.effect.unwrap(), &lab, 1);
    let t = e.protected(token, message, 2).unwrap();
    assert!(t.event.is_none());
    let SendEffect::Commit {
        token: old_callback,
        operation,
        command_digest,
        message,
    } = t.effect.unwrap()
    else {
        panic!()
    };
    let id = s
        .commit_sealed(operation, command_digest, &message, &lab.roster, 2)
        .unwrap();
    let ciphertext = s.chunk(id, 0).unwrap();
    assert!(!ciphertext.windows(9).any(|b| b == b"SYNTHETIC"));
    assert_recipient_can_verify(&s, id, &lab, 2);
    drop(e);
    drop(s); // Process boundary modeled by dropping/reopening both owners.
    let mut s = fixture.open();
    let mut e = engine(2);
    let effect = e.start(request(), 101).unwrap().effect.unwrap();
    let t = resolve(&mut e, &mut s, effect, 101);
    assert_eq!(
        t.event,
        Some(SendEvent::Accepted {
            operation,
            object: id
        })
    );
    assert!(t.effect.is_none());
    assert_eq!(s.chunk(id, 0).unwrap(), ciphertext);
    assert_eq!(s.stats().unwrap().objects, 1);
    assert!(e.committed(old_callback, id).is_err());
    // The re-opened store still has the signed announcement and all envelope
    // chunks required for a recipient to authenticate and decrypt it.
    assert_recipient_can_verify(&s, id, &lab, 2);
}
#[test]
fn failed_protection_and_expired_request_never_claim_acceptance() {
    let fixture = Fixture::new();
    let mut s = fixture.open();
    let mut e = engine(1);
    let effect = e.start(request(), 1).unwrap().effect.unwrap();
    let t = resolve(&mut e, &mut s, effect, 1);
    let token = t.effect.unwrap().token();
    assert_eq!(
        e.failed(token, DurableError::CryptoUnavailable)
            .unwrap()
            .event,
        Some(SendEvent::NeedsRetry {
            operation: OperationId([1; 16]),
            error: DurableError::CryptoUnavailable
        })
    );
    assert_eq!(s.stats().unwrap().objects, 0);
    assert_eq!(s.stats().unwrap().outbox, 0);
    let before = s.stats().unwrap();
    let expired = SendRequest::new(
        OperationId([9; 16]),
        request().policy().clone(),
        Zeroizing::new(b"expired".to_vec()),
    )
    .unwrap();
    let effect = e.start(expired, 101).unwrap().effect.unwrap();
    let t = resolve(&mut e, &mut s, effect, 101);
    assert!(matches!(
        t.event,
        Some(SendEvent::NeedsRetry {
            error: DurableError::Expired,
            ..
        })
    ));
    assert_eq!(s.stats().unwrap(), before);
}

#[test]
fn real_store_pressure_requires_retry_and_keeps_the_reserved_sequence() {
    let fixture = Fixture::new();
    let mut store = Store::open(
        &fixture.0.join("node.db"),
        Zeroizing::new([17; 32]),
        MemberId([1; 32]),
        Limits {
            bytes: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    let mut e = engine(1);
    let lab = Lab::new();
    let effect = e.start(request(), 1).unwrap().effect.unwrap();
    let transition = resolve(&mut e, &mut store, effect, 1);
    let (token, message) = protect(transition.effect.unwrap(), &lab, 1);
    let transition = e.protected(token, message, 2).unwrap();
    let SendEffect::Commit {
        token,
        operation,
        command_digest,
        message,
    } = transition.effect.unwrap()
    else {
        panic!()
    };
    let failed_id = message.object().manifest().id();
    assert_eq!(
        store.commit_sealed(operation, command_digest, &message, &lab.roster, 2),
        Err(DurableError::ResourcePressure)
    );
    assert!(matches!(
        e.failed(token, DurableError::ResourcePressure)
            .unwrap()
            .event,
        Some(SendEvent::NeedsRetry { .. })
    ));
    assert_eq!(store.stats().unwrap().objects, 0);
    assert_eq!(store.stats().unwrap().outbox, 0);
    drop(store);
    drop(e);
    let mut store = fixture.open();
    let mut e = engine(2);
    let effect = e.start(request(), 3).unwrap().effect.unwrap();
    let transition = resolve(&mut e, &mut store, effect, 3);
    let (token, message) = protect(transition.effect.unwrap(), &lab, 3);
    assert_eq!(message.object().manifest().sequence(), 1);
    assert_ne!(message.object().manifest().id(), failed_id);
    let transition = e.protected(token, message, 4).unwrap();
    let SendEffect::Commit {
        token,
        operation,
        command_digest,
        message,
    } = transition.effect.unwrap()
    else {
        panic!()
    };
    let id = store
        .commit_sealed(operation, command_digest, &message, &lab.roster, 4)
        .unwrap();
    assert!(matches!(
        e.committed(token, id).unwrap().event,
        Some(SendEvent::Accepted { .. })
    ));
    assert_eq!(store.stats().unwrap().operations, 1);
    assert_eq!(store.stats().unwrap().objects, 1);
}
