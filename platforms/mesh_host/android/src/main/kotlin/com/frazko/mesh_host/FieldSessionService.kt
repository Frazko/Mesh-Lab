package com.frazko.mesh_host

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Makes an explicitly started field session visible to Android while its
 * Bluetooth and Wi-Fi Aware radios continue operating in the app process.
 *
 * This deliberately does not create a second radio stack. BluetoothAccess and
 * WifiAwareAccess remain the single owners of their platform sessions; the
 * foreground service gives that same process foreground priority while the UI
 * is backgrounded. A force-stop still ends the session by Android design.
 */
class FieldSessionService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val currentNotification = notification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                notificationId,
                currentNotification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } else {
            startForeground(notificationId, currentNotification)
        }
        // The owner of the radios is the active Flutter engine. Never have
        // Android recreate an empty service after a process kill and imply a
        // working mesh session when the encrypted links no longer exist.
        return START_NOT_STICKY
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(channelId, "Sesión de campo", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Muestra que Mesh Lab mantiene sus radios de campo activos."
                },
            )
        }
        return Notification.Builder(this, channelId)
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentTitle("Mesh Lab: sesión activa")
            .setContentText("Bluetooth y Wi‑Fi Aware siguen buscando enlaces seguros.")
            .setCategory(Notification.CATEGORY_SERVICE)
            .setOngoing(true)
            .build()
    }

    companion object {
        private const val channelId = "mesh_lab_field_session"
        private const val notificationId = 8794

        fun start(context: Context) {
            val intent = Intent(context, FieldSessionService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(intent)
            else context.startService(intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, FieldSessionService::class.java))
        }
    }
}
