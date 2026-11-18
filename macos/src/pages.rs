//! Was die App selbst in den WebView gibt: das Skript für jede Seite der Oberfläche und die
//! Seiten „Verbinden“ und „Laden“ (ohne Server, auf Milchglas wie die Oberfläche).

/// Vor dem ersten Zeichnen jeder Seite: `window.__nrzMac` (Modus und Server, siehe `ui/src/mac.rs`;
/// lokal ist, was vom eingebauten Server unter `local` kommt), die Klassen `nrz-mac` und
/// `nrz-glass` am `<html>` (Stil in `ui/src/theme.rs`, `GLASS`), das Fenster an der Kopfzeile
/// ziehen (Doppelklick zoomt) und Links zu fremden Seiten im Browser.
pub fn window_script(local: &str) -> String {
    let local = serde_json::Value::from(local);
    format!(
        "window.__nrzMac = {{ mode: location.origin === {local} ? 'local' : 'server', server: location.origin }};\n{WINDOW_SCRIPT}"
    )
}

const WINDOW_SCRIPT: &str = r#"
(function () {
  if (window.__nrzMacInit) return;
  window.__nrzMacInit = true;
  function send(m) { try { window.ipc.postMessage(m); } catch (e) {} }
  function mark() {
    var c = document.documentElement.classList;
    if (!c.contains('nrz-mac')) c.add('nrz-mac');
    if (!c.contains('nrz-glass')) c.add('nrz-glass');
  }
  function watch() {
    mark();
    new MutationObserver(mark).observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });
  }
  if (document.documentElement) watch(); else document.addEventListener('DOMContentLoaded', watch);

  // Fenster ziehen: Kopfzeile und Streifen oben (Ampel), nur ausserhalb von Bedienelementen
  var INTERACTIVE = 'button,a,input,select,textarea,label,[role=button],[role=slider],[role=menuitem],[contenteditable=true],iframe,video';
  window.addEventListener('mousedown', function (e) {
    if (e.button !== 0 || !e.target.closest || e.target.closest(INTERACTIVE)) return;
    if (e.clientY > 56 && !e.target.closest('[data-drag]')) return;
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

  // Kein Kontextmenü des Browsers (ausser in Feldern), kein Gummiband
  window.addEventListener('contextmenu', function (e) {
    if (!(e.target.closest && e.target.closest('input,textarea,[contenteditable=true]'))) e.preventDefault();
  });
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
