#ifndef MESH_ENGINE_H
#define MESH_ENGINE_H
#include <stdint.h>
#include <stddef.h>
typedef struct { uint8_t *ptr; size_t len; } MeshBuffer;
uint32_t mesh_abi_version(void);
int32_t mesh_runtime_create(uint32_t version, uint64_t *out);
int32_t mesh_runtime_request(uint64_t handle, const uint8_t *input, size_t len, MeshBuffer *out);
/* Release an unmodified returned buffer exactly once, including on decode errors. */
void mesh_buffer_release(MeshBuffer buffer);
/* Canonical bounded records for native transport adapters; payload bytes stay opaque. */
int32_t mesh_link_frame_encode(const uint8_t *input, size_t input_len, MeshBuffer *out);
int32_t mesh_link_frame_decode(const uint8_t *input, size_t input_len, MeshBuffer *out);
int32_t mesh_routed_record_encode(const uint8_t *frame, size_t frame_len,
                                  const uint8_t *record, size_t record_len, MeshBuffer *out);
int32_t mesh_routed_record_decode(const uint8_t *input, size_t input_len, MeshBuffer *out);
int32_t mesh_relay_gate_open(const uint8_t *member, size_t member_len, uint64_t *out);
int32_t mesh_relay_gate_accept(uint64_t handle, const uint8_t *frame, size_t frame_len,
                               const uint8_t *via, size_t via_len, uint64_t now, MeshBuffer *out);
int32_t mesh_relay_gate_release(uint64_t handle);
int32_t mesh_runtime_release(uint64_t handle);
/* Opens/verifies SQLCipher using protected key material. No handle or secret escapes. */
int32_t mesh_secure_store_probe(const uint8_t *key, size_t key_len, const uint8_t *member,
                                size_t member_len, const uint8_t *path, size_t path_len);
int32_t mesh_secure_store_open(const uint8_t *key, size_t key_len, const uint8_t *member,
                               size_t member_len, const uint8_t *path, size_t path_len,
                               uint64_t *out);
int32_t mesh_secure_store_release(uint64_t handle);
int32_t mesh_secure_store_policy_epoch(uint64_t handle, uint64_t now, uint64_t *out);
int32_t mesh_secure_store_accept_routed(uint64_t handle, const uint8_t *input, size_t input_len,
                                        const uint8_t *received_from, size_t received_from_len,
                                        uint64_t now, uint8_t *out);
int32_t mesh_secure_store_enqueue_text(uint64_t handle, const uint8_t *identity_seed,
                                       size_t identity_seed_len, const uint8_t *plaintext,
                                       size_t plaintext_len, uint64_t now, uint16_t *out);
int32_t mesh_secure_store_enqueue_text_with_logical_id(uint64_t handle, const uint8_t *identity_seed,
                                       size_t identity_seed_len, const uint8_t *plaintext,
                                       size_t plaintext_len, const uint8_t *logical_id,
                                       size_t logical_id_len, uint64_t now, uint16_t *out);
int32_t mesh_secure_store_latest_delivery_summary(uint64_t handle, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_delivery_summary(uint64_t handle, const uint8_t *logical_id,
                                           size_t logical_id_len, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_finalize_next_text(uint64_t handle, const uint8_t *identity_seed,
                                             size_t identity_seed_len, const uint8_t *delivery_seed,
                                             size_t delivery_seed_len, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_outbox_record(uint64_t handle, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_receipt_record(uint64_t handle, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_receipt_ack_record(uint64_t handle, const uint8_t *identity_seed,
                                             size_t identity_seed_len, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_record(uint64_t handle, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_received_from(uint64_t handle, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_receipt_record(uint64_t handle, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_receipt_received_from(uint64_t handle, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_receipt_ack_record(uint64_t handle, uint16_t slot, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_store_relay_receipt_ack_received_from(uint64_t handle, uint64_t now, MeshBuffer *out);
/* Opaque native-host Noise session. Every returned MeshBuffer is released once. */
int32_t mesh_secure_session_start(uint64_t store_handle, const uint8_t *session_seed,
                                  size_t session_seed_len, const uint8_t *member,
                                  size_t member_len, uint8_t role, uint64_t now, uint64_t *out);
int32_t mesh_secure_session_write(uint64_t handle, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_session_read(uint64_t handle, const uint8_t *input, size_t input_len,
                                 uint64_t now);
int32_t mesh_secure_session_finish(uint64_t handle, uint64_t now);
int32_t mesh_secure_session_authenticate(uint64_t handle, const uint8_t *identity_seed,
                                         size_t identity_seed_len, uint64_t now, MeshBuffer *out);
int32_t mesh_secure_session_send(uint64_t handle, const uint8_t *input, size_t input_len,
                                 uint64_t now, MeshBuffer *out);
int32_t mesh_secure_session_receive(uint64_t handle, const uint8_t *input, size_t input_len,
                                    uint64_t now, MeshBuffer *out);
int32_t mesh_secure_session_authenticated(uint64_t handle, uint8_t *out);
int32_t mesh_secure_session_peer(uint64_t handle, uint8_t *out);
int32_t mesh_secure_session_release(uint64_t handle);
int32_t mesh_secure_store_create_group(uint64_t handle, const uint8_t *identity_seed,
                                       size_t identity_seed_len, const uint8_t *delivery_seed,
                                       size_t delivery_seed_len, const uint8_t *member,
                                       size_t member_len, uint64_t now, uint64_t *out);
int32_t mesh_secure_store_issue_enrollment(uint64_t handle, const uint8_t *identity_seed,
                                           size_t identity_seed_len, const uint8_t *request,
                                           size_t request_len, uint64_t now, MeshBuffer *out);
/* Validates a public enrollment request and returns only its applicant member ID. */
int32_t mesh_enrollment_request_member(const uint8_t *request, size_t request_len,
                                       uint64_t now, uint8_t *out);
int32_t mesh_secure_store_install_policy(uint64_t handle, const uint8_t *bundle,
                                         size_t bundle_len, uint64_t now, uint64_t *out);
int32_t mesh_secure_store_export_policy(uint64_t handle, uint64_t now, MeshBuffer *out);
int32_t mesh_create_enrollment_request(const uint8_t *identity_seed, size_t identity_seed_len,
                                       const uint8_t *delivery_seed, size_t delivery_seed_len,
                                       const uint8_t *invitation, size_t invitation_len,
                                       uint64_t now, MeshBuffer *out);
// Native-only key port. Output is a 32-byte public Ed25519 key, released normally.
int32_t mesh_identity_public(const uint8_t *seed, size_t len, MeshBuffer *out);
#endif
