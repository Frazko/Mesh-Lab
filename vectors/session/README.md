# Noise XX known-answer vector

`noise-xx-cacophony.json` is the single XX/25519/ChaChaPoly/SHA256 case extracted
unchanged from Snow 0.10.0's `tests/vectors/cacophony.txt` (Apache-2.0 OR MIT package).
Source: https://docs.rs/crate/snow/0.10.0/source/tests/vectors/cacophony.txt
Original vector collection: Cacophony; see the attribution in Snow's vector tests.

The Rust test uses the production resolver, consumes the published ephemeral
samples through the one-shot entropy adapter, and checks all three handshake
messages, final handshake hash and three transport messages byte for byte.
This tests Noise itself; lab certificate confirmation and replay framing are tested
separately. All keys here are public test fixtures and never used by native apps.
