use super::*;
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn arr<const N: usize>(s: &str) -> [u8; N] {
    hex(s).try_into().unwrap()
}
// Synthetic entropy is private to cfg(test), never exported to host adapters.
struct Fixture(u8);
impl RandomSource for Fixture {
    fn fill(&mut self, b: &mut [u8]) -> Result<()> {
        for v in b {
            self.0 = self.0.wrapping_add(1);
            *v = self.0;
        }
        Ok(())
    }
}
struct Fail;
impl RandomSource for Fail {
    fn fill(&mut self, b: &mut [u8]) -> Result<()> {
        b.fill(9);
        Err(CryptoError::RandomUnavailable)
    }
}
fn scope() -> Scope {
    Scope {
        group: [11; 32],
        epoch: 1,
    }
}
fn context() -> ChunkContext {
    ChunkContext {
        scope: scope(),
        object_context_id: [1; 32],
        origin: [2; 32],
        origin_sequence: 3,
        audience_digest: [4; 32],
        namespace: "mesh.lab.text".into(),
        schema: 1,
        chunk_count: 2,
        lifetime: 600,
        priority: 1,
    }
}
fn recipient(secret: &DeliverySecret) -> Recipient {
    Recipient {
        member: [21; 32],
        delivery_key_id: [22; 32],
        public_key: secret.public_key(),
    }
}
#[test]
fn rfc8032_ed25519_vector_and_strict_weak_key_rejection() {
    let key = IdentitySigningKey::import(Zeroizing::new(arr(
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
    )));
    assert_eq!(
        key.public_key(),
        arr("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
    );
    let signature=arr("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
    assert_eq!(key.0.sign(b"").to_bytes(), signature);
    key.0
        .verifying_key()
        .verify_strict(b"", &Signature::from_bytes(&signature))
        .unwrap();
    let mut weak = [0; 32];
    weak[0] = 1;
    let mut forged = [0; 64];
    forged[0] = 1;
    assert!(verify(weak, scope(), Domain::Object, b"", forged).is_err());
}
#[test]
fn signature_domains_group_epoch_payload_and_key_are_bound() {
    let a = IdentitySigningKey::generate(&mut Fixture(0)).unwrap();
    let b = IdentitySigningKey::generate(&mut Fixture(90)).unwrap();
    for domain in [
        Domain::Certificate,
        Domain::Object,
        Domain::Receipt,
        Domain::Transition,
        Domain::Recovery,
        Domain::Presence,
    ] {
        let sig = a.sign(scope(), domain, b"canonical fixture").unwrap();
        verify(a.public_key(), scope(), domain, b"canonical fixture", sig).unwrap();
        for other in [
            Domain::Certificate,
            Domain::Object,
            Domain::Receipt,
            Domain::Transition,
            Domain::Recovery,
            Domain::Presence,
        ] {
            if other != domain {
                assert!(verify(a.public_key(), scope(), other, b"canonical fixture", sig).is_err())
            }
        }
        assert!(verify(b.public_key(), scope(), domain, b"canonical fixture", sig).is_err());
        for changed in [
            Scope {
                epoch: 2,
                ..scope()
            },
            Scope {
                group: [12; 32],
                ..scope()
            },
        ] {
            assert!(verify(a.public_key(), changed, domain, b"canonical fixture", sig).is_err())
        }
        assert!(verify(a.public_key(), scope(), domain, b"changed", sig).is_err());
    }
    assert!(a
        .sign(scope(), Domain::Object, &vec![0; MAX_SIGNED_BYTES + 1])
        .is_err());
}
#[test]
fn rfc8439_aead_vector() {
    let key = arr("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let nonce = arr("070000004041424344454647");
    let aad = hex("50515253c0c1c2c3c4c5c6c7");
    let plaintext=b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let expected=hex("d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691");
    assert_eq!(seal_raw(&key, nonce, &aad, plaintext).unwrap(), expected);
    assert_eq!(&*open_raw(&key, nonce, &aad, &expected).unwrap(), plaintext);
    let mut corrupt = expected;
    corrupt[0] ^= 1;
    assert!(open_raw(&key, nonce, &aad, &corrupt).is_err());
}
#[test]
fn hpke_rfc9180_appendix_a2_known_answer() {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../vectors/crypto/hpke-rfc9180.json")).unwrap();
    let v = &json["vector"];
    let bytes = |field: &str| hex(v[field].as_str().unwrap());
    let (sk, pk) = Kem::derive_keypair(&bytes("ikmR"));
    assert_eq!(&pk.to_bytes()[..], bytes("pkRm"));
    struct Once(Vec<u8>);
    impl RandomSource for Once {
        fn fill(&mut self, dest: &mut [u8]) -> Result<()> {
            assert_eq!(dest.len(), self.0.len());
            dest.copy_from_slice(&self.0);
            self.0.clear();
            Ok(())
        }
    }
    let mut entropy = Once(bytes("ikmE"));
    let mut rng = HpkeRandom {
        source: &mut entropy,
        failed: false,
    };
    let (enc, mut sender) = hpke::setup_sender::<HpkeAead, HkdfSha256, Kem, _>(
        &OpModeS::Base,
        &pk,
        &bytes("info"),
        &mut rng,
    )
    .unwrap();
    assert!(!rng.failed);
    assert_eq!(&enc.to_bytes()[..], bytes("enc"));
    let e = &v["encryptions"][0];
    let pt = hex(e["pt"].as_str().unwrap());
    let aad = hex(e["aad"].as_str().unwrap());
    let ct = hex(e["ct"].as_str().unwrap());
    assert_eq!(sender.seal(&pt, &aad).unwrap(), ct);
    let mut receiver = hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(
        &OpModeR::Base,
        &sk,
        &enc,
        &bytes("info"),
    )
    .unwrap();
    assert_eq!(receiver.open(&ct, &aad).unwrap(), pt);
}
#[test]
fn wrapped_keys_and_chunk_context_bind_every_field() {
    let mut rng = Fixture(0);
    let secret = DeliverySecret::generate(&mut rng).unwrap();
    let recipient = recipient(&secret);
    let ctx = context();
    let mut sealer = ContentSealer::generate(ctx.clone(), &mut rng).unwrap();
    let wrapped = sealer.wrap_for(recipient, &mut rng).unwrap();
    let nonce = sealer.base_nonce();
    let key = unwrap_key(&secret, scope(), ctx.object_context_id, recipient, &wrapped).unwrap();
    let ct = sealer.seal_next(b"synthetic payload").unwrap();
    assert_eq!(
        &**open_chunk(&key, &ctx, nonce, 0, &ct).unwrap(),
        b"synthetic payload"
    );
    let mut changes = Vec::new();
    macro_rules! change {
        ($field:ident,$value:expr) => {{
            let mut c = ctx.clone();
            c.$field = $value;
            changes.push(c);
        }};
    }
    change!(
        scope,
        Scope {
            epoch: 2,
            ..scope()
        }
    );
    change!(
        scope,
        Scope {
            group: [1; 32],
            ..scope()
        }
    );
    change!(object_context_id, [9; 32]);
    change!(origin, [9; 32]);
    change!(origin_sequence, 4);
    change!(audience_digest, [9; 32]);
    change!(namespace, "other".into());
    change!(schema, 2);
    change!(chunk_count, 3);
    change!(lifetime, 601);
    change!(priority, 2);
    for changed in changes {
        assert!(open_chunk(&key, &changed, nonce, 0, &ct).is_err())
    }
    assert!(open_chunk(&key, &ctx, nonce, 1, &ct).is_err());
    let mut changed = recipient;
    changed.member[0] ^= 1;
    assert!(unwrap_key(&secret, scope(), ctx.object_context_id, changed, &wrapped).is_err());
    changed = recipient;
    changed.delivery_key_id[0] ^= 1;
    assert!(unwrap_key(&secret, scope(), ctx.object_context_id, changed, &wrapped).is_err());
    assert!(unwrap_key(
        &secret,
        Scope {
            epoch: 2,
            ..scope()
        },
        ctx.object_context_id,
        recipient,
        &wrapped
    )
    .is_err());
    assert!(unwrap_key(&secret, scope(), [8; 32], recipient, &wrapped).is_err());
    let stranger = DeliverySecret::generate(&mut rng).unwrap();
    assert!(unwrap_key(
        &stranger,
        scope(),
        ctx.object_context_id,
        recipient,
        &wrapped
    )
    .is_err());
    let second = sealer.seal_next(b"synthetic payload").unwrap();
    assert_ne!(ct, second);
    assert_eq!(sealer.seal_next(b"extra"), Err(CryptoError::NonceExhausted));
}
#[test]
fn rng_failures_low_order_keys_bounds_and_nonce_overflow_fail_closed() {
    assert!(matches!(
        ContentSealer::generate(context(), &mut Fail),
        Err(CryptoError::RandomUnavailable)
    ));
    assert!(matches!(
        IdentitySigningKey::generate(&mut Fail),
        Err(CryptoError::RandomUnavailable)
    ));
    assert!(matches!(
        DeliverySecret::generate(&mut Fail),
        Err(CryptoError::RandomUnavailable)
    ));
    let key = ContentKey::generate(&mut Fixture(0)).unwrap();
    let secret = DeliverySecret::generate(&mut Fixture(42)).unwrap();
    let recipient = recipient(&secret);
    assert_eq!(
        wrap_key(&key, scope(), [0; 32], recipient, &mut Fail),
        Err(CryptoError::RandomUnavailable)
    );
    assert!(wrap_key(
        &key,
        scope(),
        [0; 32],
        Recipient {
            public_key: [0; 32],
            ..recipient
        },
        &mut Fixture(0)
    )
    .is_err());
    assert_eq!(nonce_for([255; 12], 1), Err(CryptoError::NonceExhausted));
    let mut ctx = context();
    ctx.chunk_count = 65;
    assert!(matches!(
        ContentSealer::generate(ctx, &mut Fixture(0)),
        Err(CryptoError::InvalidInput)
    ));
    let mut sealer = ContentSealer::generate(context(), &mut Fixture(0)).unwrap();
    assert_eq!(
        sealer.seal_next(&vec![0; 1025]),
        Err(CryptoError::InvalidInput)
    );
}
#[cfg(feature = "os-rng")]
#[test]
fn production_entropy_key_generation_smoke() {
    let a = IdentitySigningKey::generate(&mut OsRandom).unwrap();
    let b = IdentitySigningKey::generate(&mut OsRandom).unwrap();
    assert_ne!(a.public_key(), b.public_key());
    let a = DeliverySecret::generate(&mut OsRandom).unwrap();
    let b = DeliverySecret::generate(&mut OsRandom).unwrap();
    assert_ne!(a.public_key(), b.public_key());
}
