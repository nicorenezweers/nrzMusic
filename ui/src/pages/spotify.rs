//! Spotify: Verbinden mit dem Webplayer-Cookie `sp_dc` (ohne Premium, Anmeldefenster der
//! Android-App) oder über eine eigene
//! Spotify-App (Client-ID, Anmeldung im neuen Tab, danach ggf. die Adresse einfügen),
//! Lieblingssongs (seitenweise) und Playlists.
//! Abgespielt und heruntergeladen wird über die passenden Titel von YouTube Music.

use crate::api;
use crate::components::cards::{GRID, SpPlaylistCard};
use crate::components::common::*;
use crate::components::download::DownloadBadge;
use crate::components::overlay::*;
use crate::components::tracks::{ITEM, Item, client_xy, coarse_pointer, pick_playlist};
use crate::downloads::{self, DlState};
use crate::i18n::{tr, trf};
use crate::pages::LoadError;
use crate::spotify::{self, MATCHING, STATUS, STATUS_ERROR, TASK};
use crate::{Route, player, state, util};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::*;
use std::rc::Rc;
use wasm_bindgen::JsValue;

/// Lieblingssongs pro Seite (Spotify erlaubt höchstens 50).
const PAGE: usize = 50;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Liked,
    Playlists,
}

// ------------------------------------------------------------------ Seiten

/// `/spotify` – auch Ziel der Anmeldung (`?connected=…` bzw. `?error=…`).
#[component]
pub fn Spotify(connected: String, error: String) -> Element {
    let nav = use_navigator();
    use_effect(use_reactive!(|connected, error| {
        if connected.is_empty() && error.is_empty() {
            return;
        }
        if error.is_empty() {
            toast(tr("Mit Spotify verbunden."));
        } else {
            let msg = error.replace('+', " ");
            let msg = urlencoding::decode(&msg).map(|m| m.into_owned()).unwrap_or(msg);
            toast_error(trf("Spotify-Anmeldung fehlgeschlagen: {0}", &[&tr(&msg)]));
        }
        spotify::reload_status();
        nav.replace(Route::Spotify { connected: String::new(), error: String::new() });
    }));
    rsx! {
        SpotifyGate {
            SpotifyHeader { tab: Tab::Liked }
            LikedSongs {}
        }
    }
}

#[component]
pub fn SpotifyLiked() -> Element {
    rsx! {
        SpotifyGate {
            SpotifyHeader { tab: Tab::Liked }
            LikedSongs {}
        }
    }
}

#[component]
pub fn SpotifyPlaylists() -> Element {
    rsx! {
        SpotifyGate {
            SpotifyHeader { tab: Tab::Playlists }
            PlaylistGrid {}
        }
    }
}

#[component]
pub fn SpotifyPlaylistPage(id: String) -> Element {
    rsx! {
        SpotifyGate { PlaylistView { id } }
    }
}

// ------------------------------------------------------------------ Einrichten, verbinden

/// Zeigt die Anleitung (keine Client-ID), "Verbinden" (nicht angemeldet) oder den Inhalt.
#[component]
fn SpotifyGate(children: Element) -> Element {
    // Beim Öffnen frisch prüfen (abgelaufene Anmeldung, inzwischen eingerichtet …)
    use_future(spotify::load_status);
    let status = STATUS.read().clone();
    match status {
        None => match STATUS_ERROR.read().clone() {
            Some(e) => rsx! {
                SpTitle {}
                Callout { text: e, error: true,
                    button { class: "{btn(BTN_OUTLINE)} {BTN_SM}", onclick: move |_| spotify::reload_status(), icons::RefreshCw { class: "size-4" } {tr("Erneut versuchen")} }
                }
            },
            None => rsx! {
                SpTitle {}
                Loading {}
            },
        },
        Some(s) if !s.connected => {
            let admin = !crate::account::enabled() || crate::account::is_admin();
            rsx! {
                SpTitle {}
                div { class: "flex max-w-2xl flex-col gap-6",
                    CookieConnect {}
                    if s.configured {
                        ConnectCard { status: s }
                    } else if admin {
                        details { class: "group rounded-lg border",
                            summary { class: "flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3 text-sm font-medium",
                                {tr("Alternative: eigene Spotify-App (Premium nötig)")}
                                icons::ChevronDown { class: "size-4 text-muted-foreground transition-transform group-open:rotate-180" }
                            }
                            div { class: "border-t p-4", SetupGuide { status: s } }
                        }
                    }
                    LinkImport {}
                }
            }
        }
        Some(_) => children,
    }
}

#[component]
fn SpTitle() -> Element {
    rsx! {
        div { class: "mb-6 flex items-center gap-3",
            div { class: "grid size-10 shrink-0 place-items-center rounded-xl bg-muted", SpotifyIcon { class: "size-5" } }
            h1 { class: "text-2xl font-semibold tracking-tight md:text-3xl", "Spotify" }
        }
    }
}

#[component]
fn Step(n: u32, #[props(into)] title: String, children: Element) -> Element {
    rsx! {
        li { class: "flex gap-3",
            span { class: "grid size-6 shrink-0 place-items-center rounded-full bg-primary text-xs font-semibold text-primary-foreground", "{n}" }
            div { class: "flex min-w-0 flex-1 flex-col gap-2",
                p { class: "text-sm font-medium leading-6", "{title}" }
                {children}
            }
        }
    }
}

/// Adresse zum Kopieren.
#[component]
pub fn CopyField(value: String) -> Element {
    let mut copied = use_signal(|| false);
    let v = value.clone();
    rsx! {
        div { class: "flex items-stretch gap-2",
            code { class: "min-w-0 flex-1 select-all break-all rounded-md border bg-muted/40 px-3 py-2 font-mono text-[13px]", "{value}" }
            button { class: "{btn(BTN_OUTLINE)} {BTN_ICON} h-auto min-h-9", r#type: "button", title: tr("Kopieren"),
                onclick: move |_| {
                    if util::copy_text(&v) {
                        copied.set(true);
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(1500).await;
                            copied.set(false);
                        });
                    } else {
                        toast_error(tr("Kopieren ging nicht – bitte von Hand markieren."));
                    }
                },
                if copied() { icons::Check { class: "size-4" } } else { icons::Copy { class: "size-4" } }
            }
        }
    }
}

