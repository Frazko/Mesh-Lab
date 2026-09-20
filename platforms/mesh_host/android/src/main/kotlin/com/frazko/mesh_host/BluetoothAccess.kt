package com.frazko.mesh_host

import android.Manifest
import android.app.Activity
import android.media.MediaPlayer
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.BroadcastReceiver
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import io.flutter.plugin.common.PluginRegistry
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.Continuation
import kotlin.coroutines.resume
import java.util.UUID
import java.util.ArrayDeque
import java.io.File
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.Executors

/** BLE discovery, enrollment, and the protected GATT transport. */
internal data class CertifiedIncomingText(
    val authorId: String,
    val objectId: String,
    val verifiedAtUnixSeconds: Long,
    val body: String,
)

/** Metadata for audio that native code has already verified and committed.
 * The file name is derived only from the certified object ID. */
internal data class CertifiedIncomingVoice(
    val authorId: String,
    val objectId: String,
    val logicalId: String,
    val verifiedAtUnixSeconds: Long,
    val durationMillis: Long,
    val context: String,
)

internal class BluetoothAccess(
    private val context: Context,
    private val identity: SecureIdentity,
    private val onPolicyChanged: () -> Unit,
) : PluginRegistry.RequestPermissionsResultListener {
    private var activity: Activity? = null
    private var permissionContinuation: Continuation<Unit>? = null
    private val manager get() = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
    private var scanner: BluetoothLeScanner? = null
    private var advertiser: BluetoothLeAdvertiser? = null
    private var gattServer: BluetoothGattServer? = null
    private var requested = false
    private var scanning = false
    private var advertising = false
    // `addService` completes asynchronously. Advertising before this becomes
    // true lets an iPhone connect to an empty GATT database, leaving the link
    // stuck after MTU negotiation without ever reaching the Noise handshake.
    private var serviceReady = false
    private val peers = mutableSetOf<String>()
    private val clients = mutableMapOf<String, BluetoothGatt>()
    private val serverDevices = mutableMapOf<String, android.bluetooth.BluetoothDevice>()
    // ATT payload defaults to 20 bytes.  CoreBluetooth commonly negotiates a
    // larger MTU; retain that per central so Android-to-iPhone voice does not
    // needlessly fall back to the legacy packet size.
    private val serverWriteSizes = mutableMapOf<String, Int>()
    private var probes = 0L
    private var messages = 0L
    private var lastMessage = ""
    // This queue is the only native-to-Dart path intended for product actions.
    // Entries are appended strictly after Rust commits the receipt, and are
    // drained atomically so concurrent messages cannot be collapsed into a
    // mutable “last message” snapshot.
    private val verifiedIncoming = ArrayDeque<CertifiedIncomingText>()
    private val verifiedIncomingVoice = ArrayDeque<CertifiedIncomingVoice>()
    // Text travels on the preferred local Wi-Fi link and, while available, the
    // authenticated BLE link as a second encrypted copy.  The small envelope
    // makes that redundancy invisible to the conversation history.
    private val recentTextIds = LinkedHashSet<Int>()
    // A voice note is split into independent encrypted records. Track every
    // (note, chunk) pair we forwarded so a multi-hop group never bounces the
    // same audio chunk back and forth between radios.
    private val recentVoiceFrames = LinkedHashSet<Long>()
    // Roster updates are signed policy bundles. They travel over an already
    // authenticated link in bounded chunks so existing members converge after
    // a new member is enrolled, without another QR or local network.
    private val recentPolicyFrames = LinkedHashSet<String>()
    private val forwardedRelayFrames = mutableMapOf<String, ByteArray>()
    // MAX_OBJECT_BYTES / CHUNK_BYTES is 64 in the shared Rust contract; slot
    // zero is the announcement. This bound prevents a corrupt native result
    // from turning a radio callback into an unbounded drain loop.
    private val maxDurableRelaySlots = 65
    // At most eight outstanding full-size objects are prepared in one radio
    // turn. The SQLCipher outbox retains anything beyond this scheduler slice.
    private val maxDurableOriginSlots = 520
    private val policyAssemblies = mutableMapOf<String, PolicyAssembly>()
    private val clientSessions = mutableMapOf<BluetoothGatt, SessionLink>()
    private val serverSessions = mutableMapOf<String, SessionLink>()
    private val clientWrites = mutableMapOf<BluetoothGatt, ArrayDeque<ByteArray>>()
    private val serverWrites = mutableMapOf<String, ArrayDeque<ByteArray>>()
    private val inbound = mutableMapOf<String, BleFrameCodec.Assembler>()
    private var enrollmentDetail = ""
    // `null` preserves the laboratory's explicit open-enrollment mode. Product
    // adapters set a concrete roster before they start discovery; then this
    // host fails closed before a private authority can issue a policy.
    @Volatile private var enrollmentAllowedMembers: Set<String>? = null
    @Volatile private var enrollmentAuthorityEnabled = true
    private var receivedVoices = 0L
    private var lastVoiceDurationMillis = 0L
    private var lastVoiceFile: File? = null
    private var voicePlayer: MediaPlayer? = null
    private val voiceAssemblies = mutableMapOf<Int, VoiceAssembly>()
    // Wi-Fi Aware hands this class authenticated NDP sockets. They are never
    // listeners on the current network and never consume a typed local address.
    private val awarePeers = LinkedHashMap<Socket, AwarePeer>()
    private var awareDetail = "Wi-Fi Aware esperando un vecino autenticado."
    private val awareExecutor = Executors.newCachedThreadPool { r ->
        Thread(r, "mesh-aware").apply { isDaemon = true }
    }

    private data class VoiceAssembly(
        val total: Int,
        val durationMillis: Long,
        val chunks: MutableMap<Int, ByteArray> = mutableMapOf(),
    )
    private data class PolicyAssembly(
        val digest: ByteArray,
        val total: Int,
        val chunks: MutableMap<Int, ByteArray> = mutableMapOf(),
    )

    private val adapterStateReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action != BluetoothAdapter.ACTION_STATE_CHANGED) return
            when (intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR)) {
                BluetoothAdapter.STATE_OFF -> synchronized(this@BluetoothAccess) {
                    // Keep the requested state: only the radio changed, never
                    // the encrypted group that the user already created.
                    stopDiscoveryInternal()
                }
                BluetoothAdapter.STATE_ON -> synchronized(this@BluetoothAccess) {
                    if (requested) startDiscoveryInternal()
                }
            }
        }
    }

    init {
        context.applicationContext.registerReceiver(
            adapterStateReceiver,
            IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED),
        )
    }

    private data class SessionLink(
        val handle: Long,
        val initiator: Boolean,
        var stage: Int,
        var heartbeatStarted: Boolean = false,
        var lastHeartbeatAck: Int = Int.MIN_VALUE,
        var presenceAnnounced: Boolean = false,
        var originOutboxDrained: Boolean = false,
    )

    private data class AwarePeer(
        val socket: Socket,
        val input: DataInputStream,
        val output: DataOutputStream,
        val link: SessionLink,
    )

    fun attach(activity: Activity) { this.activity = activity }
    fun detach() {
        activity = null
        permissionContinuation?.resume(Unit)
        permissionContinuation = null
    }

    @Synchronized fun info(): BluetoothInfo {
        val adapter = manager.adapter
            ?: return BluetoothInfo(false, false, false, false, 0, 0, false, messages, lastMessage, "Este teléfono no tiene Bluetooth disponible.")
        val directAuthenticated = authenticatedAwarePeers().isNotEmpty()
        if (!hasPermissions()) {
            return BluetoothInfo(true, false, false, false, 0, 0, directAuthenticated, messages, lastMessage,
                if (directAuthenticated) "Wi‑Fi Aware directo sigue conectado." else "Autoriza Bluetooth para descubrir teléfonos del laboratorio.")
        }
        if (!adapter.isEnabled) {
            // Aware is independent of Bluetooth. Never turn a healthy direct
            // Wi-Fi Aware session red merely because BLE was switched off.
            return BluetoothInfo(true, true, false, false, 0, probes, directAuthenticated, messages, lastMessage,
                if (directAuthenticated) "Bluetooth está apagado. Wi‑Fi Aware directo sigue conectado de forma segura."
                else "Bluetooth está apagado. Wi‑Fi Aware puede seguir buscando vecinos.")
        }
        val active = scanning || advertising
        // The delivery layer is shared by Bluetooth and
        // infrastructure-free Wi-Fi Aware sockets. Aware authentication must
        // make the field-session UI usable just like a GATT authentication.
        val authenticated = clientSessions.values.any { NativeRuntime.sessionAuthenticated(it.handle) } ||
            serverSessions.values.any { NativeRuntime.sessionAuthenticated(it.handle) } ||
            directAuthenticated
        return if (active) {
            val detail = if (authenticated) "Teléfono del grupo conectado de forma segura."
            else enrollmentDetail.ifEmpty { "Buscando Mesh Lab compatibles: ${peers.size} detectado(s)." }
            BluetoothInfo(true, true, true, true, peers.size.toLong(), probes, authenticated, messages, lastMessage, detail)
        } else {
            BluetoothInfo(true, true, true, false, 0, probes, authenticated, messages, lastMessage, "Bluetooth listo. La conexión segura se inicia al tener un grupo.")
        }
    }

    suspend fun prepare(): BluetoothInfo {
        if (!hasPermissions()) requestPermissions()
        return info()
    }

    private fun authenticatedAwarePeers(): List<AwarePeer> = awarePeers.values.filter {
        NativeRuntime.sessionAuthenticated(it.link.handle)
    }

    private fun awarePeerFor(link: SessionLink): AwarePeer? =
        awarePeers.values.firstOrNull { it.link === link }

    private fun acceptAwareSocketInternal(socket: Socket, initiator: Boolean = false) {
        Log.i(logTag, "Wi-Fi Aware socket accepted from ${socket.inetAddress.hostAddress}")
        try { socket.tcpNoDelay = true } catch (_: Exception) { }
        val input = DataInputStream(BufferedInputStream(socket.getInputStream()))
        val output = DataOutputStream(BufferedOutputStream(socket.getOutputStream()))
        val handle = try { NativeRuntime.startSession(identity.groupMaterial(), initiator) } catch (_: Exception) {
            try { socket.close() } catch (_: Exception) { }
            awareDetail = "No se pudo proteger el canal Wi-Fi Aware."
            return
        }
        val peer = AwarePeer(socket, input, output, SessionLink(handle, initiator, 0))
        awarePeers[socket] = peer
        awareDetail = "Protegiendo conexión Wi-Fi Aware…"
        if (initiator) awareWrite(peer, NativeRuntime.sessionWrite(handle))
        awareExecutor.execute {
            try {
                while (!socket.isClosed) {
                    val size = input.readInt()
                    if (size !in 1..maxAwareFrame) break
                    val raw = ByteArray(size); input.readFully(raw)
                    synchronized(this@BluetoothAccess) { receiveAware(peer, raw) }
                }
            } catch (error: Exception) { Log.i(logTag, "Wi-Fi Aware reader closed: ${error.javaClass.simpleName}") }
            synchronized(this@BluetoothAccess) {
                if (awarePeers[socket] === peer) {
                    closeAwarePeer(peer)
                    if (awarePeers.isEmpty()) {
                        awareDetail = "Wi-Fi Aware sin teléfonos conectados. Esperando reconexión…"
                    }
                    publishAwarePresence()
                }
            }
        }
    }

    private fun receiveAware(peer: AwarePeer, raw: ByteArray) {
        val link = peer.link
        try {
            if (!link.initiator && link.stage == 0) {
                NativeRuntime.sessionRead(link.handle, raw); link.stage = 1
                awareWrite(peer, NativeRuntime.sessionWrite(link.handle))
            } else if (!link.initiator && link.stage == 1) {
                NativeRuntime.sessionRead(link.handle, raw); NativeRuntime.sessionFinish(link.handle); link.stage = 2
                awareWrite(peer, NativeRuntime.sessionAuthenticate(link.handle, identity.groupMaterial()))
            } else if (link.initiator && link.stage == 0) {
                NativeRuntime.sessionRead(link.handle, raw); link.stage = 1
                awareWrite(peer, NativeRuntime.sessionWrite(link.handle)); NativeRuntime.sessionFinish(link.handle)
                awareWrite(peer, NativeRuntime.sessionAuthenticate(link.handle, identity.groupMaterial()))
            } else acceptProtected(link, raw)
            if (NativeRuntime.sessionAuthenticated(link.handle)) {
                awareDetail = "Wi-Fi Aware seguro activo con ${authenticatedAwarePeers().size} teléfono(s) del grupo."
                if (!link.originOutboxDrained) {
                    link.originOutboxDrained = true
                    drainDurableOriginOutbox()
                    drainDurableReceiptOutbox()
                    drainDurableRelayQueue()
                    drainDurableRelayReceiptQueue()
                    drainDurableRelayReceiptAckQueue()
                }
                startAwareHeartbeat(link)
                if (!link.presenceAnnounced) {
                    link.presenceAnnounced = true
                    publishAwarePresence()
                }
            }
        } catch (error: Exception) {
            Log.w(logTag, "Wi-Fi Aware protected record rejected at stage ${link.stage}", error)
            closeAwarePeer(peer)
            awareDetail = "Wi-Fi Aware rechazó la conexión. Bluetooth sigue disponible."
            publishAwarePresence()
        }
    }

    /** A stale Wi-Fi Aware stream must let the caller fall back to Bluetooth. */
    private fun awareWrite(peer: AwarePeer, raw: ByteArray): Boolean {
        if (raw.isEmpty() || raw.size > maxAwareFrame) return false
        return try {
            peer.output.writeInt(raw.size)
            peer.output.write(raw)
            peer.output.flush()
            true
        } catch (error: Exception) {
            Log.i(logTag, "Wi-Fi Aware write failed: ${error.javaClass.simpleName}")
            closeAwarePeer(peer)
            false
        }
    }

    /** Socket I/O is forbidden on Flutter's Android main thread. The queue
     * also serializes Noise records with the Wi-Fi Aware reader/handshake. */
    private fun queueAwarePayloads(link: SessionLink, payloads: List<ByteArray>): Boolean {
        if (payloads.isEmpty() || awarePeerFor(link) == null) return false
        awareExecutor.execute {
            synchronized(this@BluetoothAccess) {
                val peer = awarePeerFor(link) ?: return@synchronized
                if (!NativeRuntime.sessionAuthenticated(link.handle)) return@synchronized
                try {
                    for (payload in payloads) {
                        if (!awareWrite(peer, NativeRuntime.sessionSend(link.handle, payload))) return@synchronized
                    }
                } catch (error: Exception) {
                    Log.w(logTag, "Wi-Fi Aware protected write failed", error)
                    closeAwarePeer(peer)
                    publishAwarePresence()
                }
            }
        }
        return true
    }

    private fun closeAwarePeer(peer: AwarePeer) {
        awarePeers.remove(peer.socket)
        NativeRuntime.releaseSession(peer.link.handle)
        try { peer.input.close() } catch (_: Exception) { }
        try { peer.output.close() } catch (_: Exception) { }
        try { peer.socket.close() } catch (_: Exception) { }
    }

    /** Sends the current group membership to every phone after a join or loss. */
    private fun publishAwarePresence() {
        val peers = authenticatedAwarePeers()
        val count = peers.size.coerceAtMost(255)
        val payload = byteArrayOf(presenceMarker, count.toByte())
        for (peer in peers) {
            try { awareWrite(peer, NativeRuntime.sessionSend(peer.link.handle, payload)) }
            catch (_: Exception) { closeAwarePeer(peer) }
        }
        if (peers.isNotEmpty()) {
            awareDetail = "Wi-Fi Aware seguro activo con $count teléfono(s) del grupo."
        }
    }

    /** The coordinator probes every member. An NDP stream can keep a stale stream
     * alive after one iPhone disables Wi-Fi. */
    private fun startAwareHeartbeat(link: SessionLink) {
        if (link.heartbeatStarted) return
        link.heartbeatStarted = true
        awareExecutor.execute {
            var keepRunning = true
            while (keepRunning) {
                try { Thread.sleep(1_000) } catch (_: InterruptedException) { keepRunning = false }
                if (!keepRunning) break
                var id = 0
                synchronized(this@BluetoothAccess) {
                    val peer = awarePeerFor(link)
                    if (peer == null || !NativeRuntime.sessionAuthenticated(link.handle)) {
                        keepRunning = false
                    } else {
                        id = SecureRandom().nextInt()
                        link.lastHeartbeatAck = Int.MIN_VALUE
                        val payload = byteArrayOf(heartbeatMarker) + byteArrayOf(
                            (id ushr 24).toByte(), (id ushr 16).toByte(),
                            (id ushr 8).toByte(), id.toByte(),
                        )
                        try {
                            if (!awareWrite(peer, NativeRuntime.sessionSend(link.handle, payload))) keepRunning = false
                        } catch (_: Exception) {
                            closeAwarePeer(peer)
                            keepRunning = false
                        }
                    }
                }
                if (!keepRunning) break
                try { Thread.sleep(1_250) } catch (_: InterruptedException) { keepRunning = false }
                if (!keepRunning) break
                synchronized(this@BluetoothAccess) {
                    val peer = awarePeerFor(link)
                    if (peer == null || !NativeRuntime.sessionAuthenticated(link.handle)) {
                        keepRunning = false
                    } else if (link.lastHeartbeatAck != id) {
                        closeAwarePeer(peer)
                        awareDetail = "Wi-Fi Aware dejó de responder. Esperando reconexión…"
                        publishAwarePresence()
                        keepRunning = false
                    }
                }
            }
        }
    }

    private fun closeAwareSockets() {
        val peers = awarePeers.values.toList()
        for (peer in peers) closeAwarePeer(peer)
    }

    @Synchronized fun startDiscovery(): BluetoothInfo {
        requested = true
        startDiscoveryInternal()
        return info()
    }

    private fun startDiscoveryInternal() {
        val adapter = manager.adapter ?: return
        if (!hasPermissions() || !adapter.isEnabled || scanning || advertising) return
        peers.clear()
        probes = 0
        try {
            gattServer = manager.openGattServer(context, gattServerCallback)
            val server = gattServer ?: return
            serviceReady = false
            if (!server.addService(gattService())) {
                Log.w(logTag, "Could not queue Mesh Lab GATT service")
                stopDiscoveryInternal()
                return
            }
            // Android is the group authority in this lab. While it owns a
            // group it advertises and waits for the iPhone central; this avoids
            // two simultaneous GATT client links racing each other.
            if (!hasGroup()) {
                scanner = manager.adapter.bluetoothLeScanner
                scanner?.startScan(
                    listOf(ScanFilter.Builder().setServiceUuid(serviceParcelUuid).build()),
                    ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build(), scanCallback,
                )
                scanning = scanner != null
            }
        } catch (_: SecurityException) {
            stopDiscoveryInternal()
        }
    }

    private fun startAdvertisingAfterServiceReady() {
        if (!requested || !serviceReady || advertising) return
        advertiser = manager.adapter?.bluetoothLeAdvertiser
        if (advertiser == null) {
            Log.w(logTag, "Bluetooth advertiser is unavailable")
            return
        }
        try {
            advertiser?.startAdvertising(advertiseSettings, advertiseData, advertiseCallback)
        } catch (_: SecurityException) {
            Log.w(logTag, "Bluetooth advertising permission was lost")
        }
    }

    @Synchronized fun stopDiscovery(): BluetoothInfo {
        requested = false
        stopDiscoveryInternal()
        return info()
    }

    private fun stopDiscoveryInternal() {
        try { if (scanning) scanner?.stopScan(scanCallback) } catch (_: SecurityException) { }
        try { if (advertising) advertiser?.stopAdvertising(advertiseCallback) } catch (_: SecurityException) { }
        scanning = false
        advertising = false
        serviceReady = false
        peers.clear()
        probes = 0
        clients.values.forEach { it.close() }
        clients.clear(); serverDevices.clear(); serverWriteSizes.clear()
        clientSessions.values.forEach { NativeRuntime.releaseSession(it.handle) }
        serverSessions.values.forEach { NativeRuntime.releaseSession(it.handle) }
        clientSessions.clear(); serverSessions.clear(); clientWrites.clear(); serverWrites.clear(); inbound.clear()
        enrollmentDetail = ""
        gattServer?.close()
        gattServer = null
        scanner = null
        advertiser = null
    }

    private fun gattService(): BluetoothGattService {
        val rx = BluetoothGattCharacteristic(rxUuid, BluetoothGattCharacteristic.PROPERTY_WRITE,
            BluetoothGattCharacteristic.PERMISSION_WRITE)
        val tx = BluetoothGattCharacteristic(txUuid, BluetoothGattCharacteristic.PROPERTY_NOTIFY, 0)
        tx.addDescriptor(BluetoothGattDescriptor(clientConfigUuid,
            BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE))
        return BluetoothGattService(serviceUuid, BluetoothGattService.SERVICE_TYPE_PRIMARY).apply {
            addCharacteristic(rx)
            addCharacteristic(tx)
        }
    }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            synchronized(this@BluetoothAccess) {
                if (peers.add(result.device.address)) {
                    clients[result.device.address] = result.device.connectGatt(
                        context, false, gattClientCallback, android.bluetooth.BluetoothDevice.TRANSPORT_LE,
                    )
                }
            }
        }
        override fun onScanFailed(errorCode: Int) { synchronized(this@BluetoothAccess) { scanning = false } }
    }
    private val gattServerCallback = object : BluetoothGattServerCallback() {
        override fun onServiceAdded(status: Int, service: BluetoothGattService) {
            if (service.uuid != serviceUuid) return
            synchronized(this@BluetoothAccess) {
                if (status != BluetoothGatt.GATT_SUCCESS) {
                    Log.w(logTag, "Mesh Lab GATT service failed to start: $status")
                    stopDiscoveryInternal()
                    return
                }
                serviceReady = true
                Log.i(logTag, "Mesh Lab GATT service is ready")
                startAdvertisingAfterServiceReady()
            }
        }
        override fun onConnectionStateChange(device: android.bluetooth.BluetoothDevice, status: Int, newState: Int) {
            synchronized(this@BluetoothAccess) {
                if (status == BluetoothGatt.GATT_SUCCESS && newState == android.bluetooth.BluetoothProfile.STATE_CONNECTED) {
                    peers.add(device.address); serverDevices[device.address] = device
                    serverWriteSizes[device.address] = legacyAttPayload
                } else if (newState == android.bluetooth.BluetoothProfile.STATE_DISCONNECTED) {
                    peers.remove(device.address); serverDevices.remove(device.address); closeServer(device.address)
                }
            }
        }
        override fun onMtuChanged(device: android.bluetooth.BluetoothDevice, mtu: Int) {
            synchronized(this@BluetoothAccess) {
                // Android reports the complete ATT MTU including its 3-byte
                // header. Keep an iOS-safe upper bound for notifications.
                val payload = (mtu - attHeaderBytes).coerceIn(legacyAttPayload, maxAttPayload)
                serverWriteSizes[device.address] = payload
                Log.i(logTag, "ATT payload for ${device.address}: $payload bytes")
            }
        }
        override fun onCharacteristicWriteRequest(
            device: android.bluetooth.BluetoothDevice, requestId: Int,
            characteristic: BluetoothGattCharacteristic, preparedWrite: Boolean,
            responseNeeded: Boolean, offset: Int, value: ByteArray,
        ) {
            synchronized(this@BluetoothAccess) {
                // CoreBluetooth waits for the ATT write response before it
                // processes a notification generated by that write. Reply
                // first; sending the Noise frame before this acknowledgement
                // made the iPhone silently drop the entire response burst.
                if (responseNeeded) gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
                if (!preparedWrite && offset == 0 && characteristic.uuid == rxUuid) {
                    Log.i(logTag, "GATT write from ${device.address}: ${value.size} bytes")
                    receiveFromServer(device, value)
                }
            }
        }
        override fun onDescriptorWriteRequest(device: android.bluetooth.BluetoothDevice, requestId: Int,
            descriptor: BluetoothGattDescriptor, preparedWrite: Boolean, responseNeeded: Boolean,
            offset: Int, value: ByteArray) {
            if (descriptor.uuid == clientConfigUuid && value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)) {
                synchronized(this@BluetoothAccess) {
                    Log.i(logTag, "Notifications enabled by ${device.address}")
                    serverWrites.putIfAbsent(device.address, ArrayDeque())
                }
            }
            if (responseNeeded) gattServer?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
        }
        override fun onNotificationSent(device: android.bluetooth.BluetoothDevice, status: Int) {
            synchronized(this@BluetoothAccess) {
                Log.i(logTag, "Notification sent to ${device.address}: $status")
                pumpServer(device)
            }
        }
    }
    private val gattClientCallback = object : BluetoothGattCallback() {
        override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS && newState == android.bluetooth.BluetoothProfile.STATE_CONNECTED) {
                gatt.discoverServices()
            } else if (newState == android.bluetooth.BluetoothProfile.STATE_DISCONNECTED) {
                synchronized(this@BluetoothAccess) {
                    clients.entries.removeIf { it.value == gatt }
                    clientSessions.remove(gatt)?.let { NativeRuntime.releaseSession(it.handle) }
                    clientWrites.remove(gatt); inbound.remove("c:${gatt.device.address}")
                }
                gatt.close()
            }
        }
        override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) return
            val service = gatt.getService(serviceUuid) ?: return
            val tx = service.getCharacteristic(txUuid) ?: return
            gatt.setCharacteristicNotification(tx, true)
            val descriptor = tx.getDescriptor(clientConfigUuid) ?: return
            descriptor.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            gatt.writeDescriptor(descriptor)
        }
        override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
            if (descriptor.uuid != clientConfigUuid || status != BluetoothGatt.GATT_SUCCESS) return
            synchronized(this@BluetoothAccess) { startClient(gatt) }
        }
        override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
            if (characteristic.uuid == txUuid) synchronized(this@BluetoothAccess) { receiveFromClient(gatt, value) }
        }
        override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
            synchronized(this@BluetoothAccess) { pumpClient(gatt) }
        }
    }
    private val advertiseCallback = object : AdvertiseCallback() {
        override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
            synchronized(this@BluetoothAccess) {
                advertising = true
                Log.i(logTag, "Mesh Lab advertising started")
            }
        }
        override fun onStartFailure(errorCode: Int) {
            synchronized(this@BluetoothAccess) {
                advertising = false
                Log.w(logTag, "Mesh Lab advertising failed: $errorCode")
            }
        }
    }
    private fun startClient(gatt: BluetoothGatt) {
        if (clientSessions.containsKey(gatt)) return
        if (!hasGroup()) {
            enrollmentDetail = "Teléfono Mesh Lab encontrado. Incorporando el grupo por Bluetooth…"
            enqueueClient(gatt, listOf(byteArrayOf(enrollmentHello.toByte())))
            return
        }
        try {
            val handle = NativeRuntime.startSession(identity.groupMaterial(), true)
            clientSessions[gatt] = SessionLink(handle, true, 0)
            enqueueClient(gatt, listOf(NativeRuntime.sessionWrite(handle)))
        } catch (_: Exception) { gatt.disconnect() }
    }
    private fun receiveFromServer(device: android.bluetooth.BluetoothDevice, fragment: ByteArray) {
        val key = "s:${device.address}"
        val framed = inbound.getOrPut(key) { BleFrameCodec.Assembler() }.accept(fragment) ?: return
        val raw = try { NativeBridge.linkFrameDecode(framed) } catch (_: Exception) { return }
        Log.i(logTag, "Decoded ${raw.size} byte record from ${device.address}")
        if (handleEnrollmentFromClient(raw, device)) return
        var link = serverSessions[device.address]
        try {
            if (link == null) {
                val handle = NativeRuntime.startSession(identity.groupMaterial(), false)
                link = SessionLink(handle, false, 0)
                serverSessions[device.address] = link
            }
            when (link.stage) {
                0 -> {
                    NativeRuntime.sessionRead(link.handle, raw)
                    link.stage = 1
                    Log.i(logTag, "Noise step 1 accepted from ${device.address}")
                    enqueueServer(device, listOf(NativeRuntime.sessionWrite(link.handle)))
                }
                1 -> {
                    NativeRuntime.sessionRead(link.handle, raw)
                    NativeRuntime.sessionFinish(link.handle)
                    link.stage = 2
                    Log.i(logTag, "Noise step 2 accepted from ${device.address}")
                    enqueueServer(device, listOf(NativeRuntime.sessionAuthenticate(link.handle, identity.groupMaterial())))
                }
                else -> acceptProtected(link, raw)
            }
        } catch (error: Exception) {
            Log.w(logTag, "Rejected GATT record from ${device.address}", error)
            closeServer(device.address)
        }
    }
    private fun receiveFromClient(gatt: BluetoothGatt, fragment: ByteArray) {
        val key = "c:${gatt.device.address}"
        val framed = inbound.getOrPut(key) { BleFrameCodec.Assembler() }.accept(fragment) ?: return
        val raw = try { NativeBridge.linkFrameDecode(framed) } catch (_: Exception) { return }
        if (handleEnrollmentFromServer(raw, gatt)) return
        val link = clientSessions[gatt] ?: return
        try {
            when (link.stage) {
                0 -> {
                    NativeRuntime.sessionRead(link.handle, raw)
                    link.stage = 1
                    val third = NativeRuntime.sessionWrite(link.handle)
                    NativeRuntime.sessionFinish(link.handle)
                    val proof = NativeRuntime.sessionAuthenticate(link.handle, identity.groupMaterial())
                    enqueueClient(gatt, listOf(third, proof))
                }
                else -> acceptProtected(link, raw)
            }
        } catch (_: Exception) { closeClient(gatt) }
    }
    private fun acceptProtected(link: SessionLink, raw: ByteArray) {
        val data = NativeRuntime.sessionReceive(link.handle, raw)
        if (NativeRuntime.sessionAuthenticated(link.handle)) {
            probes++
            Log.i(logTag, "Secure link authenticated")
            if (!link.originOutboxDrained) {
                link.originOutboxDrained = true
                drainDurableOriginOutbox()
                drainDurableReceiptOutbox()
                drainDurableRelayQueue()
                drainDurableRelayReceiptQueue()
                drainDurableRelayReceiptAckQueue()
            }
        }
        if (acceptRoutedPayload(link, data)) return
        if (data.firstOrNull() == receiptMarker && data.size == receiptBytes) {
            if (awarePeerFor(link) != null) awareDetail = "✓ Texto confirmado por un teléfono del grupo mediante Wi-Fi Aware."
            Log.i(logTag, "Wi-Fi Aware text receipt received")
            return
        }
        if (data.firstOrNull() == heartbeatMarker && data.size == receiptBytes) {
            sendAwareControl(link, byteArrayOf(heartbeatAckMarker) + data.copyOfRange(1, receiptBytes))
            return
        }
        if (data.firstOrNull() == heartbeatAckMarker && data.size == receiptBytes) {
            if (awarePeerFor(link) != null) link.lastHeartbeatAck = readInt(data, 1)
            return
        }
        if (data.firstOrNull() == presenceMarker && data.size == 2) return
        if (data.firstOrNull() == policyMarker) {
            if (acceptPolicyChunk(data)) relayPayload(link, data)
            return
        }
        if (data.isNotEmpty()) {
            if (data.firstOrNull() == textMarker && data.size >= textHeaderBytes) {
                sendAwareReceipt(link, data.copyOfRange(1, textHeaderBytes))
            }
            val shouldRelay = if (!acceptVoiceChunk(data)) {
                val text = unwrapText(data) ?: return
                messages++
                lastMessage = text.toString(Charsets.UTF_8)
                Log.i(logTag, "Protected text received via ${if (awarePeerFor(link) != null) "Wi-Fi Aware" else "Bluetooth"}")
                true
            } else shouldRelayVoiceFrame(data)
            if (shouldRelay) relayPayload(link, data)
        }
    }

    /** Ingests only the durable wire format. Legacy chat payloads deliberately
     * keep their existing path until the sender/drain executor is complete. */
    private fun acceptRoutedPayload(link: SessionLink, data: ByteArray): Boolean {
        if (data.size < 96 || data[0] != 0x72.toByte() || data[1] != 1.toByte()) return false
        val frame = data.copyOfRange(2, 93)
        val relayKey = frame.copyOfRange(1, 17).joinToString("") { "%02x".format(it.toInt() and 255) }
        val kind = data[95].toInt() and 255
        val forwarded = try {
            when (kind) {
                1, 3, 4 -> {
                    val material = identity.groupMaterial()
                    try { NativeRuntime.openRelayGate(material.member) } finally { material.wipe() }
                    val decision = NativeRuntime.acceptRelayFrame(frame, NativeRuntime.sessionPeer(link.handle))
                    if (decision.size != 92 || decision[0] != 1.toByte()) return true
                    decision.copyOfRange(1, 92).also { forwardedRelayFrames[relayKey] = it }
                }
                2 -> forwardedRelayFrames[relayKey] ?: return true
                else -> return true
            }
        } catch (_: Exception) { return true }
        val persisted = byteArrayOf(0x72, 1) + forwarded + data.copyOfRange(93, data.size)
        try {
            val accepted = NativeRuntime.acceptRoutedRecord(
                persisted, NativeRuntime.sessionPeer(link.handle),
            )
            if (accepted == 2) {
                finalizeDurableText()
                drainDurableRelayQueue()
            } else if (accepted == 3) {
                drainDurableRelayReceiptQueue()
                drainDurableReceiptAckOutbox()
            } else if (accepted == 4) {
                drainDurableRelayReceiptAckQueue()
            }
        } catch (_: Exception) { }
        return true
    }

    /** Runs only after the final chunk committed to SQLCipher. The persisted
     * ingress member is compared to each authenticated neighbor, so this
     * node never sends the object right back over the link that supplied it.
     * The queue remains intact: receipts are the future removal authority. */
    private fun drainDurableRelayQueue() {
        val receivedFrom = try { NativeRuntime.relayReceivedFrom() } catch (_: Exception) { return }
        if (receivedFrom.size != 32) return
        val records = ArrayList<ByteArray>(maxDurableRelaySlots)
        for (slot in 0 until maxDurableRelaySlots) {
            val record = try { NativeRuntime.relayRecord(slot) } catch (_: Exception) { return }
            if (record.isEmpty()) break
            if (record.size > 4096) return
            records += record
        }
        if (records.isEmpty()) return
        for (peer in authenticatedAwarePeers()) {
            if (!isIngressNeighbor(peer.link, receivedFrom)) queueAwarePayloads(peer.link, records)
        }
        for ((gatt, link) in clientSessions) {
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueClient(gatt, records.map { NativeRuntime.sessionSend(link.handle, it) }) }
                catch (_: Exception) { }
            }
        }
        for ((address, link) in serverSessions) {
            val device = serverDevices[address] ?: continue
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueServer(device, records.map { NativeRuntime.sessionSend(link.handle, it) }) }
                catch (_: Exception) { }
            }
        }
    }

    /** Receipt relay work is persisted separately from object custody. The
     * same ingress-neighbor exclusion prevents an immediate return loop after
     * a process restart or a temporary loss of the route toward the origin. */
    private fun drainDurableRelayReceiptQueue() {
        val receivedFrom = try { NativeRuntime.relayReceiptReceivedFrom() } catch (_: Exception) { return }
        if (receivedFrom.size != 32) return
        val records = ArrayList<ByteArray>(maxDurableOriginSlots)
        for (slot in 0 until maxDurableOriginSlots) {
            val record = try { NativeRuntime.relayReceiptRecord(slot) } catch (_: Exception) { return }
            if (record.isEmpty()) break
            if (record.size > 4096) return
            records += record
        }
        if (records.isEmpty()) return
        for (peer in authenticatedAwarePeers()) {
            if (!isIngressNeighbor(peer.link, receivedFrom)) queueAwarePayloads(peer.link, records)
        }
        for ((gatt, link) in clientSessions) {
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueClient(gatt, records.map { NativeRuntime.sessionSend(link.handle, it) }) }
                catch (_: Exception) { }
            }
        }
        for ((address, link) in serverSessions) {
            val device = serverDevices[address] ?: continue
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueServer(device, records.map { NativeRuntime.sessionSend(link.handle, it) }) }
                catch (_: Exception) { }
            }
        }
    }

    /** ACK custody survives the relay restart that historically made a remote
     * recipient keep retrying until object expiry. */
    private fun drainDurableRelayReceiptAckQueue() {
        val receivedFrom = try { NativeRuntime.relayReceiptAckReceivedFrom() } catch (_: Exception) { return }
        if (receivedFrom.size != 32) return
        val records = ArrayList<ByteArray>(maxDurableOriginSlots)
        for (slot in 0 until maxDurableOriginSlots) {
            val record = try { NativeRuntime.relayReceiptAckRecord(slot) } catch (_: Exception) { return }
            if (record.isEmpty()) break
            if (record.size > 4096) return
            records += record
        }
        if (records.isEmpty()) return
        for (peer in authenticatedAwarePeers()) {
            if (!isIngressNeighbor(peer.link, receivedFrom)) queueAwarePayloads(peer.link, records)
        }
        for ((gatt, link) in clientSessions) {
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueClient(gatt, records.map { NativeRuntime.sessionSend(link.handle, it) }) } catch (_: Exception) { }
            }
        }
        for ((address, link) in serverSessions) {
            val device = serverDevices[address] ?: continue
            if (!isIngressNeighbor(link, receivedFrom) && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueServer(device, records.map { NativeRuntime.sessionSend(link.handle, it) }) } catch (_: Exception) { }
            }
        }
    }

    /** Presents a durable text only after native Rust has verified every
     * encrypted chunk and SQLCipher has committed its signed receipt. */
    /** Parses completion packet v2 from Rust. The packet is created only after
     * signature, roster, chunk, and receipt-commit verification. Do not
     * convert malformed or legacy packets into product events. */
    private fun finalizeDurableText() {
        val packet = try { NativeRuntime.finalizeNextDurableText(identity.groupMaterial()) }
        catch (_: Exception) { return }
        val headerBytes = 76
        if (packet.size < headerBytes || packet[0] != 0x74.toByte() || packet[1] != 2.toByte()) return
        val objectId = packet.copyOfRange(2, 34).joinToString("") { "%02x".format(it.toInt() and 255) }
        val authorId = packet.copyOfRange(34, 66).joinToString("") { "%02x".format(it.toInt() and 255) }
        val verifiedAt = readLong(packet, 66)
        if (verifiedAt <= 0) return
        val receiptLength = readU16(packet, 74)
        val textLengthOffset = headerBytes + receiptLength
        if (textLengthOffset + 2 > packet.size) return
        val textLength = readU16(packet, textLengthOffset)
        val textOffset = textLengthOffset + 2
        if (textLength == 0 || textOffset + textLength != packet.size) return
        val payload = packet.copyOfRange(textOffset, packet.size)
        if (payload.size >= durableVoiceHeaderBytes && payload[0] == voiceMarker &&
            (payload[1] == durableVoiceVersion || payload[1] == durableVoiceContextVersion)) {
            val duration = readInt(payload, 2).toLong()
            val logicalId = payload.copyOfRange(6, 22).joinToString("") { "%02x".format(it.toInt() and 255) }
            val contextOffset = durableVoiceHeaderBytes
            val contextLength = if (payload[1] == durableVoiceContextVersion) {
                if (payload.size < durableVoiceContextHeaderBytes) return
                readU16(payload, contextOffset)
            } else 0
            val audioOffset = if (payload[1] == durableVoiceContextVersion) contextOffset + 2 + contextLength else contextOffset
            if (contextLength > maxVoiceContextBytes || audioOffset >= payload.size) return
            val productContext = if (contextLength == 0) "" else try {
                payload.copyOfRange(contextOffset + 2, audioOffset).toString(Charsets.UTF_8)
            } catch (_: Exception) { return }
            val audio = payload.copyOfRange(audioOffset, payload.size)
            if (duration !in 1..maxVoiceDurationMillis || !logicalId.matches(Regex("[0-9a-f]{32}")) || audio.isEmpty() || audio.size > maxVoiceBytes) return
            try {
                val folder = File(context.cacheDir, "mesh-voice").apply { mkdirs() }
                val file = File(folder, "$objectId.m4a")
                file.writeBytes(audio)
                lastVoiceFile = file
                lastVoiceDurationMillis = duration
                receivedVoices++
                if (verifiedIncomingVoice.size == 64) verifiedIncomingVoice.removeFirst()
                verifiedIncomingVoice.addLast(CertifiedIncomingVoice(authorId, objectId, logicalId, verifiedAt, duration, productContext))
            } catch (_: Exception) { return }
            drainDurableReceiptOutbox(); return
        }
        val text = try { payload.toString(Charsets.UTF_8) } catch (_: Exception) { return }
        messages++
        lastMessage = text
        synchronized(verifiedIncoming) {
            if (verifiedIncoming.size == 64) verifiedIncoming.removeFirst()
            verifiedIncoming.addLast(CertifiedIncomingText(authorId, objectId, verifiedAt, text))
        }
        Log.i(logTag, "Certified durable text committed locally")
        drainDurableReceiptOutbox()
    }

    fun drainVerifiedIncomingText(): List<CertifiedIncomingText> = synchronized(verifiedIncoming) {
        buildList(verifiedIncoming.size) {
            while (verifiedIncoming.isNotEmpty()) add(verifiedIncoming.removeFirst())
        }
    }

    fun drainVerifiedIncomingVoice(): List<CertifiedIncomingVoice> = synchronized(verifiedIncomingVoice) {
        buildList(verifiedIncomingVoice.size) {
            while (verifiedIncomingVoice.isNotEmpty()) add(verifiedIncomingVoice.removeFirst())
        }
    }

    /** Drains a bounded durable source slice on every healthy direct edge.
     * A caller can safely retry this after reconnection: every object keeps
     * its same route identity and downstream custody suppresses duplicates. */
    private fun drainDurableOriginOutbox(): Boolean {
        val records = ArrayList<ByteArray>(maxDurableOriginSlots)
        for (slot in 0 until maxDurableOriginSlots) {
            val record = try { NativeRuntime.outboxRecord(slot) } catch (_: Exception) { return false }
            if (record.isEmpty()) break
            if (record.size > 4096) return false
            records += record
        }
        if (records.isEmpty()) return false
        sendAllProtected(records)
        return true
    }

    /** Replays persisted receipts after every reconnection. The origin accepts
     * each certified actor once, so duplicate radio copies cannot overstate
     * group delivery. */
    private fun drainDurableReceiptOutbox(): Boolean {
        val records = ArrayList<ByteArray>(maxDurableOriginSlots)
        for (slot in 0 until maxDurableOriginSlots) {
            val record = try { NativeRuntime.receiptRecord(slot) } catch (_: Exception) { return false }
            if (record.isEmpty()) break
            if (record.size > 4096) return false
            records += record
        }
        if (records.isEmpty()) return false
        sendAllProtected(records)
        return true
    }

    /** The origin signs an ACK only after it has committed a recipient's
     * receipt. Replays are idempotent at the recipient and never expose keys. */
    private fun drainDurableReceiptAckOutbox(): Boolean {
        val records = ArrayList<ByteArray>(maxDurableOriginSlots)
        for (slot in 0 until maxDurableOriginSlots) {
            val material = try { identity.groupMaterial() } catch (_: Exception) { return false }
            val record = try { NativeRuntime.receiptAckRecord(material, slot) } catch (_: Exception) { return false }
            if (record.isEmpty()) break
            if (record.size > 4096) return false
            records += record
        }
        if (records.isEmpty()) return false
        sendAllProtected(records)
        return true
    }

    private fun isIngressNeighbor(link: SessionLink, receivedFrom: ByteArray): Boolean =
        try { NativeRuntime.sessionPeer(link.handle).contentEquals(receivedFrom) }
        catch (_: Exception) { true }

    private fun sendAwareReceipt(link: SessionLink, id: ByteArray) {
        sendAwareControl(link, byteArrayOf(receiptMarker) + id)
    }

    private fun sendAwareControl(link: SessionLink, payload: ByteArray) {
        if (awarePeerFor(link) == null || !NativeRuntime.sessionAuthenticated(link.handle)) return
        queueAwarePayloads(link, listOf(payload))
    }

    /** The relay decrypts once, removes duplicates, then encrypts the same
     * group payload for every other authenticated radio. This is the hop that
     * lets a Wi-Fi Aware Android reach an iPhone connected through BLE. */
    private fun relayPayload(source: SessionLink, payload: ByteArray) {
        for (peer in authenticatedAwarePeers()) {
            if (peer.link !== source) queueAwarePayloads(peer.link, listOf(payload))
        }
        for ((gatt, link) in clientSessions) {
            if (link !== source && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueClient(gatt, listOf(NativeRuntime.sessionSend(link.handle, payload))) } catch (_: Exception) { }
            }
        }
        for ((address, link) in serverSessions) {
            val device = serverDevices[address] ?: continue
            if (link !== source && NativeRuntime.sessionAuthenticated(link.handle)) {
                try { enqueueServer(device, listOf(NativeRuntime.sessionSend(link.handle, payload))) } catch (_: Exception) { }
            }
        }
    }

    /** Propagate policy only after the authority has persisted the next roster.
     * Every recipient re-verifies the signed bundle in SQLCipher before it
     * changes its radio selector; a relay never invents policy content. */
    private fun broadcastPolicyUpdate(policy: ByteArray) {
        if (policy.isEmpty() || policy.size > maxPolicyBytes) return
        val digest = MessageDigest.getInstance("SHA-256").digest(policy)
        val total = (policy.size + maxPolicyChunkPayload - 1) / maxPolicyChunkPayload
        if (total !in 1..maxPolicyChunks) return
        val payloads = (0 until total).map { index ->
            val from = index * maxPolicyChunkPayload
            val until = minOf(from + maxPolicyChunkPayload, policy.size)
            policyPacket(digest, index, total, policy.copyOfRange(from, until))
        }
        sendAllProtected(payloads)
    }

    /** Returns true exactly once per policy chunk. When all chunks are present,
     * installation is transactional and the Aware selector refreshes itself. */
    private fun acceptPolicyChunk(data: ByteArray): Boolean {
        if (data.size !in (policyHeaderBytes + 1)..(policyHeaderBytes + maxPolicyChunkPayload)) return false
        val digest = data.copyOfRange(1, 33)
        val index = data[33].toInt() and 255
        val total = data[34].toInt() and 255
        if (total !in 1..maxPolicyChunks || index !in 0 until total) return false
        val digestKey = digest.joinToString("") { "%02x".format(it.toInt() and 255) }
        val frameKey = "$digestKey:$index"
        if (!recentPolicyFrames.add(frameKey)) return false
        if (recentPolicyFrames.size > maxRecentPolicyFrames) recentPolicyFrames.remove(recentPolicyFrames.first())
        val assembly = policyAssemblies[digestKey]
        if (assembly == null) {
            if (policyAssemblies.size >= maxPolicyAssemblies) policyAssemblies.clear()
            policyAssemblies[digestKey] = PolicyAssembly(digest, total)
        } else if (assembly.total != total || !MessageDigest.isEqual(assembly.digest, digest)) {
            policyAssemblies.remove(digestKey)
            return false
        }
        val target = policyAssemblies[digestKey] ?: return false
        target.chunks[index] = data.copyOfRange(policyHeaderBytes, data.size)
        if (target.chunks.size != target.total) return true
        val policy = ByteArray(target.chunks.values.sumOf { it.size })
        var offset = 0
        for (position in 0 until target.total) {
            val chunk = target.chunks[position] ?: return true
            chunk.copyInto(policy, offset); offset += chunk.size
        }
        policyAssemblies.remove(digestKey)
        if (!MessageDigest.isEqual(MessageDigest.getInstance("SHA-256").digest(policy), digest)) return true
        try {
            NativeRuntime.installPolicy(policy)
            onPolicyChanged()
            Log.i(logTag, "Installed authenticated roster update")
        } catch (_: Exception) {
            Log.w(logTag, "Rejected authenticated roster update")
        }
        return true
    }

    private fun shouldRelayVoiceFrame(data: ByteArray): Boolean {
        if (data.firstOrNull() != voiceMarker || data.size < voiceHeaderBytes) return false
        val id = readInt(data, 1)
        val index = readU16(data, 5)
        val total = readU16(data, 7)
        if (total !in 1..maxVoiceChunks || index !in 0 until total) return false
        val key = (id.toLong() shl 16) xor index.toLong()
        if (!recentVoiceFrames.add(key)) return false
        if (recentVoiceFrames.size > maxRecentVoiceFrames) recentVoiceFrames.remove(recentVoiceFrames.first())
        return true
    }

    private fun textPacket(bytes: ByteArray): ByteArray {
        val id = SecureRandom().nextInt()
        recentTextIds.add(id)
        if (recentTextIds.size > maxRecentTextIds) recentTextIds.remove(recentTextIds.first())
        return byteArrayOf(textMarker) + byteArrayOf(
            (id ushr 24).toByte(), (id ushr 16).toByte(),
            (id ushr 8).toByte(), id.toByte(),
        ) + bytes
    }

    /** Returns null when this is the duplicate copy from the other radio. */
    private fun unwrapText(data: ByteArray): ByteArray? {
        if (data.firstOrNull() != textMarker || data.size < textHeaderBytes) return data
        val id = readInt(data, 1)
        if (!recentTextIds.add(id)) return null
        if (recentTextIds.size > maxRecentTextIds) {
            recentTextIds.remove(recentTextIds.first())
        }
        return data.copyOfRange(textHeaderBytes, data.size)
    }

    private fun hasGroup(): Boolean = try { NativeRuntime.policyEpoch() > 0L } catch (_: Exception) { false }

    /**
     * Installs a product-authorized roster for automatic enrollment. The native
     * host validates the signed request first, so this never trusts a BLE
     * address or a Dart-provided request identity. Passing null restores the
     * deliberate open-lab behavior; an empty roster denies every applicant.
     */
    fun setEnrollmentAllowedMembers(members: Set<String>?, authorityEnabled: Boolean = true) {
        enrollmentAllowedMembers = members
        enrollmentAuthorityEnabled = authorityEnabled
    }

    private fun enrollmentRequestAllowed(request: ByteArray): Boolean {
        val allowed = enrollmentAllowedMembers ?: return true
        if (!enrollmentAuthorityEnabled) return false
        val member = try { NativeRuntime.enrollmentRequestMember(request) }
            catch (_: Exception) { return false }
        val fingerprint = member.joinToString("") { "%02x".format(it.toInt() and 255) }
        return fingerprint in allowed
    }

    /** Handles public, one-time enrollment before a protected session exists. */
    private fun handleEnrollmentFromClient(raw: ByteArray, device: android.bluetooth.BluetoothDevice): Boolean {
        val kind = raw.firstOrNull()?.toInt()?.and(255) ?: return false
        if (kind !in enrollmentHello..enrollmentPolicy) return false
        Log.i(logTag, "Enrollment record $kind from ${device.address}")
        if (!hasGroup()) return true
        when (kind) {
            enrollmentHello -> {
                // A phone can arrive here after a failed session for a stale
                // group. Its new enrollment must not reuse that old Noise
                // handle when it starts the fresh handshake below.
                closeServer(device.address)
                val policy = try { NativeRuntime.exportInvitation() } catch (_: Exception) {
                    enrollmentDetail = "No se pudo preparar la incorporación Bluetooth."
                    return true
                }
                enrollmentDetail = "Enviando invitación segura por Bluetooth…"
                Log.i(logTag, "Sending invitation to ${device.address}")
                enrollmentFrame(enrollmentInvitation, policy)?.let { enqueueServer(device, listOf(it)) }
            }
            enrollmentRequest -> {
                closeServer(device.address)
                val request = raw.copyOfRange(1, raw.size)
                if (request.isEmpty() || request.size > 512) return true
                if (!enrollmentRequestAllowed(request)) {
                    enrollmentDetail = "Solicitud no autorizada para este convoy."
                    Log.w(logTag, "Rejected enrollment outside the authorized roster")
                    return true
                }
                // A repeat is idempotent: after an authorization the current
                // policy contains the same iPhone certificate and can be installed.
                val policy = try { NativeRuntime.issueEnrollment(identity.groupMaterial(), request) }
                    catch (_: Exception) { try { NativeRuntime.exportInvitation() } catch (_: Exception) { null } }
                if (policy == null) {
                    enrollmentDetail = "Android no pudo autorizar este teléfono."
                    return true
                }
                // Existing members keep their active authenticated links while
                // this new member receives its enrollment response. Fan out the
                // same signed policy now, so all radios converge on the exact
                // roster that selected their two Aware neighbors.
                broadcastPolicyUpdate(policy)
                onPolicyChanged()
                enrollmentDetail = "Segundo teléfono autorizado por Bluetooth."
                Log.i(logTag, "Sending enrolled policy to ${device.address}")
                enrollmentFrame(enrollmentPolicy, policy)?.let { enqueueServer(device, listOf(it)) }
            }
        }
        return true
    }

    private fun handleEnrollmentFromServer(raw: ByteArray, gatt: BluetoothGatt): Boolean {
        val kind = raw.firstOrNull()?.toInt()?.and(255) ?: return false
        if (kind !in enrollmentHello..enrollmentPolicy) return false
        if (hasGroup()) return true
        val payload = raw.copyOfRange(1, raw.size)
        when (kind) {
            enrollmentInvitation -> {
                val request = try { NativeRuntime.createEnrollmentRequest(identity.groupMaterial(), payload) }
                    catch (_: Exception) { enrollmentDetail = "No se pudo validar la invitación Bluetooth."; return true }
                enrollmentDetail = "Solicitando incorporación al Android…"
                enrollmentFrame(enrollmentRequest, request)?.let { enqueueClient(gatt, listOf(it)) }
            }
            enrollmentPolicy -> {
                try { NativeRuntime.installPolicy(payload) }
                catch (_: Exception) { enrollmentDetail = "Android entregó un grupo no válido."; return true }
                onPolicyChanged()
                enrollmentDetail = "✓ Grupo incorporado. Protegiendo la conexión Bluetooth…"
                startClient(gatt)
            }
        }
        return true
    }

    private fun enrollmentFrame(kind: Int, payload: ByteArray): ByteArray? {
        if (payload.size > 4159) {
            enrollmentDetail = "El grupo es demasiado grande para esta incorporación Bluetooth."
            return null
        }
        return byteArrayOf(kind.toByte()) + payload
    }
    @Synchronized fun sendText(text: String, logicalId: String): Boolean {
        val bytes = text.toByteArray(Charsets.UTF_8)
        val id = decodeLogicalId(logicalId) ?: return false
        if (text.isBlank() || bytes.size > 500) return false
        val queued = try { NativeRuntime.enqueueDurableText(identity.groupMaterial(), bytes, id) }
        catch (_: Exception) { return false }
        if (queued <= 0) return false
        // A radio may be temporarily absent. The action is still accepted
        // because its encrypted outbox entry survived first; reconnect drains
        // the same record again rather than creating a second chat item.
        drainDurableOriginOutbox()
        return true
    }
    private fun decodeLogicalId(value: String): ByteArray? {
        if (!value.matches(Regex("[0-9a-f]{32}"))) return null
        return try {
            ByteArray(16) { index ->
                value.substring(index * 2, index * 2 + 2).toInt(16).toByte()
            }
        } catch (_: NumberFormatException) {
            null
        }
    }

    /** Wi-Fi Aware provides an infrastructure-free socket. Reuse the proven
     * Noise framing and payload delivery machinery rather than creating a
     * second, subtly different security protocol for that radio. */
    fun acceptAwareSocket(socket: Socket, initiator: Boolean) {
        synchronized(this) { acceptAwareSocketInternal(socket, initiator) }
    }

    /** Encoded AAC/M4A only. PCM never crosses the Flutter boundary or BLE link. */
    @Synchronized fun sendVoice(audio: ByteArray, durationMillis: Long, logicalId: String): Boolean =
        sendVoiceInternal(audio, durationMillis, logicalId, context = null)

    /** Product context is encrypted inside the same signed durable object as the
     * audio. It is bounded before allocation and never used for host routing. */
    @Synchronized fun sendVoiceWithContext(audio: ByteArray, durationMillis: Long, logicalId: String, context: String): Boolean =
        sendVoiceInternal(audio, durationMillis, logicalId, context)

    private fun sendVoiceInternal(audio: ByteArray, durationMillis: Long, logicalId: String, context: String?): Boolean {
        if (audio.isEmpty() || audio.size > maxVoiceBytes || durationMillis !in 1..maxVoiceDurationMillis) return false
        val id = decodeLogicalId(logicalId) ?: return false
        val contextBytes = context?.toByteArray(Charsets.UTF_8)
        if (contextBytes != null && (contextBytes.isEmpty() || contextBytes.size > maxVoiceContextBytes)) return false
        val headerBytes = if (contextBytes == null) durableVoiceHeaderBytes else durableVoiceContextHeaderBytes + contextBytes.size
        // The durable object, not just AAC bytes, is limited by the shared
        // SQLCipher record ceiling.
        if (headerBytes + audio.size > maxVoiceBytes) return false
        val durable = ByteArray(headerBytes + audio.size)
        durable[0] = voiceMarker
        durable[1] = if (contextBytes == null) durableVoiceVersion else durableVoiceContextVersion
        writeInt(durable, 2, durationMillis.toInt())
        id.copyInto(durable, 6)
        if (contextBytes != null) {
            writeU16(durable, durableVoiceHeaderBytes, contextBytes.size)
            contextBytes.copyInto(durable, durableVoiceContextHeaderBytes)
        }
        audio.copyInto(durable, headerBytes)
        val queued = try { NativeRuntime.enqueueDurableText(identity.groupMaterial(), durable, id) } catch (_: Exception) { return false }
        if (queued <= 0) return false
        drainDurableOriginOutbox()
        return true
    }

    @Synchronized fun voiceInfo(): VoiceInfo = VoiceInfo(
        receivedCount = receivedVoices,
        lastDurationMillis = lastVoiceDurationMillis,
        ready = lastVoiceFile?.isFile == true,
        detail = if (lastVoiceFile?.isFile == true) {
            "Nota de voz recibida. Lista para reproducir."
        } else {
            "Aún no hay una nota de voz recibida."
        },
    )

    @Synchronized fun playLastVoice(): Boolean = playVoiceFile(lastVoiceFile?.takeIf { it.isFile })

    /** A product can only request a host-private file using the immutable ID
     * emitted from the receipt-backed voice FIFO. */
    @Synchronized fun playVoice(objectId: String): Boolean {
        if (!objectId.matches(Regex("[0-9a-f]{64}"))) return false
        return playVoiceFile(File(File(context.cacheDir, "mesh-voice"), "$objectId.m4a").takeIf { it.isFile })
    }

    private fun playVoiceFile(file: File?): Boolean {
        val playable = file ?: return false
        return try {
            voicePlayer?.release()
            voicePlayer = MediaPlayer().apply {
                setDataSource(playable.path)
                prepare()
                start()
            }
            true
        } catch (_: Exception) {
            voicePlayer?.release(); voicePlayer = null
            false
        }
    }

    private fun sendProtected(payloads: List<ByteArray>): Boolean = sendAllProtected(payloads)

    /** A logical group object starts on every live direct edge. Downstream
     * dedupe prevents loops; stopping after the first radio would leave a
     * sparse ring only half-covered. */
    private fun sendAllProtected(payloads: List<ByteArray>): Boolean {
        var delivered = false
        for (peer in authenticatedAwarePeers()) {
            delivered = queueAwarePayloads(peer.link, payloads) || delivered
        }
        for ((gatt, link) in clientSessions) {
            if (!NativeRuntime.sessionAuthenticated(link.handle)) continue
            delivered = try {
                enqueueClient(gatt, payloads.map { NativeRuntime.sessionSend(link.handle, it) }); true
            } catch (_: Exception) { delivered }
        }
        for ((address, link) in serverSessions) {
            val device = serverDevices[address] ?: continue
            if (!NativeRuntime.sessionAuthenticated(link.handle)) continue
            delivered = try {
                enqueueServer(device, payloads.map { NativeRuntime.sessionSend(link.handle, it) }); true
            } catch (_: Exception) { delivered }
        }
        return delivered
    }

    /** Returns true only for a syntactically valid voice payload. */
    private fun acceptVoiceChunk(data: ByteArray): Boolean {
        if (data.firstOrNull() != voiceMarker) return false
        if (data.size <= voiceHeaderBytes) return true
        val id = readInt(data, 1)
        val index = readU16(data, 5)
        val total = readU16(data, 7)
        val durationMillis = readU16(data, 9).toLong()
        if (total !in 1..maxVoiceChunks || index !in 0 until total ||
            durationMillis !in 1..maxVoiceDurationMillis) return true
        val part = data.copyOfRange(voiceHeaderBytes, data.size)
        val assembly = voiceAssemblies[id]
        if (assembly == null) {
            if (voiceAssemblies.size >= 2) voiceAssemblies.clear()
            voiceAssemblies[id] = VoiceAssembly(total, durationMillis)
        } else if (assembly.total != total || assembly.durationMillis != durationMillis) {
            voiceAssemblies.remove(id)
            return true
        }
        val target = voiceAssemblies[id] ?: return true
        target.chunks.putIfAbsent(index, part)
        if (target.chunks.size != target.total) return true
        val audio = ByteArray(target.chunks.values.sumOf { it.size })
        var offset = 0
        for (position in 0 until target.total) {
            val bytes = target.chunks[position] ?: return true
            bytes.copyInto(audio, offset); offset += bytes.size
        }
        voiceAssemblies.remove(id)
        if (audio.isEmpty() || audio.size > maxVoiceBytes) return true
        try {
            val folder = File(context.cacheDir, "mesh-voice").apply { mkdirs() }
            val file = File(folder, "latest.m4a")
            file.writeBytes(audio)
            lastVoiceFile = file
            lastVoiceDurationMillis = target.durationMillis
            receivedVoices++
        } catch (_: Exception) { }
        return true
    }

    private fun voicePacket(id: Int, index: Int, total: Int, durationMillis: Long, audio: ByteArray): ByteArray {
        val result = ByteArray(voiceHeaderBytes + audio.size)
        result[0] = voiceMarker
        writeInt(result, 1, id)
        writeU16(result, 5, index); writeU16(result, 7, total); writeU16(result, 9, durationMillis.toInt())
        audio.copyInto(result, voiceHeaderBytes)
        return result
    }
    private fun policyPacket(digest: ByteArray, index: Int, total: Int, bytes: ByteArray): ByteArray {
        val result = ByteArray(policyHeaderBytes + bytes.size)
        result[0] = policyMarker
        digest.copyInto(result, 1)
        result[33] = index.toByte(); result[34] = total.toByte()
        bytes.copyInto(result, policyHeaderBytes)
        return result
    }
    private fun readInt(bytes: ByteArray, offset: Int): Int =
        ((bytes[offset].toInt() and 255) shl 24) or ((bytes[offset + 1].toInt() and 255) shl 16) or
            ((bytes[offset + 2].toInt() and 255) shl 8) or (bytes[offset + 3].toInt() and 255)
    private fun readU16(bytes: ByteArray, offset: Int): Int =
        ((bytes[offset].toInt() and 255) shl 8) or (bytes[offset + 1].toInt() and 255)
    private fun readLong(bytes: ByteArray, offset: Int): Long {
        var value = 0L
        for (index in offset until offset + 8) value = (value shl 8) or (bytes[index].toLong() and 255)
        return value
    }
    private fun writeInt(bytes: ByteArray, offset: Int, value: Int) {
        bytes[offset] = (value ushr 24).toByte(); bytes[offset + 1] = (value ushr 16).toByte()
        bytes[offset + 2] = (value ushr 8).toByte(); bytes[offset + 3] = value.toByte()
    }
    private fun writeU16(bytes: ByteArray, offset: Int, value: Int) {
        bytes[offset] = (value ushr 8).toByte(); bytes[offset + 1] = value.toByte()
    }
    private fun enqueueClient(gatt: BluetoothGatt, raw: List<ByteArray>) {
        val queue = clientWrites.getOrPut(gatt) { ArrayDeque() }
        raw.forEach { queue.addAll(BleFrameCodec.split(NativeBridge.linkFrameEncode(it), legacyAttPayload)) }
        pumpClient(gatt)
    }
    private fun pumpClient(gatt: BluetoothGatt) {
        val queue = clientWrites[gatt] ?: return
        if (queue.isEmpty()) return
        val rx = gatt.getService(serviceUuid)?.getCharacteristic(rxUuid) ?: return
        rx.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
        rx.value = queue.removeFirst()
        if (!gatt.writeCharacteristic(rx)) queue.clear()
    }
    private fun enqueueServer(device: android.bluetooth.BluetoothDevice, raw: List<ByteArray>) {
        val queue = serverWrites.getOrPut(device.address) { ArrayDeque() }
        val maximumWrite = serverWriteSizes[device.address] ?: legacyAttPayload
        raw.forEach { queue.addAll(BleFrameCodec.split(NativeBridge.linkFrameEncode(it), maximumWrite)) }
        pumpServer(device)
    }
    private fun pumpServer(device: android.bluetooth.BluetoothDevice) {
        val queue = serverWrites[device.address] ?: return
        if (queue.isEmpty()) return
        val tx = gattServer?.getService(serviceUuid)?.getCharacteristic(txUuid) ?: return
        tx.value = queue.removeFirst()
        val sent = gattServer?.notifyCharacteristicChanged(device, tx, false) == true
        Log.i(logTag, "Queue notification to ${device.address}: $sent")
        if (!sent) queue.clear()
    }
    private fun closeClient(gatt: BluetoothGatt) {
        clientSessions.remove(gatt)?.let { NativeRuntime.releaseSession(it.handle) }
        clientWrites.remove(gatt); inbound.remove("c:${gatt.device.address}")
        gatt.disconnect()
    }
    private fun closeServer(address: String) {
        serverSessions.remove(address)?.let { NativeRuntime.releaseSession(it.handle) }
        serverWrites.remove(address); serverWriteSizes.remove(address); inbound.remove("s:$address")
    }
    private val advertiseSettings = AdvertiseSettings.Builder()
        .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY).setConnectable(true).build()
    private val advertiseData = AdvertiseData.Builder().addServiceUuid(serviceParcelUuid).build()

    private fun hasPermissions(): Boolean = Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
        requiredPermissions.all { context.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }

    private suspend fun requestPermissions() {
        val target = activity ?: return
        suspendCancellableCoroutine { continuation ->
            if (permissionContinuation != null) {
                continuation.resume(Unit)
                return@suspendCancellableCoroutine
            }
            permissionContinuation = continuation
            target.requestPermissions(requiredPermissions, permissionRequestCode)
            continuation.invokeOnCancellation { permissionContinuation = null }
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray): Boolean {
        if (requestCode != permissionRequestCode) return false
        permissionContinuation?.resume(Unit)
        permissionContinuation = null
        return true
    }

    private companion object {
        const val enrollmentHello = 0xf0
        const val enrollmentInvitation = 0xf1
        const val enrollmentRequest = 0xf2
        const val enrollmentPolicy = 0xf3
        const val voiceMarker: Byte = 0x56
        const val durableVoiceVersion: Byte = 2
        const val durableVoiceContextVersion: Byte = 3
        const val durableVoiceHeaderBytes = 22
        const val durableVoiceContextHeaderBytes = 24
        const val maxVoiceContextBytes = 512
        const val textMarker: Byte = 0x7f
        const val receiptMarker: Byte = 0x7e
        const val heartbeatMarker: Byte = 0x7d
        const val heartbeatAckMarker: Byte = 0x7c
        const val presenceMarker: Byte = 0x7b
        const val policyMarker: Byte = 0x7a
        const val textHeaderBytes = 5
        const val receiptBytes = 5
        const val maxRecentTextIds = 64
        const val maxRecentVoiceFrames = 128
        const val maxRecentPolicyFrames = 128
        const val voiceHeaderBytes = 11
        const val maxVoicePayload = 4096
        const val maxVoiceChunks = 16
        const val maxVoiceBytes = 48 * 1024
        const val maxVoiceDurationMillis = 8_000L
        const val policyHeaderBytes = 35
        const val maxPolicyChunkPayload = 3_500
        const val maxPolicyChunks = 20
        const val maxPolicyBytes = 64 * 1024
        const val maxPolicyAssemblies = 2
        const val maxAwareFrame = 64 * 1024
        const val legacyAttPayload = 20
        const val maxAttPayload = 185
        const val attHeaderBytes = 3
        const val logTag = "MeshBle"
        const val permissionRequestCode = 4187
        val serviceUuid: UUID = UUID.fromString("3c2865e0-1b51-49b4-9f22-4f15d5667761")
        val rxUuid: UUID = UUID.fromString("3c2865e1-1b51-49b4-9f22-4f15d5667761")
        val txUuid: UUID = UUID.fromString("3c2865e2-1b51-49b4-9f22-4f15d5667761")
        val serviceParcelUuid = ParcelUuid(serviceUuid)
        val clientConfigUuid: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        val requiredPermissions = arrayOf(
            Manifest.permission.BLUETOOTH_SCAN,
            Manifest.permission.BLUETOOTH_CONNECT,
            Manifest.permission.BLUETOOTH_ADVERTISE,
            Manifest.permission.NEARBY_WIFI_DEVICES,
        )
    }
}
