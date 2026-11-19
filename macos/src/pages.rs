//! Was die App selbst in den WebView gibt: das Skript für jede Seite der Oberfläche und die
//! Seiten „Verbinden“ und „Laden“ (ohne Server, auf Milchglas wie die Oberfläche).

/// Vor dem ersten Zeichnen jeder Seite der Oberfläche: `window.__nrzMac` (Modus und Server, siehe
/// `ui/src/mac.rs`; lokal ist, was vom eingebauten Server unter `local` kommt), die Klassen
/// `nrz-mac` und `nrz-glass` am `<html>` und [`NATIVE_CSS`] – das Aussehen der App kommt so aus der
/// App selbst und hängt nicht von der Version des Servers ab.
pub fn window_script(local: &str) -> String {
    let local = serde_json::Value::from(local);
    let css = serde_json::Value::from(NATIVE_CSS);
    format!(
        "window.__nrzMac = {{ mode: location.origin === {local} ? 'local' : 'server', server: location.origin }};\nwindow.__nrzMacCss = {css};\n{WINDOW_SCRIPT}"
    )
}

/// Wie eine native Mac-App: Milchglas statt deckender Flächen, Systemschrift (SF Pro), Pfeil statt
/// Hand, kein Markieren von Bedienelementen, kein Ziehen von Bildern, kein Fokusrahmen nach
/// Mausklicks, kein Gummiband am Fensterrand. Die oberste Reihe (Logo der Seitenleiste, Suche)
/// steht auf der Höhe der Ampel, das Logo rechts daneben (`--nrz-tl-y`/`--nrz-tl-r` misst die App,
/// siehe `lights`).
pub const NATIVE_CSS: &str = r#"
html.nrz-mac {
  --nrz-tl-y: 26px; --nrz-tl-r: 79px;
  --font-sans: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", sans-serif;
}
html.nrz-glass {
  --sidenav: oklch(0.22 0 0 / 0.26);
  --sidenav-border: oklch(1 0 0 / 0.08);
  --sidenav-accent: oklch(1 0 0 / 0.1);
  --player: oklch(0.12 0 0 / 0.42);
  --card: oklch(1 0 0 / 0.05);
  --surface: oklch(1 0 0 / 0.06);
  --secondary: oklch(1 0 0 / 0.08);
  --muted: oklch(1 0 0 / 0.07);
  --accent: oklch(1 0 0 / 0.1);
  --popover: oklch(0.2 0 0 / 0.86);
  --background: oklch(0.16 0 0 / 0.9);
  --border: oklch(1 0 0 / 0.1);
  --input: oklch(1 0 0 / 0.14);
}
html.nrz-glass, html.nrz-glass body, html.nrz-glass #main { background: transparent !important; }
/* Grund des Fensters: kaum getönt – der Schreibtisch scheint weichgezeichnet durch */
html.nrz-glass body { background: oklch(0.12 0 0 / 0.14) !important; }
/* Anmeldung und andere ganzseitige Flächen ebenfalls auf Glas */
html.nrz-glass .min-h-dvh.bg-background, html.nrz-glass .h-dvh.bg-background { background: transparent !important; }
html.nrz-glass .bg-popover, html.nrz-glass [role="dialog"], html.nrz-glass [role="alertdialog"], html.nrz-glass [role="menu"] {
  -webkit-backdrop-filter: blur(30px) saturate(180%); backdrop-filter: blur(30px) saturate(180%);
}
html.nrz-glass .bg-player { -webkit-backdrop-filter: blur(24px) saturate(160%); backdrop-filter: blur(24px) saturate(160%); }
html.nrz-glass ::-webkit-scrollbar { width: 9px; height: 9px; }
html.nrz-glass ::-webkit-scrollbar-thumb { background: oklch(1 0 0 / 0.2); border: 2px solid transparent; background-clip: padding-box; border-radius: 9999px; }
html.nrz-glass ::-webkit-scrollbar-track, html.nrz-glass ::-webkit-scrollbar-corner { background: transparent; }