#[component]
fn SetupGuide(status: SpotifyStatus) -> Element {
    let mut id = use_signal(|| status.client_id.clone());
    let mut busy = use_signal(|| false);
    let redirect = if status.redirect_uri.is_empty() { spotify::fallback_redirect_uri() } else { status.redirect_uri.clone() };
    rsx! {
        div { class: "flex flex-col gap-6",
            Section { title: tr("Spotify einrichten"),
                description: tr("nrzMusic liest deine Lieblingssongs und Playlists über eine eigene, kostenlose Spotify-App. Abgespielt und heruntergeladen wird über YouTube Music."),
                ol { class: "grid gap-6",
                    Step { n: 1, title: tr("App anlegen"),
                        p { class: "text-sm text-muted-foreground", {tr("Im Spotify Developer Dashboard „Create app“ wählen und bei den APIs „Web API“ ankreuzen. Name und Beschreibung sind frei. Wer die App anlegt, braucht Spotify Premium.")} }
                        a { class: "{btn(BTN_OUTLINE)} {BTN_SM} w-fit", href: "https://developer.spotify.com/dashboard", target: "_blank", rel: "noopener noreferrer",
                            icons::ExternalLink { class: "size-4" }
                            {tr("Developer Dashboard öffnen")}
                        }
                    }
                    Step { n: 2, title: tr("Redirect URI eintragen"),
                        p { class: "text-sm text-muted-foreground", {tr("Diese Adresse genau so unter „Redirect URIs“ eintragen:")} }
                        CopyField { value: redirect }
                        p { class: "text-xs text-muted-foreground",
                            {tr("Spotify verlangt HTTPS oder eine Loopback-IP wie http://127.0.0.1:8350/… – „localhost“ ist nicht erlaubt. Eine andere Adresse (z. B. die eigene Domain mit HTTPS) lässt sich in den Einstellungen festlegen.")}
                            " "
                            Link { class: "font-medium underline-offset-4 hover:underline", to: Route::SettingsPage {}, {tr("Zu den Einstellungen")} }
                        }
                    }
                    Step { n: 3, title: tr("Client-ID hier einfügen"),
                        p { class: "text-sm text-muted-foreground", {tr("Sie steht in der App unter „Basic Information“.")} }
                        form { class: "flex flex-col gap-2 sm:flex-row",
                            onsubmit: move |e| {
                                e.prevent_default();
                                let v = id().trim().to_string();
                                if v.is_empty() || busy() {
                                    return;
                                }
                                spawn(async move {
                                    busy.set(true);
                                    spotify::save_client_id(v).await;
                                    busy.set(false);
                                });
                            },
                            input { class: "{INPUT} font-mono", placeholder: tr("Client-ID"), autocomplete: "off", spellcheck: "false",
                                value: "{id}", oninput: move |e| id.set(e.value()),
                            }
                            button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: busy() || id().trim().is_empty(),
                                if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::Check { class: "size-4" } }
                                {tr("Speichern")}
                            }
                        }
                    }
                    Step { n: 4, title: tr("Dein Konto freischalten"),
                        p { class: "text-sm text-muted-foreground", {tr("Im Entwicklungsmodus darf die App nur eingetragene Konten lesen (höchstens 5): in der App unter „Settings → User Management“ Name und E-Mail-Adresse des Spotify-Kontos hinzufügen – sonst schlägt jeder Aufruf mit 403 fehl.")} }
                        p { class: "text-xs text-muted-foreground", {tr("Nach 6 Monaten beendet Spotify die Anmeldung – dann einfach neu verbinden.")} }
                    }
                }
            }
        }
    }
}

/// Verbinden mit dem Cookie `sp_dc` des Spotify-Webplayers – ohne eigene App, ohne Premium.
/// Angemeldet wird im Anmeldefenster der Android-App, die das Cookie selbst ausliest; im
/// Browser geht das nicht (Spotify lässt sich nicht einbetten, fremde Cookies sind tabu).
#[component]
fn CookieConnect() -> Element {
    let mut sending = use_signal(|| false);
    // Cookie aus dem Anmeldefenster der App an den Server
    use_effect(move || {
        let Some(c) = crate::android::SPOTIFY_COOKIE() else { return };
        *crate::android::SPOTIFY_COOKIE.write() = None;
        spawn(async move {
            sending.set(true);
            let res = api::post::<SpotifyStatus, _>("/spotify/cookie", &SpotifyCookie { cookie: c }).await;
            sending.set(false);
            match res {
                Ok(s) => {
                    if s.connected {
                        toast(tr("Mit Spotify verbunden."));
                    }
                    *STATUS.write() = Some(s);
                }
                Err(e) => toast_error(e),
            }
        });
    });
    let busy = sending() || crate::android::SPOTIFY_LOGIN_OPEN();
    let description = tr("Ohne Premium und ohne eigene Spotify-App: nrzMusic meldet sich mit dem Cookie deines Spotify-Webplayers an und liest Lieblingssongs und Playlists. Abgespielt wird über YouTube Music.");
    if !crate::android::has_spotify_login() {
        let text = if crate::android::available() {
            tr("Die Spotify-Anmeldung braucht eine neuere Version der Android-App – bitte die App aktualisieren.")
        } else {
            tr("Die Spotify-Anmeldung läuft in der Android-App: dort auf dieser Seite „Bei Spotify anmelden“ tippen. Danach sind Lieblingssongs und Playlists auch hier im Browser da.")
        };
        return rsx! {
            Section { title: tr("Mit Spotify verbinden"), description,
                div { class: "grid gap-3",
                    Callout { text }
                    a { class: "{btn(BTN_OUTLINE)} w-fit", href: "/static/nrzMusic.apk", download: "nrzMusic.apk",
                        icons::Download { class: "size-4" }
                        {tr("APK herunterladen")}
                    }
                }
            }
        };
    }
    rsx! {
        Section { title: tr("Mit Spotify verbinden"), description,
            div { class: "grid gap-3",
                button { class: "{btn(BTN_PRIMARY)} w-fit", disabled: busy, onclick: move |_| crate::android::spotify_login(),
                    if busy { icons::LoaderCircle { class: "size-4 animate-spin" } } else { SpotifyIcon { class: "size-4" } }
                    {tr("Bei Spotify anmelden")}
                }
                p { class: "text-sm text-muted-foreground", {tr("Es öffnet sich die Anmeldeseite von Spotify (Free reicht). Danach holt die App das Anmelde-Cookie und schliesst das Fenster von selbst.")} }
                p { class: "text-xs text-muted-foreground", {tr("Das Cookie ist so viel wert wie dein Passwort – es bleibt nur auf diesem Server, die App behält es nicht. Es gilt etwa ein Jahr; abmelden auf open.spotify.com macht es ungültig. Inoffiziell: Spotify kann diesen Weg jederzeit ändern.")} }
            }
        }
    }
}

