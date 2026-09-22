package com.frazko.mesh_host

/** A GATT connection or successful write is not proof of a live remote app. */
internal class BleLinkHealth(private val began: Long = now()) {
    private var lastVerified: Long? = null
    private var lastProbe = began
    fun received(now: Long, authenticated: Boolean) {
        if (authenticated) lastVerified = now
    }
    fun expired(now: Long): Boolean = now - (lastVerified ?: began) >= 12_000
    fun probeDue(now: Long): Boolean {
        if (lastVerified == null || expired(now) || now - lastProbe < 3_000) return false
        lastProbe = now
        return true
    }
    companion object { fun now(): Long = System.nanoTime() / 1_000_000 }
}
