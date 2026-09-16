package com.frazko.mesh_host

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Native-only seed bundle. Android Keystore wraps four independent 32-byte slots. */
internal class SecureIdentity(context: Context) {
    private val directory = File(context.noBackupFilesDir, "mesh-keys")
    private val alias = "com.frazko.mesh-lab.installation-v1"
    private val aad = "MeshLab/NativeKeys/v1".toByteArray(Charsets.US_ASCII)
    internal class StoreMaterial(val databaseKey: ByteArray, val member: ByteArray) {
        fun wipe() { databaseKey.fill(0); member.fill(0) }
    }
    internal class GroupMaterial(
        val identitySeed: ByteArray,
        val deliverySeed: ByteArray,
        val sessionSeed: ByteArray,
        val member: ByteArray,
    ) {
        fun wipe() { identitySeed.fill(0); deliverySeed.fill(0); sessionSeed.fill(0); member.fill(0) }
    }

    private fun loadMaterial(): ByteArray = synchronized(lock) {
        check(directory.isDirectory || directory.mkdirs()) { "KEY_STORAGE_UNAVAILABLE" }
        val path = File(directory, "installation-v1.bin")
        val file = AtomicFile(path)
        val keystore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val exists = path.exists() || File(path.path + ".bak").exists() || File(path.path + ".new").exists()
        val material: ByteArray
        if (exists) {
            val key = keystore.getKey(alias, null) as? SecretKey ?: error("KEY_STORAGE_UNAVAILABLE")
            val encoded = file.openRead().use { input ->
                val bytes = ByteArray(157); var offset = 0
                while (offset < bytes.size) { val n = input.read(bytes, offset, bytes.size - offset); check(n > 0) { "KEY_STORAGE_CORRUPT" }; offset += n }
                check(input.read() == -1 && bytes[0] == 1.toByte()) { "KEY_STORAGE_CORRUPT" }; bytes
            }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, encoded.copyOfRange(1, 13)))
            cipher.updateAAD(aad)
            material = cipher.doFinal(encoded, 13, 144)
        } else {
            // An orphan wrapping key is not permission to silently replace an identity.
            check(!keystore.containsAlias(alias)) { "KEY_STORAGE_INCOMPLETE" }
            material = ByteArray(128).also { SecureRandom().nextBytes(it) }
            try {
                val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
                generator.init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256).setRandomizedEncryptionRequired(true).build())
                val key = generator.generateKey()
                val cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key); cipher.updateAAD(aad)
                val ciphertext = cipher.doFinal(material)
                check(cipher.iv.size == 12 && ciphertext.size == 144) { "KEY_STORAGE_UNAVAILABLE" }
                val output = file.startWrite()
                try { output.write(byteArrayOf(1) + cipher.iv + ciphertext); file.finishWrite(output) }
                catch (error: Exception) { file.failWrite(output); throw error }
            } catch (error: Exception) { material.fill(0); throw error }
        }
        check(material.size == 128) { "KEY_STORAGE_CORRUPT" }
        material
    }
    private fun publicKey(material: ByteArray): ByteArray {
        val seed = material.copyOfRange(0, 32)
        return try {
            NativeBridge.identityPublic(seed).also { check(it.size == 32) { "KEY_STORAGE_UNAVAILABLE" } }
        } finally { seed.fill(0) }
    }
    fun prepare(): String {
        val material = loadMaterial()
        try {
            return MessageDigest.getInstance("SHA-256").digest(publicKey(material))
                .joinToString("") { "%02x".format(it.toInt() and 255) }
        } finally { material.fill(0) }
    }
    fun storeMaterial(): StoreMaterial {
        val material = loadMaterial()
        try {
            return StoreMaterial(material.copyOfRange(96, 128), publicKey(material))
        } finally { material.fill(0) }
    }
    fun groupMaterial(): GroupMaterial {
        val material = loadMaterial()
        try {
            return GroupMaterial(
                material.copyOfRange(0, 32),
                material.copyOfRange(32, 64),
                material.copyOfRange(64, 96),
                publicKey(material),
            )
        } finally { material.fill(0) }
    }

    /** A stable, public-only short identifier for choosing an Aware link role.
     * It is derived from the installation public key, never from group secret
     * material or a hardware address. */
    fun awareNodeId(): ByteArray {
        val material = loadMaterial()
        return try {
            MessageDigest.getInstance("SHA-256").digest(publicKey(material)).copyOfRange(0, 8)
        } finally { material.fill(0) }
    }
    private companion object { val lock = Any() }
}