/* Oberste Reihe auf der Höhe der Ampel, Logo und Name rechts daneben */
html.nrz-mac aside { padding-top: 0 !important; }
html.nrz-mac aside > a:first-child {
  height: calc(var(--nrz-tl-y) * 2) !important;
  padding-left: max(1.25rem, calc(var(--nrz-tl-r) + 14px)) !important;
  gap: 0.5rem !important;
}
html.nrz-mac aside > a:first-child img { width: 1.5rem; height: 1.5rem; border-radius: 0.375rem; }
html.nrz-mac header > div:first-child { height: calc(var(--nrz-tl-y) * 2) !important; }
html.nrz-mac header { border-bottom-color: transparent !important; }

/* Bedienung wie in AppKit */
html.nrz-mac, html.nrz-mac body { font-family: var(--font-sans) !important; overscroll-behavior: none; -webkit-font-smoothing: antialiased; }
html.nrz-mac body { -webkit-user-select: none; user-select: none; cursor: default; }
html.nrz-mac :is(input, textarea, [contenteditable="true"], [contenteditable=""], pre, code, .select-text) { -webkit-user-select: text; user-select: text; }
html.nrz-mac :is(a, button, summary, label, [role="button"], [role="link"], [role="tab"], [role="menuitem"], [role="option"], .cursor-pointer, input[type="range"], input[type="checkbox"]) { cursor: default !important; }
html.nrz-mac :is(input:not([type]), input[type="text"], input[type="search"], input[type="url"], input[type="email"], input[type="password"], textarea) { cursor: text !important; }
html.nrz-mac :is(img, svg, a, video, canvas) { -webkit-user-drag: none; }
html.nrz-mac :focus:not(:focus-visible) { outline: none !important; box-shadow: none; }
/* Update-Knopf unten in der Seitenleiste (über der Spielerleiste), siehe `window.__nrzUpdate` */
#nrz-update {
  position: fixed; left: 12px; bottom: 92px; z-index: 60; width: 216px; display: flex; align-items: center; gap: 10px;
  padding: 9px 12px; border-radius: 12px; border: 1px solid oklch(1 0 0 / 0.14);
  background: oklch(0.24 0 0 / 0.72); color: oklch(0.985 0 0); font: 500 12.5px/1.3 -apple-system, BlinkMacSystemFont, sans-serif;
  -webkit-backdrop-filter: blur(24px) saturate(170%); backdrop-filter: blur(24px) saturate(170%);
  box-shadow: 0 8px 24px rgb(0 0 0 / 0.3); cursor: default; text-align: left; -webkit-user-select: none; user-select: none;
  animation: nrz-up 260ms cubic-bezier(0.16, 1, 0.3, 1);
}
#nrz-update:hover { background: oklch(0.3 0 0 / 0.78); }
#nrz-update:disabled { opacity: 1; }
#nrz-update .dot { width: 8px; height: 8px; border-radius: 9999px; background: oklch(0.72 0.17 145); flex: none; box-shadow: 0 0 0 3px oklch(0.72 0.17 145 / 0.25); }
#nrz-update .txt { flex: 1; min-width: 0; }
#nrz-update .sub { display: block; color: oklch(0.75 0 0); font-weight: 400; font-size: 11.5px; }
#nrz-update .bar { position: absolute; left: 12px; right: 12px; bottom: 5px; height: 2px; border-radius: 2px; background: oklch(1 0 0 / 0.12); overflow: hidden; }
#nrz-update .bar i { display: block; height: 100%; background: oklch(0.95 0 0); width: 0; transition: width 200ms; }
@keyframes nrz-up { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: none; } }
"#;

