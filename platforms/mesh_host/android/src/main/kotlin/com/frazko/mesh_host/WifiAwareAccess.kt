package com.frazko.mesh_host

import android.content.Context
import android.content.BroadcastReceiver
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.aware.PeerHandle
import android.net.wifi.aware.AttachCallback
import android.net.wifi.aware.AwarePairingConfig
import android.net.wifi.aware.Characteristics
import android.net.wifi.aware.DiscoverySessionCallback
import android.net.wifi.aware.PublishConfig
import android.net.wifi.aware.PublishDiscoverySession
import android.net.wifi.aware.SubscribeConfig
import android.net.wifi.aware.SubscribeDiscoverySession
import android.net.wifi.aware.WifiAwareManager
import android.net.wifi.aware.WifiAwareSession
import android.net.wifi.aware.WifiAwareNetworkInfo
import android.net.wifi.aware.WifiAwareNetworkSpecifier
import android.os.Handler
import android.os.Looper
import android.os.Build
import java.security.MessageDigest
import java.net.ServerSocket
import java.net.Socket
import java.net.Inet6Address
import java.util.concurrent.Executors

/** Owns only Wi-Fi Aware discovery. Data paths are added after the physical
 * iOS↔Android interoperability gate; this class never falls back to LAN/IP. */
