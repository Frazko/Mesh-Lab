use super::*;
use mesh_crypto::DeliverySecret;
use mesh_protocol::{issue_certificate, CertificateClaims};
struct Rng(u8);
impl RandomSource for Rng {
    fn fill(&mut self, b: &mut [u8]) -> mesh_crypto::Result<()> {
        self.0 = self.0.wrapping_add(1);
        b.fill(self.0);
        Ok(())
    }
}
struct Lab {
    root: IdentitySigningKey,
    signers: Vec<IdentitySigningKey>,
    certs: Vec<Vec<u8>>,
    roster: VerifiedRoster,
}
impl Lab {
    fn new() -> Self {
        let root = IdentitySigningKey::import(Zeroizing::new([42; 32]));
        let signers: Vec<_> = (1..=3)
            .map(|i| IdentitySigningKey::import(Zeroizing::new([i; 32])))
            .collect();
        let scope = Scope {
            group: [7; 32],
            epoch: 1,
        };
        let certs: Vec<_> = (0..3)
            .map(|i| {
                issue_certificate(
                    &root,
                    &CertificateClaims {
                        group: scope.group,
                        epoch: scope.epoch,
                        member: MemberId([i as u8 + 1; 32]),
                        signing_key: signers[i].public_key(),
                        delivery_key: DeliverySecret::import(Zeroizing::new([i as u8 + 10; 32]))
                            .unwrap()
                            .public_key(),
                        valid_from: 0,
                        valid_until: 1000,
                        serial: i as u64 + 1,
                    },
                )
                .unwrap()
            })
            .collect();
        let roster = VerifiedRoster::verify(root.public_key(), scope, &certs, &[], 1).unwrap();
        Self {
            root,
            signers,
            certs,
            roster,
        }
    }
    fn handshake(&self, role: Role, local: u8, expected: u8, rng: &mut Rng) -> Handshake {
        Handshake::start(
            Config {
                role,
                local: MemberId([local; 32]),
                expected_peer: Some(MemberId([expected; 32])),
                now: 1,
            },
            &self.roster,
            &SessionSecret::import(Zeroizing::new([local + 30; 32])),
            rng,
        )
        .unwrap()
    }
    fn pair(&self, rng: &mut Rng) -> (Session, Session) {
        let mut a = self.handshake(Role::Initiator, 1, 2, rng);
        let mut b = self.handshake(Role::Responder, 2, 1, rng);
        b.read(&a.write(2).unwrap(), 2).unwrap();
        a.read(&b.write(2).unwrap(), 2).unwrap();
        b.read(&a.write(2).unwrap(), 2).unwrap();
        (
            a.finish(&self.roster, 3).unwrap(),
            b.finish(&self.roster, 3).unwrap(),
        )
    }
    fn authenticate(&self, a: &mut Session, b: &mut Session) {
        let x = a.authentication(&self.signers[0], &self.roster, 4).unwrap();
        let y = b.authentication(&self.signers[1], &self.roster, 4).unwrap();
        assert!(matches!(
            b.receive(&x, &self.roster, 4),
            Ok(Incoming::Authenticated(id)) if id == MemberId([1;32])
        ));
        assert!(matches!(
            a.receive(&y, &self.roster, 4),
            Ok(Incoming::Authenticated(id)) if id == MemberId([2;32])
        ));
    }
}
fn data(s: &mut Session, bytes: &[u8], r: &VerifiedRoster) -> Vec<u8> {
    match s.receive(bytes, r, 5).unwrap() {
        Incoming::Data(b) => b.to_vec(),
        _ => panic!("expected data"),
    }
}
#[test]
fn xx_requires_mutual_certificate_proof_before_application_traffic() {
    let l = Lab::new();
    let (mut a, mut b) = l.pair(&mut Rng(70));
    assert_eq!(a.id(), b.id());
    assert!(!a.is_authenticated());
    assert_eq!(a.send(b"early", &l.roster, 4), Err(Error::WrongPhase));
    l.authenticate(&mut a, &mut b);
    assert!(a.is_authenticated() && b.is_authenticated());
    let frame = a.send(b"hello", &l.roster, 5).unwrap();
    assert!(!frame.windows(5).any(|s| s == b"hello"));
    assert_eq!(data(&mut b, &frame, &l.roster), b"hello");
    let frame = b.send(&vec![7; MAX_PAYLOAD], &l.roster, 5).unwrap();
    assert_eq!(data(&mut a, &frame, &l.roster), vec![7; MAX_PAYLOAD]);
}
#[test]
fn replay_reorder_and_forged_high_packet_number_do_not_corrupt_the_window() {
    let l = Lab::new();
    let (mut a, mut b) = l.pair(&mut Rng(80));
    l.authenticate(&mut a, &mut b);
    let x = a.send(b"one", &l.roster, 5).unwrap();
    let y = a.send(b"two", &l.roster, 5).unwrap();
    let mut forged = x.clone();
    forged[33..41].copy_from_slice(&999u64.to_be_bytes());
    assert!(b.receive(&forged, &l.roster, 5).is_err());
    assert_eq!(data(&mut b, &y, &l.roster), b"two");
    assert_eq!(data(&mut b, &x, &l.roster), b"one");
    assert!(matches!(b.receive(&x, &l.roster, 5), Err(Error::Replay)));
    for _ in 0..64 {
        let f = a.send(b"next", &l.roster, 5).unwrap();
        data(&mut b, &f, &l.roster);
    }
    assert!(matches!(b.receive(&y, &l.roster, 5), Err(Error::Replay)));
}
#[test]
fn reconnect_has_fresh_keys_packet_space_and_rejects_old_frames() {
    let l = Lab::new();
    let mut rng = Rng(70);
    let (mut a, mut b) = l.pair(&mut rng);
    l.authenticate(&mut a, &mut b);
    let old = a.send(b"old", &l.roster, 5).unwrap();
    let id = a.id();
    let (mut a, mut b) = l.pair(&mut rng);
    assert_ne!(a.id(), id);
    l.authenticate(&mut a, &mut b);
    assert!(b.receive(&old, &l.roster, 5).is_err());
    assert_eq!(
        data(&mut b, &a.send(b"new", &l.roster, 5).unwrap(), &l.roster),
        b"new"
    );
}
#[test]
fn unexpected_certified_peer_and_reflected_identity_are_rejected() {
    let l = Lab::new();
    let mut rng = Rng(40);
    let mut a = l.handshake(Role::Initiator, 3, 2, &mut rng);
    let mut b = l.handshake(Role::Responder, 2, 1, &mut rng);
    b.read(&a.write(2).unwrap(), 2).unwrap();
    a.read(&b.write(2).unwrap(), 2).unwrap();
    b.read(&a.write(2).unwrap(), 2).unwrap();
    let mut a = a.finish(&l.roster, 3).unwrap();
    let mut b = b.finish(&l.roster, 3).unwrap();
    let f = a.authentication(&l.signers[2], &l.roster, 4).unwrap();
    assert!(matches!(
        b.receive(&f, &l.roster, 4),
        Err(Error::Authentication)
    ));
    assert!(matches!(b.receive(&f, &l.roster, 4), Err(Error::Closed)));
    let (mut a, mut b) = l.pair(&mut rng);
    let f = a.authentication(&l.signers[0], &l.roster, 4).unwrap();
    assert!(a.receive(&f, &l.roster, 4).is_err());
    assert!(matches!(
        b.receive(&f, &l.roster, 4),
        Ok(Incoming::Authenticated(_))
    ));
    assert!(!b.is_authenticated());
}
#[test]
fn wrong_signature_and_cross_transcript_proof_close_the_session() {
    let l = Lab::new();
    for cross in [false, true] {
        let (mut a, mut b) = l.pair(&mut Rng(20));
        let body = proof_body(a.scope, if cross { [0; 32] } else { a.id }, a.local, a.role);
        let signature = l.signers[if cross { 0 } else { 2 }]
            .sign(a.scope, Domain::Session, &body)
            .unwrap();
        let mut w = Writer::default();
        w.array(2);
        w.bytes(&a.local.0);
        w.bytes(&signature);
        let mut c = vec![0];
        c.extend(w.finish());
        let f = a.frame(0, &c).unwrap();
        assert!(matches!(
            b.receive(&f, &l.roster, 4),
            Err(Error::Authentication)
        ));
        assert!(!b.is_authenticated());
        a.close();
    }
}
#[test]
fn timeout_changed_roster_bad_lengths_and_nonce_limit_fail_closed() {
    let l = Lab::new();
    let mut a = l.handshake(Role::Initiator, 1, 2, &mut Rng(10));
    assert_eq!(a.write(31), Err(Error::Expired));
    assert_eq!(a.write(32), Err(Error::Closed));
    let (mut a, mut b) = l.pair(&mut Rng(30));
    let changed =
        VerifiedRoster::verify(l.root.public_key(), l.roster.scope(), &l.certs, &[999], 4).unwrap();
    assert_eq!(
        a.authentication(&l.signers[0], &changed, 4),
        Err(Error::Authentication)
    );
    assert_eq!(
        b.authentication(&l.signers[1], &l.roster, 31),
        Err(Error::Expired)
    );
    let (mut a, mut b) = l.pair(&mut Rng(60));
    l.authenticate(&mut a, &mut b);
    assert_eq!(
        a.send(&vec![0; MAX_PAYLOAD + 1], &l.roster, 5),
        Err(Error::InvalidInput)
    );
    let f = a.send(b"hello", &l.roster, 5).unwrap();
    for n in 0..f.len() {
        assert!(b.receive(&f[..n], &l.roster, 5).is_err());
    }
    assert_eq!(data(&mut b, &f, &l.roster), b"hello");
    a.next = MAX_PACKETS;
    assert_eq!(a.send(b"x", &l.roster, 5), Err(Error::Exhausted));
    assert!(!a.is_authenticated());
}
#[test]
fn phase_rng_failure_and_low_order_dh_are_rejected() {
    struct Bad;
    impl RandomSource for Bad {
        fn fill(&mut self, b: &mut [u8]) -> mesh_crypto::Result<()> {
            b.fill(99);
            Err(mesh_crypto::CryptoError::RandomUnavailable)
        }
    }
    let l = Lab::new();
    assert!(matches!(
        Handshake::start(
            Config {
                role: Role::Initiator,
                local: MemberId([1; 32]),
                expected_peer: None,
                now: 1
            },
            &l.roster,
            &SessionSecret::import(Zeroizing::new([1; 32])),
            &mut Bad
        ),
        Err(Error::RandomUnavailable)
    ));
    let mut b = l.handshake(Role::Responder, 2, 1, &mut Rng(3));
    assert_eq!(b.write(2), Err(Error::WrongPhase));
    b.read(&[0; 32], 2).unwrap();
    assert_eq!(b.write(2), Err(Error::Authentication));
    assert_eq!(b.write(2), Err(Error::Closed));
}
#[test]
fn mismatched_scope_cannot_complete_noise() {
    let l = Lab::new();
    let mut a = l.handshake(Role::Initiator, 1, 2, &mut Rng(4));
    let mut b = l.handshake(Role::Responder, 2, 1, &mut Rng(8));
    let prologue = prologue(Scope {
        group: [8; 32],
        epoch: 1,
    });
    let ephemeral = Zeroizing::new([9; 32]);
    b.state = Some(
        Builder::with_resolver(
            NOISE_NAME.parse().unwrap(),
            Box::new(provider::Resolver(std::sync::Mutex::new(Some(ephemeral)))),
        )
        .local_private_key(&[32; 32])
        .unwrap()
        .prologue(&prologue)
        .unwrap()
        .build_responder()
        .unwrap(),
    );
    b.read(&a.write(2).unwrap(), 2).unwrap();
    assert_eq!(a.read(&b.write(2).unwrap(), 2), Err(Error::Authentication));
}
#[test]
fn cacophony_known_answer_vector_with_the_production_resolver() {
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    let v: serde_json::Value = serde_json::from_str(include_str!(
        "../../../vectors/session/noise-xx-cacophony.json"
    ))
    .unwrap();
    let build = |prefix: &str| {
        let key = Zeroizing::new(hex(v[format!("{prefix}_static")].as_str().unwrap()));
        let ephemeral = Zeroizing::new(
            hex(v[format!("{prefix}_ephemeral")].as_str().unwrap())
                .try_into()
                .unwrap(),
        );
        let prologue = hex(v[format!("{prefix}_prologue")].as_str().unwrap());
        let b = Builder::with_resolver(
            NOISE_NAME.parse().unwrap(),
            Box::new(provider::Resolver(std::sync::Mutex::new(Some(ephemeral)))),
        )
        .local_private_key(&key)
        .unwrap()
        .prologue(&prologue)
        .unwrap();
        if prefix == "init" {
            b.build_initiator().unwrap()
        } else {
            b.build_responder().unwrap()
        }
    };
    let (mut a, mut b) = (build("init"), build("resp"));
    let messages = v["messages"].as_array().unwrap();
    for (i, m) in messages.iter().take(3).enumerate() {
        let (payload, cipher) = (
            hex(m["payload"].as_str().unwrap()),
            hex(m["ciphertext"].as_str().unwrap()),
        );
        let (sender, receiver) = if i % 2 == 0 {
            (&mut a, &mut b)
        } else {
            (&mut b, &mut a)
        };
        let mut out = vec![0; 1024];
        let n = sender.write_message(&payload, &mut out).unwrap();
        assert_eq!(&out[..n], cipher);
        let mut clear = vec![0; 1024];
        let n = receiver.read_message(&cipher, &mut clear).unwrap();
        assert_eq!(&clear[..n], payload);
    }
    assert_eq!(
        a.get_handshake_hash(),
        hex(v["handshake_hash"].as_str().unwrap())
    );
    assert_eq!(a.get_handshake_hash(), b.get_handshake_hash());
    let (a, b) = (
        a.into_stateless_transport_mode().unwrap(),
        b.into_stateless_transport_mode().unwrap(),
    );
    for (i, m) in messages.iter().skip(3).enumerate() {
        let (payload, cipher) = (
            hex(m["payload"].as_str().unwrap()),
            hex(m["ciphertext"].as_str().unwrap()),
        );
        let (sender, receiver) = if i % 2 == 0 { (&b, &a) } else { (&a, &b) };
        let nonce = (i / 2) as u64;
        let mut out = vec![0; 1024];
        let n = sender.write_message(nonce, &payload, &mut out).unwrap();
        assert_eq!(&out[..n], cipher);
        let mut clear = vec![0; 1024];
        let n = receiver.read_message(nonce, &cipher, &mut clear).unwrap();
        assert_eq!(&clear[..n], payload);
    }
}