const WINDOW_SCRIPT: &str = r#"
(function () {
  if (window.__nrzMacInit) return;
  window.__nrzMacInit = true;
  function send(m) { try { window.ipc.postMessage(m); } catch (e) {} }
  var de = document.documentElement;
  function style() {
    var s0 = document.getElementById('nrz-mac-css');
    if (s0) {
      // Immer zuletzt im <head>, damit die Regeln der App gewinnen
      if (document.head && (s0.parentNode !== document.head || s0 !== document.head.lastElementChild)) document.head.appendChild(s0);
      return;
    }
    var s = document.createElement('style'); s.id = 'nrz-mac-css'; s.textContent = window.__nrzMacCss;
    (document.head || de).appendChild(s);
  }
  function mark() {
    var c = de.classList;
    if (!c.contains('nrz-mac')) c.add('nrz-mac');
    if (!c.contains('nrz-glass')) c.add('nrz-glass');
  }
  function watch() {
    mark(); style();
    new MutationObserver(mark).observe(de, { attributes: true, attributeFilter: ['class'] });
    var head = function () {
      if (!document.head) return;
      new MutationObserver(style).observe(document.head, { childList: true });
      style();
    };
    if (document.head) head(); else document.addEventListener('DOMContentLoaded', head);
  }
  if (de) watch(); else document.addEventListener('DOMContentLoaded', function () { de = document.documentElement; watch(); });

  // Lage der Ampel (von der App gemessen); null = Vollbild ohne Ampel
  window.__nrzLights = function (y, r) {
    de.style.setProperty('--nrz-tl-y', (y || 26) + 'px');
    de.style.setProperty('--nrz-tl-r', (r || 0) + 'px');
  };

  // Fenster ziehen: oberste Reihe (Höhe der Ampel), nur ausserhalb von Bedienelementen
  var INTERACTIVE = 'button,a,input,select,textarea,label,[role=button],[role=slider],[role=menuitem],[role=tab],[contenteditable=true],iframe,video,#yt-host';
  window.addEventListener('mousedown', function (e) {
    if (e.button !== 0 || !e.target.closest || e.target.closest(INTERACTIVE)) return;
    var y = parseFloat(getComputedStyle(de).getPropertyValue('--nrz-tl-y')) || 26;
    if (e.clientY > y * 2 && !e.target.closest('[data-drag]')) return;
    send(e.detail === 2 ? 'window:zoom' : 'window:drag');
  }, true);

  // Links zu anderen Seiten öffnet der Browser des Systems
  document.addEventListener('click', function (e) {
    var a = e.target.closest && e.target.closest('a[href]');
    if (!a) return;
    var u;
    try { u = new URL(a.getAttribute('href'), location.href); } catch (_) { return; }
    if (u.origin === location.origin || !/^(https?|mailto):$/.test(u.protocol)) return;
    e.preventDefault();
    send('open:' + u.href);
  }, true);

  // Kein Kontextmenü des Browsers (ausser in Feldern)
  window.addEventListener('contextmenu', function (e) {
    if (!(e.target.closest && e.target.closest('input,textarea,[contenteditable=true]'))) e.preventDefault();
  });

  // Mediensteuerung aus dem Menü „Steuerung“: die Aktionen, die die Oberfläche bei
  // navigator.mediaSession anmeldet (dieselben wie für Medientasten und Kontrollzentrum)
  var actions = {};
  var ms = navigator.mediaSession;
  if (ms && ms.setActionHandler) {
    var orig = ms.setActionHandler.bind(ms);
    ms.setActionHandler = function (name, fn) {
      actions[name] = fn;
      try { return orig(name, fn); } catch (e) {}
    };
  }
  window.__nrzMedia = function (cmd) {
    var run = function (n, d) { if (actions[n]) { actions[n](d || { action: n }); return true; } return false; };
    if (cmd === 'toggle') {
      var playing = ms && ms.playbackState === 'playing';
      if (!run(playing ? 'pause' : 'play')) {
        document.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true }));
      }
    } else if (cmd === 'next') run('nexttrack');
    else if (cmd === 'prev') run('previoustrack');
    else if (cmd === 'forward') run('seekforward', { action: 'seekforward', seekOffset: 10 });
    else if (cmd === 'backward') run('seekbackward', { action: 'seekbackward', seekOffset: 10 });
  };

  // Menü: Suchen (⌘F) und Einstellungen (⌘,)
  window.__nrzFocusSearch = function () {
    var i = document.querySelector('header input[type=search], header input');
    if (i) { i.focus(); i.select(); }
  };
  window.__nrzOpenSettings = function () {
    var a = document.querySelector('a[href="/settings"]');
    if (a) a.click(); else location.href = '/settings';
  };
  // Update-Knopf (die App lädt Updates selbst und meldet den Stand):
  // state = 'progress' (text = Version, p = 0…1), 'ready' (Version), 'installing', sonst weg
  window.__nrzUpdate = function (state, version, p) {
    var b = document.getElementById('nrz-update');
    if (!state || state === 'none') { if (b) b.remove(); return; }
    if (!document.body) { document.addEventListener('DOMContentLoaded', function () { window.__nrzUpdate(state, version, p); }); return; }
    if (!b) {
      b = document.createElement('button'); b.id = 'nrz-update'; b.type = 'button';
      b.innerHTML = '<span class="dot"></span><span class="txt"></span>';
      b.addEventListener('click', function () { if (b.dataset.state === 'ready') send('update:install'); });
      document.body.appendChild(b);
    }
    b.dataset.state = state;
    var t = b.querySelector('.txt');
    var bar = b.querySelector('.bar');
    if (state === 'progress') {
      t.innerHTML = 'Update ' + version + '<span class="sub">wird geladen …</span>';
      if (!bar) { bar = document.createElement('span'); bar.className = 'bar'; bar.innerHTML = '<i></i>'; b.appendChild(bar); }
      bar.firstChild.style.width = (p >= 0 ? Math.round(p * 100) : 30) + '%';
      b.disabled = true; b.title = '';
    } else {
      if (bar) bar.remove();
      if (state === 'installing') { t.innerHTML = 'Update ' + version + '<span class="sub">wird installiert …</span>'; b.disabled = true; }
      else { t.innerHTML = 'Update auf ' + version + '<span class="sub">Klicken zum Neustarten</span>'; b.disabled = false; b.title = 'nrzMusic ' + version + ' installieren und neu starten'; }
    }
  };
})();
"#;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const STYLE: &str = r#"
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  html, body { margin: 0; height: 100%; background: oklch(0.14 0 0 / 28%); color: oklch(0.985 0 0);
    font: 13px/1.5 -apple-system, BlinkMacSystemFont, "Inter", sans-serif; -webkit-font-smoothing: antialiased;
    -webkit-user-select: none; user-select: none; cursor: default; overflow: hidden; }
  .drag { position: fixed; inset: 0 0 auto 0; height: 40px; }
  main { height: 100%; display: grid; place-items: center; padding: 48px 24px 24px; }
  .box { width: 100%; max-width: 440px; }
  .logo { display: flex; align-items: center; gap: 12px; margin-bottom: 28px; }
  .logo img { width: 44px; height: 44px; border-radius: 11px; box-shadow: 0 6px 18px rgb(0 0 0 / .35); }
  .logo b { font-size: 22px; letter-spacing: -.02em; }
  .card { border: 1px solid oklch(1 0 0 / 12%); background: oklch(1 0 0 / 5%); border-radius: 14px; padding: 18px 20px; margin-bottom: 12px; }
  .card h2 { margin: 0 0 4px; font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .tag { font-size: 11px; font-weight: 500; padding: 1px 8px; border-radius: 99px; background: oklch(1 0 0 / 12%); color: oklch(0.85 0 0); }
  p { margin: 0 0 14px; color: oklch(0.72 0 0); }
  form { display: flex; gap: 8px; }
  input { flex: 1; min-width: 0; height: 34px; padding: 0 12px; border-radius: 9px; border: 1px solid oklch(1 0 0 / 15%);
    background: oklch(0 0 0 / 25%); color: inherit; font: inherit; outline: none; -webkit-user-select: text; user-select: text; }
  input:focus { border-color: oklch(1 0 0 / 35%); box-shadow: 0 0 0 3px oklch(1 0 0 / 8%); }
  button { height: 34px; padding: 0 14px; border-radius: 9px; border: 1px solid oklch(1 0 0 / 14%); background: oklch(1 0 0 / 8%);
    color: inherit; font: inherit; font-weight: 500; white-space: nowrap; }
  button:hover { background: oklch(1 0 0 / 13%); }
  button.primary { background: oklch(0.95 0 0); color: oklch(0.2 0 0); border-color: transparent; }
  button.primary:hover { background: oklch(0.88 0 0); }
  button:disabled { opacity: .5; }
  .err { color: oklch(0.75 0.15 25); margin: 10px 0 0; min-height: 1em; }
  .foot { display: flex; justify-content: flex-end; margin-top: 8px; }
  .spin { width: 22px; height: 22px; border-radius: 50%; border: 2px solid oklch(1 0 0 / 20%); border-top-color: oklch(1 0 0 / 85%);
    animation: s .8s linear infinite; margin: 0 auto 14px; }
  @keyframes s { to { transform: rotate(360deg); } }
  code { font: 12px ui-monospace, Menlo, monospace; color: oklch(0.85 0 0); }
"#;

const DRAG_SCRIPT: &str = r#"
  function send(m) { try { window.ipc.postMessage(m); } catch (e) {} }
  window.addEventListener('mousedown', function (e) {
    if (e.button !== 0 || e.target.closest('button,input,a,form')) return;
    send(e.detail === 2 ? 'window:zoom' : 'window:drag');
  });
  window.addEventListener('contextmenu', function (e) { if (!e.target.closest('input')) e.preventDefault(); });
"#;

/// Erster Start und „Server wechseln …“: mit einer Server-Instanz verbinden (empfohlen) oder
/// ohne Server auf diesem Mac. `cancel`: es läuft schon etwas, „Abbrechen“ kehrt dorthin zurück.
pub fn connect_page(icon: &str, server: &str, error: &str, cancel: bool, music_dir: &str) -> String {
    let cancel = if cancel { r#"<div class="foot"><button type="button" onclick="send('connect:cancel')">Abbrechen</button></div>"# } else { "" };
    format!(
        r#"<!doctype html><html lang="de"><head><meta charset="utf-8"><title>nrzMusic</title><style>{STYLE}</style></head>
<body><div class="drag"></div><main><div class="box">
  <div class="logo"><img src="{icon}" alt=""><b>nrzMusic</b></div>
  <div class="card">
    <h2>Mit deinem Server verbinden <span class="tag">empfohlen</span></h2>
    <p>Streame Bibliothek, Playlists, Downloads und YouTube Music deiner nrzMusic-Instanz – alles läuft dort, der Mac spielt nur ab.</p>
    <form id="f"><input id="u" placeholder="https://music.example.ch oder 192.168.1.5:8350" value="{server}" autofocus spellcheck="false" autocapitalize="off" autocomplete="url">
      <button class="primary" id="b">Verbinden</button></form>
    <div class="err" id="e">{error}</div>
  </div>
  <div class="card">
    <h2>Ohne Server nutzen</h2>
    <p>nrzMusic läuft ganz auf diesem Mac und spielt die Musik aus <code>{music_dir}</code>. YouTube Music und Downloads brauchen <code>yt-dlp</code>, <code>ffmpeg</code> und <code>deno</code> (Homebrew).</p>
    <button type="button" onclick="send('connect:local')">Lokal starten</button>
  </div>
  {cancel}
</div></main>
<script>
{DRAG_SCRIPT}
  var f = document.getElementById('f'), u = document.getElementById('u'), b = document.getElementById('b'), e = document.getElementById('e');
  f.addEventListener('submit', function (ev) {{
    ev.preventDefault();
    if (!u.value.trim()) {{ u.focus(); return; }}
    b.disabled = true; b.textContent = 'Verbinde …'; e.textContent = '';
    send('connect:server:' + u.value.trim());
  }});
  window.__nrzConnectError = function (msg) {{ b.disabled = false; b.textContent = 'Verbinden'; e.textContent = msg; }};
  u.select();
</script></body></html>"#,
        server = esc(server),
        error = esc(error),
        music_dir = esc(music_dir),
    )
}

/// Während der eingebaute Server startet bzw. die Verbindung geprüft wird.
pub fn loading_page(icon: &str, text: &str) -> String {
    format!(
        r#"<!doctype html><html lang="de"><head><meta charset="utf-8"><title>nrzMusic</title><style>{STYLE}</style></head>
<body><div class="drag"></div><main><div class="box" style="text-align:center">
  <div class="spin"></div><p>{text}</p>
  <img src="{icon}" alt="" style="width:0;height:0">
</div></main><script>{DRAG_SCRIPT}</script></body></html>"#,
        text = esc(text),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_escaped() {
        let page = connect_page("data:,", "\"><script>", "<b>", true, "~/Music");
        assert!(!page.contains("\"><script>\""));
        assert!(page.contains("&quot;&gt;&lt;script&gt;"));
        assert!(page.contains("connect:cancel"));
        assert!(!connect_page("data:,", "", "", false, "").contains("connect:cancel"));
    }

    #[test]
    fn script_knows_local_origin() {
        let s = window_script("http://127.0.0.1:18350");
        assert!(s.starts_with(r#"window.__nrzMac = { mode: location.origin === "http://127.0.0.1:18350" ? 'local'"#));
    }
}
