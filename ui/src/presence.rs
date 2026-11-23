//! Jetzt läuft – Sync zwischen Geräten. Ist es im Konto eingeschaltet, meldet dieses Gerät, was
//! es spielt (bei jeder Änderung und alle 10 s, solange es spielt), und alle Geräte bekommen den
//! Stand als Ereignis. Spielt gerade ein *anderes* Gerät, zeigt die App unten „Hört auf …“ mit
//! der Stelle auf die Sekunde und „Hier weiterhören“.

use crate::api;
use crate::i18n::{tr, trf};
use crate::player;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{NowPlaying, NowPlayingReport, PresenceSettings, PresenceUpdate};

/// Einstellung des Kontos (`None` = noch nicht geladen).
pub static SETTINGS: GlobalSignal<Option<PresenceSettings>> = Signal::global(|| None);
/// Letzter Stand des Kontos und wann er hier ankam (lokale Millisekunden).
pub static REMOTE: GlobalSignal<Option<(NowPlaying, f64)>> = Signal::global(|| None);
/// Takt für die Anzeige der Stelle (einmal pro Sekunde).
static CLOCK: GlobalSignal<u64> = Signal::global(|| 0);

const ID_KEY: &str = "nrzmusic-device-id";
const NAME_KEY: &str = "nrzmusic-device-name";

pub fn enabled() -> bool {
    SETTINGS.peek().as_ref().is_some_and(|s| s.enabled)
}

/// Zufällige, gespeicherte ID dieses Geräts.
pub fn device_id() -> String {
    if let Some(id) = crate::util::storage_get(ID_KEY).filter(|s| !s.is_empty()) {
        return id;
    }
    let id: String = (0..16).map(|_| format!("{:x}", crate::util::random_index(16))).collect();
    crate::util::storage_set(ID_KEY, &id);
    id
}

fn user_agent() -> String {
    web_sys::window().and_then(|w| w.navigator().user_agent().ok()).unwrap_or_default()
}

fn touch_mac() -> bool {
    // iPadOS gibt sich als Mac aus – hat aber einen Touchscreen
    web_sys::window().is_some_and(|w| w.navigator().max_touch_points() > 1)
}

/// Art des Geräts: `computer`, `phone`, `tablet` oder `browser`.
pub fn device_kind() -> &'static str {
    let ua = user_agent();
    if ua.contains("iPad") || (ua.contains("Macintosh") && touch_mac()) || (ua.contains("Android") && !ua.contains("Mobile")) {
        "tablet"
    } else if ua.contains("iPhone") || ua.contains("Android") {
        "phone"
    } else if crate::mac::info().is_some() {
        "computer"
    } else {
        "browser"
    }
}

/// Name, wie er bei den anderen erscheint: selbst gewählt, sonst erraten.
pub fn device_name() -> String {
    crate::util::storage_get(NAME_KEY)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(auto_name)
}

pub fn set_device_name(name: &str) {
    crate::util::storage_set(NAME_KEY, name.trim());
}

