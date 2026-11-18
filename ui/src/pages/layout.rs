//! Rahmen wie bei nrzPaper: Seitenleiste links (Navigation, Playlists, Scan-Status),
//! Kopf mit Suche, unten die Spielerleiste; auf dem Handy Navigation unten und die
//! Seitenleiste als Sheet.

use crate::components::common::*;
use crate::components::player_ui::{MiniPlayer, NowPlaying, PlayerBar, QueuePanel};
use crate::components::download::ProgressRing;
use crate::components::tracks::{NewPlaylistDialog, PlaylistPicker, TrackMenuHost, new_playlist};
use crate::pages::spotify::SpMenuHost;
use crate::i18n::{tr, trf};
use crate::state::{LIVE, PLAYLISTS, SCAN, STATUS};
use crate::upload::{self, DRAGGING};
use crate::{ICON, Route, downloads, spotify, util};
use dioxus::prelude::*;

/// Bereich einer Seite (für die Hervorhebung in der Navigation).
fn section(r: &Route) -> &'static str {
    match r {
        Route::Home {} => "home",
        Route::Feed {} => "feed",
        Route::StatsPage {} => "stats",
        Route::Search { .. } => "search",
        Route::Songs {} => "songs",
        Route::Albums {} | Route::AlbumPage { .. } => "albums",
        Route::Artists {} | Route::ArtistPage { .. } => "artists",
        Route::Playlists {} | Route::PlaylistPage { .. } => "playlists",
        Route::Favorites {} => "favorites",
        Route::History {} => "history",
        Route::YouTube { .. } | Route::YtAlbumPage { .. } | Route::YtArtistPage { .. } | Route::YtPlaylistPage { .. } => "youtube",
        Route::Library {} => "library",
        Route::Downloads {} => "downloads",
        Route::Spotify { .. } | Route::SpotifyLiked {} | Route::SpotifyPlaylists {} | Route::SpotifyPlaylistPage { .. } => "spotify",
        Route::SettingsPage {} => "settings",
        Route::SharedPage { .. } => "shared",
        Route::AccountPage {} => "account",
        Route::NotFound { .. } => "",
    }
}

#[component]
pub fn Layout() -> Element {
    let route = use_route::<Route>();
    let mut mobile_open = use_signal(|| false);
    // Neue Seite: nach oben und das Sheet schliessen
    use_effect(use_reactive!(|route| {
        let _ = &route;
        mobile_open.set(false);
        if let Some(main) = util::document().and_then(|d| d.get_element_by_id("content")) {
            main.set_scroll_top(0);
        }
    }));
    // Handy: Höhe des schwebenden Docks (Mini-Player + Navigation) für Inhalt und Overlays
    let dock = if crate::player::current().is_some() { "7.5rem" } else { "3.5rem" };
    rsx! {
        div {
            class: "relative flex h-dvh flex-col overflow-hidden",
            style: "--dock: {dock}",
            ondragenter: move |e| {
                if upload::has_files(&e) {
                    e.prevent_default();
                    *DRAGGING.write() = true;
                }
            },
            ondragover: move |e| {
                if upload::has_files(&e) {
                    e.prevent_default();
                    if !*DRAGGING.peek() {
                        *DRAGGING.write() = true;
                    }
                }
            },
            div { class: "flex min-h-0 flex-1",
                // Seitenleiste (Desktop)
                aside { class: "hidden w-60 shrink-0 flex-col border-r border-sidenav-border bg-sidenav md:flex",
                    Sidebar {}
                }
                // Seitenleiste (Handy) als Sheet
                if mobile_open() {
                    div { class: "fixed inset-0 z-50 bg-black/60 md:hidden", onclick: move |_| mobile_open.set(false) }
                    aside { class: "pt-safe pb-safe fixed inset-y-0 left-0 z-50 flex w-72 max-w-[85vw] flex-col border-r bg-sidenav shadow-xl animate-in md:hidden",
                        Sidebar {}
                    }
                }
                div { class: "relative flex min-w-0 flex-1 flex-col",
                    TopBar { on_menu: move |_| mobile_open.set(true) }
                    main { id: "content", class: "m-content px-safe min-h-0 flex-1 overflow-y-auto overscroll-contain",
                        div { class: "mx-auto w-full max-w-[120rem] p-4 md:p-6", Outlet::<Route> {} }
                    }
                }
            }
            PlayerBar {}
            // Handy: schwebendes Dock mit Mini-Player und Navigation (Inhalt läuft darunter durch)
            div { class: "m-dock md:hidden",
                MiniPlayer {}
                BottomNav {}
            }
        }
        QueuePanel {}
        NowPlaying {}
        // Brauchen den Router (Verweise), liegen darum hier
        TrackMenuHost {}
        SpMenuHost {}
        PlaylistPicker {}
        NewPlaylistDialog {}
        crate::share::ShareDialogHost {}
        upload::DropOverlay {}
    }
}

