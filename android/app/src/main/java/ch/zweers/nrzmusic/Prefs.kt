package ch.zweers.nrzmusic

import android.content.Context
import android.net.Uri

/** Einstellungen der App: nur die Adresse des Servers. */
object Prefs {
    private const val FILE = "nrzmusic"
    private const val KEY_SERVER = "server"

    fun server(ctx: Context): String {
        val saved = ctx.getSharedPreferences(FILE, Context.MODE_PRIVATE).getString(KEY_SERVER, null)
        return normalize(saved ?: ctx.getString(R.string.default_server))
            ?: ctx.getString(R.string.default_server)
    }

    fun setServer(ctx: Context, url: String): Boolean {
        val n = normalize(url) ?: return false
        ctx.getSharedPreferences(FILE, Context.MODE_PRIVATE).edit().putString(KEY_SERVER, n).apply()
        return true
    }

    /** "music.zweers.ch" → "https://music.zweers.ch", ohne Schrägstrich am Ende. */
    fun normalize(input: String): String? {
        var s = input.trim().trimEnd('/')
        if (s.isEmpty()) return null
        if (!s.startsWith("http://") && !s.startsWith("https://")) s = "https://$s"
        val uri = Uri.parse(s)
        if (uri.host.isNullOrBlank()) return null
        return s
    }

    /** Gehört die Adresse zum eingestellten Server? */
    fun isServer(ctx: Context, url: String?): Boolean {
        if (url == null) return false
        val a = Uri.parse(server(ctx))
        val b = Uri.parse(url)
        return a.scheme == b.scheme && a.host == b.host && a.port == b.port
    }
}