#[component]
fn ConnectCard(status: SpotifyStatus) -> Element {
    let mut url = use_signal(|| None::<String>);
    let mut pasted = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut sending = use_signal(|| false);
    let login = move |_| {
        // Android-App: im selben Fenster anmelden (Spotify leitet mit dem Cookie der App zurück)
        if crate::android::available() {
            spawn(async move {
                busy.set(true);
                match api::action_json::<SpotifyLogin>("/spotify/login").await {
                    Ok(l) => {
                        if let Some(w) = web_sys::window() {
                            let _ = w.location().set_href(&l.authorize_url);
                        }
                    }
                    Err(e) => {
                        busy.set(false);
                        spotify::report(e);
                    }
                }
            });
            return;
        }
        // Den Tab gleich beim Klick öffnen (sonst blockiert ihn der Browser), die Adresse danach setzen
        let tab = web_sys::window().and_then(|w| w.open_with_url_and_target("about:blank", "_blank").ok().flatten());
        if let Some(t) = &tab {
            let _ = t.set_opener(&JsValue::NULL);
        }
        spawn(async move {
            busy.set(true);
            match api::action_json::<SpotifyLogin>("/spotify/login").await {
                Ok(l) => {
                    match &tab {
                        Some(t) => {
                            let _ = t.location().set_href(&l.authorize_url);
                        }
                        None => util::open_external(&l.authorize_url),
                    }
                    url.set(Some(l.authorize_url));
                }
                Err(e) => {
                    if let Some(t) = &tab {
                        let _ = t.close();
                    }
                    spotify::report(e);
                }
            }
            busy.set(false);
        });
    };
    let submit = move || {
        let u = pasted().trim().to_string();
        if u.is_empty() || sending() {
            return;
        }
        spawn(async move {
            sending.set(true);
            match api::post::<SpotifyStatus, _>("/spotify/callback", &SpotifyCallback { url: u }).await {
                Ok(s) => {
                    sending.set(false);
                    if s.connected {
                        toast(tr("Mit Spotify verbunden."));
                    }
                    *STATUS.write() = Some(s);
                }
                Err(e) => {
                    sending.set(false);
                    toast_error(e);
                }
            }
        });
    };
    let redirect = status.redirect_uri.clone();
    rsx! {
        div { class: "flex flex-col gap-6",
            Section { title: tr("Oder über die eigene Spotify-App"),
                description: tr("Offizielle Anmeldung über die eingerichtete Client-ID – dafür muss der Besitzer der App Spotify Premium haben und dein Konto in der App eingetragen sein."),
                div { class: "grid gap-5",
                    div { class: "flex flex-wrap items-center gap-3",
                        button { class: btn(BTN_PRIMARY), disabled: busy(), onclick: login,
                            if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { SpotifyIcon { class: "size-4" } }
                            {tr("Mit Spotify verbinden")}
                        }
                        if let Some(u) = url() {
                            a { class: "inline-flex items-center gap-1.5 text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline", href: "{u}", target: "_blank", rel: "noopener noreferrer",
                                icons::ExternalLink { class: "size-3.5" }
                                {tr("Anmeldeseite erneut öffnen")}
                            }
                        }
                    }
                    if url().is_some() {
                        div { class: "grid gap-3 rounded-lg border bg-muted/30 p-4",
                            p { class: "text-sm font-medium", {tr("Schritt 2: Adresse einfügen")} }
                            p { class: "text-sm text-muted-foreground",
                                {tr("Nach der Anmeldung landest du auf einer Seite, die evtl. nicht lädt (127.0.0.1) – kopiere die Adresse aus der Adresszeile und füge sie hier ein.")}
                            }
                            form { class: "flex flex-col gap-2 sm:flex-row",
                                onsubmit: move |e| {
                                    e.prevent_default();
                                    submit();
                                },
                                input { class: INPUT, r#type: "url", autocomplete: "off", spellcheck: "false",
                                    placeholder: "http://127.0.0.1:…/api/v1/spotify/callback?code=…",
                                    value: "{pasted}", oninput: move |e| pasted.set(e.value()),
                                }
                                button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: sending() || pasted().trim().is_empty(),
                                    if sending() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::Link2 { class: "size-4" } }
                                    {tr("Verbinden")}
                                }
                            }
                            p { class: "text-xs text-muted-foreground", {tr("Führt die Anmeldung direkt zurück zu nrzMusic, ist nichts weiter zu tun.")} }
                        }
                    }
                    p { class: "text-xs text-muted-foreground",
                        {trf("Meldet Spotify einen Fehler? Prüfe, ob die Redirect URI ({0}) in der Spotify-App eingetragen ist und dein Konto unter „Settings → User Management“ steht.", &[&redirect])}
                        " "
                        Link { class: "font-medium underline-offset-4 hover:underline", to: Route::SettingsPage {}, {tr("Client-ID ändern")} }
                    }
                }
            }
        }
    }
}