#[component]
fn TopBar(on_menu: EventHandler<()>) -> Element {
    let nav = use_navigator();
    let route = use_route::<Route>();
    let on_yt = section(&route) == "youtube";
    let current_q = match &route {
        Route::Search { q } | Route::YouTube { q, .. } => q.clone(),
        _ => String::new(),
    };
    let mut text = use_signal(|| current_q.clone());
    // Suche in der Adresse geändert → Feld nachziehen
    use_effect(use_reactive!(|current_q| text.set(current_q)));
    let on_search = matches!(route, Route::Search { .. });
    let tab = match &route {
        Route::YouTube { tab, .. } => tab.clone(),
        _ => String::new(),
    };
    rsx! {
        header { class: "m-header max-md:glass max-md:border-b max-md:border-white/5 shrink-0 md:border-b",
          div { class: "px-safe flex h-14 items-center gap-2 px-3 md:gap-3 md:px-6",
            button { class: "press grid size-9 shrink-0 place-items-center overflow-hidden rounded-xl md:hidden", title: tr("Menü"), onclick: move |_| on_menu.call(()),
                img { src: ICON, alt: "", class: "size-9 rounded-xl" }
            }
            form { class: "relative w-full max-w-xl",
                role: "search",
                onsubmit: move |e| {
                    e.prevent_default();
                    let q = text().trim().to_string();
                    if on_yt {
                        nav.push(Route::YouTube { q, tab: tab.clone() });
                    } else {
                        nav.push(Route::Search { q });
                    }
                },
                if on_yt {
                    icons::Youtube { class: "pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" }
                } else {
                    icons::Search { class: "pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" }
                }
                input {
                    id: "global-search",
                    class: tw_merge::tw_merge!(INPUT, "h-10 rounded-full bg-muted/50 pl-9 md:h-9"),
                    r#type: "search",
                    autocomplete: "off",
                    placeholder: if on_yt { tr("Auf YouTube Music suchen …") } else if util::is_phone() { tr("Titel, Alben, Künstler suchen …") } else { tr("Titel, Alben, Künstler suchen …  ( / )") },
                    value: "{text}",
                    oninput: move |e| {
                        let v = e.value();
                        text.set(v.clone());
                        // Auf der Suchseite gleich beim Tippen suchen
                        if on_search {
                            nav.replace(Route::Search { q: v });
                        }
                    },
                }
            }
            div { class: "ml-auto flex items-center gap-1",
                ActivityBadges {}
                ScanBadge {}
                if !crate::account::enabled() || crate::account::is_admin() {
                    Link { class: tw_merge::tw_merge!(btn(BTN_GHOST), BTN_SM, "hidden lg:inline-flex"), to: Route::Library {},
                        icons::Upload { class: "size-4" }
                        {tr("Hochladen")}
                    }
                }
            }
          }
        }
    }
}

