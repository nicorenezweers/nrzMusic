package ch.zweers.nrzmusic

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.app.DownloadManager
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.os.Message
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import java.lang.ref.WeakReference

/**
 * nrzMusic in einer WebView: die Oberfläche kommt vom Server (Standard music.zweers.ch),
 * angemeldet wird dort mit dem eigenen Konto. Dazu kommen die Brücke [Bridge]
 * (aufs Handy laden, Benachrichtigung), Hochladen, Downloads und eine Ansicht ohne
 * Verbindung mit dem Weg in die Offline-Bibliothek.
 */
class MainActivity : Activity() {
    private lateinit var web: WebView
    private lateinit var offlinePane: View
    private lateinit var offlineText: TextView
    private var fileCallback: ValueCallback<Array<Uri>>? = null

    /** Zeigt die WebView gerade eine Seite des eigenen Servers? (für die Brücke) */
    @Volatile
    var trustedPage = false

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        current = WeakReference(this)
        KeepAliveService.ensureChannel(this)
        DownloadService.ensureChannel(this)

        web = WebView(this).apply {
            setBackgroundColor(Color.parseColor("#171717"))
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.mediaPlaybackRequiresUserGesture = false
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            settings.setSupportMultipleWindows(true)
            settings.javaScriptCanOpenWindowsAutomatically = true
            settings.userAgentString = settings.userAgentString + " nrzMusicAndroid/" + BuildConfig.VERSION_NAME
            addJavascriptInterface(Bridge(this@MainActivity), "NrzAndroid")
            webViewClient = Client()
            webChromeClient = Chrome()
            setDownloadListener { url, ua, disposition, mime, _ -> systemDownload(url, ua, disposition, mime) }
        }
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, false)

        offlineText = TextView(this).apply {
            setTextColor(Color.parseColor("#E5E5E5"))
            textSize = 18f
            gravity = Gravity.CENTER
        }
        val hint = TextView(this).apply {
            setTextColor(Color.parseColor("#A3A3A3"))
            textSize = 14f
            gravity = Gravity.CENTER
            text = getString(R.string.no_connection_hint)
            setPadding(0, dp(8), 0, dp(24))
        }
        fun button(label: Int, onClick: () -> Unit) = Button(this).apply {
            text = getString(label)
            setOnClickListener { onClick() }
        }
        offlinePane = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(dp(32), dp(32), dp(32), dp(32))
            setBackgroundColor(Color.parseColor("#171717"))
            visibility = View.GONE
            addView(offlineText)
            addView(hint)
            addView(button(R.string.open_offline) { startActivity(Intent(this@MainActivity, OfflineActivity::class.java)) })
            addView(button(R.string.retry) { reload() })
            addView(button(R.string.change_server) { askServer() })
        }
        setContentView(FrameLayout(this).apply {
            addView(web, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            addView(offlinePane, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        })

        DownloadService.listener = { key, state -> notifyOffline(key, state) }

        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }

        val start = intent?.data?.toString()?.takeIf { Prefs.isServer(this, it) } ?: Prefs.server(this)
        if (savedInstanceState != null) web.restoreState(savedInstanceState) else web.loadUrl(start)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.data?.toString()?.takeIf { Prefs.isServer(this, it) }?.let { web.loadUrl(it) }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        web.saveState(outState)
    }

    @Deprecated("Activity.onBackPressed")
    override fun onBackPressed() {
        if (offlinePane.visibility == View.VISIBLE) {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        } else if (web.canGoBack()) {
            web.goBack()
        } else {
            // Nicht beenden – sonst endet die Wiedergabe; nur in den Hintergrund
            moveTaskToBack(true)
        }
    }

    override fun onDestroy() {
        if (current?.get() === this) current = null
        DownloadService.listener = null
        KeepAliveService.stop(this)
        web.destroy()
        super.onDestroy()
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private fun reload() {
        offlinePane.visibility = View.GONE
        web.loadUrl(Prefs.server(this))
    }

    private fun showOffline() {
        offlineText.text = getString(R.string.no_connection, Uri.parse(Prefs.server(this)).host ?: "")
        offlinePane.visibility = View.VISIBLE
    }

    private fun askServer() {
        val input = EditText(this).apply {
            setText(Prefs.server(this@MainActivity))
            setSingleLine()
        }
        AlertDialog.Builder(this)
            .setTitle(R.string.server_prompt)
            .setView(input)
            .setPositiveButton(R.string.save) { _, _ ->
                if (Prefs.setServer(this, input.text.toString())) reload()
            }
            .setNegativeButton(R.string.cancel, null)
            .show()
    }

    fun openSpotifyLogin() {
        @Suppress("DEPRECATION")
        startActivityForResult(Intent(this, SpotifyLoginActivity::class.java), SPOTIFY_REQUEST)
    }

    /** Fortschritt der Offline-Downloads an die Oberfläche. */
    fun notifyOffline(key: String, state: String) {
        val payload = org.json.JSONObject().put("key", key).put("state", state).toString()
        runOnUiThread {
            web.evaluateJavascript("window.nrzAndroidOffline && window.nrzAndroidOffline(${org.json.JSONObject.quote(payload)})", null)
        }
    }

    private fun systemDownload(url: String, ua: String, disposition: String?, mime: String?) {
        runCatching {
            val req = DownloadManager.Request(Uri.parse(url))
                .setMimeType(mime)
                .addRequestHeader("User-Agent", ua)
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
                .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, URLUtil.guessFileName(url, disposition, mime))
            if (Prefs.isServer(this, url)) {
                CookieManager.getInstance().getCookie(url)?.let { req.addRequestHeader("Cookie", it) }
            }
            getSystemService(DownloadManager::class.java).enqueue(req)
        }
    }

    /** Darf in der App bleiben: eigener Server und Spotifys Anmeldung (sie leitet zurück). */
    private fun stayInApp(url: String): Boolean {
        if (Prefs.isServer(this, url)) return true
        val host = Uri.parse(url).host ?: return false
        return host == "accounts.spotify.com" || host == "challenge.spotify.com"
    }

    private fun openExternal(url: String) {
        runCatching { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
    }

    private inner class Client : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val url = request.url.toString()
            if (stayInApp(url)) return false
            openExternal(url)
            return true
        }

        override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
            trustedPage = Prefs.isServer(this@MainActivity, url)
        }

        override fun onPageFinished(view: WebView, url: String?) {
            trustedPage = Prefs.isServer(this@MainActivity, url)
        }

        override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
            if (request.isForMainFrame) showOffline()
        }
    }

    private inner class Chrome : WebChromeClient() {
        override fun onShowFileChooser(view: WebView, callback: ValueCallback<Array<Uri>>, params: FileChooserParams): Boolean {
            fileCallback?.onReceiveValue(null)
            fileCallback = callback
            return try {
                startActivityForResult(params.createIntent().putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true), FILE_REQUEST)
                true
            } catch (_: Exception) {
                fileCallback = null
                false
            }
        }

        /** `window.open`: eigene Seiten in der App, alles andere im Browser. */
        override fun onCreateWindow(view: WebView, isDialog: Boolean, isUserGesture: Boolean, resultMsg: Message): Boolean {
            val probe = WebView(this@MainActivity)
            probe.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(v: WebView, request: WebResourceRequest): Boolean {
                    val url = request.url.toString()
                    if (stayInApp(url)) web.loadUrl(url) else openExternal(url)
                    v.destroy()
                    return true
                }
            }
            (resultMsg.obj as WebView.WebViewTransport).webView = probe
            resultMsg.sendToTarget()
            return true
        }
    }

    @Deprecated("Activity.onActivityResult")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == FILE_REQUEST) {
            val cb = fileCallback
            fileCallback = null
            if (cb == null) return
            val uris = mutableListOf<Uri>()
            if (resultCode == RESULT_OK && data != null) {
                data.clipData?.let { clip -> for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri) }
                if (uris.isEmpty()) data.data?.let { uris.add(it) }
            }
            cb.onReceiveValue(if (uris.isEmpty()) null else uris.toTypedArray())
            return
        }
        if (requestCode == SPOTIFY_REQUEST) {
            // Das Cookie nur an eine Seite des eigenen Servers; abgebrochen = null
            val cookie = data?.getStringExtra(SpotifyLoginActivity.EXTRA_COOKIE)?.takeIf { resultCode == RESULT_OK }
            if (!trustedPage) return
            val arg = if (cookie == null) "null" else org.json.JSONObject.quote(cookie)
            web.evaluateJavascript("window.nrzAndroidSpotify && window.nrzAndroidSpotify($arg)", null)
            return
        }
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
    }

    companion object {
        private const val FILE_REQUEST = 7
        private const val SPOTIFY_REQUEST = 8
        private var current: WeakReference<MainActivity>? = null

        /** Knopf der Benachrichtigung/Kopfhörer an die Oberfläche. */
        fun command(cmd: String) {
            val a = current?.get() ?: return
            val safe = cmd.filter { it.isLetter() }
            a.runOnUiThread {
                a.web.evaluateJavascript("window.nrzAndroidCommand && window.nrzAndroidCommand('$safe')", null)
            }
        }

        /** Offline-Wiedergabe beginnt: die Oberfläche anhalten. */
        fun pauseWeb() = command("pause")
    }
}
