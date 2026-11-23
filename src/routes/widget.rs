//! Jetzt läuft – öffentlich über die geheime Adresse eines Kontos (nur wenn es das in den
//! Einstellungen eingeschaltet hat):
//!
//! - `GET /widget/<token>`: eigenständige Seite zum Einbetten (iframe, Notion, OBS, Homepage …):
//!   Cover, Titel, Künstler, Fortschritt auf die Sekunde und „Hört auf <Gerät>“. Fragt alle
//!   5 Sekunden den Stand ab und zählt dazwischen selbst weiter. Optionen in der Adresse:
//!   `?theme=light`, `?lang=en`, `?compact=1`.
//! - `POST /mcp/<token>`: MCP-Server (Streamable HTTP, JSON-RPC 2.0) mit dem Werkzeug
//!   `now_playing` – damit kann ein KI-Assistent fragen, was gerade läuft.
//!
//! Beides liegt ausserhalb von `/api`, die Anmeldung (`auth::Gate`) greift hier nicht; ohne
//! gültiges, öffentliches Token antworten beide mit 404.

use crate::config::Config;
use crate::db::presence;
use crate::error::AppError;
use crate::routes::spa::HostHeader;
use rocket::http::{ContentType, Header, Status};
use rocket::serde::json::{json, Json, Value};
use rocket::{get, post, routes, Responder, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![widget, mcp, mcp_get]
}

#[derive(Responder)]
#[response(content_type = "html")]
pub struct WidgetPage {
    body: String,
    csp: Header<'static>,
    frame: Header<'static>,
    cache: Header<'static>,
}

/// Eigene Sicherheitsregeln: einbettbar überall, Skript und Stil nur inline, Bilder von überall
/// (Cover von YouTube), Daten nur vom eigenen Server.
const WIDGET_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
    img-src 'self' data: https:; connect-src 'self'; frame-ancestors *; base-uri 'none'; form-action 'none'";

#[get("/widget/<token>")]
async fn widget(token: &str, host: HostHeader, pool: &State<SqlitePool>, config: &State<Config>) -> Result<WidgetPage, Status> {
    // Unbekannt oder nicht öffentlich: 404 (und nichts verraten)
    presence::public(pool, token, &host.base(config)).await.map_err(|_| Status::NotFound)?;
    Ok(WidgetPage {
        body: WIDGET_HTML.replace("__TOKEN__", token),
        csp: Header::new("Content-Security-Policy", WIDGET_CSP),
        // Ungültiger Wert = keine Sperre (die Sicherheits-Fairing setzt sonst SAMEORIGIN);
        // massgeblich ist ohnehin `frame-ancestors *`
        frame: Header::new("X-Frame-Options", "ALLOWALL"),
        cache: Header::new("Cache-Control", "no-cache"),
    })
}

const PROTOCOL_VERSION: &str = "2025-06-18";

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[derive(Responder)]
pub enum McpReply {
    Json(Json<Value>),
    /// Benachrichtigung ohne Antwort
    Accepted(Status),
}

/// MCP über HTTP: eine JSON-RPC-Nachricht pro POST, Antwort als JSON.
#[post("/mcp/<token>", data = "<body>")]
async fn mcp(
    token: &str,
    body: Json<Value>,
    host: HostHeader,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> Result<McpReply, Status> {
    let base = host.base(config);
    // Token zuerst prüfen – sonst 404 wie beim Widget
    let state = match presence::public(pool, token, &base).await {
        Ok(s) => s,
        Err(AppError::NotFound) => return Err(Status::NotFound),
        Err(_) => return Err(Status::InternalServerError),
    };
    let msg = body.into_inner();
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    if msg.get("id").is_none() {
        // notifications/initialized & Co.
        return Ok(McpReply::Accepted(Status::Accepted));
    }
    let reply = match method {
        "initialize" => {
            let version = msg
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION)
                .to_string();
            rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "nrzMusic", "title": "nrzMusic – Jetzt läuft", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Use the now_playing tool to see which song this nrzMusic user is listening to right now, on which device, and how far into the song they are."
                }),
            )
        }
        "ping" => rpc_result(&id, json!({})),
        "tools/list" => rpc_result(
            &id,
            json!({ "tools": [{
                "name": "now_playing",
                "title": "Jetzt läuft",
                "description": "Returns the song this nrzMusic user is listening to right now: title, artist, album, cover image URL, position and length (to the second), whether it is playing or paused, and the device (e.g. MacBook, iPhone, Chrome).",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
                "annotations": { "readOnlyHint": true, "openWorldHint": false }
            }]}),
        ),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or_default();
            if name != "now_playing" {
                rpc_error(&id, -32602, &format!("Unknown tool: {name}"))
            } else {
                let text = if !state.active {
                    format!("{} is not listening to anything right now.", state.user)
                } else {
                    format!(
                        "{} is {} \"{}\" by {}{} – {} / {} – on {}.{}",
                        state.user,
                        if state.playing { "listening to" } else { "paused on" },
                        state.title,
                        state.artist,
                        state.album.as_deref().map(|a| format!(" (album: {a})")).unwrap_or_default(),
                        clock(state.position_ms),
                        clock(state.duration_ms),
                        if state.device_name.is_empty() { "an unknown device" } else { &state.device_name },
                        state.artwork.as_deref().map(|a| format!(" Cover: {a}")).unwrap_or_default(),
                    )
                };
                let mut content = vec![json!({ "type": "text", "text": text })];
                if let Some(a) = &state.artwork {
                    content.push(json!({ "type": "resource_link", "uri": a, "name": "cover", "mimeType": "image/jpeg" }));
                }
                rpc_result(
                    &id,
                    json!({
                        "content": content,
                        "structuredContent": state,
                        "isError": false
                    }),
                )
            }
        }
        _ => rpc_error(&id, -32601, &format!("Method not found: {method}")),
    };
    Ok(McpReply::Json(Json(reply)))
}