/// Öffentliche Spotify-Playlist per Link übernehmen – ohne Spotify-Anmeldung (über Spotifys
/// Einbettungsseite, höchstens 100 Titel).
#[component]
pub fn LinkImport() -> Element {
    let mut link = use_signal(String::new);
    let valid = playlist_id(&link()).is_some();
    rsx! {
        Section { title: tr("Playlist per Link übernehmen"),
            description: tr("Link einer öffentlichen Spotify-Playlist einfügen – auch ohne Spotify-Anmeldung. Jeder Titel wird auf YouTube Music gesucht; Spotify zeigt dafür höchstens die ersten 100 Titel."),
            form { class: "grid gap-3",
                onsubmit: move |e| {
                    e.prevent_default();
                    if let Some(id) = playlist_id(&link()) {
                        spawn_forever(spotify::import(id, tr("Playlist"), 0, true, false));
                        link.set(String::new());
                    }
                },
                input { class: INPUT, r#type: "url", autocomplete: "off", spellcheck: "false",
                    placeholder: "https://open.spotify.com/playlist/…",
                    value: "{link}", oninput: move |e| link.set(e.value()),
                }
                div { class: "flex flex-wrap gap-2",
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: !valid,
                        icons::Import { class: "size-4" }
                        {tr("Als Playlist übernehmen")}
                    }
                    if downloads::available() {
                        button { r#type: "button", class: btn(BTN_OUTLINE), disabled: !valid,
                            onclick: move |_| {
                                if let Some(id) = playlist_id(&link()) {
                                    spawn_forever(spotify::import(id, tr("Playlist"), 0, true, true));
                                    link.set(String::new());
                                }
                            },
                            icons::Download { class: "size-4" }
                            {tr("Übernehmen und herunterladen")}
                        }
                    }
                }
            }
        }
    }
}