internal class WifiAwareAccess(
    context: Context,
    private val hasGroup: () -> Boolean,
    private val discoveryTag: () -> ByteArray?,
    private val nodeId: () -> ByteArray,
    private val neighborNodeIds: () -> List<ByteArray>?,
    private val acceptSocket: (Socket, Boolean) -> Unit,
) {
    private val appContext = context.applicationContext
    private val handler = Handler(Looper.getMainLooper())
    private val manager = appContext.getSystemService(Context.WIFI_AWARE_SERVICE) as? WifiAwareManager
    private val connectivity = appContext.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
    private var session: WifiAwareSession? = null
    private var publisher: PublishDiscoverySession? = null
    private var subscriber: SubscribeDiscoverySession? = null
    private var activeTag: ByteArray? = null
    private val peers = linkedMapOf<Int, PeerHandle>()
    private val peerNodeIds = linkedMapOf<Int, ByteArray>()
    private val directLinks = linkedSetOf<Int>()
    private val pendingLinks = linkedSetOf<Int>()
    private val callbacks = linkedMapOf<Int, ConnectivityManager.NetworkCallback>()
    private val servers = linkedMapOf<Int, ServerSocket>()
    private val executor = Executors.newCachedThreadPool { r -> Thread(r, "mesh-aware").apply { isDaemon = true } }
    private var nextMessageId = 1
    private var detail = "Wi‑Fi Aware esperando una sesión de campo."
    private var requested = false
    private var attaching = false
    private var retry: Runnable? = null
    private val pairingRequests = linkedSetOf<Int>()
    private val joinMarker: Byte = 0x31
    private val readyMarker: Byte = 0x32
    // This is the advertised Wi-Fi Aware service identifier, not a LAN
    // hostname.  It must exactly match the service declared by the iPhone in
    // Info.plist; otherwise both radios can be healthy while discovering two
    // unrelated services forever.
    private val serviceName = "_meshlab._tcp"

    // Availability can change while a NAN session still appears allocated to
    // the app (for example after Wi-Fi is toggled). Listen to the platform
    // signal so recovery starts immediately instead of waiting for a poll or
    // a person to touch the button again.
    private val availabilityReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action != WifiAwareManager.ACTION_WIFI_AWARE_STATE_CHANGED) return
            synchronized(this@WifiAwareAccess) {
                if (!requested) return
                closeDirectLinks()
                publisher?.close(); subscriber?.close(); session?.close()
                publisher = null; subscriber = null; session = null; activeTag = null; peers.clear(); peerNodeIds.clear(); attaching = false
                detail = if (manager?.isAvailable == true) {
                    "Wi‑Fi Aware cambió de estado. Reconectando…"
                } else {
                    "Wi‑Fi Aware no está disponible. Esperando al radio…"
                }
                scheduleRetry()
            }
        }
    }

    init {
        appContext.registerReceiver(
            availabilityReceiver,
            IntentFilter(WifiAwareManager.ACTION_WIFI_AWARE_STATE_CHANGED),
        )
    }

    @Synchronized fun info(): AwareInfo {
        refreshForGroupChange()
        val supported = appContext.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_AWARE)
        val enabled = supported && manager?.isAvailable == true
        val active = session != null && publisher != null && subscriber != null
        val canPairWithSystem = supportsSystemPairing()
        val state = when {
            !supported -> "unsupported"
            !enabled -> "unavailable"
            !hasGroup() -> "needs_group"
            !requested -> "stopped"
            !active -> "discovering"
            directLinks.isNotEmpty() -> "connected"
            peers.isNotEmpty() -> "peer_detected"
            !canPairWithSystem -> "direct_only"
            else -> "discovering"
        }
        val text = when {
            !supported -> "Este teléfono no soporta Wi‑Fi Aware. Bluetooth seguirá como respaldo."
            !enabled -> "Wi‑Fi Aware está apagado o no disponible para el sistema."
            !hasGroup() -> "Crea o incorpora el grupo antes de abrir Wi‑Fi Aware."
            active && !canPairWithSystem && directLinks.isEmpty() -> "Wi‑Fi Aware directo activo. Este Android no puede emparejar Wi‑Fi Aware con iPhone."
            else -> detail
        }
        return AwareInfo(supported, enabled, active, peers.size.toLong(), 2, state, text)
    }

    /**
     * iOS shows DeviceDiscoveryUI only after a nearby peer asks for an Aware
     * pairing. Advertise the standard opportunistic method on Android 14+
     * devices that report pairing support. The operating systems still own the
     * user's approval and the Aware-layer credentials; Mesh Lab's encrypted
     * group proof runs separately before app data is accepted.
     */
    private fun systemPairingConfig(): AwarePairingConfig? {
        if (!supportsSystemPairing()) return null
        return AwarePairingConfig.Builder()
            .setPairingSetupEnabled(true)
            .setPairingVerificationEnabled(true)
            .setPairingCacheEnabled(true)
            .setBootstrappingMethods(AwarePairingConfig.PAIRING_BOOTSTRAPPING_OPPORTUNISTIC)
            .build()
    }

    private fun supportsSystemPairing(): Boolean =
        Build.VERSION.SDK_INT >= 34 && manager?.characteristics?.isAwarePairingSupported == true

    private fun pairingCipherSuite(): Int {
        if (Build.VERSION.SDK_INT < 36) return Characteristics.WIFI_AWARE_CIPHER_SUITE_NCS_PK_128
        val supported = manager?.characteristics?.supportedPairingCipherSuites ?: 0
        return if (supported and Characteristics.WIFI_AWARE_CIPHER_SUITE_NCS_PK_PASN_128 != 0) {
            Characteristics.WIFI_AWARE_CIPHER_SUITE_NCS_PK_PASN_128
        } else {
            Characteristics.WIFI_AWARE_CIPHER_SUITE_NCS_PK_128
        }
    }

    @Synchronized fun start(): AwareInfo {
        requested = true
        startAttach()
        return info()
    }

    /** The platform can revoke NAN resources while Bluetooth remains healthy.
     * Keep one bounded retry pending; a user leaving the session cancels it. */
    @Synchronized private fun startAttach() {
        if (!requested || attaching || session != null) return
        if (!hasGroup()) { detail = "Crea o incorpora el grupo antes de abrir Wi‑Fi Aware."; return }
        val tag = discoveryTag()
        if (tag == null || tag.size != 16) {
            detail = "No se pudo preparar el identificador seguro del grupo. Reconectando…"
            scheduleRetry()
            return
        }
        val aware = manager
        if (aware == null || !aware.isAvailable) {
            detail = "Wi‑Fi Aware no está disponible. Esperando al radio…"
            scheduleRetry()
            return
        }
        retry?.let { handler.removeCallbacks(it) }; retry = null
        attaching = true
        detail = "Iniciando descubrimiento directo Wi‑Fi Aware…"
        handler.post {
            aware.attach(object : AttachCallback() {
                override fun onAttached(next: WifiAwareSession) {
                    synchronized(this@WifiAwareAccess) {
                        attaching = false
                        if (!requested) { next.close(); return }
                        session = next
                        startDiscovery(next, tag)
                    }
                }
                override fun onAttachFailed() {
                    synchronized(this@WifiAwareAccess) {
                        attaching = false
                        detail = "Wi‑Fi Aware no pudo reservar radio. Reconectando…"
                        scheduleRetry()
                    }
                }
            }, handler)
        }
    }

    /** A new member advances the verified roster and therefore its discovery
     * tag. Refresh NAN immediately so existing members converge without a
     * person cycling the session control. */
    @Synchronized private fun refreshForGroupChange() {
        val current = activeTag ?: return
        if (!requested || session == null) return
        val expected = discoveryTag() ?: return
        if (MessageDigest.isEqual(current, expected)) return
        closeDirectLinks()
        publisher?.close(); subscriber?.close(); session?.close()
        publisher = null; subscriber = null; session = null; activeTag = null; peers.clear(); peerNodeIds.clear(); attaching = false
        detail = "La membresía del grupo cambió. Actualizando Wi‑Fi Aware…"
        scheduleRetry()
    }

    /** Called after the verified roster changes over an authenticated channel.
     * It makes discovery converge on the same two-neighbor plan without asking
     * the person to stop and restart their field session. */
    @Synchronized fun policyChanged() = refreshForGroupChange()

    @Synchronized private fun scheduleRetry() {
        if (!requested || session != null || retry != null) return
        val work = Runnable {
            synchronized(this@WifiAwareAccess) {
                retry = null
                startAttach()
            }
        }
        retry = work
        handler.postDelayed(work, 2_000)
    }

    @Synchronized private fun startDiscovery(next: WifiAwareSession, groupTag: ByteArray) {
        activeTag = groupTag.copyOf()
        val localNode = nodeId()
        if (localNode.size != nodeIdBytes) {
            detail = "No se pudo identificar este teléfono para Wi‑Fi Aware."
            return
        }
        val serviceInfo = groupTag + localNode
        val pairingConfig = systemPairingConfig()
        val callback = object : DiscoverySessionCallback() {
            override fun onPublishStarted(value: PublishDiscoverySession) { synchronized(this@WifiAwareAccess) { publisher = value; updateActiveDetail() } }
            override fun onSubscribeStarted(value: SubscribeDiscoverySession) { synchronized(this@WifiAwareAccess) { subscriber = value; updateActiveDetail() } }
            override fun onServiceDiscovered(peerHandle: PeerHandle, serviceSpecificInfo: ByteArray, matchFilter: MutableList<ByteArray>) {
                synchronized(this@WifiAwareAccess) {
                    // Matching the service name alone could count a nearby, unrelated
                    // Mesh Lab installation. The native store derives this short tag
                    // from the verified group policy; it never passes through Flutter.
                    if (belongsToGroup(groupTag, serviceSpecificInfo)) {
                        val key = peerHandle.hashCode()
                        peers[key] = peerHandle
                        val remoteNode = serviceSpecificInfo.copyOfRange(groupTag.size, groupTag.size + nodeIdBytes)
                        peerNodeIds[key] = remoteNode
                        // The lower node id acts as server. Only the other phone
                        // sends the initial in-band request, preventing two
                        // conflicting NDPs between the same pair.
                        if (isPlannedNeighbor(remoteNode) && compareNodeIds(localNode, remoteNode) > 0 && reserveDirectSlot(key)) {
                            pendingLinks.add(key)
                            sendAware(subscriber, peerHandle, byteArrayOf(joinMarker) + localNode)
                            detail = "Vecino del grupo detectado; solicitando enlace Wi‑Fi Aware…"
                        } else updateActiveDetail()
                    } else if (pairingConfig != null && pairingRequests.add(peerHandle.hashCode())) {
                        // DeviceDiscoveryUI on iOS is intentionally waiting for this
                        // request. It presents the person's system consent before a
                        // peer is paired; we do not treat discovery itself as group
                        // membership.
                        try {
                            subscriber?.initiatePairingRequest(
                                peerHandle,
                                "Mesh Lab Android",
                                pairingCipherSuite(),
                                null,
                            )
                            detail = "Solicitud segura de Wi‑Fi Aware enviada al teléfono cercano…"
                        } catch (_: SecurityException) {
                            detail = "El sistema rechazó la solicitud de enlace Wi‑Fi Aware."
                        }
                    }
                }
            }
            override fun onMessageReceived(peerHandle: PeerHandle, message: ByteArray) {
                synchronized(this@WifiAwareAccess) {
                    if (message.isEmpty()) return
                    val key = peerHandle.hashCode()
                    when (message[0]) {
                        joinMarker -> {
                            if (message.size != 1 + nodeIdBytes || compareNodeIds(localNode, message.copyOfRange(1, message.size)) >= 0) return
                            peers[key] = peerHandle
                            peerNodeIds[key] = message.copyOfRange(1, message.size)
                            if (!isPlannedNeighbor(peerNodeIds[key] ?: byteArrayOf()) || !reserveDirectSlot(key)) {
                                updateActiveDetail()
                                return
                            }
                            pendingLinks.add(key)
                            ensureServerLink(peerHandle, key, groupTag)
                        }
                        readyMarker -> {
                            if (key !in pendingLinks || callbacks.containsKey(key)) return
                            requestClientLink(peerHandle, key, groupTag)
                        }
                    }
                }
            }
            override fun onMessageSendFailed(messageId: Int) { synchronized(this@WifiAwareAccess) {
                detail = "No se pudo negociar el enlace Wi‑Fi Aware. Reintentando descubrimiento…"
            } }
            override fun onPairingSetupRequestReceived(peerHandle: android.net.wifi.aware.PeerHandle, requestId: Int) {
                synchronized(this@WifiAwareAccess) {
                    try {
                        publisher?.acceptPairingRequest(
                            requestId,
                            peerHandle,
                            "Mesh Lab Android",
                            pairingCipherSuite(),
                            null,
                        )
                        detail = "Confirmando enlace seguro Wi‑Fi Aware…"
                    } catch (_: SecurityException) {
                        detail = "El sistema rechazó la confirmación de Wi‑Fi Aware."
                    }
                }
            }
            override fun onPairingSetupSucceeded(peerHandle: android.net.wifi.aware.PeerHandle, alias: String) {
                synchronized(this@WifiAwareAccess) {
                    detail = "Wi‑Fi Aware vinculó $alias. Preparando el enlace del grupo…"
                }
            }
            override fun onPairingSetupFailed(peerHandle: android.net.wifi.aware.PeerHandle) {
                synchronized(this@WifiAwareAccess) {
                    pairingRequests.remove(peerHandle.hashCode())
                    detail = "El teléfono cercano no completó el enlace Wi‑Fi Aware. Reintentando…"
                }
            }
            override fun onServiceLost(peerHandle: android.net.wifi.aware.PeerHandle, reason: Int) {
                synchronized(this@WifiAwareAccess) {
                    // Android delivers this as soon as NAN stops seeing a previously
                    // matched service. Keep the network status honest without a poll.
                    val key = peerHandle.hashCode()
                    peers.remove(key); peerNodeIds.remove(key)
                    closeDirectLink(key)
                    updateActiveDetail()
                }
            }
            override fun onSessionTerminated() { synchronized(this@WifiAwareAccess) {
                closeDirectLinks()
                publisher = null; subscriber = null; session = null; activeTag = null; peers.clear(); peerNodeIds.clear()
                if (requested) { detail = "Wi‑Fi Aware se reinició. Reconectando…"; scheduleRetry() }
            } }
        }
        val publish = PublishConfig.Builder()
            .setServiceName(serviceName)
            .setServiceSpecificInfo(serviceInfo)
            .apply { pairingConfig?.let { setPairingConfig(it) } }
            .build()
        val subscribe = SubscribeConfig.Builder()
            .setServiceName(serviceName)
            .setServiceSpecificInfo(serviceInfo)
            .apply { pairingConfig?.let { setPairingConfig(it) } }
            .build()
        next.publish(publish, callback, handler)
        next.subscribe(subscribe, callback, handler)
    }

    @Synchronized private fun updateActiveDetail() {
        detail = if (publisher != null && subscriber != null) {
            when {
                directLinks.isNotEmpty() -> "Wi‑Fi Aware conectado de forma directa con ${directLinks.size} teléfono(s) del grupo."
                pendingLinks.isNotEmpty() -> "Vecino del grupo detectado; preparando enlace Wi‑Fi Aware…"
                else -> "Wi‑Fi Aware activo: ${peers.size} vecino(s) del grupo detectado(s)."
            }
        } else "Activando publicación y búsqueda Wi‑Fi Aware…"
    }

    private fun belongsToGroup(groupTag: ByteArray, info: ByteArray): Boolean =
        info.size == groupTag.size + nodeIdBytes &&
            MessageDigest.isEqual(groupTag, info.copyOfRange(0, groupTag.size))

    private fun compareNodeIds(a: ByteArray, b: ByteArray): Int {
        for (index in a.indices) {
            val result = (a[index].toInt() and 255).compareTo(b[index].toInt() and 255)
            if (result != 0) return result
        }
        return 0
    }

    /** Discovery remains broad enough to show nearby members, while NDP
     * creation is limited to the two neighbors derived in Rust from the
     * certified roster. A short-id collision still cannot authenticate because
     * the socket runs the full native Noise membership proof. */
    private fun isPlannedNeighbor(remoteNode: ByteArray): Boolean =
        remoteNode.size == nodeIdBytes &&
            neighborNodeIds()?.any { candidate ->
                candidate.size == nodeIdBytes && MessageDigest.isEqual(candidate, remoteNode)
            } == true

    /** Reserve no more than the profile's two NDPs. A known in-flight or live
     * link keeps its reservation; every other nearby group member remains a
     * discovery candidate until the native overlay controller selects it. */
    private fun reserveDirectSlot(key: Int): Boolean =
        key in pendingLinks || key in directLinks || directLinks.size + pendingLinks.size < maxDirectLinks

    private fun securePassphrase(groupTag: ByteArray): String = groupTag.joinToString("") { "%02x".format(it.toInt() and 255) }

    private fun sendAware(source: SubscribeDiscoverySession?, peer: PeerHandle, payload: ByteArray) {
        val active = source ?: return
        try { active.sendMessage(peer, nextMessageId++, payload) }
        catch (_: Exception) { detail = "El radio Wi‑Fi Aware no pudo enviar la solicitud de enlace." }
    }

    private fun sendAware(source: PublishDiscoverySession?, peer: PeerHandle, payload: ByteArray) {
        val active = source ?: return
        try { active.sendMessage(peer, nextMessageId++, payload) }
        catch (_: Exception) { detail = "El radio Wi‑Fi Aware no pudo confirmar el enlace." }
    }

    /** The lower stable node id owns the Wi-Fi Aware listener. The higher id receives this
     * ready record and requests the matching secure NDP. Android's framework
     * supplies the peer IPv6 address and port only after the NDP is available. */
    private fun ensureServerLink(peer: PeerHandle, key: Int, groupTag: ByteArray) {
        if (callbacks.containsKey(key) || directLinks.contains(key) || !reserveDirectSlot(key)) return
        val manager = connectivity ?: run { detail = "El sistema no expuso la conexión Wi‑Fi Aware."; return }
        val server = try { ServerSocket(0) } catch (_: Exception) {
            detail = "No se pudo preparar el canal Wi‑Fi Aware."; return
        }
        servers[key] = server; pendingLinks.add(key)
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { synchronized(this@WifiAwareAccess) {
                if (!requested || callbacks[key] !== this) return
                detail = "Canal Wi‑Fi Aware listo; esperando conexión del teléfono del grupo…"
                executor.execute {
                    try {
                        val socket = server.accept()
                        synchronized(this@WifiAwareAccess) {
                            if (!requested || callbacks[key] !== this || !isNdpSocket(network, socket)) {
                                socket.close()
                                return@synchronized
                            }
                            directLinks.add(key); pendingLinks.remove(key); updateActiveDetail()
                        }
                        acceptSocket(socket, false)
                    } catch (_: Exception) { }
                }
            } }
            override fun onUnavailable() { synchronized(this@WifiAwareAccess) { failDirectLink(key, "Wi‑Fi Aware no pudo abrir el enlace directo. Reintentando…") } }
            override fun onLost(network: Network) { synchronized(this@WifiAwareAccess) { closeDirectLink(key); updateActiveDetail() } }
        }
        callbacks[key] = callback
        try {
            val specifier = WifiAwareNetworkSpecifier.Builder(publisher ?: run { server.close(); return }, peer)
                .setPskPassphrase(securePassphrase(groupTag))
                .setPort(server.localPort)
                .build()
            manager.requestNetwork(NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI_AWARE)
                .setNetworkSpecifier(specifier).build(), callback)
            // The subscriber must request its matching side before this NDP can
            // become available. Sending only from onAvailable deadlocks both
            // ends: the server waits for the client, and the client waits for
            // the server's ready message.
            sendAware(publisher, peer, byteArrayOf(readyMarker))
            detail = "Autorizando enlace directo Wi‑Fi Aware…"
        } catch (_: Exception) { failDirectLink(key, "El sistema rechazó el enlace directo Wi‑Fi Aware.") }
    }

    private fun requestClientLink(peer: PeerHandle, key: Int, groupTag: ByteArray) {
        if (callbacks.containsKey(key) || directLinks.contains(key) || !reserveDirectSlot(key)) return
        val manager = connectivity ?: run { detail = "El sistema no expuso la conexión Wi‑Fi Aware."; return }
        val callback = object : ConnectivityManager.NetworkCallback() {
            private var network: Network? = null
            private var info: WifiAwareNetworkInfo? = null
            override fun onAvailable(value: Network) { synchronized(this@WifiAwareAccess) { network = value; openClientSocket(key, this, value, info) } }
            override fun onCapabilitiesChanged(value: Network, capabilities: NetworkCapabilities) {
                synchronized(this@WifiAwareAccess) {
                    info = capabilities.transportInfo as? WifiAwareNetworkInfo
                    openClientSocket(key, this, value, info)
                }
            }
            override fun onUnavailable() { synchronized(this@WifiAwareAccess) { failDirectLink(key, "Wi‑Fi Aware no pudo conectarse al teléfono cercano. Reintentando…") } }
            override fun onLost(value: Network) { synchronized(this@WifiAwareAccess) { closeDirectLink(key); updateActiveDetail() } }
        }
        callbacks[key] = callback
        try {
            val specifier = WifiAwareNetworkSpecifier.Builder(subscriber ?: return, peer)
                .setPskPassphrase(securePassphrase(groupTag)).build()
            manager.requestNetwork(NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI_AWARE)
                .setNetworkSpecifier(specifier).build(), callback)
            detail = "Conectando directamente por Wi‑Fi Aware…"
        } catch (_: Exception) { failDirectLink(key, "El sistema rechazó el enlace directo Wi‑Fi Aware.") }
    }

    /** A server port is allocated before the NDP exists because Android needs
     * that port inside WifiAwareNetworkSpecifier. The kernel listener is
     * therefore allowed to accept once, but only the connection whose local
     * address belongs to the granted Wi-Fi Aware network reaches Noise. A
     * packet delivered through the ordinary Wi-Fi/cellular network is closed
     * before any group material or application payload is processed. */
    private fun isNdpSocket(network: Network, socket: Socket): Boolean {
        val local = socket.localAddress as? Inet6Address ?: return false
        val addresses = connectivity?.getLinkProperties(network)?.linkAddresses ?: return false
        return addresses.any { it.address == local }
    }

    private fun openClientSocket(key: Int, callback: ConnectivityManager.NetworkCallback, network: Network, info: WifiAwareNetworkInfo?) {
        if (directLinks.contains(key) || info == null || callbacks[key] !== callback) return
        val address = info.peerIpv6Addr ?: return
        val port = info.port
        if (port !in 1..65535) return
        directLinks.add(key); pendingLinks.remove(key); updateActiveDetail()
        executor.execute {
            try { acceptSocket(network.socketFactory.createSocket(address, port), true) }
            catch (_: Exception) { synchronized(this@WifiAwareAccess) { failDirectLink(key, "El canal Wi‑Fi Aware se cerró antes de autenticarse.") } }
        }
    }

    private fun failDirectLink(key: Int, message: String) { closeDirectLink(key); detail = message }

    private fun closeDirectLink(key: Int) {
        directLinks.remove(key); pendingLinks.remove(key)
        servers.remove(key)?.let { try { it.close() } catch (_: Exception) { } }
        callbacks.remove(key)?.let { callback -> try { connectivity?.unregisterNetworkCallback(callback) } catch (_: Exception) { } }
    }

    private fun closeDirectLinks() {
        (callbacks.keys + servers.keys + directLinks + pendingLinks).toSet().forEach(::closeDirectLink)
    }

    @Synchronized fun stop(): AwareInfo {
        requested = false
        retry?.let { handler.removeCallbacks(it) }; retry = null; attaching = false
        // Closing discovery alone leaves existing data paths alive. A product
        // scope transition must tear down those authenticated sockets too.
        closeDirectLinks()
        publisher?.close(); subscriber?.close(); session?.close()
        publisher = null; subscriber = null; session = null; activeTag = null; peers.clear()
        detail = "Wi‑Fi Aware detenido por el usuario."
        return info()
    }

    /** Plugin teardown is distinct from leaving a session: release the OS
     * receiver as well so an engine recreation cannot retain this host. */
    @Synchronized fun dispose() {
        stop()
        try { appContext.unregisterReceiver(availabilityReceiver) }
        catch (_: IllegalArgumentException) { /* already detached */ }
    }

    private companion object {
        const val nodeIdBytes = 8
        const val maxDirectLinks = 2
    }
}