/// Kleiner Hinweis im Kopf, solange eingelesen wird.
#[component]
fn ScanBadge() -> Element {
    let Some(p) = *SCAN.read() else { return rsx! {} };
    let text = if p.total > 0 { format!("{} / {}", util::number(p.done), util::number(p.total)) } else { tr("startet …") };
    rsx! {
        Link { class: "flex items-center gap-2 rounded-full border bg-muted/40 px-3 py-1 text-xs text-muted-foreground hover:text-foreground", to: Route::Library {},
            icons::LoaderCircle { class: "size-3.5 animate-spin" }
            span { class: "hidden sm:inline", {tr("Einlesen")} }
            span { class: "tabular-nums", "{text}" }
        }
    }
}

/// Hinweise im Kopf: laufender Spotify-Abgleich bzw. -Import, laufende Downloads (Handy).
#[component]
fn ActivityBadges() -> Element {
    let matching = *spotify::MATCHING.read();
    let task = spotify::TASK.read().clone().filter(|(_, d, n)| *n == 0 || d < n);
    let open = downloads::open_count();
    let running = downloads::JOBS.read().iter().find(|j| j.status == nrzmusic_types::DownloadStatus::Running).map(|j| j.progress);
    const BADGE: &str = "flex items-center gap-2 rounded-full border bg-muted/40 px-3 py-1 text-xs text-muted-foreground hover:text-foreground";
    let sp = match (matching, task) {
        (Some((d, n)), _) => Some((tr("Abgleich mit YouTube Music"), d as u64, n as u64)),
        (None, Some((t, d, n))) => Some((spotify::task_label(&t), d, n)),
        _ => None,
    };
    rsx! {
        if let Some((label, d, n)) = sp {
            Link { class: BADGE, to: Route::SpotifyLiked {}, title: "{label}",
                icons::LoaderCircle { class: "size-3.5 animate-spin" }
                span { class: "hidden sm:inline", "Spotify" }
                if n > 0 { span { class: "tabular-nums", "{d}/{n}" } }
            }
        }
        if open > 0 {
            Link { class: "{BADGE} md:hidden", to: Route::Downloads {}, title: tr("Downloads"),
                match running {
                    Some(p) => rsx! { ProgressRing { value: p, class: "size-3.5 text-foreground" } },
                    None => rsx! { icons::Download { class: "size-3.5" } },
                }
                span { class: "tabular-nums", "{open}" }
            }
        }
    }
}

#[component]
fn NavLink(to: Route, #[props(into)] label: String, icon: Element, active: bool, #[props(into, optional)] badge: Option<String>) -> Element {
    let c = if active {
        "bg-sidenav-accent font-medium text-sidenav-accent-foreground"
    } else {
        "text-muted-foreground hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground"
    };
    rsx! {
        Link { to, class: "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors [&_svg]:size-4 [&_svg]:shrink-0 {c}",
            {icon}
            span { class: "flex-1 truncate", "{label}" }
            if let Some(b) = badge {
                span { class: "min-w-5 rounded-full bg-muted px-1.5 text-center text-[11px] leading-5 tabular-nums text-foreground", "{b}" }
            }
        }
    }
}