/// Kein Ereignisstrom vom Server (SSE) – nach MCP-Spezifikation 405.
#[get("/mcp/<_token>")]
fn mcp_get(_token: &str) -> (Status, (ContentType, &'static str)) {
    (Status::MethodNotAllowed, (ContentType::Plain, "Use POST (MCP Streamable HTTP, JSON responses)."))
}

/// Die Widget-Seite: kein Framework, ein Stück HTML mit Stil und Skript.
const WIDGET_HTML: &str = r##"<!DOCTYPE html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>nrzMusic – Jetzt läuft</title>
<style>
:root{--fg:#fff;--muted:rgba(255,255,255,.62);--card:rgba(24,24,27,.78);--line:rgba(255,255,255,.14);--track:rgba(255,255,255,.18);--accent:#4ade80;--shadow:0 12px 40px rgba(0,0,0,.35)}
.light{--fg:#111;--muted:rgba(0,0,0,.55);--card:rgba(255,255,255,.82);--line:rgba(0,0,0,.08);--track:rgba(0,0,0,.12);--accent:#16a34a;--shadow:0 12px 40px rgba(0,0,0,.12)}
*{box-sizing:border-box;margin:0;padding:0}
html,body{background:transparent;height:100%}
body{font:14px/1.35 -apple-system,BlinkMacSystemFont,"SF Pro Text","Segoe UI",Inter,Roboto,Helvetica,Arial,sans-serif;color:var(--fg);display:flex;align-items:center;justify-content:center;padding:8px;-webkit-font-smoothing:antialiased}
.card{position:relative;width:100%;max-width:440px;border-radius:22px;overflow:hidden;background:var(--card);border:1px solid var(--line);box-shadow:var(--shadow);isolation:isolate}
.bg{position:absolute;inset:-40px;background-size:cover;background-position:center;filter:blur(40px) saturate(1.5);opacity:.55;z-index:-1;transition:background-image .6s}
.light .bg{opacity:.35}
.main{display:flex;gap:14px;padding:14px;align-items:center}
.art{width:84px;height:84px;border-radius:14px;object-fit:cover;flex-shrink:0;background:var(--track);box-shadow:0 6px 18px rgba(0,0,0,.35)}
.meta{min-width:0;flex:1;display:flex;flex-direction:column;gap:2px}
.status{display:flex;align-items:center;gap:6px;font-size:11px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
.title{font-size:16px;font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:var(--fg);text-decoration:none}
a.title:hover{text-decoration:underline}
.artist{color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bar{height:4px;border-radius:4px;background:var(--track);margin-top:8px;overflow:hidden}
.fill{height:100%;width:0;background:var(--fg);border-radius:4px;transition:width .9s linear}
.times{display:flex;justify-content:space-between;font-size:11px;color:var(--muted);margin-top:4px;font-variant-numeric:tabular-nums}
.device{display:flex;align-items:center;gap:8px;padding:9px 14px;border-top:1px solid var(--line);font-size:13px;color:var(--accent);font-weight:600;background:rgba(0,0,0,.12)}
.light .device{background:rgba(0,0,0,.03)}
.device svg{width:16px;height:16px;flex-shrink:0}
.device .brand{margin-left:auto;color:var(--muted);font-weight:500;font-size:11px}
.eq{display:inline-flex;gap:2px;align-items:flex-end;height:11px}
.eq i{width:3px;background:var(--accent);border-radius:1px;height:30%;animation:eq 1s ease-in-out infinite}
.eq i:nth-child(2){animation-delay:-.4s}.eq i:nth-child(3){animation-delay:-.7s}
.paused .eq i{animation:none;height:30%;background:var(--muted)}
@keyframes eq{0%,100%{height:25%}50%{height:100%}}
.idle .main{opacity:.7}
.compact .art{width:56px;height:56px;border-radius:10px}.compact .main{padding:10px;gap:10px}.compact .title{font-size:14px}.compact .device{padding:7px 10px;font-size:12px}
.hidden{display:none!important}
</style>
</head>
<body>
<div class="card" id="card">
  <div class="bg" id="bg"></div>
  <div class="main">
    <img class="art" id="art" alt="" referrerpolicy="no-referrer">
    <div class="meta">
      <div class="status"><span class="eq"><i></i><i></i><i></i></span><span id="status"></span></div>
      <a class="title" id="title" target="_blank" rel="noopener"></a>
      <div class="artist" id="artist"></div>
      <div class="bar"><div class="fill" id="fill"></div></div>
      <div class="times"><span id="pos">0:00</span><span id="dur">0:00</span></div>
    </div>
  </div>
  <div class="device" id="device"><span id="icon"></span><span id="devtext"></span><span class="brand">nrzMusic</span></div>
</div>
<script>
(function(){
  var TOKEN="__TOKEN__";
  var q=new URLSearchParams(location.search);
  var en=(q.get("lang")||"").toLowerCase().indexOf("en")===0;
  var T=en?{playing:"Now playing",paused:"Paused",idle:"Not playing",on:"Listening on ",nothing:"Nothing playing right now",device:"a device"}
         :{playing:"Läuft gerade",paused:"Pausiert",idle:"Gerade still",on:"Hört auf ",nothing:"Gerade läuft nichts",device:"einem Gerät"};
  if(en)document.documentElement.lang="en";
  var card=document.getElementById("card");
  if(q.get("theme")==="light"||(q.get("theme")==="auto"&&matchMedia("(prefers-color-scheme: light)").matches))card.classList.add("light");
  if(q.get("compact")==="1")card.classList.add("compact");
  var ICONS={
    computer:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M2 20h20"/></svg>',
    phone:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="2" width="12" height="20" rx="3"/><path d="M11 18h2"/></svg>',
    tablet:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="2" width="16" height="20" rx="2"/><path d="M11 18h2"/></svg>',
    browser:'<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15 15 0 0 1 0 20a15 15 0 0 1 0-20"/></svg>'
  };
  var $=function(id){return document.getElementById(id)};
  var s=null,base=0,at=0;
  function clock(ms){var t=Math.max(0,Math.floor(ms/1000));return Math.floor(t/60)+":"+("0"+t%60).slice(-2)}
  function pos(){if(!s)return 0;var p=base+(s.playing?(Date.now()-at):0);return s.duration_ms>0?Math.min(p,s.duration_ms):p}
  function tick(){
    if(!s||!s.active)return;
    var p=pos();
    $("pos").textContent=clock(p);
    $("fill").style.width=(s.duration_ms>0?Math.min(100,p/s.duration_ms*100):0)+"%";
  }
  function render(){
    var active=s&&s.active;
    card.classList.toggle("idle",!active);
    card.classList.toggle("paused",!active||!s.playing);
    $("status").textContent=!active?T.idle:(s.playing?T.playing:T.paused);
    var title=$("title");
    title.textContent=active?s.title:T.nothing;
    if(active&&s.link){title.href=s.link}else{title.removeAttribute("href")}
    $("artist").textContent=active?s.artist+(s.album?" · "+s.album:""):(s?s.user:"");
    $("dur").textContent=active?clock(s.duration_ms):"0:00";
    var art=$("art");
    if(active&&s.artwork){if(art.getAttribute("src")!==s.artwork){art.src=s.artwork;$("bg").style.backgroundImage='url("'+s.artwork.replace(/"/g,"%22")+'")'}art.classList.remove("hidden")}
    else{art.removeAttribute("src");$("bg").style.backgroundImage="none"}
    $("device").classList.toggle("hidden",!active);
    $("icon").innerHTML=ICONS[active&&s.device_kind]||ICONS.browser;
    $("devtext").textContent=T.on+((active&&s.device_name)||T.device);
    if(!active){$("pos").textContent="0:00";$("fill").style.width="0"}
    tick();
  }
  function load(){
    fetch("/api/v1/public/now-playing/"+TOKEN,{cache:"no-store"}).then(function(r){if(!r.ok)throw r.status;return r.json()}).then(function(d){
      s=d;base=d.position_ms;at=Date.now();render();
    }).catch(function(){}).then(function(){setTimeout(load,5000)});
  }
  setInterval(tick,1000);
  document.addEventListener("visibilitychange",function(){if(!document.hidden)tick()});
  render();load();
})();
</script>
</body>
</html>
"##;

#[cfg(test)]
mod tests {
    #[test]
    fn widget_has_token_placeholder() {
        assert_eq!(super::WIDGET_HTML.matches("__TOKEN__").count(), 1);
    }
}
