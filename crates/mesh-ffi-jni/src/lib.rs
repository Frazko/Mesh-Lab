//! JNI exports only; protocol and runtime state stay in mesh-runtime.
use jni::{
    objects::{JByteArray, JClass, JString},
    sys::{jboolean, jbyteArray, jint, jlong},
    JNIEnv,
};
fn failure(env: &mut JNIEnv, code: i32) {
    let _ = env.throw_new("java/lang/IllegalStateException", format!("MESH_{code}"));
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_abiVersion(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    1
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_create(
    mut env: JNIEnv,
    _class: JClass,
    version: jint,
) -> jlong {
    match mesh_ffi_c::create(version as u32) {
        Ok(id) => id as jlong,
        Err(e) => {
            failure(&mut env, e as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_request(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    input: JByteArray,
) -> jbyteArray {
    // Reject before JNI copies/allocates the input. No Java exceptions cross into Rust.
    let result = mesh_ffi_c::guarded(|| {
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=64).contains(&len) {
            return Err(mesh_types_error());
        }
        let bytes = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::request(handle as u64, &bytes)
    });
    match result {
        Ok(bytes) => match env.byte_array_from_slice(&bytes) {
            Ok(array) => array.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(e) => {
            failure(&mut env, e as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_linkFrameEncode(
    env: JNIEnv,
    _class: JClass,
    input: JByteArray,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=4160).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::link_frame_encode(&input)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_linkFrameDecode(
    env: JNIEnv,
    _class: JClass,
    input: JByteArray,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(3..=4163).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::link_frame_decode(&input)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_routedRecordEncode(
    env: JNIEnv,
    _class: JClass,
    frame: JByteArray,
    record: JByteArray,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if env
            .get_array_length(&frame)
            .map_err(|_| mesh_types_error())?
            != 91
        {
            return Err(mesh_types_error());
        }
        let record_len = env
            .get_array_length(&record)
            .map_err(|_| mesh_types_error())?;
        if !(1..=4003).contains(&record_len) {
            return Err(mesh_types_error());
        }
        let frame = env
            .convert_byte_array(&frame)
            .map_err(|_| mesh_types_error())?;
        let record = env
            .convert_byte_array(&record)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::routed_record_encode(&frame, &record)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_routedRecordDecode(
    env: JNIEnv,
    _class: JClass,
    input: JByteArray,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=4096).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::routed_record_decode(&input)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_relayGateOpen(
    mut env: JNIEnv,
    _class: JClass,
    member: JByteArray,
) -> jlong {
    let result = mesh_ffi_c::guarded(|| {
        if env
            .get_array_length(&member)
            .map_err(|_| mesh_types_error())?
            != 32
        {
            return Err(mesh_types_error());
        }
        let member = env
            .convert_byte_array(&member)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::relay_gate_open(&member)
    });
    match result {
        Ok(handle) => handle as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_relayGateAccept(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    frame: JByteArray,
    via: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&frame)
                .map_err(|_| mesh_types_error())?
                != 91
            || env.get_array_length(&via).map_err(|_| mesh_types_error())? != 32
        {
            return Err(mesh_types_error());
        }
        let frame = env
            .convert_byte_array(&frame)
            .map_err(|_| mesh_types_error())?;
        let via = env
            .convert_byte_array(&via)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::relay_gate_accept(handle as u64, &frame, &via, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_relayGateRelease(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if let Err(error) = mesh_ffi_c::relay_gate_release(handle as u64) {
        failure(&mut env, error as i32);
    }
}
fn mesh_types_error() -> mesh_types::Error {
    mesh_types::Error::InvalidArgument
}
fn output_bytes(mut env: JNIEnv, result: Result<Vec<u8>, mesh_types::Error>) -> jbyteArray {
    match result {
        Ok(bytes) => env
            .byte_array_from_slice(&bytes)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreProbe(
    mut env: JNIEnv,
    _class: JClass,
    key: JByteArray,
    member: JByteArray,
    path: JString,
) {
    let result = mesh_ffi_c::guarded(|| {
        if env.get_array_length(&key).map_err(|_| mesh_types_error())? != 32
            || env
                .get_array_length(&member)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let key = zeroize::Zeroizing::new(
            env.convert_byte_array(&key)
                .map_err(|_| mesh_types_error())?,
        );
        let member = env
            .convert_byte_array(&member)
            .map_err(|_| mesh_types_error())?;
        let path = env.get_string(&path).map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_probe(
            &key,
            &member,
            path.to_str().map_err(|_| mesh_types_error())?,
        )
    });
    if let Err(error) = result {
        failure(&mut env, error as i32);
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreOpen(
    mut env: JNIEnv,
    _class: JClass,
    key: JByteArray,
    member: JByteArray,
    path: JString,
) -> jlong {
    let result = mesh_ffi_c::guarded(|| {
        if env.get_array_length(&key).map_err(|_| mesh_types_error())? != 32
            || env
                .get_array_length(&member)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let key = zeroize::Zeroizing::new(
            env.convert_byte_array(&key)
                .map_err(|_| mesh_types_error())?,
        );
        let member = env
            .convert_byte_array(&member)
            .map_err(|_| mesh_types_error())?;
        let path = env.get_string(&path).map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_open(
            &key,
            &member,
            path.to_str().map_err(|_| mesh_types_error())?,
        )
    });
    match result {
        Ok(handle) => handle as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelease(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if let Err(error) = mesh_ffi_c::secure_store_release(handle as u64) {
        failure(&mut env, error as i32);
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStorePolicyEpoch(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jlong {
    match mesh_ffi_c::secure_store_policy_epoch(handle as u64, now as u64) {
        Ok(epoch) => epoch as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreDiscoveryTag(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    output_bytes(
        env,
        mesh_ffi_c::secure_store_discovery_tag(handle as u64, now as u64).map(|tag| tag.to_vec()),
    )
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreAwareNeighbors(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    output_bytes(
        env,
        mesh_ffi_c::secure_store_aware_neighbors(handle as u64, now as u64),
    )
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreAcceptRouted(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    input: JByteArray,
    received_from: JByteArray,
    now: jlong,
) -> jint {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=4096).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        let received_from = env
            .convert_byte_array(&received_from)
            .map_err(|_| mesh_types_error())?;
        if received_from.len() != 32 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_accept_routed(handle as u64, &input, &received_from, now as u64)
    });
    match result {
        Ok(status) => status as jint,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreEnqueueText(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    plaintext: JByteArray,
    now: jlong,
) -> jint {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let length = env
            .get_array_length(&plaintext)
            .map_err(|_| mesh_types_error())?;
        if !(1..=48 * 1024).contains(&length) {
            return Err(mesh_types_error());
        }
        let identity_seed = env
            .convert_byte_array(&identity_seed)
            .map_err(|_| mesh_types_error())?;
        let plaintext = env
            .convert_byte_array(&plaintext)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_enqueue_text(handle as u64, &identity_seed, &plaintext, now as u64)
    });
    match result {
        Ok(count) => count as jint,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreEnqueueTextWithLogicalId(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    plaintext: JByteArray,
    logical_id: JByteArray,
    now: jlong,
) -> jint {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&logical_id)
                .map_err(|_| mesh_types_error())?
                != 16
        {
            return Err(mesh_types_error());
        }
        let length = env
            .get_array_length(&plaintext)
            .map_err(|_| mesh_types_error())?;
        if !(1..=48 * 1024).contains(&length) {
            return Err(mesh_types_error());
        }
        let identity_seed = env
            .convert_byte_array(&identity_seed)
            .map_err(|_| mesh_types_error())?;
        let plaintext = env
            .convert_byte_array(&plaintext)
            .map_err(|_| mesh_types_error())?;
        let logical_id: [u8; 16] = env
            .convert_byte_array(&logical_id)
            .map_err(|_| mesh_types_error())?
            .try_into()
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_enqueue_text_with_logical_id(
            handle as u64,
            &identity_seed,
            &plaintext,
            logical_id,
            now as u64,
        )
    });
    match result {
        Ok(count) => count as jint,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreLatestDeliverySummary(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_latest_delivery_summary(handle as u64, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreDeliverySummary(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    logical_id: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let logical_id = env
            .convert_byte_array(&logical_id)
            .map_err(|_| mesh_types_error())?;
        let logical_id: [u8; 16] = logical_id.try_into().map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_delivery_summary(handle as u64, logical_id, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreOutboxRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 || !(0..=u16::MAX as jint).contains(&slot) {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_outbox_record(handle as u64, slot as u16, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreReceiptRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 || !(0..=u16::MAX as jint).contains(&slot) {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_receipt_record(handle as u64, slot as u16, now as u64)
    });
    output_bytes(env, result)
}
/// Completes one locally addressed durable text only after the encrypted store
/// transaction has committed its signed receipt. Output stays inside Kotlin.
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreFinalizeNextText(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    delivery_seed: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&delivery_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let identity_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let delivery_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&delivery_seed)
                .map_err(|_| mesh_types_error())?,
        );
        mesh_ffi_c::secure_store_finalize_next_text(
            handle as u64,
            &identity_seed,
            &delivery_seed,
            now as u64,
        )
    });
    output_bytes(env, result)
}
/// Builds one origin-signed receipt acknowledgement. The KeyStore seed is
/// converted only for this native signing call and drops immediately after.
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreReceiptAckRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || !(0..=u16::MAX as jint).contains(&slot)
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let identity_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        mesh_ffi_c::secure_store_receipt_ack_record(
            handle as u64,
            &identity_seed,
            slot as u16,
            now as u64,
        )
    });
    output_bytes(env, result)
}
/// Reads one canonical durable record that is eligible for relay. A zero-length
/// array is an ordinary empty queue result, not an error or a delivery claim.
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 || !(0..=u16::MAX as jint).contains(&slot) {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_record(handle as u64, slot as u16, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayReceivedFrom(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_received_from(handle as u64, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayReceiptRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 || !(0..=u16::MAX as jint).contains(&slot) {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_receipt_record(handle as u64, slot as u16, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayReceiptReceivedFrom(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_receipt_received_from(handle as u64, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayReceiptAckRecord(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    slot: jint,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 || !(0..=u16::MAX as jint).contains(&slot) {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_receipt_ack_record(handle as u64, slot as u16, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreRelayReceiptAckReceivedFrom(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_relay_receipt_ack_received_from(handle as u64, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionStart(
    mut env: JNIEnv,
    _class: JClass,
    store_handle: jlong,
    session_seed: JByteArray,
    member: JByteArray,
    role: jint,
    now: jlong,
) -> jlong {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || !(0..=1).contains(&role)
            || env
                .get_array_length(&session_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&member)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&session_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let member = env
            .convert_byte_array(&member)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_session_start(
            store_handle as u64,
            &seed,
            &member,
            role as u8,
            now as u64,
        )
    });
    match result {
        Ok(handle) => handle as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionWrite(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_session_write(handle as u64, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionRead(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    input: JByteArray,
    now: jlong,
) {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=96).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_session_read(handle as u64, &input, now as u64)
    });
    if let Err(error) = result {
        failure(&mut env, error as i32);
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionFinish(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) {
    let result = if now <= 0 {
        Err(mesh_types_error())
    } else {
        mesh_ffi_c::secure_session_finish(handle as u64, now as u64)
    };
    if let Err(error) = result {
        failure(&mut env, error as i32);
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionAuthenticate(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        mesh_ffi_c::secure_session_authenticate(handle as u64, &seed, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionSend(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    input: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(1..=4096).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_session_send(handle as u64, &input, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionReceive(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    input: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let len = env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?;
        if !(58..=4154).contains(&len) {
            return Err(mesh_types_error());
        }
        let input = env
            .convert_byte_array(&input)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_session_receive(handle as u64, &input, now as u64)
    });
    output_bytes(env, result)
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionAuthenticated(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    match mesh_ffi_c::secure_session_authenticated(handle as u64) {
        Ok(value) => u8::from(value),
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionPeer(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jbyteArray {
    match mesh_ffi_c::secure_session_peer(handle as u64) {
        Ok(member) => env
            .byte_array_from_slice(&member)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureSessionRelease(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if let Err(error) = mesh_ffi_c::secure_session_release(handle as u64) {
        failure(&mut env, error as i32);
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreCreateGroup(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    delivery_seed: JByteArray,
    member: JByteArray,
    now: jlong,
) -> jlong {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&delivery_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&member)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let identity_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let delivery_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&delivery_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let member = env
            .convert_byte_array(&member)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_create_group(
            handle as u64,
            &identity_seed,
            &delivery_seed,
            &member,
            now as u64,
        )
    });
    match result {
        Ok(epoch) => epoch as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreIssueEnrollment(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    identity_seed: JByteArray,
    request: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let request_length = env
            .get_array_length(&request)
            .map_err(|_| mesh_types_error())?;
        if !(1..=512).contains(&request_length) {
            return Err(mesh_types_error());
        }
        let identity_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let request = env
            .convert_byte_array(&request)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_issue_enrollment(
            handle as u64,
            &identity_seed,
            &request,
            now as u64,
        )
    });
    match result {
        Ok(bytes) => env
            .byte_array_from_slice(&bytes)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_enrollmentRequestMember(
    mut env: JNIEnv,
    _class: JClass,
    request: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let request_length = env
            .get_array_length(&request)
            .map_err(|_| mesh_types_error())?;
        if !(1..=512).contains(&request_length) {
            return Err(mesh_types_error());
        }
        let request = env
            .convert_byte_array(&request)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::enrollment_request_member(&request, now as u64).map(|member| member.0.to_vec())
    });
    match result {
        Ok(member) => env
            .byte_array_from_slice(&member)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreInstallPolicy(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    bundle: JByteArray,
    now: jlong,
) -> jlong {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        let length = env
            .get_array_length(&bundle)
            .map_err(|_| mesh_types_error())?;
        if !(1..=mesh_protocol::MAX_POLICY_BUNDLE as i32).contains(&length) {
            return Err(mesh_types_error());
        }
        let bundle = env
            .convert_byte_array(&bundle)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::secure_store_install_policy(handle as u64, &bundle, now as u64)
    });
    match result {
        Ok(epoch) => epoch as jlong,
        Err(error) => {
            failure(&mut env, error as i32);
            0
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_secureStoreExportPolicy(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0 {
            return Err(mesh_types_error());
        }
        mesh_ffi_c::secure_store_export_policy(handle as u64, now as u64)
    });
    match result {
        Ok(bytes) => env
            .byte_array_from_slice(&bytes)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_createEnrollmentRequest(
    mut env: JNIEnv,
    _class: JClass,
    identity_seed: JByteArray,
    delivery_seed: JByteArray,
    invitation: JByteArray,
    now: jlong,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if now <= 0
            || env
                .get_array_length(&identity_seed)
                .map_err(|_| mesh_types_error())?
                != 32
            || env
                .get_array_length(&delivery_seed)
                .map_err(|_| mesh_types_error())?
                != 32
        {
            return Err(mesh_types_error());
        }
        let invitation_length = env
            .get_array_length(&invitation)
            .map_err(|_| mesh_types_error())?;
        if !(1..=mesh_protocol::MAX_POLICY_BUNDLE as i32).contains(&invitation_length) {
            return Err(mesh_types_error());
        }
        let identity_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&identity_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let delivery_seed = zeroize::Zeroizing::new(
            env.convert_byte_array(&delivery_seed)
                .map_err(|_| mesh_types_error())?,
        );
        let invitation = env
            .convert_byte_array(&invitation)
            .map_err(|_| mesh_types_error())?;
        mesh_ffi_c::create_enrollment_request_from_policy(
            &identity_seed,
            &delivery_seed,
            &invitation,
            now as u64,
        )
    });
    match result {
        Ok(bytes) => env
            .byte_array_from_slice(&bytes)
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(error) => {
            failure(&mut env, error as i32);
            std::ptr::null_mut()
        }
    }
}
#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_release(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if let Err(e) = mesh_ffi_c::release(handle as u64) {
        failure(&mut env, e as i32);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_frazko_mesh_1host_NativeBridge_identityPublic(
    mut env: JNIEnv,
    _class: JClass,
    input: JByteArray,
) -> jbyteArray {
    let result = mesh_ffi_c::guarded(|| {
        if env
            .get_array_length(&input)
            .map_err(|_| mesh_types_error())?
            != 32
        {
            return Err(mesh_types_error());
        }
        let bytes = zeroize::Zeroizing::new(
            env.convert_byte_array(&input)
                .map_err(|_| mesh_types_error())?,
        );
        mesh_ffi_c::identity_public(&bytes)
    });
    match result {
        Ok(bytes) => env
            .byte_array_from_slice(&bytes)
            .map(|a| a.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(e) => {
            failure(&mut env, e as i32);
            std::ptr::null_mut()
        }
    }
}
