package ch.zweers.nrzmusic

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Ein Titel auf dem Handy. */
data class OfflineTrack(
    /** "local:<id>" oder "yt:<videoId>" – derselbe Schlüssel wie im Server */
    val key: String,
    val title: String,
    val artist: String,
    val album: String,
    val durationMs: Long,
    /** Dateiname im Offline-Ordner */
    val file: String,
    /** Dateiname des Covers (oder leer) */
    val artwork: String,
    val size: Long,
    val addedAt: Long,
) {
    fun toJson(): JSONObject = JSONObject()
        .put("key", key).put("title", title).put("artist", artist).put("album", album)
        .put("duration_ms", durationMs).put("file", file).put("artwork", artwork)
        .put("size", size).put("added_at", addedAt)

    companion object {
        fun fromJson(o: JSONObject) = OfflineTrack(
            key = o.getString("key"),
            title = o.optString("title"),
            artist = o.optString("artist"),
            album = o.optString("album"),
            durationMs = o.optLong("duration_ms"),
            file = o.getString("file"),
            artwork = o.optString("artwork"),
            size = o.optLong("size"),
            addedAt = o.optLong("added_at"),
        )
    }
}

/**
 * Die Offline-Bibliothek: Dateien in `files/offline/`, das Verzeichnis als `index.json`.
 * Alle Zugriffe sind synchronisiert (Downloads laufen in einem eigenen Thread).
 */
object OfflineStore {
    private var cache: MutableList<OfflineTrack>? = null

    fun dir(ctx: Context): File = File(ctx.filesDir, "offline").apply { mkdirs() }
    private fun index(ctx: Context) = File(dir(ctx), "index.json")

    @Synchronized
    fun all(ctx: Context): List<OfflineTrack> {
        cache?.let { return it.toList() }
        val list = mutableListOf<OfflineTrack>()
        val f = index(ctx)
        if (f.isFile) {
            runCatching {
                val arr = JSONArray(f.readText())
                for (i in 0 until arr.length()) {
                    val t = OfflineTrack.fromJson(arr.getJSONObject(i))
                    if (File(dir(ctx), t.file).isFile) list.add(t)
                }
            }
        }
        cache = list
        return list.toList()
    }

    @Synchronized
    fun has(ctx: Context, key: String) = all(ctx).any { it.key == key }

    @Synchronized
    fun keys(ctx: Context): List<String> = all(ctx).map { it.key }

    @Synchronized
    fun put(ctx: Context, t: OfflineTrack) {
        val list = all(ctx).filter { it.key != t.key }.toMutableList()
        list.add(0, t)
        save(ctx, list)
    }

    @Synchronized
    fun remove(ctx: Context, key: String): Boolean {
        val list = all(ctx).toMutableList()
        val t = list.firstOrNull { it.key == key } ?: return false
        File(dir(ctx), t.file).delete()
        if (t.artwork.isNotEmpty()) File(dir(ctx), t.artwork).delete()
        list.remove(t)
        save(ctx, list)
        return true
    }

    fun file(ctx: Context, t: OfflineTrack) = File(dir(ctx), t.file)
    fun artwork(ctx: Context, t: OfflineTrack) = if (t.artwork.isEmpty()) null else File(dir(ctx), t.artwork).takeIf { it.isFile }

    fun totalSize(ctx: Context) = all(ctx).sumOf { it.size }

    private fun save(ctx: Context, list: List<OfflineTrack>) {
        val arr = JSONArray()
        list.forEach { arr.put(it.toJson()) }
        val tmp = File(dir(ctx), "index.json.tmp")
        tmp.writeText(arr.toString())
        tmp.renameTo(index(ctx))
        cache = list.toMutableList()
    }

    /** Sicherer Dateiname aus dem Schlüssel ("yt:abc" → "yt_abc"). */
    fun safeName(key: String) = key.replace(Regex("[^A-Za-z0-9_-]"), "_").take(80)
}
