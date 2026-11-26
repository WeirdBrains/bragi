package org.weirdbrains.bragi

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

// The relay wake path (B4): while "Wake through your relay" is on, this foreground service
// keeps Bragi's process (and so the Rust core's /weirdnet/wake/1 connection to the home relay)
// running in the background, with a low-importance notification saying so. It does no work of
// its own: the core holds the connection and syncs when a wake arrives. Off by default; the
// Flutter settings start and stop it (WakeChannel). No Google services, so F-Droid builds
// have a working wake path.
class WakeService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val n = notification(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING)
        } else {
            startForeground(NOTIFICATION_ID, n)
        }
        return START_STICKY
    }

    companion object {
        private const val CHANNEL = "relay-wake"
        private const val NOTIFICATION_ID = 4747

        fun start(context: Context) {
            val i = Intent(context, WakeService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(i) else context.startService(i)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, WakeService::class.java))
        }

        private fun notification(context: Context): Notification {
            val open = PendingIntent.getActivity(
                context, 0, Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val b = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val nm = context.getSystemService(NotificationManager::class.java)
                nm.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Wake through your relay", NotificationManager.IMPORTANCE_MIN).apply {
                        description = "Shown while Bragi keeps a connection to your home relay so your devices can wake it."
                        setShowBadge(false)
                    },
                )
                Notification.Builder(context, CHANNEL)
            } else {
                @Suppress("DEPRECATION")
                Notification.Builder(context)
            }
            return b.setSmallIcon(R.mipmap.ic_launcher)
                .setContentTitle("Bragi can be woken")
                .setContentText("Connected to your relay for wake-ups. Turn it off in Settings.")
                .setContentIntent(open)
                .setOngoing(true)
                .build()
        }
    }
}
