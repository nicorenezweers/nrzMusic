package ch.zweers.nrzmusic

import android.content.Intent
import android.webkit.JavascriptInterface
import org.json.JSONArray

/**
 * `window.NrzAndroid` für die Web-Oberfläche (siehe `ui/src/android.rs`). Antwortet nur,
 * solange die WebView eine Seite des eingestellten Servers zeigt.
 */
class Bridge(private val activity: MainActivity) {
    private val ctx = activity.applicationContext

    private fun trusted() = activity.trustedPage

    @JavascriptInterface
    fun version(): String = BuildConfig.VERSION_NAME

    /** Titel aufs Handy laden; `json` = Liste von {key, url, title, artist, album, duration_ms, artwork}. */
    @JavascriptInterface
    fun download(json: String) {
        if (!trusted()) return
        val items = runCatching { JSONArray(json) }.getOrNull() ?: return
        val ok = JSONArray()
        for (i in 0 until items.length()) {
            val o = items.optJSONObject(i) ?: continue
            // Audio nur vom eigenen Server
            if (Prefs.isServer(ctx, o.optString("url"))) ok.put(o)
        }
        if (ok.length() > 0) DownloadService.enqueue(ctx, ok.toString())
    }

    @JavascriptInterface
    fun remove(key: String) {
        if (!trusted()) return
        if (OfflineStore.remove(ctx, key)) activity.notifyOffline(key, "removed")
    }

    @JavascriptInterface
    fun offlineKeys(): String {
        if (!trusted()) return "[]"
        return JSONArray(OfflineStore.keys(ctx)).toString()
    }

    @JavascriptInterface
    fun openOffline() {
        activity.runOnUiThread {
            activity.startActivity(Intent(activity, OfflineActivity::class.java))
        }
    }

    /** Spotify-Anmeldung im eigenen Fenster; das Cookie kommt über `window.nrzAndroidSpotify`. */
    @JavascriptInterface
    fun spotifyLogin() {
        if (!trusted()) return
        activity.runOnUiThread { activity.openSpotifyLogin() }
    }

    /** Wiedergabe der Oberfläche: {playing, title, artist, artwork}. */
    @JavascriptInterface
    fun mediaState(json: String) {
        if (!trusted()) return
        KeepAliveService.update(ctx, json)
    }
}
