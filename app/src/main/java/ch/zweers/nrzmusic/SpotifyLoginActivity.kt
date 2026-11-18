package ch.zweers.nrzmusic

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView

/**
 * Anmeldung bei Spotify in einem eigenen Fenster der App: die Anmeldeseite von Spotify in
 * einer WebView, ohne Brücke zur Oberfläche. Sobald Spotify das Cookie `sp_dc` setzt, liest
 * die App es aus dem Cookie-Speicher (auch HttpOnly), gibt es an [MainActivity] zurück und
 * löscht die Spotify-Cookies wieder – das Cookie liegt danach nur noch auf dem Server.
 */
class SpotifyLoginActivity : Activity() {
    private lateinit var web: WebView
    private lateinit var progress: ProgressBar
    private var done = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Frisch anmelden: alte Spotify-Cookies (anderes Konto, abgelaufen) weg
        clearSpotifyCookies {}

        web = WebView(this).apply {
            setBackgroundColor(Color.parseColor("#121212"))
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            // Ohne "; wv" lässt Spotify mehr Anmeldewege zu
            settings.userAgentString = settings.userAgentString.replace("; wv", "")
            webViewClient = Client()
        }
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, true)

        val title = TextView(this).apply {
            text = getString(R.string.spotify_login_title)
            setTextColor(Color.parseColor("#E5E5E5"))
            textSize = 16f
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        }
        val close = Button(this).apply {
            text = getString(R.string.cancel)
            setOnClickListener { finishWith(null) }
        }
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(16), dp(4), dp(8), dp(4))
            setBackgroundColor(Color.parseColor("#171717"))
            addView(title)
            addView(close)
        }
        progress = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
            isIndeterminate = true
            visibility = View.GONE
        }
        val body = FrameLayout(this).apply {
            addView(web, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            addView(progress, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(4), Gravity.TOP))
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#171717"))
            addView(bar)
            addView(body, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        })

        web.loadUrl(LOGIN_URL)
    }

    @Deprecated("Activity.onBackPressed")
    override fun onBackPressed() {
        if (web.canGoBack()) web.goBack() else finishWith(null)
    }

    override fun onDestroy() {
        web.destroy()
        super.onDestroy()
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    /** Steht `sp_dc` schon im Cookie-Speicher? */
    private fun spDc(): String? {
        val cm = CookieManager.getInstance()
        for (url in COOKIE_URLS) {
            val line = cm.getCookie(url) ?: continue
            for (part in line.split(';')) {
                val (name, value) = part.trim().split('=', limit = 2).takeIf { it.size == 2 } ?: continue
                if (name == "sp_dc" && value.isNotBlank()) return value
            }
        }
        return null
    }

    private fun check() {
        if (done) return
        val cookie = spDc() ?: return
        finishWith(cookie)
    }

    private fun finishWith(cookie: String?) {
        if (done) return
        done = true
        if (cookie != null) setResult(RESULT_OK, Intent().putExtra(EXTRA_COOKIE, cookie)) else setResult(RESULT_CANCELED)
        clearSpotifyCookies { finish() }
    }

    /** Nur die Cookies von Spotify löschen (die Sitzung bei nrzMusic bleibt). */
    private fun clearSpotifyCookies(then: () -> Unit) {
        val cm = CookieManager.getInstance()
        for (url in COOKIE_URLS) {
            val line = cm.getCookie(url) ?: continue
            for (part in line.split(';')) {
                val name = part.trim().substringBefore('=').takeIf { it.isNotEmpty() } ?: continue
                for (domain in listOf(".spotify.com", Uri.parse(url).host)) {
                    cm.setCookie(url, "$name=; Max-Age=0; Path=/; Domain=$domain")
                }
                cm.setCookie(url, "$name=; Max-Age=0; Path=/")
            }
        }
        cm.flush()
        then()
    }

    private inner class Client : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val url = request.url
            // Nur Spotify-Seiten über https; "spotify:"/"intent:" (App öffnen) und fremde Seiten nicht
            val host = url.host ?: return true
            val spotify = host == "spotify.com" || host.endsWith(".spotify.com")
            return !(url.scheme == "https" && (spotify || SIGN_IN_HOSTS.any { host == it || host.endsWith(".$it") }))
        }

        override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
            progress.visibility = View.VISIBLE
            check()
        }

        override fun onPageFinished(view: WebView, url: String?) {
            progress.visibility = View.GONE
            check()
        }
    }

    companion object {
        const val EXTRA_COOKIE = "sp_dc"
        private const val LOGIN_URL = "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F"
        private val COOKIE_URLS = listOf("https://open.spotify.com", "https://accounts.spotify.com", "https://www.spotify.com")

        /** Anmelden mit Google, Apple oder Facebook führt über diese Seiten. */
        private val SIGN_IN_HOSTS = listOf("accounts.google.com", "appleid.apple.com", "facebook.com", "spotifycdn.com", "scdn.co")
    }
}
