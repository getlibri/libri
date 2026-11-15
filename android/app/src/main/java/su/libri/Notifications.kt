package su.libri

import android.Manifest
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationCompat
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage
import su.libri.core.Host

/**
 * Notifications through UnifiedPush: the phone's distributor (ntfy, on the
 * ship's own server) holds one connection for every app and wakes this one.
 * The host sends the same encrypted Web Push it sends a browser (push.rs),
 * so the push server sees only ciphertext, and the connector decrypts it
 * here. No Google service is involved.
 */
object Bell {
    private const val CHANNEL = "replies"

    fun prefs(context: Context) = context.getSharedPreferences("libri", Context.MODE_PRIVATE)

    /** The host's address, as the app keeps it. */
    fun host(context: Context) = prefs(context).getString("address", null)?.let { runCatching { Host(it) }.getOrNull() }

    fun on(context: Context) = UnifiedPush.getAckDistributor(context) != null

    fun distributors(context: Context) = UnifiedPush.getDistributors(context)

    /** Picks the phone's distributor and registers with the host's key. */
    fun turnOn(activity: Activity, key: String, then: (Boolean) -> Unit) {
        if (ActivityCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
            if (ok) UnifiedPush.register(activity, vapid = key)
            then(ok)
        }
    }

    fun turnOff(context: Context) {
        val endpoint = prefs(context).getString("push-endpoint", null)
        UnifiedPush.unregister(context)
        if (endpoint != null) Thread { runCatching { host(context)?.pushDrop(endpoint) } }.start()
        prefs(context).edit().remove("push-endpoint").apply()
    }

    /** Subscribes another host to this phone's endpoint, once it is picked. */
    fun follow(context: Context, host: Host) {
        val p = prefs(context)
        val endpoint = p.getString("push-endpoint", null) ?: return
        val p256dh = p.getString("push-p256dh", null) ?: return
        val auth = p.getString("push-auth", null) ?: return
        runCatching { host.pushAdd(endpoint, p256dh, auth) }
    }

    /** Whether the app is on screen, where the chat shows the news itself. */
    @Volatile var shown = false

    fun show(context: Context, title: String, body: String) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, "Replies and news", NotificationManager.IMPORTANCE_HIGH))
        val open = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val note = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notify)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        // One at a time, the latest, as the page's notifications replace
        // each other by tag.
        manager.notify(1, note)
    }
}

class LibriPushService : PushService() {
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        val keys = endpoint.pubKeySet ?: return
        Bell.prefs(this).edit()
            .putString("push-endpoint", endpoint.url)
            .putString("push-p256dh", keys.pubKey)
            .putString("push-auth", keys.auth)
            .apply()
        Thread { runCatching { Bell.host(this)?.pushAdd(endpoint.url, keys.pubKey, keys.auth) } }.start()
    }

    override fun onMessage(message: PushMessage, instance: String) {
        if (!message.decrypted || Bell.shown) return
        val note = runCatching { JSONObject(String(message.content)) }.getOrNull() ?: return
        Bell.show(this, note.optString("title", "Libri"), note.optString("body"))
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {}

    override fun onUnregistered(instance: String) {
        Bell.prefs(this).edit().remove("push-endpoint").apply()
    }
}