/// Playlist-ID aus Link, URI oder ID (wie `nrzmusic_spotify::playlist_id`).
fn playlist_id(input: &str) -> Option<String> {
    let s = input.trim();
    let candidate = if let Some((_, rest)) = s.split_once(":playlist:") {
        rest
    } else if s.contains("://") || s.starts_with("open.spotify.com") {
        let (_, rest) = s.split_once("/playlist/")?;
        rest.split(['?', '#', '/']).next().unwrap_or("")
    } else {
        s
    };
    let id = candidate.trim();
    (!id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

// ------------------------------------------------------------------ Verbunden

fn product_label(p: &str) -> String {
    match p {
        "premium" => "Premium".into(),
        "free" | "open" => tr("Free"),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

#[component]
fn SpotifyHeader(tab: Tab) -> Element {
    let status = STATUS.read().clone().unwrap_or_default();
    let user = status.user.clone();
    let name = user
        .as_ref()
        .map(|u| u.display_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| u.id.clone()))
        .unwrap_or_else(|| tr("Verbunden"));
    // Im Entwicklungsmodus liefert Spotify meist kein Abo – dann nichts anzeigen
    let product = user.as_ref().and_then(|u| u.product.clone()).map(|p| product_label(p.trim())).filter(|p| !p.is_empty());
    let chip = |on: bool| {
        if on {
            "shrink-0 rounded-full bg-primary px-3.5 py-1.5 text-sm font-medium text-primary-foreground"
        } else {
            "shrink-0 rounded-full bg-muted px-3.5 py-1.5 text-sm text-muted-foreground hover:text-foreground"
        }
    };
    rsx! {
        div { class: "mb-5 flex flex-col gap-4",
            div { class: "flex items-center gap-3",
                Cover { src: user.as_ref().and_then(|u| u.image.clone()), round: true, class: "size-12", icon: rsx! { SpotifyIcon {} } }
                div { class: "min-w-0 flex-1",
                    p { class: "flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground",
                        SpotifyIcon { class: "size-3.5" }
                        "Spotify"
                        if let Some(p) = product { " · {p}" }
                    }
                    h1 { class: "truncate text-2xl font-semibold tracking-tight md:text-3xl", "{name}" }
                }
                button { class: "{btn(BTN_PRIMARY)} {BTN_SM}", title: tr("Lieblingssongs und alle Playlists als nrzMusic-Playlists übernehmen (ein erneuter Klick aktualisiert sie)"),
                    disabled: TASK.read().is_some(),
                    onclick: move |_| {
                        spawn_forever(async {
                            let ok = confirm(Ask::new(
                                tr("Alles von Spotify übernehmen?"),
                                tr("Lieblingssongs und alle Playlists werden als nrzMusic-Playlists angelegt – schon übernommene werden aktualisiert. Jeder Titel wird auf YouTube Music gesucht; das läuft im Hintergrund und kann eine Weile dauern."),
                            ).confirm(tr("Alles übernehmen"))).await;
                            if ok {
                                spotify::import("all".into(), tr("Alles"), 0, true, false).await;
                            }
                        });
                    },
                    icons::Import { class: "size-4" }
                    span { class: "max-sm:hidden", {tr("Alles übernehmen")} }
                }
                button { class: "{btn(BTN_OUTLINE)} {BTN_SM}", title: tr("Von Spotify trennen"),
                    onclick: move |_| {
                        spawn(spotify::disconnect());
                    },
                    icons::Unplug { class: "size-4" }
                    span { class: "max-sm:hidden", {tr("Trennen")} }
                }
            }
            if !state::youtube_enabled() {
                Callout { error: true, text: tr("YouTube Music ist ausgeschaltet – Spotify-Titel lassen sich dann weder abspielen noch herunterladen.") }
            }
            div { class: "shelf -mx-4 flex gap-2 overflow-x-auto px-4",
                Link { class: chip(tab == Tab::Liked), to: Route::SpotifyLiked {}, {tr("Lieblingssongs")} }
                Link { class: chip(tab == Tab::Playlists), to: Route::SpotifyPlaylists {}, {tr("Playlists")} }
            }
        }
    }
}

/// Fortschritt von Abgleich und Hintergrundaufgaben.
#[component]
fn SpProgress(#[props(optional)] loading: Option<(usize, u64)>) -> Element {
    let m = *MATCHING.read();
    let task = TASK.read().clone();
    if m.is_none() && task.is_none() && loading.is_none() {
        return rsx! {};
    }
    rsx! {
        div { class: "mb-4 grid gap-2",
            if let Some((d, n)) = loading {
                Bar { label: tr("Lade Lieblingssongs …"), done: d as u64, total: n }
            }
            if let Some((d, n)) = m {
                Bar { label: tr("Abgleich mit YouTube Music …"), done: d as u64, total: n as u64 }
            }
            if let Some((t, d, n)) = task {
                Bar { label: spotify::task_label(&t), done: d, total: n }
            }
        }
    }
}

#[component]
fn Bar(#[props(into)] label: String, done: u64, total: u64) -> Element {
    let finished = total > 0 && done >= total;
    let pct = if total > 0 { (done as f64 / total as f64 * 100.0).clamp(2.0, 100.0) } else { 3.0 };
    rsx! {
        div { class: "rounded-lg border bg-muted/30 p-3",
            div { class: "flex items-center gap-2 text-sm",
                if finished {
                    icons::Check { class: "size-4 shrink-0 text-success" }
                } else {
                    icons::LoaderCircle { class: "size-4 shrink-0 animate-spin" }
                }
                span { class: "min-w-0 flex-1 truncate", "{label}" }
                if total > 0 {
                    span { class: "shrink-0 tabular-nums text-muted-foreground", {format!("{} / {}", util::number(done), util::number(total))} }
                }
            }
            div { class: "mt-2 h-1.5 overflow-hidden rounded-full bg-muted",
                div { class: "h-full rounded-full bg-primary transition-[width]", style: "width: {pct}%" }
            }
        }
    }
}

#[component]
fn SpPlayButtons(disabled: bool, #[props(default)] busy: bool, on_play: EventHandler<bool>) -> Element {
    rsx! {
        button { class: "{btn(BTN_PRIMARY)} rounded-full px-5", disabled: disabled || busy, onclick: move |_| on_play.call(false),
            if busy { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::Play { class: "size-4 fill-current" } }
            {tr("Alle abspielen")}
        }
        button { class: "{btn(BTN_OUTLINE)} rounded-full", disabled: disabled || busy, onclick: move |_| on_play.call(true),
            icons::Shuffle { class: "size-4" }
            {tr("Zufall")}
        }
    }
}

/// "Als Playlist übernehmen" und "Alle herunterladen" (laufen auf dem Server).
#[component]
fn SpImportButtons(source: String, name: String, count: usize) -> Element {
    let (s1, n1) = (source.clone(), name.clone());
    rsx! {
        button { class: btn(BTN_OUTLINE), disabled: count == 0, title: tr("Legt eine nrzMusic-Playlist mit den Treffern auf YouTube Music an."),
            onclick: move |_| {
                spawn_forever(spotify::import(s1.clone(), n1.clone(), count, true, false));
            },
            icons::Import { class: "size-4" }
            {tr("Als Playlist übernehmen")}
        }
        if downloads::available() {
            button { class: btn(BTN_OUTLINE), disabled: count == 0, title: tr("Lädt alle Treffer mit yt-dlp in den Musikordner."),
                onclick: move |_| {
                    spawn_forever(spotify::import(source.clone(), name.clone(), count, false, true));
                },
                icons::Download { class: "size-4" }
                {tr("Alle herunterladen")}
            }
        }
    }
}

// ------------------------------------------------------------------ Lieblingssongs

/// Alle restlichen Seiten laden (für "Alle abspielen"/"Zufall"); `false` bei Fehler.
async fn load_all(mut items: Signal<Vec<SpTrack>>, mut total: Signal<u64>, mut loading: Signal<bool>, mut progress: Signal<Option<(usize, u64)>>) -> bool {
    while *loading.peek() {
        gloo_timers::future::TimeoutFuture::new(100).await;
    }
    loading.set(true);
    let mut ok = true;
    loop {
        let have = items.peek().len();
        let all = *total.peek();
        if have as u64 >= all {
            break;
        }
        progress.set(Some((have, all)));
        match api::get::<Page<SpTrack>>(&format!("/spotify/liked?offset={have}&limit={PAGE}")).await {
            Ok(p) => {
                if p.items.is_empty() {
                    break;
                }
                items.write().extend(p.items);
                total.set(p.total);
            }
            Err(e) => {
                spotify::report(e);
                ok = false;
                break;
            }
        }
    }
    progress.set(None);
    loading.set(false);
    ok
}

#[component]
fn LikedSongs() -> Element {
    let items = use_signal(Vec::<SpTrack>::new);
    let total = use_signal(|| 0u64);
    let loading = use_signal(|| false);
    let first = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let progress = use_signal(|| None::<(usize, u64)>);

    let load_more = move || {
        let (mut items, mut total, mut loading, mut first, mut error) = (items, total, loading, first, error);
        if *loading.peek() || (!*first.peek() && items.peek().len() as u64 >= *total.peek()) {
            return;
        }
        let offset = items.peek().len();
        loading.set(true);
        spawn(async move {
            match api::get::<Page<SpTrack>>(&format!("/spotify/liked?offset={offset}&limit={PAGE}")).await {
                Ok(p) => {
                    items.write().extend(p.items);
                    total.set(p.total);
                    error.set(None);
                }
                Err(e) => {
                    spotify::handle_error(&e);
                    error.set(Some(e.message));
                }
            }
            loading.set(false);
            first.set(false);
        });
    };
    use_effect(load_more);

    let play_all = move |shuffle: bool| {
        spawn(async move {
            if load_all(items, total, loading, progress).await {
                spotify::play(items.peek().clone(), 0, shuffle);
            }
        });
    };

    let list = items.read().clone();
    let shown = list.len() as u64;
    let all = total();
    let l_match = list.clone();
    rsx! {
        div { class: "mb-4 flex flex-wrap items-center gap-2",
            SpPlayButtons { disabled: shown == 0, busy: progress().is_some(), on_play: play_all }
            SpImportButtons { source: "liked", name: tr("Lieblingssongs"), count: all as usize }
            Dropdown { title: tr("Mehr"), align: Align::End, class: "min-w-60",
                trigger_class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full",
                trigger: rsx! { icons::Ellipsis { class: "size-5" } },
                MenuItem { disabled: shown == 0, onclick: move |_| spotify::match_all(l_match.clone()), icons::SearchCheck {} {tr("Geladene mit YouTube Music abgleichen")} }
                MenuItem { onclick: move |_| util::open_external("https://open.spotify.com/collection/tracks"), icons::ExternalLink {} {tr("Auf Spotify öffnen")} }
            }
            if all > 0 {
                span { class: "ml-auto text-sm text-muted-foreground", {util::tracks_count(all)} }
            }
        }
        SpProgress { loading: progress() }
        if first() && loading() {
            Loading {}
        } else if let (Some(e), true) = (error(), list.is_empty()) {
            LoadError { message: e }
        } else if list.is_empty() {
            EmptyState { title: tr("Keine Lieblingssongs"), description: tr("Titel, die du auf Spotify mit dem Herz markierst, erscheinen hier."), icon: rsx! { icons::Heart {} } }
        } else {
            SpTrackList { tracks: list }
            if let Some(e) = error() {
                div { class: "mt-4", LoadError { message: e } }
            } else if shown < all {
                div {
                    class: "flex justify-center py-6",
                    // Nachladen, sobald das Ende sichtbar wird
                    onvisible: move |e| {
                        if e.is_intersecting().unwrap_or(false) {
                            load_more();
                        }
                    },
                    button { class: btn(BTN_OUTLINE), disabled: loading(), onclick: move |_| load_more(),
                        if loading() { icons::LoaderCircle { class: "size-4 animate-spin" } }
                        {trf("Mehr laden ({0} von {1})", &[&shown, &all])}
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Playlists

#[component]
fn PlaylistGrid() -> Element {
    let data = use_resource(|| async { api::get::<Vec<SpPlaylistSummary>>("/spotify/playlists").await });
    use_effect(move || {
        if let Some(Err(e)) = &*data.read() {
            spotify::handle_error(e);
        }
    });
    match &*data.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
        Some(Ok(l)) if l.is_empty() => rsx! {
            EmptyState { title: tr("Keine Playlists"), description: tr("Eigene und gefolgte Playlists von Spotify erscheinen hier."), icon: rsx! { icons::ListMusic {} } }
        },
        Some(Ok(l)) => {
            let (readable, locked): (Vec<_>, Vec<_>) = l.iter().cloned().partition(|p| p.readable);
            let gap = if readable.is_empty() { "" } else { "mt-10" };
            rsx! {
                SpProgress {}
                if !readable.is_empty() {
                    div { class: GRID,
                        for p in readable {
                            SpPlaylistCard { key: "{p.id}", playlist: p }
                        }
                    }
                }
                if !locked.is_empty() {
                    section { class: gap,
                        h2 { class: "text-lg font-semibold tracking-tight", {tr("Gefolgte Playlists")} }
                        p { class: "mt-1 mb-4 max-w-2xl text-sm text-muted-foreground",
                            {tr("Spotify gibt über die Anmeldung nur Titel eigener und gemeinsamer Playlists frei. nrzMusic holt die Titel dieser Playlists über Spotifys öffentliche Seite (höchstens 100 Titel).")}
                        }
                        div { class: GRID,
                            for p in locked {
                                SpPlaylistCard { key: "{p.id}", playlist: p }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PlaylistView(id: String) -> Element {
    let pid = id.clone();
    let data = use_resource(use_reactive!(|pid| async move {
        match api::get::<SpPlaylist>(&format!("/spotify/playlists/{}", api::enc(&pid))).await {
            // Nicht freigegeben (gefolgte Playlist): über die öffentliche Seite
            Err(e) if e.status == 403 || e.status == 404 => {
                api::get::<SpPlaylist>(&format!("/spotify/public/{}", api::enc(&pid))).await.map_err(|_| e)
            }
            r => r,
        }
    }));
    use_effect(move || {
        if let Some(Err(e)) = &*data.read() {
            spotify::handle_error(e);
        }
    });
    match &*data.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! {
            SpTitle {}
            LoadError { message: e.to_string() }
        },
        Some(Ok(p)) => {
            let s = p.summary.clone();
            let tracks = p.tracks.clone();
            let total_ms: u64 = tracks.iter().map(|t| t.duration_ms).sum();
            let count = tracks.len().max(s.track_count as usize);
            let mut meta = vec![util::tracks_count(count as u64)];
            if total_ms > 0 {
                meta.push(util::long_duration(total_ms));
            }
            let (t_play, t_match) = (tracks.clone(), tracks.clone());
            let url = spotify::playlist_url(&s.id);
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: s.image.clone(), class: "aspect-square w-full rounded-xl", eager: true, icon: rsx! { icons::ListMusic {} } } },
                    backdrop: s.image.clone(),
                    kind: tr("Playlist · Spotify"),
                    title: s.name.clone(),
                    subtitle: rsx! {
                        span { class: "inline-flex items-center gap-1.5 text-muted-foreground",
                            SpotifyIcon { class: "size-4" }
                            if let Some(o) = s.owner.clone() { span { class: "font-medium text-foreground", "{o}" } }
                        }
                    },
                    meta: meta.join(" · "),
                    SpPlayButtons { disabled: tracks.is_empty(), on_play: move |sh: bool| spotify::play(t_play.clone(), 0, sh) }
                    SpImportButtons { source: s.id.clone(), name: s.name.clone(), count }
                    Dropdown { title: tr("Mehr"), class: "min-w-60",
                        trigger_class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full",
                        trigger: rsx! { icons::Ellipsis { class: "size-5" } },
                        MenuItem { disabled: t_match.is_empty(), onclick: move |_| spotify::match_all(t_match.clone()), icons::SearchCheck {} {tr("Mit YouTube Music abgleichen")} }
                        MenuItem { onclick: move |_| util::open_external(&url), icons::ExternalLink {} {tr("Auf Spotify öffnen")} }
                    }
                }
                SpProgress {}
                if tracks.is_empty() {
                    EmptyState { title: tr("Diese Playlist ist leer"), icon: rsx! { icons::ListMusic {} } }
                } else {
                    SpTrackList { tracks }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Titelliste

/// Abgleich-Zustand eines Spotify-Titels: offen, kein Treffer, gefunden, auf dem Server.
#[component]
fn MatchBadge(track: SpTrack) -> Element {
    match spotify::match_state(&track) {
        None => rsx! {
            span { class: "inline-flex shrink-0 text-muted-foreground/50 [&_svg]:pointer-events-none", title: tr("Noch nicht mit YouTube Music abgeglichen"),
                icons::CircleDashed { class: "size-3.5" }
            }
        },
        Some(None) => rsx! {
            span { class: "inline-flex shrink-0 text-warning [&_svg]:pointer-events-none", title: tr("Nicht auf YouTube Music gefunden"),
                icons::CircleSlash { class: "size-3.5" }
            }
        },
        Some(Some(y)) => {
            if downloads::state_of(&y.video_id).is_some() {
                rsx! { DownloadBadge { video_id: y.video_id.clone() } }
            } else {
                let tip = trf("Auf YouTube Music gefunden: {0} – {1}", &[&y.title, &y.artist_line()]);
                rsx! {
                    span { class: "inline-flex shrink-0 text-muted-foreground [&_svg]:pointer-events-none", title: "{tip}",
                        icons::Youtube { class: "size-3.5" }
                    }
                }
            }
        }
    }
}

/// Spotify-Titel. Doppelklick/Enter (Touch: Tipp) spielt ab dieser Zeile.
#[component]
fn SpTrackList(tracks: Vec<SpTrack>) -> Element {
    let list = Rc::new(tracks);
    let current = player::current_key();
    let playing = player::PLAYING();
    rsx! {
        div { class: "flex flex-col", role: "list",
            for (i , t) in list.iter().cloned().enumerate() {
                {
                    let yt_key = spotify::match_state(&t).flatten().map(|y| format!("yt:{}", y.video_id));
                    let is_current = yt_key.is_some() && yt_key == current;
                    let (l1, l2, l3, l4) = (list.clone(), list.clone(), list.clone(), list.clone());
                    let (t_ctx, t_btn) = (t.clone(), t.clone());
                    let added = t.added_at.as_deref().map(util::date_iso).unwrap_or_default();
                    rsx! {
                        div {
                            key: "{i}-{t.id}",
                            role: "listitem",
                            tabindex: "0",
                            class: format!(
                                "group grid cursor-default select-none grid-cols-[2.5rem_minmax(0,1fr)_auto] items-center gap-3 rounded-lg px-2 py-1.5 outline-none transition-colors hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50 md:grid-cols-[2.5rem_minmax(0,3fr)_minmax(0,2fr)_auto] {}",
                                if is_current { "bg-accent/40" } else { "" },
                            ),
                            onclick: move |_| {
                                if coarse_pointer() {
                                    spotify::play((*l1).clone(), i, false);
                                }
                            },
                            ondoubleclick: move |_| spotify::play((*l2).clone(), i, false),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    spotify::play((*l3).clone(), i, false);
                                }
                            },
                            oncontextmenu: move |e: MouseEvent| {
                                e.prevent_default();
                                let (x, y) = client_xy(&e);
                                open_menu(x, y, t_ctx.clone());
                            },
                            div { class: "relative grid size-10 place-items-center",
                                Cover { src: t.image.clone(), class: "size-10" }
                                button {
                                    class: "absolute inset-0 grid place-items-center rounded-md bg-black/50 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100",
                                    title: tr("Abspielen"),
                                    onclick: move |e: MouseEvent| {
                                        e.stop_propagation();
                                        if is_current {
                                            player::toggle();
                                        } else {
                                            spotify::play((*l4).clone(), i, false);
                                        }
                                    },
                                    if is_current && playing {
                                        icons::Pause { class: "size-4 fill-current pointer-events-none" }
                                    } else {
                                        icons::Play { class: "size-4 fill-current pointer-events-none" }
                                    }
                                }
                            }
                            div { class: "flex min-w-0 flex-col",
                                div { class: "flex min-w-0 items-center gap-1.5",
                                    span { class: if is_current { "truncate text-sm font-semibold" } else { "truncate text-sm font-medium" }, "{t.name}" }
                                    if t.explicit { Tag { text: "E" } }
                                }
                                div { class: "flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground",
                                    MatchBadge { track: t.clone() }
                                    span { class: "truncate", "{t.artist_line()}" }
                                }
                            }
                            div { class: "hidden min-w-0 text-sm text-muted-foreground md:block",
                                span { class: "block truncate", "{t.album}" }
                            }
                            div { class: "flex items-center gap-1",
                                if !added.is_empty() {
                                    span { class: "hidden w-24 truncate text-right text-xs text-muted-foreground lg:block", "{added}" }
                                }
                                span { class: "hidden w-12 text-right text-xs tabular-nums text-muted-foreground sm:block", {util::clock_ms(t.duration_ms)} }
                                button { class: ICON_BTN, title: tr("Mehr"),
                                    onclick: move |e: MouseEvent| {
                                        e.stop_propagation();
                                        let (x, y) = client_xy(&e);
                                        open_menu(x, y, t_btn.clone());
                                    },
                                    icons::Ellipsis { class: "size-4" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Kontextmenü

static MENU: GlobalSignal<Option<(f64, f64, SpTrack)>> = Signal::global(|| None);

fn open_menu(x: f64, y: f64, t: SpTrack) {
    *MENU.write() = Some((x, y, t));
}

/// Menü schliessen; `true`, wenn eines offen war (Esc).
pub fn close_menu() -> bool {
    let open = MENU.peek().is_some();
    if open {
        *MENU.write() = None;
    }
    open
}

/// Treffer suchen (falls nötig) und dann etwas damit tun.
fn with_match(t: SpTrack, f: impl FnOnce(YtTrack) + 'static) {
    spawn_forever(async move {
        if let Some(y) = spotify::resolve(&t).await {
            f(y);
        }
    });
}

/// Kontextmenü eines Spotify-Titels (liegt einmal im Rahmen der App).
#[component]
pub fn SpMenuHost() -> Element {
    let Some((x, y, t)) = MENU.read().clone() else { return rsx! {} };
    let (vw, vh) = web_sys::window()
        .map(|w| (w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1024.0), w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(768.0)))
        .unwrap_or((1024.0, 768.0));
    const W: f64 = 248.0;
    const H: f64 = 360.0;
    let left = if x + W + 8.0 > vw { (x - W).max(8.0) } else { x };
    let top = if y + H + 8.0 > vh { (vh - H - 8.0).max(8.0) } else { y };
    let close = move || *MENU.write() = None;
    let found = spotify::match_state(&t);
    let yt = found.clone().flatten();
    let dl = yt.as_ref().and_then(|y| downloads::state_of(&y.video_id));
    let (t1, t2, t3, t4, t5) = (t.clone(), t.clone(), t.clone(), t.clone(), t.clone());
    rsx! {
        div { class: "fixed inset-0 z-[65]", onclick: move |_| close(), oncontextmenu: move |e| { e.prevent_default(); close(); } }
        div {
            class: "fixed z-[66] w-[15.5rem] overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl animate-in",
            style: "left: {left}px; top: {top}px",
            role: "menu",
            onkeydown: move |e| if e.key() == Key::Escape { close() },
            div { class: "truncate px-2 pt-1 pb-1.5 text-xs font-medium text-muted-foreground", "{t.name}" }
            if found == Some(None) {
                p { class: "flex items-center gap-1.5 px-2 pb-1.5 text-xs text-warning", icons::CircleSlash { class: "size-3.5" } {tr("Nicht auf YouTube Music gefunden")} }
            }
            Item { onclick: move |_| { close(); with_match(t1.clone(), |y| player::play_now(vec![PlayableTrack::Youtube(y)], 0)); }, icons::Play {} {tr("Abspielen")} }
            Item { onclick: move |_| { close(); with_match(t2.clone(), |y| player::play_next(vec![PlayableTrack::Youtube(y)])); }, icons::ListStart {} {tr("Als Nächstes")} }
            Item { onclick: move |_| { close(); with_match(t3.clone(), |y| player::add_to_queue(vec![PlayableTrack::Youtube(y)])); }, icons::ListEnd {} {tr("Zur Warteschlange")} }
            Item { onclick: move |_| { close(); with_match(t4.clone(), |y| pick_playlist(vec![TrackRef::Youtube(y)])); }, icons::ListPlus {} {tr("Zur Playlist hinzufügen …")} }
            match dl {
                Some(DlState::Downloaded(id)) => rsx! {
                    a { class: ITEM, role: "menuitem", href: "/api/v1/tracks/{id}/download", download: "", title: tr("Auf dem Server"), onclick: move |_| close(),
                        icons::HardDriveDownload {} {tr("Datei herunterladen")}
                    }
                },
                Some(DlState::Queued(id) | DlState::Running(id, _)) => rsx! {
                    Item { onclick: move |_| { close(); downloads::remove(id); }, icons::CircleX {} {tr("Download abbrechen")} }
                },
                Some(DlState::Failed(id, _)) => rsx! {
                    Item { onclick: move |_| { close(); downloads::retry(id); }, icons::RotateCw {} {tr("Erneut herunterladen")} }
                },
                None if downloads::available() => rsx! {
                    Item { onclick: move |_| { close(); with_match(t5.clone(), |y| downloads::download(vec![y])); }, icons::Download {} {tr("Herunterladen")} }
                },
                None => rsx! {},
            }
            div { class: "-mx-1 my-1 h-px bg-border" }
            a { class: ITEM, role: "menuitem", href: spotify::track_url(&t.id), target: "_blank", rel: "noopener noreferrer", onclick: move |_| close(),
                SpotifyIcon {} {tr("Auf Spotify öffnen")}
            }
            if let Some(y) = yt {
                a { class: ITEM, role: "menuitem", href: util::yt_music_url(&y.video_id), target: "_blank", rel: "noopener noreferrer", onclick: move |_| close(),
                    icons::ExternalLink {} {tr("Auf YouTube Music öffnen")}
                }
            }
        }
    }
}
