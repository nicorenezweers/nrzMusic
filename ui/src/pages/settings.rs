//! Einstellungen: YouTube Music (an/aus, Sprache, Region), Downloads (yt-dlp), Spotify,
//! Wiedergabe, Sprache der Oberfläche, Bibliothek und Version.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::{toast, toast_error};
use crate::i18n::{self, LANG, LANGUAGES, tr, trf};
use crate::pages::LoadError;
use crate::pages::spotify::CopyField;
use crate::state::{self, STATUS};
use crate::ui::kbd::Kbd;
use crate::{Route, downloads, player, spotify, util};
use dioxus::prelude::*;
use nrzmusic_types::Settings;

const HL: &[(&str, &str)] = &[
    ("de", "Deutsch"),
    ("en", "English"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("es", "Español"),
    ("pt", "Português"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("tr", "Türkçe"),
    ("ja", "日本語"),
];

/// Zielformate der Downloads: Code, Name, Erklärung (deutsch, wird übersetzt).
const FORMATS: &[(&str, &str, &str)] = &[
    ("mp3", "MP3", "Läuft überall. Wird umgewandelt (VBR V0) – braucht ffmpeg."),
    ("m4a", "M4A", "AAC wie von YouTube geliefert, ohne Umwandlung."),
    ("opus", "Opus", "Ohne Umwandlung, sehr effizient – nicht jeder Player kann es."),
];

const GL: &[(&str, &str)] = &[
    ("CH", "Schweiz"),
    ("DE", "Deutschland"),
    ("AT", "Österreich"),
    ("LI", "Liechtenstein"),
    ("FR", "France"),
    ("IT", "Italia"),
    ("ES", "España"),
    ("NL", "Nederland"),
    ("GB", "United Kingdom"),
    ("US", "United States"),
    ("CA", "Canada"),
    ("JP", "日本"),
];

#[component]
pub fn SettingsPage() -> Element {
    let mut form = use_signal(|| None::<Settings>);
    let mut busy = use_signal(|| false);
    let loaded = use_resource(|| async { api::get::<Settings>("/settings").await });
    use_effect(move || {
        if let Some(Ok(s)) = &*loaded.read() {
            form.set(Some(s.clone()));
        }
    });
    use_future(state::load_status);
    let status = STATUS.read().clone();
    let admin = !crate::account::enabled() || crate::account::is_admin();

    let save = move |s: Settings| {
        spawn(async move {
            busy.set(true);
            match api::put::<Settings, _>("/settings", &s).await {
                Ok(saved) => {
                    form.set(Some(saved));
                    toast(tr("Einstellungen gespeichert."));
                    futures_util::future::join3(state::load_status(), downloads::load(), spotify::load_status()).await;
                }
                Err(e) => toast_error(e),
            }
            busy.set(false);
        });
    };

    rsx! {
        div { class: "mx-auto flex max-w-3xl flex-col gap-6",
            PageHeader { title: tr("Einstellungen") }

            if !admin {
                Callout { text: tr("Die Einstellungen des Servers (YouTube, Downloads, Spotify-App, Konten) ändert ein Admin. Hier stellst du ein, was nur für diesen Browser gilt.") }
            }
            if admin {
                Section { title: tr("YouTube Music"), description: tr("Metadaten und Tonspur kommen über den Server (yt-dlp) – ohne YouTube-Konto, ohne Werbung. Heruntergeladene Titel spielen als Datei."),
                    match (form(), &*loaded.read()) {
                        (_, Some(Err(e))) => rsx! { LoadError { message: e.to_string() } },
                        (None, _) => rsx! { Loading {} },
                        (Some(s), _) => {
                            let (s1, s2, s3) = (s.clone(), s.clone(), s.clone());
                            rsx! {
                                div { class: "grid gap-5",
                                    Toggle { checked: s.youtube_enabled, disabled: busy(), label: tr("YouTube Music verwenden"),
                                        onchange: move |on: bool| save(Settings { youtube_enabled: on, ..s1.clone() }),
                                    }
                                    div { class: "grid gap-4 sm:grid-cols-2",
                                        Field { label: tr("Sprache der Inhalte"), hint: tr("Titel von Regalen, Vorschläge (hl)"),
                                            select { class: SELECT, disabled: busy() || !s.youtube_enabled,
                                                onchange: move |e| save(Settings { youtube_language: e.value(), ..s2.clone() }),
                                                for (c , n) in HL.iter().copied() {
                                                    option { value: c, selected: s.youtube_language == c, "{n} ({c})" }
                                                }
                                                if !HL.iter().any(|(c, _)| *c == s.youtube_language) {
                                                    option { value: "{s.youtube_language}", selected: true, "{s.youtube_language}" }
                                                }
                                            }
                                        }
                                        Field { label: tr("Region"), hint: tr("Startseite und Charts (gl)"),
                                            select { class: SELECT, disabled: busy() || !s.youtube_enabled,
                                                onchange: move |e| save(Settings { youtube_region: e.value(), ..s3.clone() }),
                                                for (c , n) in GL.iter().copied() {
                                                    option { value: c, selected: s.youtube_region == c, "{n} ({c})" }
                                                }
                                                if !GL.iter().any(|(c, _)| *c == s.youtube_region) {
                                                    option { value: "{s.youtube_region}", selected: true, "{s.youtube_region}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Section { title: tr("Downloads"),
                    description: tr("YouTube-Titel mit yt-dlp in den Musikordner laden (unter „YouTube/…“). Danach spielen sie als Datei – auch in der Android-App offline."),
                    actions: rsx! {
                        Link { class: "{btn(BTN_OUTLINE)} {BTN_SM}", to: Route::Downloads {}, icons::Download { class: "size-4" } {tr("Zu den Downloads")} }
                    },
                    match form() {
                        None => rsx! { Loading {} },
                        Some(s) => {
                            let (s1, s2) = (s.clone(), s.clone());
                            rsx! {
                                div { class: "grid gap-5",
                                    Toggle { checked: s.downloads_enabled, disabled: busy(), label: tr("Downloads erlauben"),
                                        onchange: move |on: bool| save(Settings { downloads_enabled: on, ..s1.clone() }),
                                    }
                                    div { class: "grid gap-1.5",
                                        span { class: "text-sm font-medium leading-none", {tr("Format")} }
                                        div { class: "grid gap-2 sm:grid-cols-3",
                                            for (code , name , hint) in FORMATS.iter().copied() {
                                                {
                                                    let base = s.clone();
                                                    let on = s.download_format == code;
                                                    rsx! {
                                                        button {
                                                            key: "{code}",
                                                            r#type: "button",
                                                            disabled: busy() || !s.downloads_enabled,
                                                            "aria-pressed": "{on}",
                                                            class: if on { "flex flex-col items-start gap-1 rounded-md border border-primary bg-accent px-3 py-2 text-left disabled:opacity-50" } else { "flex flex-col items-start gap-1 rounded-md border px-3 py-2 text-left hover:bg-accent/60 disabled:opacity-50" },
                                                            onclick: move |_| {
                                                                if !on {
                                                                    save(Settings { download_format: code.to_string(), ..base.clone() });
                                                                }
                                                            },
                                                            span { class: "text-sm font-medium", "{name}" }
                                                            span { class: "text-xs text-muted-foreground", {tr(hint)} }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        span { class: "text-xs text-muted-foreground", {tr("Gilt für neue Downloads.")} }
                                    }
                                    Toggle { checked: s.ytdlp_auto_update, disabled: busy() || !s.downloads_enabled, label: tr("yt-dlp selbst aktuell halten (beim Start und täglich)"),
                                        onchange: move |on: bool| save(Settings { ytdlp_auto_update: on, ..s2.clone() }),
                                    }
                                }
                            }
                        }
                    }
                }

                Section { title: "Spotify",
                    description: tr("Lieblingssongs und Playlists aus Spotify – gespielt und heruntergeladen über YouTube Music. Dafür braucht es eine eigene Spotify-App; die Anleitung steht auf der Spotify-Seite."),
                    actions: rsx! {
                        Link { class: "{btn(BTN_OUTLINE)} {BTN_SM}", to: Route::Spotify { connected: String::new(), error: String::new() }, SpotifyIcon { class: "size-4" } {tr("Zur Spotify-Seite")} }
                    },
                    match form() {
                        None => rsx! { Loading {} },
                        Some(s) => rsx! { SpotifySettings { settings: s, busy: busy(), on_save: save } },
                    }
                }
                if crate::account::enabled() {
                    Section { title: tr("Konten"), description: tr("Wer ein Konto hat, hat eigene Playlists, Favoriten, Verlauf und Spotify. Musikordner und Downloads teilen sich alle."),
                        actions: rsx! {
                            Link { class: "{btn(BTN_OUTLINE)} {BTN_SM}", to: Route::AccountPage {}, icons::Users { class: "size-4" } {tr("Konten verwalten")} }
                        },
                        match form() {
                            None => rsx! { Loading {} },
                            Some(s) => {
                                let s1 = s.clone();
                                rsx! {
                                    Toggle { checked: s.registration_open, disabled: busy(), label: tr("Neue Konten dürfen sich selbst registrieren"),
                                        onchange: move |on: bool| save(Settings { registration_open: on, ..s1.clone() }),
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Section { title: tr("Wiedergabe"),
                div { class: "grid gap-2",
                    Toggle { checked: player::AUTOPLAY_RADIO(), label: tr("Am Ende der Warteschlange endlos weiterspielen (Radio und „Für dich“)"),
                        onchange: move |on: bool| {
                            *player::AUTOPLAY_RADIO.write() = on;
                            util::storage_set("nrzmusic-autoplay-radio", if on { "on" } else { "off" });
                        },
                    }
                    p { class: "text-xs text-muted-foreground", {tr("Nach einem YouTube-Titel mit ähnlichen Titeln, sonst mit Empfehlungen aus deinem Feed. Gilt für diesen Browser.")} }
                }
            }

            Section { title: tr("Für dich"), description: tr("Gilt für dein Konto."),
                PrefsForm {}
            }

            Section { title: tr("Geteilte Links"), description: tr("Links zu Titeln, deinen Likes und Playlists. Wer einen Link hat, kann die Titel ohne Konto anhören."),
                SharesList {}
            }

            Section { title: tr("Sprache der Oberfläche"), description: tr("Gilt für diesen Browser."),
                div { class: "flex flex-wrap gap-2",
                    for (code , name) in LANGUAGES.iter().copied() {
                        button {
                            class: if *LANG.read() == code { "flex items-center gap-2 rounded-md border border-primary bg-accent px-3 py-2 text-sm" } else { "flex items-center gap-2 rounded-md border px-3 py-2 text-sm hover:bg-accent/60" },
                            onclick: move |_| i18n::set_language(code, true),
                            span { "{name}" }
                            span { class: "font-mono text-[10px] uppercase text-muted-foreground", "{code}" }
                        }
                    }
                }
            }

            Section { title: tr("Bibliothek"),
                actions: rsx! {
                    if admin {
                        button { class: "{btn(BTN_OUTLINE)} {BTN_SM}", onclick: move |_| { spawn(state::rescan()); }, icons::RefreshCw { class: "size-4" } {tr("Neu einlesen")} }
                    }
                },
                if let Some(s) = status.clone() {
                    dl { class: "grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm",
                        dt { class: "text-muted-foreground", {tr("Musikordner")} }
                        dd { class: "font-mono break-all", "{s.music_dir}" }
                        dt { class: "text-muted-foreground", {tr("Titel")} }
                        dd { class: "tabular-nums", {util::number(s.tracks)} }
                        dt { class: "text-muted-foreground", {tr("Alben")} }
                        dd { class: "tabular-nums", {util::number(s.albums)} }
                        dt { class: "text-muted-foreground", {tr("Künstler")} }
                        dd { class: "tabular-nums", {util::number(s.artists)} }
                        dt { class: "text-muted-foreground", {tr("Gesamtdauer")} }
                        dd { {util::long_duration(s.total_duration_ms)} }
                        dt { class: "text-muted-foreground", {tr("Grösse")} }
                        dd { {util::size(s.total_size)} }
                        dt { class: "text-muted-foreground", {tr("Überwachung")} }
                        dd { if s.watching { {tr("live")} } else { {tr("nur periodischer Scan")} } }
                        dt { class: "text-muted-foreground", {tr("Zuletzt eingelesen")} }
                        dd { {s.last_scan.map(util::datetime).unwrap_or_else(|| "–".into())} }
                    }
                } else {
                    Loading {}
                }
            }

            Section { title: tr("Tastenkürzel"),
                div { class: "grid gap-2 text-sm sm:grid-cols-2",
                    Shortcut { keys: vec!["Leertaste".into()], label: tr("Abspielen / Pause") }
                    Shortcut { keys: vec!["←".into(), "→".into()], label: tr("10 Sekunden zurück / vor") }
                    Shortcut { keys: vec!["Shift".into(), "← →".into()], label: tr("Vorheriger / nächster Titel") }
                    Shortcut { keys: vec!["M".into()], label: tr("Stumm") }
                    Shortcut { keys: vec!["/".into()], label: tr("Suchen") }
                }
            }

            Section { title: tr("Über nrzMusic"),
                dl { class: "grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm",
                    dt { class: "text-muted-foreground", {tr("Server")} }
                    dd { class: "tabular-nums", {status.as_ref().map(|s| s.version.clone()).unwrap_or_else(|| "–".into())} }
                    dt { class: "text-muted-foreground", {tr("Oberfläche")} }
                    dd { class: "tabular-nums", {env!("CARGO_PKG_VERSION")} }
                    dt { class: "text-muted-foreground", {tr("Lizenz")} }
                    dd { "AGPL-3.0-or-later · " {tr("Komponenten von rust-ui (MIT)")} }
                }
            }
        }
    }
}

#[component]
fn Shortcut(keys: Vec<String>, #[props(into)] label: String) -> Element {
    rsx! {
        div { class: "flex items-center justify-between gap-3 rounded-md border px-3 py-2",
            span { class: "text-muted-foreground", "{label}" }
            span { class: "flex gap-1",
                for k in keys {
                    Kbd { {if k == "Leertaste" { tr("Leertaste") } else { k.clone() }} }
                }
            }
        }
    }
}

#[component]
fn SpotifySettings(settings: Settings, busy: bool, on_save: EventHandler<Settings>) -> Element {
    let mut client_id = use_signal(String::new);
    let mut redirect = use_signal(String::new);
    let init = settings.clone();
    use_effect(use_reactive!(|init| {
        client_id.set(init.spotify_client_id.clone());
        redirect.set(init.spotify_redirect_uri.clone());
    }));
    let status = spotify::STATUS.read().clone().unwrap_or_default();
    // Platzhalter: der Standard des Servers (solange keine eigene Adresse gilt, liefert ihn der Status)
    let default_uri = if settings.spotify_redirect_uri.trim().is_empty() && !status.redirect_uri.is_empty() {
        status.redirect_uri.clone()
    } else {
        spotify::fallback_redirect_uri()
    };
    let changed = client_id().trim() != settings.spotify_client_id || redirect().trim() != settings.spotify_redirect_uri;
    let base = settings.clone();
    let user = status.user.clone();
    rsx! {
        form { class: "grid gap-5",
            onsubmit: move |e| {
                e.prevent_default();
                on_save.call(Settings {
                    spotify_client_id: client_id().trim().to_string(),
                    spotify_redirect_uri: redirect().trim().to_string(),
                    ..base.clone()
                });
            },
            div { class: "grid gap-4 sm:grid-cols-2",
                Field { label: tr("Client-ID"), hint: tr("Aus der eigenen App im Spotify Developer Dashboard (Besitzer braucht Premium). Leer = nur Anmeldung mit dem Webplayer-Cookie."),
                    input { class: "{INPUT} font-mono", autocomplete: "off", spellcheck: "false", value: "{client_id}", oninput: move |e| client_id.set(e.value()) }
                }
                Field { label: tr("Redirect URI"), hint: tr("Muss exakt so in der Spotify-App stehen – HTTPS oder eine Loopback-IP wie 127.0.0.1 („localhost“ ist nicht erlaubt), mit Port und Pfad. Leer = Standard."),
                    input { class: "{INPUT} font-mono", r#type: "url", autocomplete: "off", spellcheck: "false", placeholder: "{default_uri}", value: "{redirect}", oninput: move |e| redirect.set(e.value()) }
                }
            }
            if !status.redirect_uri.is_empty() {
                div { class: "grid gap-1.5",
                    span { class: "text-xs text-muted-foreground", {tr("In der Spotify-App einzutragen:")} }
                    CopyField { value: status.redirect_uri.clone() }
                }
            }
            div { class: "flex flex-wrap items-center justify-between gap-3",
                div { class: "flex items-center gap-2 text-sm",
                    span { class: if status.connected { "size-2 rounded-full bg-success" } else if status.configured { "size-2 rounded-full bg-warning" } else { "size-2 rounded-full bg-muted-foreground/50" } }
                    if status.connected {
                        span {
                            {trf("Verbunden als {0}", &[&user.as_ref().map(|u| u.display_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| u.id.clone())).unwrap_or_default()])}
                        }
                        if status.via_cookie {
                            span { class: "text-muted-foreground", {tr("(Webplayer-Cookie)")} }
                        }
                    } else {
                        span { {tr("Nicht verbunden")} }
                        Link { class: "font-medium underline-offset-4 hover:underline", to: Route::Spotify { connected: String::new(), error: String::new() }, {tr("Verbinden")} }
                    }
                }
                div { class: "flex items-center gap-2",
                    if status.connected {
                        button { r#type: "button", class: "{btn(BTN_OUTLINE)} {BTN_SM}", onclick: move |_| { spawn(spotify::disconnect()); },
                            icons::Unplug { class: "size-4" }
                            {tr("Trennen")}
                        }
                    }
                    button { r#type: "submit", class: "{btn(BTN_PRIMARY)} {BTN_SM}", disabled: busy || !changed, {tr("Speichern")} }
                }
            }
        }
    }
}

/// Einstellungen des eigenen Kontos (Herunterladen beim Liken).
#[component]
fn PrefsForm() -> Element {
    let mut prefs = use_signal(|| None::<nrzmusic_types::UserPrefs>);
    use_future(move || async move {
        if let Ok(p) = api::get::<nrzmusic_types::UserPrefs>("/prefs").await {
            prefs.set(Some(p));
        }
    });
    let Some(p) = prefs() else { return rsx! { Loading {} } };
    rsx! {
        div { class: "grid gap-2",
            Toggle { checked: p.auto_download_likes, disabled: !downloads::available(),
                label: tr("Gelikte YouTube-Titel automatisch herunterladen"),
                onchange: move |on: bool| {
                    let body = nrzmusic_types::UserPrefs { auto_download_likes: on };
                    prefs.set(Some(body.clone()));
                    spawn(async move {
                        match api::put::<nrzmusic_types::UserPrefs, _>("/prefs", &body).await {
                            Ok(p) => {
                                *state::AUTO_DOWNLOAD_LIKES.write() = p.auto_download_likes;
                                prefs.set(Some(p));
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
            }
            p { class: "text-xs text-muted-foreground",
                if downloads::available() {
                    {tr("Tippst du bei einem YouTube-Titel aufs Herz, landet er gleich unter „Downloads“ und im Musikordner.")}
                } else {
                    {tr("Downloads sind auf dem Server ausgeschaltet oder yt-dlp fehlt.")}
                }
            }
        }
    }
}

/// Eigene geteilte Links mit Aufrufen; kopieren oder löschen.
#[component]
fn SharesList() -> Element {
    let mut version = use_signal(|| 0u32);
    let list = use_resource(move || async move {
        let _ = version();
        api::get::<Vec<nrzmusic_types::ShareInfo>>("/shares").await
    });
    match &*list.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
        Some(Ok(l)) if l.is_empty() => rsx! {
            p { class: "text-sm text-muted-foreground", {tr("Noch nichts geteilt. „Teilen …“ gibt es im Menü eines Titels, bei deinen Likes und bei jeder Playlist.")} }
        },
        Some(Ok(l)) => rsx! {
            div { class: "flex flex-col divide-y rounded-lg border",
                for s in l.iter().cloned() {
                    div { key: "{s.token}", class: "flex items-center gap-3 px-3 py-2",
                        match s.kind {
                            nrzmusic_types::ShareKind::Track => rsx! { icons::Music { class: "size-4 shrink-0 text-muted-foreground" } },
                            nrzmusic_types::ShareKind::Favorites => rsx! { icons::Heart { class: "size-4 shrink-0 text-muted-foreground" } },
                            nrzmusic_types::ShareKind::Playlist => rsx! { icons::ListMusic { class: "size-4 shrink-0 text-muted-foreground" } },
                        }
                        div { class: "min-w-0 flex-1",
                            p { class: "truncate text-sm font-medium",
                                if s.kind == nrzmusic_types::ShareKind::Favorites { {tr("Meine Likes")} } else if s.title.is_empty() { {tr("(gelöscht)")} } else { "{s.title}" }
                            }
                            p { class: "text-xs text-muted-foreground", {trf("{0} · {1} Aufrufe", &[&util::date(s.created_at), &s.views])} }
                        }
                        button { class: "{ICON_BTN}", title: tr("Link kopieren"),
                            onclick: { let u = crate::share::full_url(&s.path); move |_| {
                                if util::copy_text(&u) { toast(tr("Link kopiert.")) } else { toast_error(tr("Kopieren ging nicht – bitte von Hand markieren.")) }
                            } },
                            icons::Copy { class: "size-4" }
                        }
                        button { class: "{ICON_BTN} hover:text-destructive", title: tr("Link löschen"),
                            onclick: { let t = s.token.clone(); move |_| {
                                let t = t.clone();
                                spawn(async move {
                                    match api::delete(&format!("/shares/{t}")).await {
                                        Ok(()) => { toast(tr("Link gelöscht – er funktioniert nicht mehr.")); version += 1; }
                                        Err(e) => toast_error(e),
                                    }
                                });
                            } },
                            icons::Trash2 { class: "size-4" }
                        }
                    }
                }
            }
        },
    }
}