#[component]
fn Sidebar() -> Element {
    let route = use_route::<Route>();
    let s = section(&route);
    let current_playlist = match &route {
        Route::PlaylistPage { id } => Some(*id),
        _ => None,
    };
    let status = STATUS.read().clone();
    let scan = *SCAN.read();
    let open = downloads::open_count();
    let dl_badge = (open > 0).then(|| open.to_string());
    rsx! {
        Link { class: "flex h-14 shrink-0 items-center gap-2.5 px-5", to: Route::Home {},
            img { src: ICON, alt: "", class: "size-7 rounded-lg" }
            span { class: "text-[15px] font-semibold tracking-tight", "nrzMusic" }
        }
        nav { class: "flex flex-col gap-0.5 px-3 pt-1",
            NavLink { to: Route::Home {}, label: tr("Startseite"), active: s == "home", icon: rsx! { icons::House {} } }
            NavLink { to: Route::Feed {}, label: tr("Für dich"), active: s == "feed", icon: rsx! { icons::Sparkles {} } }
            NavLink { to: Route::Search { q: String::new() }, label: tr("Suche"), active: s == "search", icon: rsx! { icons::Search {} } }
            NavLink { to: Route::YouTube { q: String::new(), tab: String::new() }, label: tr("YouTube Music"), active: s == "youtube", icon: rsx! { icons::Youtube {} } }
            NavLink { to: Route::Spotify { connected: String::new(), error: String::new() }, label: "Spotify", active: s == "spotify", icon: rsx! { SpotifyIcon {} } }
        }
        p { class: "mt-5 px-5 pb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground", {tr("Bibliothek")} }
        nav { class: "flex flex-col gap-0.5 px-3",
            NavLink { to: Route::Songs {}, label: tr("Songs"), active: s == "songs", icon: rsx! { icons::Music {} } }
            NavLink { to: Route::Albums {}, label: tr("Alben"), active: s == "albums", icon: rsx! { icons::Disc3 {} } }
            NavLink { to: Route::Artists {}, label: tr("Künstler"), active: s == "artists", icon: rsx! { icons::MicVocal {} } }
            NavLink { to: Route::Favorites {}, label: tr("Favoriten"), active: s == "favorites", icon: rsx! { icons::Heart {} } }
            NavLink { to: Route::History {}, label: tr("Verlauf"), active: s == "history", icon: rsx! { icons::History {} } }
            NavLink { to: Route::StatsPage {}, label: tr("Statistiken"), active: s == "stats", icon: rsx! { icons::ChartColumn {} } }
            NavLink { to: Route::Library {}, label: tr("Ordner & Upload"), active: s == "library", icon: rsx! { icons::FolderUp {} } }
            NavLink { to: Route::Downloads {}, label: tr("Downloads"), active: s == "downloads", badge: dl_badge, icon: rsx! { icons::Download {} } }
            if crate::android::available() {
                button { class: "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm text-muted-foreground transition-colors hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground [&_svg]:size-4",
                    onclick: move |_| crate::android::open_offline(),
                    icons::Smartphone {}
                    span { class: "flex-1 truncate", {tr("Offline auf dem Handy")} }
                    span { class: "min-w-5 rounded-full bg-muted px-1.5 text-center text-[11px] leading-5 tabular-nums text-foreground", {crate::android::OFFLINE.read().len().to_string()} }
                }
            }
        }
        div { class: "mt-5 flex items-center justify-between px-5",
            Link { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground hover:text-foreground", to: Route::Playlists {}, {tr("Playlists")} }
            button { class: "grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-sidenav-accent hover:text-foreground", title: tr("Neue Playlist"),
                onclick: move |_| new_playlist(),
                icons::Plus { class: "size-4 pointer-events-none" }
            }
        }
        div { class: "mt-1 min-h-0 flex-1 overflow-y-auto px-3 pb-3",
            if PLAYLISTS.read().is_empty() {
                button { class: "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm text-muted-foreground hover:bg-sidenav-accent/60",
                    onclick: move |_| new_playlist(),
                    icons::Plus { class: "size-4" }
                    {tr("Neue Playlist")}
                }
            }
            for p in PLAYLISTS.read().iter().cloned() {
                Link {
                    key: "{p.id}",
                    to: Route::PlaylistPage { id: p.id },
                    class: if current_playlist == Some(p.id) { "flex items-center gap-2.5 rounded-md px-2 py-1 text-sm bg-sidenav-accent font-medium" } else { "flex items-center gap-2.5 rounded-md px-2 py-1 text-sm text-muted-foreground hover:bg-sidenav-accent/60 hover:text-foreground" },
                    Collage { artwork: p.artwork.clone(), class: "size-7 rounded" }
                    span { class: "min-w-0 flex-1 truncate", "{p.name}" }
                }
            }
        }
        // Unten: Scan-Status, Bibliothek, Einstellungen
        div { class: "border-t border-sidenav-border p-3",
            if let Some(p) = scan {
                div { class: "mb-2 rounded-md bg-sidenav-accent/60 px-2.5 py-2",
                    div { class: "flex items-center gap-2 text-xs",
                        icons::LoaderCircle { class: "size-3.5 animate-spin" }
                        span { class: "flex-1", {tr("Musikordner wird eingelesen")} }
                        if p.total > 0 { span { class: "tabular-nums text-muted-foreground", "{p.done}/{p.total}" } }
                    }
                    div { class: "mt-1.5 h-1 overflow-hidden rounded-full bg-muted",
                        div { class: "h-full bg-foreground/70 transition-[width]", style: format!("width: {}%", if p.total > 0 { p.done as f64 / p.total as f64 * 100.0 } else { 5.0 }) }
                    }
                }
            } else if let Some(st) = status.as_ref() {
                p { class: "mb-1 px-2.5 text-xs text-muted-foreground",
                    {trf("{0} Titel · {1}", &[&util::number(st.tracks), &util::size(st.total_size)])}
                }
            }
            NavLink { to: Route::SettingsPage {}, label: tr("Einstellungen"), active: s == "settings", icon: rsx! { icons::Settings {} } }
            AccountLink { active: s == "account" }
            if !LIVE() {
                p { class: "mt-1 flex items-center gap-1.5 px-2.5 text-[11px] text-muted-foreground",
                    span { class: "size-1.5 rounded-full bg-warning" }
                    {tr("Keine Live-Verbindung – verbinde neu …")}
                }
            }
        }
    }
}

/// Konto unten in der Seitenleiste (Name, Abmelden).
#[component]
fn AccountLink(active: bool) -> Element {
    let Some(me) = crate::account::me().filter(|_| crate::account::enabled()) else {
        return rsx! {};
    };
    let initial = me.label().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let c = if active { "bg-sidenav-accent text-sidenav-accent-foreground" } else { "text-muted-foreground hover:bg-sidenav-accent/60 hover:text-sidenav-accent-foreground" };
    rsx! {
        div { class: "mt-0.5 flex items-center gap-1",
            Link { to: Route::AccountPage {}, class: "flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-2 py-1.5 text-sm {c}",
                span { class: "grid size-5 shrink-0 place-items-center rounded-full bg-muted text-[11px] font-semibold text-foreground", "{initial}" }
                span { class: "min-w-0 flex-1 truncate", "{me.label()}" }
            }
            button { class: "grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-sidenav-accent hover:text-foreground", title: tr("Abmelden"),
                onclick: move |_| crate::account::logout(),
                icons::LogOut { class: "size-4 pointer-events-none" }
            }
        }
    }
}

#[component]
fn BottomNav() -> Element {
    let route = use_route::<Route>();
    let s = section(&route);
    let item = |active: bool| {
        if active {
            "press relative flex flex-1 flex-col items-center justify-center gap-1 text-[10px] font-semibold text-foreground [&_svg]:size-[22px] [&_svg]:stroke-[2.25]"
        } else {
            "press relative flex flex-1 flex-col items-center justify-center gap-1 text-[10px] font-medium text-muted-foreground [&_svg]:size-[22px]"
        }
    };
    let lib = matches!(s, "songs" | "albums" | "artists" | "playlists" | "favorites" | "history" | "stats" | "downloads" | "library");
    rsx! {
        nav { class: "glass pb-safe px-safe flex border-t border-white/5",
            div { class: "flex h-14 w-full",
                Link { class: item(s == "home"), to: Route::Home {}, if s == "home" { span { class: "tab-active-dot" } } icons::House {} {tr("Start")} }
                Link { class: item(s == "search"), to: Route::Search { q: String::new() }, if s == "search" { span { class: "tab-active-dot" } } icons::Search {} {tr("Suche")} }
                Link { class: item(lib), to: Route::Albums {}, if lib { span { class: "tab-active-dot" } } icons::Library {} {tr("Bibliothek")} }
                Link { class: item(s == "feed"), to: Route::Feed {}, if s == "feed" { span { class: "tab-active-dot" } } icons::Sparkles {} {tr("Für dich")} }
                Link { class: item(s == "youtube"), to: Route::YouTube { q: String::new(), tab: String::new() }, if s == "youtube" { span { class: "tab-active-dot" } } icons::Youtube {} {tr("YouTube")} }
            }
        }
    }
}
