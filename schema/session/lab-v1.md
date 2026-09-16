# Mesh Lab session profile v1

This fixed profile uses `Noise_XX_25519_ChaChaPoly_SHA256` through Snow 0.10.0.
It does not claim the complete A002/A003 contract. IK, discovery, invitation and
transport fragmentation/retransmission are not implemented here.

Canonical CBOR prologue, bound by Noise:
`["MeshLab/Session/v1", 1, group:bstr32, epoch:uint, capabilities:1, maxPayload:4096]`.
The exact suite is the Noise protocol name. No alternate suite or negotiation fallback.
The three Noise handshake messages have **empty payloads** and sizes 32, 96, 64.
Inbound handshakes are bounded to 96 bytes. Fresh CSPRNG ephemeral per attempt;
no serialization/resume of a handshake. Wrong-phase local calls fail without
advancing. Invalid received handshake or 30-second deadline consumes the state.

After XX, the Noise handshake hash (32 bytes) becomes the session ID. No application
traffic is accepted until both local AUTH has been produced and remote AUTH verified.
Each direction uses nonce 0 once for AUTH. Retransmission reuses the already-produced
bytes. Identity proof is Ed25519 over the existing Crypto/v1 prefix with Domain::Session
(purpose 6), the scope, and canonical body:
`[1, group:bstr32, epoch:uint, handshakeHash:bstr32, member:bstr32, role:0/1, 1, 4096]`.
The hash binds the Noise static and ephemeral keys and both sides of the transcript;
role is 0 for initiator and 1 for responder. Signature public key must come from
the pinned-authority verified roster. Expected peer, when supplied, must match.
AUTH plaintext is `0x00 || CBOR([member:bstr32, signature:bstr64])`.

Transport frame: `version:u8=1 || sessionId:32 || packetNumber:u64BE || ciphertext`.
Snow's stateless transport encrypts with `nonce=packetNumber` using separate direction
keys. Nonce is authenticated through the cipher operation; session ID must match
before decrypting. Authenticated data plaintext is `0x01 || applicationBytes` (1..4096).
Frame limit 4154 bytes. Version, length, session, phase and packet window are checked
before delivery. Replay bitmap is 64 packets per direction; advancement occurs only
after successful decryption and semantic verification. Out-of-order unseen packets
inside the window are accepted. Invalid ciphertext cannot push the window forward.

Data packet numbers start at 1 and stay below 2^20. Sender consumes the number before
encrypting and closes at the limit; no external nonce setter, rewind or session clone.
Reconnect currently performs a **new XX** with fresh entropy and a new packet space.
Reusing an old frame under the new session fails. Duplicate AUTH/data are replay errors;
a host that needs loss recovery retains encrypted bytes, not a rewindable crypto state.

The trusted host injects seconds and a current verified roster on AUTH/data operations.
Changes to local roster/revocations or expiry invalidate an existing session. A failed
identity proof closes the session; unauthenticated datagrams are rejected without
advancing replay state. This module provides no peer rate limiter, scheduler or radio.

Secret wrappers are not Clone/Debug/Serialize. Owned DH/cipher keys are cleared via
provider wrappers on drop; entropy/plaintext use Zeroizing. Snow internal HKDF state,
compiler/OS copies and complete memory behavior have not been audited. No claim of
full zeroization, hardware-only key use or release security certification is made.
