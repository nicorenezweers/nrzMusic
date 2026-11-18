package ch.zweers.nrzmusic

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
import android.webkit.CookieManager
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/**
 * Lädt Titel aufs Handy: ein Auftrag nach dem anderen, mit dem Sitzungs-Cookie der WebView
 * (nur zum eigenen Server). Läuft als Vordergrunddienst mit Fortschritt in der
 * Benachrichtigung, damit Android ihn nicht beendet, wenn die App im Hintergrund ist.
 */
class DownloadService : Service() {
    private val worker = Executors.newSingleThreadExecutor()
    private val pending = AtomicInteger(0)
    private val done = AtomicInteger(0)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val json = intent?.getStringExtra(EXTRA_ITEMS)
        val items = runCatching { JSONArray(json ?: "[]") }.getOrDefault(JSONArray())
        if (items.length() == 0 && pending.get() == 0) {
            stopSelf()
            return START_NOT_STICKY
        }
        pending.addAndGet(items.length())
        startInForeground()
        for (i in 0 until items.length()) {
            val item = items.getJSONObject(i)
            worker.execute {
                fetch(item)
                done.incrementAndGet()
                if (pending.decrementAndGet() == 0) {
                    done.set(0)
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                } else {
                    notifyProgress()
                }
            }
        }
        return START_NOT_STICKY
    }

    private fun startInForeground() {
        val n = progressNotification()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION_ID, n)
        }
    }

    private fun progressNotification(): Notification {
        ensureChannel(this)
        val total = done.get() + pending.get()
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, OfflineActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(getString(R.string.downloading, pending.get()))
            .setProgress(total.coerceAtLeast(1), done.get(), total == 0)
            .setOngoing(true)
            .setContentIntent(open)
            .build()
    }

    private fun notifyProgress() {
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, progressNotification())
    }

    private fun connect(url: String): HttpURLConnection {
        var current = URL(url)
        repeat(5) {
            val c = current.openConnection() as HttpURLConnection
            c.instanceFollowRedirects = false
            c.connectTimeout = 15_000
            c.readTimeout = 60_000
            c.setRequestProperty("User-Agent", "nrzMusicAndroid/${BuildConfig.VERSION_NAME}")
            // Das Cookie der Sitzung nur zum eigenen Server schicken
            if (Prefs.isServer(this, current.toString())) {
                CookieManager.getInstance().getCookie(current.toString())?.let { c.setRequestProperty("Cookie", it) }
            }
            val code = c.responseCode
            if (code in 300..399) {
                val loc = c.getHeaderField("Location") ?: return c
                c.disconnect()
                current = URL(current, loc)
            } else {
                return c
            }
        }
        throw IllegalStateException("Zu viele Weiterleitungen")
    }

    private fun extension(contentType: String?): String = when {
        contentType == null -> "bin"
        contentType.contains("mpeg") -> "mp3"
        contentType.contains("mp4") || contentType.contains("aac") -> "m4a"
        contentType.contains("flac") -> "flac"
        contentType.contains("ogg") -> "ogg"
        contentType.contains("webm") -> "webm"
        contentType.contains("wav") -> "wav"
        contentType.contains("aiff") -> "aiff"
        else -> "bin"
    }

    private fun fetch(item: JSONObject) {
        val key = item.optString("key")
        val url = item.optString("url")
        val title = item.optString("title")
        if (key.isEmpty() || !url.startsWith("http")) return
        if (OfflineStore.has(this, key)) {
            report(key, "done")
            return
        }
        val dir = OfflineStore.dir(this)
        val base = OfflineStore.safeName(key)
        try {
            report(key, "running")
            val c = connect(url)
            if (c.responseCode !in 200..299) throw IllegalStateException("HTTP ${c.responseCode}")
            val ext = extension(c.contentType)
            val tmp = File(dir, "$base.part")
            c.inputStream.use { input -> tmp.outputStream().use { input.copyTo(it, 64 * 1024) } }
            c.disconnect()
            val file = File(dir, "$base.$ext")
            if (!tmp.renameTo(file)) throw IllegalStateException("Datei nicht ablegbar")
            // Cover (optional; Bilder von YouTube/Spotify ohne Cookie)
            var art = ""
            item.optString("artwork").takeIf { it.startsWith("http") }?.let { a ->
                runCatching {
                    val ac = connect(a)
                    if (ac.responseCode in 200..299) {
                        val af = File(dir, "$base.jpg")
                        ac.inputStream.use { input -> af.outputStream().use { input.copyTo(it) } }
                        art = af.name
                    }
                    ac.disconnect()
                }
            }
            OfflineStore.put(
                this,
                OfflineTrack(
                    key = key,
                    title = title,
                    artist = item.optString("artist"),
                    album = item.optString("album"),
                    durationMs = item.optLong("duration_ms"),
                    file = file.name,
                    artwork = art,
                    size = file.length(),
                    addedAt = System.currentTimeMillis() / 1000,
                ),
            )
            report(key, "done")
        } catch (e: Exception) {
            File(dir, "$base.part").delete()
            report(key, "failed")
            val nm = getSystemService(NotificationManager::class.java)
            nm.notify(
                key.hashCode(),
                Notification.Builder(this, CHANNEL)
                    .setSmallIcon(android.R.drawable.stat_notify_error)
                    .setContentTitle(getString(R.string.download_failed, title))
                    .setContentText(e.message ?: "")
                    .setAutoCancel(true)
                    .build(),
            )
        }
    }

    private fun report(key: String, state: String) {
        listener?.invoke(key, state)
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    companion object {
        const val EXTRA_ITEMS = "items"
        private const val CHANNEL = "downloads"
        private const val NOTIFICATION_ID = 2

        /** Meldet Fortschritt an die Web-Oberfläche (setzt MainActivity). */
        @Volatile
        var listener: ((key: String, state: String) -> Unit)? = null

        fun ensureChannel(ctx: Context) {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            if (nm.getNotificationChannel(CHANNEL) == null) {
                nm.createNotificationChannel(
                    NotificationChannel(CHANNEL, ctx.getString(R.string.channel_downloads), NotificationManager.IMPORTANCE_LOW),
                )
            }
        }

        fun enqueue(ctx: Context, items: String) {
            val i = Intent(ctx, DownloadService::class.java).putExtra(EXTRA_ITEMS, items)
            ctx.startForegroundService(i)
        }
    }
}
