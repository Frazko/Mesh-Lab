//! Inject one fresh 32-byte CSPRNG sample for the sole XX ephemeral key. No
//! fallback RNG. Default primitives are wrapped to clear owned keys on drop and
//! reject all-zero X25519 shared secrets (including low-order public keys).
use snow::{
    params::{CipherChoice, DHChoice, HashChoice},
    resolvers::{CryptoResolver, DefaultResolver},
    types::{Cipher, Dh, Hash, Random},
};
use std::sync::Mutex;
use zeroize::Zeroizing;
pub(super) struct Resolver(pub Mutex<Option<Zeroizing<[u8; 32]>>>);
struct OnceRandom(Option<Zeroizing<[u8; 32]>>);
impl Random for OnceRandom {
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), snow::Error> {
        if out.len() != 32 {
            return Err(snow::Error::Rng);
        }
        let bytes = self.0.take().ok_or(snow::Error::Rng)?;
        out.copy_from_slice(&bytes[..]);
        Ok(())
    }
}
struct ClearingDh(Box<dyn Dh>);
impl Drop for ClearingDh {
    fn drop(&mut self) {
        self.0.set(&[0; 32]);
    }
}
impl Dh for ClearingDh {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn pub_len(&self) -> usize {
        32
    }
    fn priv_len(&self) -> usize {
        32
    }
    fn set(&mut self, b: &[u8]) {
        self.0.set(b)
    }
    fn generate(&mut self, r: &mut dyn Random) -> Result<(), snow::Error> {
        self.0.generate(r)
    }
    fn pubkey(&self) -> &[u8] {
        self.0.pubkey()
    }
    fn privkey(&self) -> &[u8] {
        self.0.privkey()
    }
    fn dh(&self, key: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        self.0.dh(key, out)?;
        if out[..32].iter().all(|b| *b == 0) {
            return Err(snow::Error::Dh);
        }
        Ok(())
    }
}
struct ClearingCipher(Box<dyn Cipher>);
impl Drop for ClearingCipher {
    fn drop(&mut self) {
        self.0.set(&[0; 32]);
    }
}
impl Cipher for ClearingCipher {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn set(&mut self, k: &[u8; 32]) {
        self.0.set(k)
    }
    fn encrypt(&self, n: u64, a: &[u8], p: &[u8], out: &mut [u8]) -> usize {
        self.0.encrypt(n, a, p, out)
    }
    fn decrypt(&self, n: u64, a: &[u8], c: &[u8], out: &mut [u8]) -> Result<usize, snow::Error> {
        self.0.decrypt(n, a, c, out)
    }
}
impl CryptoResolver for Resolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(Box::new(OnceRandom(self.0.lock().ok()?.take())))
    }
    fn resolve_dh(&self, c: &DHChoice) -> Option<Box<dyn Dh>> {
        if *c != DHChoice::Curve25519 {
            return None;
        }
        Some(Box::new(ClearingDh(DefaultResolver.resolve_dh(c)?)))
    }
    fn resolve_hash(&self, c: &HashChoice) -> Option<Box<dyn Hash>> {
        if *c != HashChoice::SHA256 {
            return None;
        }
        DefaultResolver.resolve_hash(c)
    }
    fn resolve_cipher(&self, c: &CipherChoice) -> Option<Box<dyn Cipher>> {
        if *c != CipherChoice::ChaChaPoly {
            return None;
        }
        Some(Box::new(ClearingCipher(DefaultResolver.resolve_cipher(c)?)))
    }
}
