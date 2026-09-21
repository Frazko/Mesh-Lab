package com.frazko.mesh_host

/** Lazy loading happens inside the error boundary, never during plugin registration. */
internal object NativeBridge {
    init { System.loadLibrary("mesh_ffi_jni") }
    @JvmStatic external fun abiVersion(): Int
    @JvmStatic external fun create(version: Int): Long
    @JvmStatic external fun request(handle: Long, input: ByteArray): ByteArray
    @JvmStatic external fun linkFrameEncode(input: ByteArray): ByteArray
    @JvmStatic external fun linkFrameDecode(input: ByteArray): ByteArray
    @JvmStatic external fun routedRecordEncode(frame: ByteArray, record: ByteArray): ByteArray
    @JvmStatic external fun routedRecordDecode(input: ByteArray): ByteArray
    @JvmStatic external fun relayGateOpen(member: ByteArray): Long
    @JvmStatic external fun relayGateAccept(handle: Long, frame: ByteArray, via: ByteArray, now: Long): ByteArray
    @JvmStatic external fun relayGateRelease(handle: Long)
    @JvmStatic external fun release(handle: Long)
    @JvmStatic external fun secureStoreProbe(key: ByteArray, member: ByteArray, path: String)
    @JvmStatic external fun secureStoreOpen(key: ByteArray, member: ByteArray, path: String): Long
    @JvmStatic external fun secureStoreRelease(handle: Long)
    @JvmStatic external fun secureStorePolicyEpoch(handle: Long, now: Long): Long
    @JvmStatic external fun secureStoreDiscoveryTag(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureStoreAwareNeighbors(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureStoreAcceptRouted(handle: Long, input: ByteArray, receivedFrom: ByteArray, now: Long): Int
    @JvmStatic external fun secureStoreEnqueueText(handle: Long, identitySeed: ByteArray, plaintext: ByteArray, now: Long): Int
    @JvmStatic external fun secureStoreEnqueueTextWithLogicalId(handle: Long, identitySeed: ByteArray, plaintext: ByteArray, logicalId: ByteArray, now: Long): Int
    @JvmStatic external fun secureStoreLatestDeliverySummary(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureStoreDeliverySummary(handle: Long, logicalId: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStoreFinalizeNextText(handle: Long, identitySeed: ByteArray, deliverySeed: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStoreOutboxRecord(handle: Long, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreReceiptRecord(handle: Long, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreReceiptAckRecord(handle: Long, identitySeed: ByteArray, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayRecord(handle: Long, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayReceivedFrom(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayReceiptRecord(handle: Long, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayReceiptReceivedFrom(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayReceiptAckRecord(handle: Long, slot: Int, now: Long): ByteArray
    @JvmStatic external fun secureStoreRelayReceiptAckReceivedFrom(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureSessionStart(storeHandle: Long, sessionSeed: ByteArray, member: ByteArray, role: Int, now: Long): Long
    @JvmStatic external fun secureSessionWrite(handle: Long, now: Long): ByteArray
    @JvmStatic external fun secureSessionRead(handle: Long, input: ByteArray, now: Long)
    @JvmStatic external fun secureSessionFinish(handle: Long, now: Long)
    @JvmStatic external fun secureSessionAuthenticate(handle: Long, identitySeed: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureSessionSend(handle: Long, input: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureSessionReceive(handle: Long, input: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureSessionAuthenticated(handle: Long): Boolean
    @JvmStatic external fun secureSessionPeer(handle: Long): ByteArray
    @JvmStatic external fun secureSessionRelease(handle: Long)
    @JvmStatic external fun secureStoreCreateGroup(handle: Long, identitySeed: ByteArray, deliverySeed: ByteArray, member: ByteArray, now: Long): Long
    @JvmStatic external fun secureStoreIssueEnrollment(handle: Long, identitySeed: ByteArray, request: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStoreCanIssueEnrollment(handle: Long, identitySeed: ByteArray, now: Long): Boolean
    @JvmStatic external fun secureStoreSignCloudRelay(handle: Long, identitySeed: ByteArray, canonical: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStorePrepareAuthorityHandoff(handle: Long, identitySeed: ByteArray, successor: ByteArray, validUntil: Long, now: Long): ByteArray
    @JvmStatic external fun enrollmentRequestMember(request: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStoreInstallPolicy(handle: Long, bundle: ByteArray, now: Long): Long
    @JvmStatic external fun secureStoreRotateAuthority(handle: Long, successorSeed: ByteArray, handoff: ByteArray, now: Long): ByteArray
    @JvmStatic external fun secureStoreInstallRotatedPolicy(handle: Long, bundle: ByteArray, handoff: ByteArray, now: Long): Long
    @JvmStatic external fun secureStoreExportPolicy(handle: Long, now: Long): ByteArray
    @JvmStatic external fun createEnrollmentRequest(identitySeed: ByteArray, deliverySeed: ByteArray, invitation: ByteArray, now: Long): ByteArray
    @JvmStatic external fun identityPublic(seed: ByteArray): ByteArray
}
