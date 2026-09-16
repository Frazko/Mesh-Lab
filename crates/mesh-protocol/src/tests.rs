use super::*;
use zeroize::Zeroizing;
struct Rng(u64);
impl RandomSource for Rng {
    fn fill(&mut self, out: &mut [u8]) -> crypto::Result<()> {
        for b in out {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = self.0 as u8;
        }
        Ok(())
    }
}
struct Lab {
    root: IdentitySigningKey,
    keys: Vec<IdentitySigningKey>,
    secrets: Vec<DeliverySecret>,
    claims: Vec<CertificateClaims>,
    certs: Vec<Vec<u8>>,
    roster: VerifiedRoster,
}
impl Lab {
    fn new() -> Self {
        let root = IdentitySigningKey::import(Zeroizing::new([42; 32]));
        let keys: Vec<_> = (1..=3)
            .map(|i| IdentitySigningKey::import(Zeroizing::new([i; 32])))
            .collect();
        let secrets: Vec<_> = (4..=6)
            .map(|i| DeliverySecret::import(Zeroizing::new([i; 32])).unwrap())
            .collect();
        let claims: Vec<_> = (0..3)
            .map(|i| CertificateClaims {
                group: [7; 32],
                member: MemberId([i as u8 + 1; 32]),
                signing_key: keys[i].public_key(),
                delivery_key: secrets[i].public_key(),
                valid_from: 1,
                valid_until: 1000,
                epoch: 1,
                serial: i as u64 + 1,
            })
            .collect();
        let certs: Vec<_> = claims
            .iter()
            .map(|c| issue_certificate(&root, c).unwrap())
            .collect();
        let roster = VerifiedRoster::verify(
            root.public_key(),
            Scope {
                group: [7; 32],
                epoch: 1,
            },
            &certs,
            &[],
            1,
        )
        .unwrap();
        Self {
            root,
            keys,
            secrets,
            claims,
            certs,
            roster,
        }
    }
    fn policy(&self) -> ObjectPolicy {
        ObjectPolicy {
            namespace: Namespace::new("mesh.lab.text").unwrap(),
            epoch: 1,
            targets: vec![MemberId([2; 32])],
            expires_at: 100,
            hop_limit: 4,
        }
    }
    fn seal(&self, plain: &[u8]) -> SealedMessage {
        seal_message(
            SealRequest {
                origin: MemberId([1; 32]),
                sequence: 1,
                policy: self.policy(),
                plaintext: plain,
                now: 1,
            },
            &self.roster,
            &self.keys[0],
            &mut Rng(12345),
        )
        .unwrap()
    }
    fn announce(&self, object: &PreparedObject) -> AuthenticatedAnnouncement {
        let mut b = Writer::default();
        b.array(3);
        b.uint(1);
        b.bytes(&self.roster.scope.group);
        b.bytes(&object.manifest().encode());
        let b = b.finish();
        authenticate_announcement(
            &signed(
                &b,
                self.keys[0]
                    .sign(self.roster.scope, Domain::Object, &b)
                    .unwrap(),
            ),
            &self.roster,
            2,
        )
        .unwrap()
    }
    fn received(
        &self,
        m: &SealedMessage,
        secret: usize,
        member: u8,
        now: u64,
    ) -> Result<VerifiedDelivery> {
        let a = authenticate_announcement(m.announcement().bytes(), &self.roster, now)?;
        verify_delivery(
            a,
            &m.object().chunks().concat(),
            &self.roster,
            MemberId([member; 32]),
            &self.secrets[secret],
            now,
        )
    }
}
#[test]
fn enrollment_request_is_bounded_signed_and_short_lived() {
    let lab = Lab::new();
    let scope = lab.roster.scope();
    let request = create_enrollment_request(
        &lab.keys[1],
        &lab.secrets[1],
        scope,
        10,
        20,
        &mut Rng(20260911),
    )
    .unwrap();
    let verified = verify_enrollment_request(&request, 11).unwrap();
    assert_eq!(verified.scope, scope);
    assert_eq!(verified.member, MemberId(lab.keys[1].public_key()));
    assert_eq!(verified.delivery_key, lab.secrets[1].public_key());
    assert!(verify_enrollment_request(&request, 20).is_err());
    let mut tampered = request;
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(verify_enrollment_request(&tampered, 11).is_err());
}
#[test]
fn signed_presence_binds_member_scope_version_and_expiry() {
    let lab = Lab::new();
    let record = PresenceRecord {
        member: MemberId([1; 32]),
        incarnation: 4,
        sequence: 9,
        expires_at: 20,
    };
    let encoded = sign_presence(&lab.keys[0], record, &lab.roster, 10).unwrap();
    assert_eq!(
        authenticate_presence(&encoded, &lab.roster, 19).unwrap(),
        record
    );
    assert!(authenticate_presence(&encoded, &lab.roster, 20).is_err());
    assert!(sign_presence(&lab.keys[1], record, &lab.roster, 10).is_err());
    let mut tampered = encoded;
    *tampered.last_mut().unwrap() ^= 1;
    assert!(authenticate_presence(&tampered, &lab.roster, 19).is_err());
}
#[test]
fn policy_bundle_is_canonical_and_rejects_trailing_or_oversized_entries() {
    let lab = Lab::new();
    let bundle = PolicyBundle {
        authority: lab.root.public_key(),
        scope: lab.roster.scope(),
        certificates: lab.certs.clone(),
        revoked: vec![],
    };
    let encoded = bundle.encode().unwrap();
    assert_eq!(PolicyBundle::decode(&encoded).unwrap(), bundle);
    let mut trailing = encoded;
    trailing.push(0);
    assert!(PolicyBundle::decode(&trailing).is_err());
    assert!(PolicyBundle {
        certificates: vec![vec![0; 1025]],
        ..bundle
    }
    .encode()
    .is_err());
}
#[test]
fn field_roster_accepts_fifty_members_but_not_fifty_one() {
    let authority = IdentitySigningKey::import(Zeroizing::new([91; 32]));
    let scope = Scope {
        group: [92; 32],
        epoch: 1,
    };
    let certificates: Vec<_> = (1..=MAX_GROUP_MEMBERS)
        .map(|index| {
            let signing = IdentitySigningKey::import(Zeroizing::new([index as u8; 32]));
            let delivery =
                DeliverySecret::import(Zeroizing::new([(index + 100) as u8; 32])).unwrap();
            issue_certificate(
                &authority,
                &CertificateClaims {
                    group: scope.group,
                    member: MemberId(signing.public_key()),
                    signing_key: signing.public_key(),
                    delivery_key: delivery.public_key(),
                    valid_from: 1,
                    valid_until: 100,
                    epoch: scope.epoch,
                    serial: index as u64,
                },
            )
            .unwrap()
        })
        .collect();
    let roster =
        VerifiedRoster::verify(authority.public_key(), scope, &certificates, &[], 1).unwrap();
    assert_eq!(roster.member_claims().len(), MAX_GROUP_MEMBERS);
    let bundle = PolicyBundle {
        authority: authority.public_key(),
        scope,
        certificates: certificates.clone(),
        revoked: vec![],
    };
    let encoded = bundle.encode().unwrap();
    assert!(encoded.len() <= MAX_POLICY_BUNDLE);
    assert_eq!(PolicyBundle::decode(&encoded).unwrap(), bundle);

    let mut too_many = certificates;
    too_many.push(too_many[0].clone());
    assert!(matches!(
        VerifiedRoster::verify(authority.public_key(), scope, &too_many, &[], 1),
        Err(DurableError::InvalidInput)
    ));
}
#[test]
fn pinned_authority_epoch_group_validity_revocation_and_duplicate_identities_are_enforced() {
    let l = Lab::new();
    let s = l.roster.scope;
    let root = l.root.public_key();
    for (r, scope, revoked, now) in [
        (l.keys[0].public_key(), s, vec![], 2),
        (
            root,
            Scope {
                group: [8; 32],
                ..s
            },
            vec![],
            2,
        ),
        (root, Scope { epoch: 2, ..s }, vec![], 2),
        (root, s, vec![2], 2),
        (root, s, vec![], 0),
        (root, s, vec![], 1000),
    ] {
        assert!(VerifiedRoster::verify(r, scope, &l.certs, &revoked, now).is_err());
    }
    let mut corrupted = l.certs.clone();
    *corrupted[0].last_mut().unwrap() ^= 1;
    assert!(VerifiedRoster::verify(root, s, &corrupted, &[], 2).is_err());
    for field in 0..4 {
        let mut c = l.claims[2].clone();
        match field {
            0 => c.member = l.claims[0].member,
            1 => c.serial = l.claims[0].serial,
            2 => c.signing_key = l.claims[0].signing_key,
            _ => c.delivery_key = l.claims[0].delivery_key,
        };
        let mut certs = l.certs.clone();
        certs[2] = issue_certificate(&l.root, &c).unwrap();
        assert!(matches!(
            VerifiedRoster::verify(root, s, &certs, &[], 2),
            Err(DurableError::Conflict)
        ));
    }
    let reverse: Vec<_> = l.certs.iter().rev().cloned().collect();
    assert_eq!(
        VerifiedRoster::verify(root, s, &reverse, &[], 2)
            .unwrap()
            .digest(),
        l.roster.digest()
    );
}
#[test]
fn strict_cbor_rejects_truncation_nonminimal_lengths_indefinite_and_trailing_data() {
    let l = Lab::new();
    let m = l.seal(b"message");
    let a = m.announcement().bytes();
    for n in 0..a.len() {
        assert!(authenticate_announcement(&a[..n], &l.roster, 2).is_err());
    }
    let mut trailing = a.to_vec();
    trailing.push(0);
    assert!(authenticate_announcement(&trailing, &l.roster, 2).is_err());
    let mut nonminimal = vec![0x98, 2];
    nonminimal.extend(&a[1..]);
    assert!(authenticate_announcement(&nonminimal, &l.roster, 2).is_err());
    for bad in [
        vec![0x9f, 0xff],
        vec![0x81; MAX_ANNOUNCEMENT + 1],
        vec![0x82, 0x5b, 255, 255, 255, 255, 255, 255, 255, 255],
    ] {
        assert!(authenticate_announcement(&bad, &l.roster, 2).is_err());
    }
    // A correctly signed body still must be canonical.
    let (b, _) = signed_parts(a, MAX_ANNOUNCEMENT).unwrap();
    let mut bad = b.to_vec();
    bad.splice(1..2, [0x18, 1]);
    let sig = l.keys[0]
        .sign(l.roster.scope, Domain::Object, &bad)
        .unwrap();
    assert!(authenticate_announcement(&signed(&bad, sig), &l.roster, 2).is_err());
    for n in 0..l.certs[0].len() {
        assert!(VerifiedRoster::verify(
            l.root.public_key(),
            l.roster.scope,
            &[l.certs[0][..n].to_vec()],
            &[],
            2
        )
        .is_err());
    }
}
#[test]
fn size_boundaries_all_chunks_and_recipient_keys_are_checked() {
    let l = Lab::new();
    let m = l.seal(&vec![88; MAX_PLAINTEXT]);
    assert!(m.object().manifest().content_len() <= MAX_OBJECT_BYTES);
    assert!(l.received(&m, 1, 2, 2).is_ok());
    assert!(l.received(&m, 2, 2, 2).is_err());
    assert!(l.received(&m, 0, 1, 2).is_err());
    assert!(l.received(&m, 1, 2, 100).is_err());
    assert!(l.received(&m, 1, 2, 0).is_err());
    for size in [0, MAX_PLAINTEXT + 1] {
        assert!(seal_message(
            SealRequest {
                origin: MemberId([1; 32]),
                sequence: 1,
                policy: l.policy(),
                plaintext: &vec![0; size],
                now: 1
            },
            &l.roster,
            &l.keys[0],
            &mut Rng(123)
        )
        .is_err());
    }
    // Valid origin signature + valid outer chunk hashes do not bypass AEAD.
    let mut bytes = m.object().chunks().concat();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = PreparedObject::from_opaque(MemberId([1; 32]), 1, l.policy(), &bytes).unwrap();
    let a = l.announce(&bad);
    assert!(verify_delivery(a, &bytes, &l.roster, MemberId([2; 32]), &l.secrets[1], 2).is_err());
}
#[test]
fn envelope_header_cannot_disagree_with_the_signed_storage_manifest() {
    let l = Lab::new();
    let m = l.seal(b"message");
    let bytes = m.object().chunks().concat();
    let mut policy = l.policy();
    policy.namespace = Namespace::new("mesh.lab.voice").unwrap();
    let bad = PreparedObject::from_opaque(MemberId([1; 32]), 1, policy, &bytes).unwrap();
    let a = l.announce(&bad);
    assert!(matches!(
        verify_delivery(a, &bytes, &l.roster, MemberId([2; 32]), &l.secrets[1], 2),
        Err(DurableError::AuthenticationFailed)
    ));
    // Every header prefix fails even when the origin re-signs the containing blob.
    let mut r = Reader::new(&bytes, MAX_OBJECT_BYTES).unwrap();
    r.array(5).unwrap();
    r.uint().unwrap();
    let header = r.bytes(2048).unwrap();
    for n in 0..header.len() {
        assert!(Header::decode(&header[..n]).is_err());
    }
}
#[test]
fn receipt_scope_actor_stage_time_and_signature_are_bound_to_the_original_object() {
    let l = Lab::new();
    let m = l.seal(b"message");
    let bytes = m.object().chunks().concat();
    let proof = l.received(&m, 1, 2, 2).unwrap();
    let receipt = proof.receipt_for_commit(&l.keys[1], 3).unwrap();
    assert!(verify_receipt(&receipt, m.announcement(), &bytes, &l.roster, 4).is_ok());
    assert!(verify_receipt(&receipt, m.announcement(), &bytes, &l.roster, 2).is_err());
    let header = envelope(m.announcement(), &bytes, &l.roster)
        .unwrap()
        .header;
    let manifest = m.object().manifest();
    for variant in 0..8 {
        let body = receipt_body(
            if variant == 0 {
                Scope {
                    group: [8; 32],
                    ..l.roster.scope
                }
            } else {
                l.roster.scope
            },
            if variant == 1 {
                [0; 32]
            } else {
                header.context_id()
            },
            if variant == 2 {
                ObjectId([0; 32])
            } else {
                manifest.id()
            },
            manifest.origin(),
            if variant == 3 { 2 } else { manifest.sequence() },
            if variant == 4 {
                MemberId([3; 32])
            } else {
                MemberId([2; 32])
            },
            if variant == 5 { 100 } else { 3 },
        );
        let mut body = body;
        if variant == 6 {
            body[2] = 2;
        }
        let sig = l.keys[if variant == 4 { 2 } else { 1 }]
            .sign(
                l.roster.scope,
                if variant == 7 {
                    Domain::Object
                } else {
                    Domain::Receipt
                },
                &body,
            )
            .unwrap();
        assert!(
            verify_receipt(
                &signed(&body, sig),
                m.announcement(),
                &bytes,
                &l.roster,
                101
            )
            .is_err(),
            "variant {variant}"
        );
    }
    for n in 0..receipt.len() {
        assert!(verify_receipt(&receipt[..n], m.announcement(), &bytes, &l.roster, 4).is_err());
    }
}
#[test]
fn canonical_header_context_and_receipt_match_independent_vectors() {
    fn hex(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    let header = Header {
        scope: Scope {
            group: [7; 32],
            epoch: 1,
        },
        origin: MemberId([1; 32]),
        sequence: 24,
        namespace: Namespace::new("mesh.lab.text").unwrap(),
        created: 1,
        expires: 100,
        hops: 4,
        targets: vec![(MemberId([2; 32]), [8; 32])],
        clear_len: 1025,
        salt: [9; 32],
    };
    assert_eq!(
        header.encode(),
        hex(include_str!("../../../vectors/protocol/header-v1.hex"))
    );
    assert_eq!(
        header.context_id().to_vec(),
        hex(include_str!("../../../vectors/protocol/context-id-v1.hex"))
    );
    assert_eq!(
        receipt_body(
            header.scope,
            header.context_id(),
            ObjectId([6; 32]),
            header.origin,
            24,
            MemberId([2; 32]),
            25
        ),
        hex(include_str!(
            "../../../vectors/protocol/receipt-body-v1.hex"
        ))
    );
}

#[test]
fn durable_records_are_canonical_and_bounded_before_store_ingestion() {
    let records = [
        DurableRecord::Announcement(vec![1, 2, 3]),
        DurableRecord::Chunk {
            object: ObjectId([9; 32]),
            index: 63,
            bytes: vec![4; CHUNK_BYTES],
        },
        DurableRecord::Receipt(vec![5, 6]),
    ];
    for record in records {
        let encoded = record.encode().unwrap();
        assert_eq!(DurableRecord::decode(&encoded), Ok(record));
        assert!(DurableRecord::decode(&encoded[..encoded.len() - 1]).is_err());
        let mut extended = encoded.clone();
        extended.push(0);
        assert!(DurableRecord::decode(&extended).is_err());
    }
    assert!(DurableRecord::Announcement(vec![]).encode().is_err());
    assert!(DurableRecord::Receipt(vec![0; MAX_RECEIPT + 1])
        .encode()
        .is_err());
    assert!(DurableRecord::Chunk {
        object: ObjectId([1; 32]),
        index: 0,
        bytes: vec![0; CHUNK_BYTES + 1]
    }
    .encode()
    .is_err());
    assert!(matches!(
        DurableRecord::decode(&[DURABLE_RECORD_MAGIC, DURABLE_RECORD_VERSION, 9]),
        Err(DurableError::UnsupportedSchema)
    ));
}
