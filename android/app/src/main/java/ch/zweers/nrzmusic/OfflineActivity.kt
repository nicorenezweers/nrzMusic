package ch.zweers.nrzmusic

import android.app.Activity
import android.app.AlertDialog
import android.content.ComponentName
import android.graphics.BitmapFactory
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.text.TextUtils
import android.text.format.Formatter
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import android.widget.Button
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import androidx.media3.ui.PlayerView
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.MoreExecutors

/**
 * Offline-Bibliothek: alle Titel, die aufs Handy geladen wurden – spielt ohne Verbindung
 * (ExoPlayer im [PlaybackService], mit Benachrichtigung und Sperrbildschirm).
 * Lange tippen löscht einen Titel vom Handy.
 */
class OfflineActivity : Activity() {
    private var future: ListenableFuture<MediaController>? = null
    private var controller: MediaController? = null
    private lateinit var list: ListView
    private lateinit var header: TextView
    private lateinit var empty: TextView
    private lateinit var playerView: PlayerView
    private var tracks: List<OfflineTrack> = emptyList()

    private val fg = Color.parseColor("#E5E5E5")
    private val muted = Color.parseColor("#A3A3A3")

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#171717"))
        }
        val title = TextView(this).apply {
            text = getString(R.string.offline_title)
            setTextColor(fg)
            textSize = 26f
            setPadding(dp(20), dp(20), dp(20), 0)
        }
        header = TextView(this).apply {
            setTextColor(muted)
            textSize = 13f
            setPadding(dp(20), dp(2), dp(20), dp(8))
        }
        val buttons = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(14), 0, dp(14), dp(8))
            addView(Button(this@OfflineActivity).apply { text = getString(R.string.play_all); setOnClickListener { play(0, false) } })
            addView(Button(this@OfflineActivity).apply { text = getString(R.string.shuffle); setOnClickListener { play(0, true) } })
            addView(Button(this@OfflineActivity).apply { text = getString(R.string.back_to_app); setOnClickListener { finish() } })
        }
        empty = TextView(this).apply {
            text = getString(R.string.offline_empty)
            setTextColor(muted)
            gravity = Gravity.CENTER
            setPadding(dp(32), dp(48), dp(32), dp(48))
        }
        list = ListView(this).apply {
            divider = null
            adapter = Adapter()
            setOnItemClickListener { _, _, pos, _ -> play(pos, false) }
            setOnItemLongClickListener { _, _, pos, _ -> confirmDelete(tracks[pos]); true }
        }
        playerView = PlayerView(this).apply {
            useController = true
            controllerShowTimeoutMs = 0
            controllerHideOnTouch = false
            setShowNextButton(true)
            setShowPreviousButton(true)
            setBackgroundColor(Color.parseColor("#1F1F1F"))
            visibility = View.GONE
        }
        root.addView(title)
        root.addView(header)
        root.addView(buttons)
        root.addView(empty)
        root.addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        root.addView(playerView, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(220)))
        setContentView(root)
    }

    override fun onStart() {
        super.onStart()
        refresh()
        val token = SessionToken(this, ComponentName(this, PlaybackService::class.java))
        val f = MediaController.Builder(this, token).buildAsync()
        future = f
        f.addListener({
            controller = runCatching { f.get() }.getOrNull()
            playerView.player = controller
            if ((controller?.mediaItemCount ?: 0) > 0) playerView.visibility = View.VISIBLE
        }, MoreExecutors.directExecutor())
    }

    override fun onStop() {
        playerView.player = null
        future?.let { MediaController.releaseFuture(it) }
        controller = null
        super.onStop()
    }

    private fun refresh() {
        tracks = OfflineStore.all(this)
        header.text = getString(R.string.offline_count, tracks.size, Formatter.formatShortFileSize(this, OfflineStore.totalSize(this)))
        empty.visibility = if (tracks.isEmpty()) View.VISIBLE else View.GONE
        (list.adapter as BaseAdapter).notifyDataSetChanged()
    }

    private fun item(t: OfflineTrack): MediaItem {
        val meta = MediaMetadata.Builder()
            .setTitle(t.title)
            .setArtist(t.artist)
            .setAlbumTitle(t.album)
            .apply { OfflineStore.artwork(this@OfflineActivity, t)?.let { setArtworkUri(Uri.fromFile(it)) } }
            .build()
        return MediaItem.Builder()
            .setMediaId(t.key)
            .setUri(Uri.fromFile(OfflineStore.file(this, t)))
            .setMediaMetadata(meta)
            .build()
    }

    private fun play(start: Int, shuffle: Boolean) {
        val c = controller ?: return
        if (tracks.isEmpty()) return
        MainActivity.pauseWeb()
        c.shuffleModeEnabled = shuffle
        c.setMediaItems(tracks.map { item(it) }, if (shuffle) (tracks.indices).random() else start, 0L)
        c.prepare()
        c.play()
        playerView.visibility = View.VISIBLE
    }

    private fun confirmDelete(t: OfflineTrack) {
        AlertDialog.Builder(this)
            .setTitle(R.string.delete_title)
            .setMessage(t.title)
            .setPositiveButton(R.string.delete) { _, _ ->
                OfflineStore.remove(this, t.key)
                DownloadService.listener?.invoke(t.key, "removed")
                refresh()
            }
            .setNegativeButton(R.string.cancel, null)
            .show()
    }

    private inner class Adapter : BaseAdapter() {
        override fun getCount() = tracks.size
        override fun getItem(position: Int) = tracks[position]
        override fun getItemId(position: Int) = position.toLong()

        override fun getView(position: Int, convertView: View?, parent: ViewGroup?): View {
            val t = tracks[position]
            val row = (convertView as? LinearLayout) ?: LinearLayout(this@OfflineActivity).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(16), dp(8), dp(16), dp(8))
                addView(ImageView(this@OfflineActivity).apply {
                    id = android.R.id.icon
                    scaleType = ImageView.ScaleType.CENTER_CROP
                }, LinearLayout.LayoutParams(dp(48), dp(48)))
                addView(LinearLayout(this@OfflineActivity).apply {
                    orientation = LinearLayout.VERTICAL
                    setPadding(dp(12), 0, 0, 0)
                    addView(TextView(this@OfflineActivity).apply {
                        id = android.R.id.text1
                        setTextColor(fg)
                        textSize = 15f
                        maxLines = 1
                        ellipsize = TextUtils.TruncateAt.END
                    })
                    addView(TextView(this@OfflineActivity).apply {
                        id = android.R.id.text2
                        setTextColor(muted)
                        textSize = 13f
                        maxLines = 1
                        ellipsize = TextUtils.TruncateAt.END
                    })
                }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            }
            row.findViewById<TextView>(android.R.id.text1).text = t.title
            row.findViewById<TextView>(android.R.id.text2).text = listOf(t.artist, t.album).filter { it.isNotBlank() }.joinToString(" · ")
            val img = row.findViewById<ImageView>(android.R.id.icon)
            val art = OfflineStore.artwork(this@OfflineActivity, t)
            if (art != null) {
                img.setImageBitmap(BitmapFactory.decodeFile(art.path, BitmapFactory.Options().apply { inSampleSize = 4 }))
            } else {
                img.setImageResource(R.drawable.cover_placeholder)
            }
            return row
        }
    }
}
