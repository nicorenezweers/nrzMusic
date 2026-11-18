package ch.zweers.nrzmusic

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import org.json.JSONObject
import java.net.URL
import java.util.concurrent.Executors

/**
 * Wiedergabe der Web-Oberfläche im Hintergrund: Vordergrunddienst mit Medien-Benachrichtigung
 * (Titel, Cover, Zurück/Pause/Weiter) und einer MediaSession für Sperrbildschirm und
 * Kopfhörertasten. Die Knöpfe gehen über [MainActivity.command] an die Oberfläche.
 */
class KeepAliveService : Service() {
    private var session: MediaSession? = null
    private val loader = Executors.newSingleThreadExecutor()
    private var artUrl: String? = null
    private var art: Bitmap? = null
    private var title = ""
    private var artist = ""
    private var playing = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannel(this)
        session = MediaSession(this, "nrzMusic").apply {
            setCallback(object : MediaSession.Callback() {
                override fun onPlay() = MainActivity.command("play")
                override fun onPause() = MainActivity.command("pause")
                override fun onSkipToNext() = MainActivity.command("next")
                override fun onSkipToPrevious() = MainActivity.command("prev")
            })
            isActive = true
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CMD -> {
                intent.getStringExtra(EXTRA_CMD)?.let { MainActivity.command(it) }
                return START_NOT_STICKY
            }
            ACTION_STOP -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
        }
        val state = runCatching { JSONObject(intent?.getStringExtra(EXTRA_STATE) ?: "{}") }.getOrDefault(JSONObject())
        playing = state.optBoolean("playing")
        title = state.optString("title")
        artist = state.optString("artist")
        val newArt = state.optString("artwork").takeIf { it.startsWith("http") }
        if (newArt != artUrl) {
            artUrl = newArt
            art = null
            newArt?.let { url ->
                loader.execute {
                    val bmp = runCatching { URL(url).openStream().use { BitmapFactory.decodeStream(it) } }.getOrNull()
                    if (artUrl == url) {
                        art = bmp
                        publish()
                    }
                }
            }
        }
        publish()
        return START_NOT_STICKY
    }

    private fun action(icon: Int, label: String, cmd: String, code: Int): Notification.Action {
        val pi = PendingIntent.getService(
            this, code,
            Intent(this, KeepAliveService::class.java).setAction(ACTION_CMD).putExtra(EXTRA_CMD, cmd),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Action.Builder(android.graphics.drawable.Icon.createWithResource(this, icon), label, pi).build()
    }

    private fun publish() {
        val s = session ?: return
        s.setMetadata(
            MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .apply { art?.let { putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) } }
                .build(),
        )
        s.setPlaybackState(
            PlaybackState.Builder()
                .setActions(
                    PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or
                        PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS,
                )
                .setState(if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f)
                .build(),
        )
        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(title.ifEmpty { getString(R.string.app_name) })
            .setContentText(artist)
            .setLargeIcon(art)
            .setContentIntent(open)
            .setOngoing(playing)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .addAction(action(android.R.drawable.ic_media_previous, "Zurück", "prev", 1))
            .addAction(
                if (playing) action(android.R.drawable.ic_media_pause, "Pause", "pause", 2)
                else action(android.R.drawable.ic_media_play, "Abspielen", "play", 3),
            )
            .addAction(action(android.R.drawable.ic_media_next, "Weiter", "next", 4))
            .setStyle(Notification.MediaStyle().setMediaSession(s.sessionToken).setShowActionsInCompactView(0, 1, 2))
            .build()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(NOTIFICATION_ID, n)
        }
        if (!playing) {
            // Pausiert: Benachrichtigung bleibt, lässt sich aber wegwischen
            stopForeground(STOP_FOREGROUND_DETACH)
        }
    }

    override fun onDestroy() {
        session?.release()
        session = null
        loader.shutdown()
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "playback"
        private const val NOTIFICATION_ID = 1
        private const val ACTION_CMD = "cmd"
        private const val ACTION_STOP = "stop"
        private const val EXTRA_CMD = "cmd"
        private const val EXTRA_STATE = "state"

        fun ensureChannel(ctx: Context) {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            if (nm.getNotificationChannel(CHANNEL) == null) {
                nm.createNotificationChannel(
                    NotificationChannel(CHANNEL, ctx.getString(R.string.channel_playback), NotificationManager.IMPORTANCE_LOW),
                )
            }
        }

        /** Zustand der Web-Wiedergabe melden (aus der Brücke). */
        fun update(ctx: Context, state: String) {
            val playing = runCatching { JSONObject(state).optBoolean("playing") }.getOrDefault(false)
            val i = Intent(ctx, KeepAliveService::class.java).putExtra(EXTRA_STATE, state)
            try {
                if (playing) ctx.startForegroundService(i) else if (running) ctx.startService(i)
            } catch (_: Exception) {
                // Im Hintergrund darf Android den Start verweigern – dann eben ohne Benachrichtigung
            }
            if (playing) running = true
        }

        fun stop(ctx: Context) {
            if (!running) return
            running = false
            runCatching { ctx.startService(Intent(ctx, KeepAliveService::class.java).setAction(ACTION_STOP)) }
        }

        @Volatile
        private var running = false
    }
}