/// Erratener Gerätename aus dem User-Agent.
pub fn auto_name() -> String {
    let ua = user_agent();
    if crate::mac::info().is_some() {
        return "Mac".into();
    }
    if crate::android::available() {
        return tr("Android-Handy");
    }
    if ua.contains("iPad") || (ua.contains("Macintosh") && touch_mac()) {
        return "iPad".into();
    }
    if ua.contains("iPhone") {
        return "iPhone".into();
    }
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("OPR/") {
        "Opera"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if ua.contains("Android") {
        "Android"
    } else if ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    if os.is_empty() { browser.to_string() } else { trf("{0} auf {1}", &[&browser, &os]) }
}

/// Einstellung laden (beim Start und nach dem Ändern).
pub async fn load() {
    if let Ok(s) = api::get::<PresenceSettings>("/presence").await {
        let on = s.enabled;
        *SETTINGS.write() = Some(s);
        if on {
            if let Ok(np) = api::get::<Option<NowPlaying>>("/now-playing").await {
                *REMOTE.write() = np.map(|n| (n, js_sys::Date::now()));
            }
        } else {
            *REMOTE.write() = None;
        }
    }
}

/// Ein- oder ausschalten.
pub async fn update(enabled: bool, public: bool) -> Result<(), api::ApiError> {
    let s = api::put::<PresenceSettings, _>("/presence", &PresenceUpdate { enabled, public }).await?;
    let on = s.enabled;
    *SETTINGS.write() = Some(s);
    if on {
        force_report();
        load().await;
    } else {
        *REMOTE.write() = None;
    }
    Ok(())
}

pub async fn rotate() -> Result<(), api::ApiError> {
    let s = api::action_json::<PresenceSettings>("/presence/rotate").await?;
    *SETTINGS.write() = Some(s);
    Ok(())
}

/// Ereignis `now_playing` vom Server.
pub fn on_event(now: Option<NowPlaying>, takeover: bool) {
    let mine = device_id();
    // Ein anderes Gerät hat übernommen: hier anhalten
    if takeover && now.as_ref().is_some_and(|n| n.device_id != mine) && *player::PLAYING.peek() {
        player::pause();
    }
    *REMOTE.write() = now.map(|n| (n, js_sys::Date::now()));
}

thread_local! {
    static FORCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Beim nächsten Takt sofort melden.
pub fn force_report() {
    FORCE.with(|f| f.set(true));
}

#[derive(PartialEq, Clone)]
struct Snap {
    key: Option<String>,
    playing: bool,
    pos: f64,
}

fn snapshot() -> Snap {
    Snap { key: player::current_key(), playing: *player::PLAYING.peek(), pos: *player::POSITION.peek() }
}

async fn send_report(takeover: bool) {
    let track = player::current();
    let rep = NowPlayingReport {
        device_id: device_id(),
        device_name: device_name(),
        device_kind: device_kind().into(),
        track,
        position_ms: (*player::POSITION.peek() * 1000.0).max(0.0) as u64,
        duration_ms: (*player::DURATION.peek() * 1000.0).max(0.0) as u64,
        playing: *player::PLAYING.peek(),
        takeover,
    };
    if let Err(e) = api::post::<serde_json::Value, _>("/now-playing", &rep).await
        && e.status == 403
    {
        // Inzwischen auf einem anderen Gerät ausgeschaltet
        load().await;
    }
}

/// Seite wird geschlossen: als pausiert melden (sendBeacon), damit die anderen nicht
/// weiterzählen.
fn page_listener() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::Closure;
    let Some(w) = web_sys::window() else { return };
    let hide = Closure::<dyn FnMut()>::new(|| {
        if !enabled() || !*player::PLAYING.peek() {
            return;
        }
        let rep = NowPlayingReport {
            device_id: device_id(),
            device_name: device_name(),
            device_kind: device_kind().into(),
            track: player::current(),
            position_ms: (*player::POSITION.peek() * 1000.0).max(0.0) as u64,
            duration_ms: (*player::DURATION.peek() * 1000.0).max(0.0) as u64,
            playing: false,
            takeover: false,
        };
        if let (Ok(body), Some(w)) = (serde_json::to_string(&rep), web_sys::window()) {
            let _ = w.navigator().send_beacon_with_opt_str(&format!("{}/now-playing", api::BASE), Some(&body));
        }
    });
    let _ = w.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref());
    hide.forget();
}

/// Läuft für immer: Takt der Anzeige und Meldungen dieses Geräts.
pub async fn run() {
    page_listener();
    load().await;
    let mut last: Option<Snap> = None;
    let mut last_sent = 0.0f64;
    // Erst melden, wenn hier wirklich etwas gespielt wurde (nicht schon beim Wiederherstellen)
    let mut armed = false;
    loop {
        gloo_timers::future::TimeoutFuture::new(1000).await;
        *CLOCK.write() += 1;
        if !enabled() {
            last = None;
            continue;
        }
        let snap = snapshot();
        if snap.playing {
            armed = true;
        }
        let forced = FORCE.with(|f| f.replace(false));
        if !armed && !forced {
            continue;
        }
        let now = js_sys::Date::now();
        let changed = match &last {
            None => true,
            Some(l) => {
                let expected = l.pos + if l.playing { (now - last_sent) / 1000.0 } else { 0.0 };
                l.key != snap.key || l.playing != snap.playing || (snap.pos - expected).abs() > 2.5
            }
        };
        let heartbeat = snap.playing && now - last_sent >= 10_000.0;
        if forced || changed || heartbeat {
            last = Some(snap);
            last_sent = now;
            spawn_forever(send_report(false));
        }
    }
}

