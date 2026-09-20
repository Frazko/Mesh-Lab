package com.frazko.mesh_host

import io.flutter.embedding.engine.plugins.FlutterPlugin
import io.flutter.embedding.engine.plugins.activity.ActivityAware
import io.flutter.embedding.engine.plugins.activity.ActivityPluginBinding
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.Executors

/** Process-scoped runtime, serial executor. Detaching Dart does not destroy it. */
internal object NativeRuntime {
    val dispatcher = Executors.newSingleThreadExecutor { r -> Thread(r, "mesh-runtime").apply { isDaemon = true } }.asCoroutineDispatcher()
    private const val labScope = "lab"
    private var handle = 0L
    private var storeHandle = 0L
    private var relayGate = 0L
    private var storeScope = labScope
    fun request(method: Int, argument: Long = 0): ByteArray {
        if (handle == 0L) {
            check(NativeBridge.abiVersion() == 1) { "MESH_2" }
            handle = NativeBridge.create(1)
        }
        return NativeBridge.request(handle, MeshEnvelope.request(method, argument))
    }
    fun prepareStore(context: android.content.Context, material: SecureIdentity.StoreMaterial) {
        if (storeHandle != 0L) { material.wipe(); return }
        val directory = File(File(context.noBackupFilesDir, "mesh-store"), storeScope)
        try {
            check(directory.isDirectory || directory.mkdirs()) { "SECURE_STORE_UNAVAILABLE" }
            storeHandle = NativeBridge.secureStoreOpen(material.databaseKey, material.member, File(directory, "state-v1.db").path)
            check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        } finally { material.wipe() }
    }
    /** Switches only after product code selected a distinct authenticated scope.
     * Each scope has its own encrypted SQLCipher file, so a policy from convoy
     * A cannot become the radio group for convoy B. */
    fun productScopeWillChange(scope: String): Boolean {
        check(scope.matches(Regex("^[0-9a-f]{32}$"))) { "INVALID_PRODUCT_SCOPE" }
        return scope != storeScope
    }
    fun selectProductScope(context: android.content.Context, material: SecureIdentity.StoreMaterial, scope: String) {
        check(scope.matches(Regex("^[0-9a-f]{32}$"))) { "INVALID_PRODUCT_SCOPE" }
        if (scope == storeScope && storeHandle != 0L) { material.wipe(); return }
        releaseStore()
        storeScope = scope
        prepareStore(context, material)
    }
    /** Ends product use of a scope without deleting its encrypted audit data.
     * A later prepare opens the isolated lab store, never the last convoy. */
    fun clearProductScope() {
        releaseStore()
        storeScope = labScope
    }
    fun releaseStore() { if (storeHandle != 0L) { NativeBridge.secureStoreRelease(storeHandle); storeHandle = 0L }; if (relayGate != 0L) { NativeBridge.relayGateRelease(relayGate); relayGate = 0L } }
    fun policyEpoch(): Long {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return NativeBridge.secureStorePolicyEpoch(storeHandle, System.currentTimeMillis() / 1000)
    }
    fun policyEpochOrNull(): Long? = if (storeHandle == 0L) null else policyEpoch()
    /** Native-only derived radio discriminator. It is never exposed to Flutter. */
    fun discoveryTagOrNull(): ByteArray? = if (storeHandle == 0L) null else {
        NativeBridge.secureStoreDiscoveryTag(storeHandle, System.currentTimeMillis() / 1000)
            .takeIf { it.size == 16 }
    }
    /** At most two public 8-byte identifiers, selected from the verified
     * roster in Rust. This never exposes the roster or any group secret. */
    fun awareNeighborIdsOrNull(): List<ByteArray>? = if (storeHandle == 0L) null else {
        val bytes = NativeBridge.secureStoreAwareNeighbors(storeHandle, System.currentTimeMillis() / 1000)
        if (bytes.size % 8 != 0 || bytes.size > 16) null
        else bytes.asList().chunked(8).map { it.toByteArray() }
    }
    fun createGroup(material: SecureIdentity.GroupMaterial): Long {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return try {
            NativeBridge.secureStoreCreateGroup(
                storeHandle,
                material.identitySeed,
                material.deliverySeed,
                material.member,
                System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun exportInvitation(): ByteArray {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return NativeBridge.secureStoreExportPolicy(storeHandle, System.currentTimeMillis() / 1000)
    }
    fun createEnrollmentRequest(material: SecureIdentity.GroupMaterial, invitation: ByteArray): ByteArray {
        return try {
            NativeBridge.createEnrollmentRequest(
                material.identitySeed, material.deliverySeed, invitation, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun issueEnrollment(material: SecureIdentity.GroupMaterial, request: ByteArray): ByteArray {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return try {
            NativeBridge.secureStoreIssueEnrollment(
                storeHandle, material.identitySeed, request, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    /** A Boolean-only authority probe. The active authority key and roster
     * remain inside Rust, so an ex-leader cannot claim admission capability. */
    fun canIssueEnrollment(material: SecureIdentity.GroupMaterial): Boolean {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return try {
            NativeBridge.secureStoreCanIssueEnrollment(
                storeHandle, material.identitySeed, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun enrollmentRequestMember(request: ByteArray): ByteArray {
        check(request.isNotEmpty() && request.size <= 512) { "MESH_1" }
        val member = NativeBridge.enrollmentRequestMember(request, System.currentTimeMillis() / 1000)
        check(member.size == 32) { "MESH_1" }
        return member
    }
    fun installPolicy(policy: ByteArray): Long {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return NativeBridge.secureStoreInstallPolicy(storeHandle, policy, System.currentTimeMillis() / 1000)
    }
    fun startSession(material: SecureIdentity.GroupMaterial, initiator: Boolean): Long {
        check(storeHandle != 0L) { "SECURE_STORE_UNAVAILABLE" }
        return try {
            NativeBridge.secureSessionStart(
                storeHandle, material.sessionSeed, material.member, if (initiator) 0 else 1,
                System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun sessionWrite(handle: Long): ByteArray = NativeBridge.secureSessionWrite(handle, System.currentTimeMillis() / 1000)
    fun sessionRead(handle: Long, frame: ByteArray) = NativeBridge.secureSessionRead(handle, frame, System.currentTimeMillis() / 1000)
    fun sessionFinish(handle: Long) = NativeBridge.secureSessionFinish(handle, System.currentTimeMillis() / 1000)
    fun sessionAuthenticate(handle: Long, material: SecureIdentity.GroupMaterial): ByteArray {
        return try { NativeBridge.secureSessionAuthenticate(handle, material.identitySeed, System.currentTimeMillis() / 1000) }
        finally { material.wipe() }
    }
    fun sessionSend(handle: Long, bytes: ByteArray): ByteArray = NativeBridge.secureSessionSend(handle, bytes, System.currentTimeMillis() / 1000)
    fun sessionReceive(handle: Long, bytes: ByteArray): ByteArray = NativeBridge.secureSessionReceive(handle, bytes, System.currentTimeMillis() / 1000)
    fun sessionAuthenticated(handle: Long): Boolean = NativeBridge.secureSessionAuthenticated(handle)
    fun sessionPeer(handle: Long): ByteArray {
        val member = NativeBridge.secureSessionPeer(handle)
        check(member.size == 32) { "MESH_1" }
        return member
    }
    fun acceptRoutedRecord(bytes: ByteArray, receivedFrom: ByteArray): Int {
        check(storeHandle != 0L && bytes.isNotEmpty() && bytes.size <= 4096 && receivedFrom.size == 32) { "MESH_1" }
        return NativeBridge.secureStoreAcceptRouted(storeHandle, bytes, receivedFrom, System.currentTimeMillis() / 1000)
    }
    /** Commits every bounded recipient audience before a radio write. The
     * KeyStore material is copied only into JNI and wiped when this returns. */
    fun enqueueDurableText(material: SecureIdentity.GroupMaterial, text: ByteArray): Int {
        check(storeHandle != 0L && text.isNotEmpty() && text.size <= 48 * 1024) { "MESH_1" }
        return try {
            NativeBridge.secureStoreEnqueueText(
                storeHandle, material.identitySeed, text, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun enqueueDurableText(material: SecureIdentity.GroupMaterial, text: ByteArray, logicalId: ByteArray): Int {
        check(storeHandle != 0L && text.isNotEmpty() && text.size <= 48 * 1024 && logicalId.size == 16) { "MESH_1" }
        return try {
            NativeBridge.secureStoreEnqueueTextWithLogicalId(
                storeHandle, material.identitySeed, text, logicalId, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    fun latestDeliverySummary(): ByteArray {
        check(storeHandle != 0L) { "MESH_1" }
        val value = NativeBridge.secureStoreLatestDeliverySummary(storeHandle, System.currentTimeMillis() / 1000)
        check(value.isEmpty() || value.size == 19) { "MESH_1" }
        return value
    }
    fun deliverySummary(logicalId: ByteArray): ByteArray {
        check(storeHandle != 0L && logicalId.size == 16) { "MESH_1" }
        val value = NativeBridge.secureStoreDeliverySummary(storeHandle, logicalId, System.currentTimeMillis() / 1000)
        check(value.isEmpty() || value.size == 19) { "MESH_1" }
        return value
    }
    fun outboxRecord(slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return NativeBridge.secureStoreOutboxRecord(storeHandle, slot, System.currentTimeMillis() / 1000)
    }
    fun receiptRecord(slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return NativeBridge.secureStoreReceiptRecord(storeHandle, slot, System.currentTimeMillis() / 1000)
    }
    fun receiptAckRecord(material: SecureIdentity.GroupMaterial, slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return try {
            NativeBridge.secureStoreReceiptAckRecord(
                storeHandle, material.identitySeed, slot, System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    /** A nonempty packet is available only after durable validation and the
     * local receipt commit. It remains native until the chat reducer consumes it. */
    fun finalizeNextDurableText(material: SecureIdentity.GroupMaterial): ByteArray {
        check(storeHandle != 0L) { "MESH_1" }
        return try {
            NativeBridge.secureStoreFinalizeNextText(
                storeHandle, material.identitySeed, material.deliverySeed,
                System.currentTimeMillis() / 1000,
            )
        } finally { material.wipe() }
    }
    /** Empty means that the durable queue currently has no eligible record at
     * this slot. The host chooses a neighbor later; Flutter never sees it. */
    fun relayRecord(slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return NativeBridge.secureStoreRelayRecord(storeHandle, slot, System.currentTimeMillis() / 1000)
    }
    fun relayReceivedFrom(): ByteArray {
        check(storeHandle != 0L) { "MESH_1" }
        val member = NativeBridge.secureStoreRelayReceivedFrom(storeHandle, System.currentTimeMillis() / 1000)
        check(member.isEmpty() || member.size == 32) { "MESH_1" }
        return member
    }
    fun relayReceiptRecord(slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return NativeBridge.secureStoreRelayReceiptRecord(
            storeHandle, slot, System.currentTimeMillis() / 1000,
        )
    }
    fun relayReceiptReceivedFrom(): ByteArray {
        check(storeHandle != 0L) { "MESH_1" }
        val member = NativeBridge.secureStoreRelayReceiptReceivedFrom(
            storeHandle, System.currentTimeMillis() / 1000,
        )
        check(member.isEmpty() || member.size == 32) { "MESH_1" }
        return member
    }
    fun relayReceiptAckRecord(slot: Int): ByteArray {
        check(storeHandle != 0L && slot in 0..0xffff) { "MESH_1" }
        return NativeBridge.secureStoreRelayReceiptAckRecord(storeHandle, slot, System.currentTimeMillis() / 1000)
    }
    fun relayReceiptAckReceivedFrom(): ByteArray {
        check(storeHandle != 0L) { "MESH_1" }
        val member = NativeBridge.secureStoreRelayReceiptAckReceivedFrom(storeHandle, System.currentTimeMillis() / 1000)
        check(member.isEmpty() || member.size == 32) { "MESH_1" }
        return member
    }
    fun openRelayGate(member: ByteArray) {
        check(member.size == 32) { "MESH_1" }
        if (relayGate == 0L) relayGate = NativeBridge.relayGateOpen(member)
        check(relayGate != 0L) { "MESH_1" }
    }
    fun acceptRelayFrame(frame: ByteArray, via: ByteArray): ByteArray {
        check(relayGate != 0L && frame.size == 91 && via.size == 32) { "MESH_1" }
        return NativeBridge.relayGateAccept(relayGate, frame, via, System.currentTimeMillis() / 1000)
    }
    fun releaseSession(handle: Long) { if (handle != 0L) NativeBridge.secureSessionRelease(handle) }
}
class MeshHostPlugin : FlutterPlugin, ActivityAware, MeshHostApi {
    private lateinit var identity: SecureIdentity
    private lateinit var appContext: android.content.Context
    private lateinit var bluetooth: BluetoothAccess
    private lateinit var aware: WifiAwareAccess
    override fun onAttachedToEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        appContext = binding.applicationContext
        identity = SecureIdentity(appContext)
        aware = WifiAwareAccess(
            appContext,
            hasGroup = { NativeRuntime.policyEpochOrNull()?.let { it > 0L } ?: false },
            discoveryTag = { try { NativeRuntime.discoveryTagOrNull() } catch (_: Exception) { null } },
            nodeId = { identity.awareNodeId() },
            neighborNodeIds = { try { NativeRuntime.awareNeighborIdsOrNull() } catch (_: Exception) { null } },
            acceptSocket = { socket, initiator -> bluetooth.acceptAwareSocket(socket, initiator) },
        )
        bluetooth = BluetoothAccess(appContext, identity) { aware.policyChanged() }
        MeshHostApi.setUp(binding.binaryMessenger, this)
    }
    override fun onDetachedFromEngine(binding: FlutterPlugin.FlutterPluginBinding) {
        MeshHostApi.setUp(binding.binaryMessenger, null)
        FieldSessionService.stop(appContext)
        aware.dispose()
        NativeRuntime.releaseStore()
    }
    override fun onAttachedToActivity(binding: ActivityPluginBinding) {
        bluetooth.attach(binding.activity)
        binding.addRequestPermissionsResultListener(bluetooth)
    }
    override fun onDetachedFromActivityForConfigChanges() = bluetooth.detach()
    override fun onReattachedToActivityForConfigChanges(binding: ActivityPluginBinding) = onAttachedToActivity(binding)
    override fun onDetachedFromActivity() = bluetooth.detach()
    private suspend fun <T> execute(work: () -> T): T = withContext(NativeRuntime.dispatcher) {
        try { work() }
        catch (e: LinkageError) { throw FlutterError("ENGINE_NOT_PACKAGED", "Falta el motor nativo para esta arquitectura.", null) }
        catch (e: Exception) {
            val code = e.message?.takeIf { it.matches(Regex("MESH_[1-6]")) } ?: "INVALID_ENVELOPE"
            throw FlutterError(code, "El motor no pudo completar la operación.", null)
        }
    }
    override suspend fun prepareIdentity(): IdentityInfo = withContext(NativeRuntime.dispatcher) {
        try {
            val fingerprint = identity.prepare()
            NativeRuntime.prepareStore(appContext, identity.storeMaterial())
            IdentityInfo(fingerprint, "Android Keystore")
        }
        catch (e: Exception) { throw FlutterError("KEY_STORAGE_UNAVAILABLE", "No se pudo abrir la identidad protegida.", null) }
        catch (e: LinkageError) { throw FlutterError("ENGINE_NOT_PACKAGED", "Falta el motor nativo actualizado.", null) }
    }
    override suspend fun groupInfo(): GroupInfo = withContext(NativeRuntime.dispatcher) {
        try {
            if (NativeRuntime.policyEpochOrNull() == null) return@withContext GroupInfo(false, 0)
            val epoch = NativeRuntime.policyEpoch()
            GroupInfo(epoch != 0L, epoch)
        } catch (e: Exception) { throw FlutterError("SECURE_STORE_UNAVAILABLE", "No se pudo leer la política del grupo.", null) }
    }
    override suspend fun createGroup(): GroupInfo = withContext(NativeRuntime.dispatcher) {
        try {
            check(NativeRuntime.policyEpochOrNull() != null) { "SECURE_STORE_UNAVAILABLE" }
            val epoch = NativeRuntime.createGroup(identity.groupMaterial())
            aware.policyChanged()
            GroupInfo(epoch > 0L, epoch)
        } catch (e: Exception) {
            throw FlutterError("GROUP_SETUP_FAILED", "No se pudo crear el grupo local.", null)
        }
    }
    override suspend fun exportInvitation(): ByteArray = withContext(NativeRuntime.dispatcher) {
        try { NativeRuntime.exportInvitation() }
        catch (e: Exception) { throw FlutterError("GROUP_TRANSFER_FAILED", "No se pudo exportar la invitación.", null) }
    }
    override suspend fun createEnrollmentRequest(invitation: ByteArray): ByteArray = withContext(NativeRuntime.dispatcher) {
        try { NativeRuntime.createEnrollmentRequest(identity.groupMaterial(), invitation) }
        catch (e: Exception) { throw FlutterError("GROUP_TRANSFER_FAILED", "No se pudo preparar la solicitud de inscripción.", null) }
    }
    override suspend fun issueEnrollment(request: ByteArray): ByteArray = withContext(NativeRuntime.dispatcher) {
        try { NativeRuntime.issueEnrollment(identity.groupMaterial(), request) }
        catch (e: Exception) { throw FlutterError("GROUP_TRANSFER_FAILED", "No se pudo emitir la inscripción.", null) }
    }
    override suspend fun canIssueEnrollment(): Boolean = withContext(NativeRuntime.dispatcher) {
        try { NativeRuntime.canIssueEnrollment(identity.groupMaterial()) }
        catch (e: Exception) {
            throw FlutterError("GROUP_AUTHORITY_UNAVAILABLE", "No se pudo comprobar la autoridad del grupo.", null)
        }
    }
    override suspend fun installPolicy(policy: ByteArray): GroupInfo = withContext(NativeRuntime.dispatcher) {
        try {
            val epoch = NativeRuntime.installPolicy(policy)
            aware.policyChanged()
            GroupInfo(epoch > 0L, epoch)
        } catch (e: Exception) { throw FlutterError("GROUP_TRANSFER_FAILED", "No se pudo instalar la política del grupo.", null) }
    }
    override suspend fun configureEnrollmentAccess(policy: EnrollmentAccessPolicy) {
        val fingerprint = Regex("^[0-9a-f]{64}$")
        val members = policy.authorizedMemberIds.map { it.lowercase() }.toSet()
        if (!policy.scopeId.matches(Regex("^[0-9a-f]{32}$")) || members.size != policy.authorizedMemberIds.size || members.size > 50 || members.any { !fingerprint.matches(it) }) {
            throw FlutterError("INVALID_ENROLLMENT_ROSTER", "La lista autorizada de la Malla no es válida.", null)
        }
        val scopeChanged = withContext(NativeRuntime.dispatcher) {
            NativeRuntime.productScopeWillChange(policy.scopeId)
        }
        // A product scope is a hard boundary. Do not retain an authenticated
        // radio session while replacing the encrypted Field store beneath it.
        if (scopeChanged) {
            bluetooth.stopDiscovery()
            aware.stop()
        }
        withContext(NativeRuntime.dispatcher) {
            NativeRuntime.selectProductScope(appContext, identity.storeMaterial(), policy.scopeId)
        }
        bluetooth.setEnrollmentAllowedMembers(members, policy.authorityEnabled)
        aware.policyChanged()
    }
    override suspend fun clearEnrollmentAccess() {
        // Clearing product authority also closes every direct link before its
        // cryptographic store is released.
        bluetooth.stopDiscovery()
        aware.stop()
        bluetooth.setEnrollmentAllowedMembers(null)
        withContext(NativeRuntime.dispatcher) { NativeRuntime.clearProductScope() }
        aware.policyChanged()
    }
    override suspend fun bluetoothInfo(): BluetoothInfo = bluetooth.info()
    override suspend fun prepareBluetooth(): BluetoothInfo = bluetooth.prepare()
    override suspend fun startBluetoothDiscovery(): BluetoothInfo {
        val info = bluetooth.startDiscovery()
        // This call originates from the explicit, visible "Conectar sesión"
        // action. A failed foreground-service promotion must not prevent a
        // still-usable foreground radio session.
        runCatching { FieldSessionService.start(appContext) }
        return info
    }
    override suspend fun stopBluetoothDiscovery(): BluetoothInfo = bluetooth.stopDiscovery()
    override suspend fun awareInfo(): AwareInfo = aware.info()
    override suspend fun startAwareDiscovery(): AwareInfo {
        val info = aware.start()
        runCatching { FieldSessionService.start(appContext) }
        return info
    }
    override suspend fun stopAwareDiscovery(): AwareInfo {
        val info = aware.stop()
        FieldSessionService.stop(appContext)
        return info
    }
    override suspend fun sendText(message: String, logicalId: String): Boolean = bluetooth.sendText(message, logicalId)
    override suspend fun drainVerifiedIncomingText(): List<VerifiedIncomingText> =
        bluetooth.drainVerifiedIncomingText().map { event ->
            VerifiedIncomingText(
                event.authorId,
                event.objectId,
                event.verifiedAtUnixSeconds,
                event.body,
            )
        }

    override suspend fun drainVerifiedIncomingVoice(): List<VerifiedIncomingVoice> =
        bluetooth.drainVerifiedIncomingVoice().map { event ->
            VerifiedIncomingVoice(
                event.authorId,
                event.objectId,
                event.logicalId,
                event.verifiedAtUnixSeconds,
                event.durationMillis,
                event.context,
            )
        }

    override suspend fun deliveryInfo(logicalId: String): DeliveryInfo = withContext(NativeRuntime.dispatcher) {
        val requested = logicalId.takeIf { it.matches(Regex("[0-9a-f]{32}")) }
            ?.let { value -> ByteArray(16) { index -> value.substring(index * 2, index * 2 + 2).toInt(16).toByte() } }
            ?: throw IllegalArgumentException("MESH_1")
        val bytes = NativeRuntime.deliverySummary(requested)
        if (bytes.isEmpty()) return@withContext DeliveryInfo("", 0, 0, "none")
        check(bytes.size == 19) { "MESH_1" }
        val id = bytes.copyOfRange(0, 16).joinToString("") { "%02x".format(it.toInt() and 0xff) }
        val state = when (bytes[18].toInt()) { 0 -> "queued"; 1 -> "partial"; 2 -> "delivered"; 3 -> "expired"; else -> throw IllegalStateException("MESH_1") }
        DeliveryInfo(
            id,
            (bytes[16].toInt() and 0xff).toLong(),
            (bytes[17].toInt() and 0xff).toLong(),
            state,
        )
    }
    override suspend fun voiceInfo(): VoiceInfo = bluetooth.voiceInfo()
    override suspend fun sendVoice(audio: ByteArray, durationMillis: Long, logicalId: String): Boolean =
        bluetooth.sendVoice(audio, durationMillis, logicalId)
    override suspend fun sendVoiceWithContext(audio: ByteArray, durationMillis: Long, logicalId: String, context: String): Boolean =
        bluetooth.sendVoiceWithContext(audio, durationMillis, logicalId, context)
    override suspend fun playLastVoice(): Boolean = bluetooth.playLastVoice()
    override suspend fun playVoice(objectId: String): Boolean = bluetooth.playVoice(objectId)
    override suspend fun engineInfo(): EngineInfo = execute {
        val i = MeshEnvelope(NativeRuntime.request(0)).info()
        EngineInfo(i.version, i.abi, i.api, i.phase, i.build)
    }
    override suspend fun subscribe(cursor: Long): EngineSnapshot = snapshot(1, cursor)
    override suspend fun verifyBridge(requestId: Long): EngineSnapshot = snapshot(2, requestId)
    private suspend fun snapshot(method: Int, argument: Long): EngineSnapshot = execute {
        val s = MeshEnvelope(NativeRuntime.request(method, argument)).snapshot(method.toLong())
        EngineSnapshot(s.runtime, s.cursor, s.probes, s.state, s.reset,
            s.events.map { DiagnosticEvent(it.sequence, it.request, it.kind) })
    }
}
