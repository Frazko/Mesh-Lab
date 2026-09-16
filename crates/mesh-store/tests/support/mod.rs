use mesh_crypto::{DeliverySecret, IdentitySigningKey, RandomSource, Scope};
use mesh_object::ObjectPolicy;
use mesh_protocol::{self as protocol, CertificateClaims, SealedMessage, VerifiedRoster};
use mesh_types::durable::*;
use zeroize::Zeroizing;
// Reproducible PUBLIC fixtures, confined to integration tests.
pub struct TestRandom(u64);
impl TestRandom {
    pub fn new() -> Self {
        Self(20260911)
    }
}
impl RandomSource for TestRandom {
    fn fill(&mut self, out: &mut [u8]) -> mesh_crypto::Result<()> {
        for b in out {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = self.0 as u8;
        }
        Ok(())
    }
}
pub struct Lab {
    pub authority: IdentitySigningKey,
    pub signers: Vec<IdentitySigningKey>,
    pub secrets: Vec<DeliverySecret>,
    pub certificates: Vec<Vec<u8>>,
    pub roster: VerifiedRoster,
}
impl Lab {
    pub fn new() -> Self {
        let authority = IdentitySigningKey::import(Zeroizing::new([42; 32]));
        let signers: Vec<_> = (0..3)
            .map(|i| IdentitySigningKey::import(Zeroizing::new([i + 3; 32])))
            .collect();
        let secrets: Vec<_> = (0..3)
            .map(|i| DeliverySecret::import(Zeroizing::new([i + 23; 32])).unwrap())
            .collect();
        let scope = Scope {
            group: [7; 32],
            epoch: 1,
        };
        let certificates: Vec<_> = (0..3)
            .map(|i| {
                protocol::issue_certificate(
                    &authority,
                    &CertificateClaims {
                        group: scope.group,
                        member: MemberId([i as u8 + 1; 32]),
                        signing_key: signers[i].public_key(),
                        delivery_key: secrets[i].public_key(),
                        valid_from: 0,
                        valid_until: 1000,
                        epoch: 1,
                        serial: i as u64 + 1,
                    },
                )
                .unwrap()
            })
            .collect();
        let roster =
            VerifiedRoster::verify(authority.public_key(), scope, &certificates, &[], 1).unwrap();
        Self {
            authority,
            signers,
            secrets,
            certificates,
            roster,
        }
    }
    pub fn policy(&self, targets: &[u8]) -> ObjectPolicy {
        ObjectPolicy {
            namespace: Namespace::new("mesh.lab.text").unwrap(),
            epoch: 1,
            targets: targets.iter().map(|id| MemberId([*id; 32])).collect(),
            expires_at: 100,
            hop_limit: 4,
        }
    }
    pub fn message(&self, targets: &[u8]) -> SealedMessage {
        protocol::seal_message(
            protocol::SealRequest {
                origin: MemberId([1; 32]),
                sequence: 1,
                policy: self.policy(targets),
                plaintext: &vec![77; 2500],
                now: 1,
            },
            &self.roster,
            &self.signers[0],
            &mut TestRandom::new(),
        )
        .unwrap()
    }
}