/// Den Stand eines anderen Geräts übernehmen: hier ab derselben Stelle spielen, dort anhalten.
pub fn take_over() {
    let Some((np, at)) = REMOTE.peek().clone() else { return };
    let pos = position_ms(&np, at) as f64 / 1000.0;
    player::play_now_at(vec![np.track.clone()], 0, pos);
    spawn_forever(async {
        // Kurz warten, bis der Player den Titel geladen hat
        gloo_timers::future::TimeoutFuture::new(400).await;
        send_report(true).await;
    });
}

/// Hochgerechnete Stelle (ms) eines Stands, der um `at` (lokal) ankam.
pub fn position_ms(np: &NowPlaying, at: f64) -> u64 {
    let mut p = np.position_ms as f64;
    if np.playing {
        p += (js_sys::Date::now() - at).max(0.0);
    }
    let p = p as u64;
    if np.duration_ms > 0 { p.min(np.duration_ms) } else { p }
}

/// Spielt gerade ein anderes Gerät (und dieses nicht dasselbe)?
fn remote_to_show() -> Option<(NowPlaying, f64)> {
    let (np, at) = REMOTE.read().clone()?;
    if np.device_id == device_id() {
        return None;
    }
    // Hier läuft schon etwas, dort ist pausiert: nichts anzeigen
    if !np.playing && player::current().is_some() {
        return None;
    }
    // Ein still gewordenes Gerät (seit 45 s keine Meldung) nicht als spielend zeigen
    let stale = np.playing && js_sys::Date::now() - at > 45_000.0;
    if stale {
        return None;
    }
    Some((np, at))
}

#[component]
pub fn DeviceIcon(kind: String, #[props(into, default)] class: String) -> Element {
    match kind.as_str() {
        "computer" => rsx! { icons::Laptop { class } },
        "phone" => rsx! { icons::Smartphone { class } },
        "tablet" => rsx! { icons::Tablet { class } },
        _ => rsx! { icons::Globe { class } },
    }
}

/// Leiste „Hört auf …“ über dem Player (Desktop) bzw. über dem Mini-Player (Handy).
#[component]
pub fn RemoteBar() -> Element {
    let _ = CLOCK();
    let Some((np, at)) = remote_to_show() else { return rsx! {} };
    let pos = position_ms(&np, at);
    let pct = if np.duration_ms > 0 { (pos as f64 / np.duration_ms as f64 * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let t = np.track.clone();
    let device = if np.device_name.is_empty() { tr("einem anderen Gerät") } else { np.device_name.clone() };
    rsx! {
        div { class: "px-safe relative shrink-0 overflow-hidden border-t border-emerald-500/25 bg-emerald-500/10",
            div { class: "flex items-center gap-3 px-3 py-1.5 md:px-4",
                crate::components::common::Cover { src: t.artwork_url(96), class: "size-8 rounded" }
                div { class: "flex min-w-0 flex-1 flex-col leading-tight",
                    span { class: "truncate text-[13px] font-medium", "{t.title()} · {t.artist()}" }
                    span { class: "flex items-center gap-1.5 text-xs font-medium text-emerald-400",
                        if np.playing {
                            icons::AudioLines { class: "size-3.5 shrink-0 animate-pulse" }
                        } else {
                            DeviceIcon { kind: np.device_kind.clone(), class: "size-3.5 shrink-0" }
                        }
                        span { class: "truncate",
                            if np.playing { {trf("Hört auf {0}", &[&device])} } else { {trf("Pausiert auf {0}", &[&device])} }
                        }
                        span { class: "shrink-0 tabular-nums text-muted-foreground",
                            "{crate::util::clock_ms(pos)} / {crate::util::clock_ms(np.duration_ms)}"
                        }
                    }
                }
                button { class: "press shrink-0 rounded-full border border-emerald-500/40 px-3 py-1 text-xs font-medium text-emerald-300 hover:bg-emerald-500/15",
                    title: tr("Ab dieser Stelle hier spielen – das andere Gerät hält an"),
                    onclick: move |_| take_over(),
                    {tr("Hier weiterhören")}
                }
            }
            div { class: "absolute inset-x-0 bottom-0 h-0.5 bg-emerald-500/15",
                div { class: "h-full bg-emerald-400 transition-[width] duration-1000 ease-linear", style: "width: {pct}%" }
            }
        }
    }
}
