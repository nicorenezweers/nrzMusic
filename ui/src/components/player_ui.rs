//! Oberfläche des Players: Spielerleiste (Desktop), kleine Leiste (Handy), "Läuft gerade"
//! (Vollbild) und die Warteschlange.

use crate::components::common::*;
use crate::components::download::{DownloadBadge, DownloadYoutubeItem};
use crate::components::overlay::*;
use crate::components::tracks::{ArtistLinks, MenuExtra, album_route, client_xy, open_menu, pick_playlist};
use crate::i18n::{tr, trf};
use crate::player::{self, Repeat};
use crate::util;
use dioxus::prelude::*;
use nrzmusic_types::PlayableTrack;

// ------------------------------------------------------------------ Bausteine

/// Positionsregler mit Zeiten. Beim Ziehen folgt er der Maus, erst beim Loslassen wird gesprungen.
#[component]
pub fn SeekBar(#[props(default)] large: bool) -> Element {
    let mut dragging = use_signal(|| None::<f64>);
    let pos = player::POSITION();
    let dur = player::DURATION();
    let shown = dragging().unwrap_or(pos);
    let pct = if dur > 0.0 { (shown / dur * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let text = if large { "text-xs" } else { "text-[11px]" };
    rsx! {
        div { class: "flex w-full items-center gap-2",
            span { class: "w-10 shrink-0 text-right tabular-nums text-muted-foreground {text}", {util::clock(shown)} }
            input {
                r#type: "range",
                class: if large { "range always" } else { "range" },
                "aria-label": tr("Position"),
                min: "0",
                max: "{dur.max(0.0)}",
                step: "0.1",
                disabled: dur <= 0.0,
                value: "{shown}",
                style: "--value: {pct}%",
                oninput: move |e| {
                    if let Ok(v) = e.value().parse::<f64>() {
                        dragging.set(Some(v));
                    }
                },
                onchange: move |e| {
                    if let Ok(v) = e.value().parse::<f64>() {
                        player::seek(v);
                    }
                    dragging.set(None);
                },
            }
            span { class: "w-10 shrink-0 tabular-nums text-muted-foreground {text}", {util::clock(dur)} }
        }
    }
}

#[component]
pub fn Volume(#[props(into, default)] class: String) -> Element {
    let v = player::VOLUME();
    let muted = player::MUTED();
    let shown = if muted { 0.0 } else { v };
    rsx! {
        div { class: "flex items-center gap-1.5 {class}",
            button { class: ICON_BTN, title: if muted { tr("Ton an (M)") } else { tr("Stumm (M)") }, onclick: move |_| player::toggle_mute(),
                if muted || v == 0.0 {
                    icons::VolumeX { class: "size-4" }
                } else if v < 0.5 {
                    icons::Volume1 { class: "size-4" }
                } else {
                    icons::Volume2 { class: "size-4" }
                }
            }
            input {
                r#type: "range",
                class: "range w-24",
                "aria-label": tr("Lautstärke"),
                min: "0",
                max: "1",
                step: "0.01",
                value: "{shown}",
                style: "--value: {shown * 100.0}%",
                oninput: move |e| {
                    if let Ok(x) = e.value().parse::<f64>() {
                        player::set_volume(x);
                    }
                },
            }
        }
    }
}

/// Zufall, zurück, Play/Pause, weiter, Wiederholen.
#[component]
pub fn Controls(#[props(default)] large: bool) -> Element {
    let playing = player::PLAYING();
    let buffering = player::BUFFERING();
    let shuffle = player::SHUFFLE();
    let repeat = player::REPEAT();
    let has = player::INDEX().is_some();
    let on = "text-foreground after:absolute after:-bottom-0.5 after:size-1 after:rounded-full after:bg-foreground relative";
    let main = if large { "size-16 [&_svg]:size-7" } else { "size-9 [&_svg]:size-4" };
    let side = if large { "size-11 [&_svg]:size-6" } else { "size-8 [&_svg]:size-4" };
    let small = if large { "size-10 [&_svg]:size-5" } else { "size-8 [&_svg]:size-4" };
    rsx! {
        div { class: if large { "flex items-center justify-between gap-2" } else { "flex items-center gap-2" },
            button { class: format!("{ICON_BTN} {small} {}", if shuffle { on } else { "" }), title: if shuffle { tr("Zufall aus") } else { tr("Zufall an") },
                "aria-pressed": "{shuffle}",
                onclick: move |_| player::toggle_shuffle(),
                icons::Shuffle {}
            }
            button { class: "{ICON_BTN} {side} text-foreground", title: tr("Zurück (Shift+←)"), disabled: !has, onclick: move |_| player::prev(),
                icons::SkipBack { class: "fill-current" }
            }
            button {
                class: "grid {main} shrink-0 cursor-pointer place-items-center rounded-full bg-primary text-primary-foreground shadow-sm transition-transform hover:scale-105 active:scale-95 disabled:opacity-40 [&_svg]:pointer-events-none",
                title: if playing { tr("Pause (Leertaste)") } else { tr("Abspielen (Leertaste)") },
                disabled: !has,
                onclick: move |_| player::toggle(),
                if buffering && playing {
                    icons::LoaderCircle { class: "animate-spin" }
                } else if playing {
                    icons::Pause { class: "fill-current" }
                } else {
                    icons::Play { class: "fill-current translate-x-[1px]" }
                }
            }
            button { class: "{ICON_BTN} {side} text-foreground", title: tr("Weiter (Shift+→)"), disabled: !has, onclick: move |_| player::next(),
                icons::SkipForward { class: "fill-current" }
            }
            button { class: format!("{ICON_BTN} {small} {}", if repeat != Repeat::Off { on } else { "" }),
                title: match repeat { Repeat::Off => tr("Wiederholen: aus"), Repeat::All => tr("Wiederholen: alle"), Repeat::One => tr("Wiederholen: diesen Titel") },
                onclick: move |_| player::cycle_repeat(),
                if repeat == Repeat::One { icons::Repeat1 {} } else { icons::Repeat {} }
            }
        }
    }
}

/// Titel und Künstler des laufenden Titels mit Verweisen.
#[component]
fn NowTitle(track: PlayableTrack, #[props(default)] large: bool) -> Element {
    let title_class = if large { "truncate text-xl font-bold tracking-tight md:text-2xl" } else { "truncate text-sm font-medium" };
    let artist_class = if large { "text-base text-muted-foreground" } else { "text-xs text-muted-foreground" };
    let album = album_route(&track);
    let close = move |_| *player::NOW_PLAYING_OPEN.write() = false;
    rsx! {
        div { class: "flex min-w-0 flex-col",
            if let Some(r) = album {
                Link { class: "{title_class} hover:underline", to: r, onclick: close, title: "{track.title()}", "{track.title()}" }
            } else {
                span { class: "{title_class}", title: "{track.title()}", "{track.title()}" }
            }
            div { class: "flex min-w-0 items-center gap-1.5 {artist_class}",
                if matches!(track, PlayableTrack::Youtube(_)) {
                    icons::Youtube { class: "size-3.5 shrink-0" }
                    if player::playing_download() {
                        span { class: "inline-flex shrink-0 [&_svg]:pointer-events-none", title: tr("Heruntergeladen – spielt die Datei aus dem Musikordner"), icons::HardDrive { class: "size-3.5" } }
                    }
                }
                span { class: "flex min-w-0", onclick: close, ArtistLinks { track: track.clone() } }
            }
        }
    }
}

// ------------------------------------------------------------------ Spielerleiste

/// Leiste unten: Desktop gross, auf dem Handy klein (Tipp öffnet "Läuft gerade").
#[component]
pub fn PlayerBar() -> Element {
    let track = player::current();
    let queue_open = player::QUEUE_OPEN();
    let Some(t) = track else {
        return rsx! {
            div { class: "hidden h-20 shrink-0 items-center justify-center border-t bg-player px-4 text-sm text-muted-foreground md:flex",
                icons::Music { class: "mr-2 size-4" }
                {tr("Nichts in der Warteschlange – doppelt auf einen Titel klicken zum Abspielen.")}
            }
        };
    };
    let t_menu = t.clone();
    rsx! {
        // Desktop
        div { class: "hidden h-20 shrink-0 grid-cols-[minmax(0,1fr)_minmax(0,2fr)_minmax(0,1fr)] items-center gap-4 border-t bg-player px-4 md:grid",
            div { class: "flex min-w-0 items-center gap-3",
                button { class: "group relative shrink-0 cursor-pointer", title: tr("Läuft gerade"), onclick: move |_| *player::NOW_PLAYING_OPEN.write() = true,
                    Cover { src: t.artwork_url(96), class: "size-14", eager: true }
                    div { class: "absolute inset-0 grid place-items-center rounded-md bg-black/50 opacity-0 transition-opacity group-hover:opacity-100",
                        icons::ChevronUp { class: "size-5" }
                    }
                }
                NowTitle { track: t.clone() }
                Heart { track: t.clone() }
            }
            div { class: "flex min-w-0 flex-col items-center gap-0.5",
                Controls {}
                div { class: "w-full max-w-2xl", SeekBar {} }
            }
            div { class: "flex min-w-0 items-center justify-end gap-1",
                button { class: tw_merge::tw_merge!(ICON_BTN, "hidden lg:grid"), title: tr("Mehr"),
                    onclick: move |e: MouseEvent| {
                        let (x, y) = client_xy(&e);
                        open_menu(x, y - 360.0, t_menu.clone(), Vec::new());
                    },
                    icons::Ellipsis { class: "size-4" }
                }
                button { class: format!("{ICON_BTN} {}", if queue_open { "bg-accent text-foreground" } else { "" }), title: tr("Warteschlange"),
                    onclick: move |_| {
                        let open = *player::QUEUE_OPEN.peek();
                        *player::QUEUE_OPEN.write() = !open;
                    },
                    icons::ListMusic { class: "size-4" }
                }
                button { class: ICON_BTN, title: tr("Läuft gerade"), onclick: move |_| *player::NOW_PLAYING_OPEN.write() = true,
                    icons::Maximize2 { class: "size-4" }
                }
                Volume { class: "ml-1" }
            }
        }
    }
}

/// Erster Fingerpunkt eines Touch-Ereignisses (Bildschirmkoordinaten).
fn touch_xy(e: &TouchEvent) -> Option<(f64, f64)> {
    let t = e.data().touches_changed();
    let p = t.first()?.client_coordinates();
    Some((p.x, p.y))
}

/// Handy: schwebender Mini-Player über der Navigation, eingefärbt mit dem Cover.
/// Tippen oder nach oben wischen öffnet "Läuft gerade", seitlich wischen springt.
#[component]
pub fn MiniPlayer() -> Element {
    let mut start = use_signal(|| None::<(f64, f64)>);
    let mut shift = use_signal(|| 0.0f64);
    let Some(t) = player::current() else { return rsx! {} };
    let art = t.artwork_url(96);
    let playing = player::PLAYING();
    let buffering = player::BUFFERING();
    let dx = shift();
    let style = if dx != 0.0 { format!("transform: translateX({dx}px); opacity: {}", 1.0 - (dx.abs() / 400.0).min(0.5)) } else { String::new() };
    rsx! {
        div { class: "px-safe h-16 px-2 pt-1 pb-1",
            div {
                class: if dx == 0.0 { "relative flex h-14 items-center gap-3 overflow-hidden rounded-2xl border border-white/10 bg-popover/80 pr-1.5 pl-1.5 shadow-[0_8px_30px_rgb(0_0_0/0.45)] backdrop-blur-2xl transition-transform" } else { "relative flex h-14 items-center gap-3 overflow-hidden rounded-2xl border border-white/10 bg-popover/80 pr-1.5 pl-1.5 shadow-[0_8px_30px_rgb(0_0_0/0.45)] backdrop-blur-2xl" },
                style: "{style}",
                ontouchstart: move |e| { start.set(touch_xy(&e)); },
                ontouchmove: move |e| {
                    if let (Some((x0, y0)), Some((x, y))) = (start(), touch_xy(&e)) {
                        let (dx, dy) = (x - x0, y - y0);
                        if dx.abs() > dy.abs() && dx.abs() > 8.0 { shift.set(dx); }
                    }
                },
                ontouchend: move |e| {
                    let s0 = start.take();
                    shift.set(0.0);
                    if let (Some((x0, y0)), Some((x, y))) = (s0, touch_xy(&e)) {
                        let (dx, dy) = (x - x0, y - y0);
                        if dx.abs() > 70.0 && dx.abs() > dy.abs() {
                            if dx < 0.0 { player::next() } else { player::prev() }
                        } else if dy < -40.0 && dy.abs() > dx.abs() {
                            *player::NOW_PLAYING_OPEN.write() = true;
                        }
                    }
                },
                ontouchcancel: move |_| { start.set(None); shift.set(0.0); },
                if let Some(a) = art.clone() {
                    img { class: "mini-tint", src: "{a}", alt: "", referrerpolicy: "no-referrer" }
                }
                div { class: "pointer-events-none absolute inset-0 bg-gradient-to-r from-black/20 to-black/40" }
                button { class: "relative flex min-w-0 flex-1 items-center gap-3 text-left", onclick: move |_| *player::NOW_PLAYING_OPEN.write() = true,
                    Cover { src: art.clone(), class: "size-11 rounded-xl shadow-md", eager: true }
                    div { class: "flex min-w-0 flex-col",
                        span { class: "truncate text-[15px] font-semibold leading-tight", "{t.title()}" }
                        span { class: "truncate text-[13px] text-foreground/65", "{t.artist()}" }
                    }
                }
                div { class: "relative flex items-center",
                    Heart { track: t.clone(), class: "size-10 text-foreground/80 [&_svg]:size-5" }
                    button { class: "press grid size-10 place-items-center rounded-full text-foreground [&_svg]:pointer-events-none",
                        title: if playing { tr("Pause") } else { tr("Abspielen") },
                        onclick: move |_| player::toggle(),
                        if buffering && playing { icons::LoaderCircle { class: "size-6 animate-spin" } }
                        else if playing { icons::Pause { class: "size-6 fill-current" } }
                        else { icons::Play { class: "size-6 fill-current translate-x-[1px]" } }
                    }
                    button { class: "press grid size-10 place-items-center rounded-full text-foreground [&_svg]:pointer-events-none", title: tr("Weiter"), onclick: move |_| player::next(),
                        icons::SkipForward { class: "size-5 fill-current" }
                    }
                }
                MiniProgress {}
            }
        }
    }
}

/// Dünner Fortschrittsbalken der kleinen Leiste (eigene Komponente: ändert sich 4× pro Sekunde).
#[component]
fn MiniProgress() -> Element {
    let pos = player::POSITION();
    let dur = player::DURATION();
    let pct = if dur > 0.0 { (pos / dur * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    rsx! {
        div { class: "pointer-events-none absolute inset-x-3 bottom-0 h-[2px] overflow-hidden rounded-full bg-white/10",
            div { class: "h-full rounded-full bg-white/85", style: "width: {pct}%" }
        }
    }
}

// ------------------------------------------------------------------ Läuft gerade

#[component]
pub fn NowPlaying() -> Element {
    let mut show_queue = use_signal(|| false);
    // Ziehen nach unten (Handy): Startpunkt und aktuelle Verschiebung
    let mut drag_from = use_signal(|| None::<f64>);
    let mut drag = use_signal(|| 0.0f64);
    if !player::NOW_PLAYING_OPEN() {
        return rsx! {};
    }
    let Some(t) = player::current() else {
        return rsx! {
            div { class: "fixed inset-0 z-50 grid place-items-center bg-background p-6",
                EmptyState { title: tr("Nichts läuft gerade"), icon: rsx! { icons::Music {} },
                    button { class: btn(BTN_OUTLINE), onclick: move |_| *player::NOW_PLAYING_OPEN.write() = false, {tr("Schliessen")} }
                }
            }
        };
    };
    let art = t.artwork_url(600);
    let close = move |_| *player::NOW_PLAYING_OPEN.write() = false;
    let dy = drag();
    let sheet_style = if dy > 0.0 { format!("transform: translateY({dy}px); border-radius: 1.5rem 1.5rem 0 0") } else { String::new() };
    let on_start = move |e: TouchEvent| drag_from.set(touch_xy(&e).map(|p| p.1));
    let on_move = move |e: TouchEvent| {
        if let (Some(y0), Some((_, y))) = (drag_from(), touch_xy(&e)) {
            drag.set((y - y0).max(0.0));
        }
    };
    let on_end = move |_: TouchEvent| {
        drag_from.set(None);
        if drag() > 120.0 {
            *player::NOW_PLAYING_OPEN.write() = false;
        }
        drag.set(0.0);
    };
    let paused = !player::PLAYING();
    let t_add = t.clone();
    let t_radio = t.clone();
    let t_menu = t.clone();
    rsx! {
        div {
            class: if drag_from().is_some() { "np-sheet fixed inset-0 z-50 flex flex-col overflow-hidden bg-background animate-up" } else { "np-sheet settle fixed inset-0 z-50 flex flex-col overflow-hidden bg-background animate-up" },
            style: "{sheet_style}",
            ontouchcancel: move |_| { drag_from.set(None); drag.set(0.0); },
            role: "region",
            "aria-label": tr("Läuft gerade"),
            tabindex: "-1",
            onkeydown: move |e| if e.key() == Key::Escape { *player::NOW_PLAYING_OPEN.write() = false },
            // Farbe vom Cover
            if let Some(a) = art.clone() {
                img { class: "pointer-events-none absolute inset-0 size-full scale-125 object-cover opacity-35 blur-3xl", src: "{a}", alt: "", referrerpolicy: "no-referrer" }
            }
            div { class: "pointer-events-none absolute inset-0 bg-gradient-to-b from-background/40 via-background/70 to-background" }

            // Griff (Handy): zeigt, dass man nach unten wischen kann
            div { class: "relative flex h-4 shrink-0 items-end justify-center md:hidden",
                ontouchstart: on_start, ontouchmove: on_move, ontouchend: on_end,
                div { class: "grabber" }
            }
            header { class: "relative flex h-14 shrink-0 items-center gap-2 px-3 md:px-6",
                ontouchstart: on_start, ontouchmove: on_move, ontouchend: on_end,
                button { class: "{ICON_BTN} size-10", title: tr("Schliessen (Esc)"), onclick: close, icons::ChevronDown { class: "size-6" } }
                p { class: "flex-1 text-center text-xs font-medium uppercase tracking-wider text-muted-foreground", {tr("Läuft gerade")} }
                button { class: format!("{ICON_BTN} size-10 lg:hidden {}", if show_queue() { "bg-accent text-foreground" } else { "" }), title: tr("Warteschlange"),
                    onclick: move |_| show_queue.toggle(),
                    icons::ListMusic { class: "size-5" }
                }
                button { class: "{ICON_BTN} size-10", title: tr("Mehr"),
                    onclick: move |e: MouseEvent| {
                        let (x, y) = client_xy(&e);
                        open_menu(x, y, t_menu.clone(), Vec::new());
                    },
                    icons::Ellipsis { class: "size-5" }
                }
            }

            div { class: "relative flex min-h-0 flex-1 gap-8 px-5 pb-6 md:px-10 lg:px-16",
                // Links: Cover, Titel, Steuerung
                div { class: if show_queue() { "hidden min-h-0 flex-1 flex-col items-center overflow-y-auto lg:flex" } else { "flex min-h-0 flex-1 flex-col items-center overflow-y-auto" },
                    div { class: "my-auto flex w-full max-w-xl flex-col gap-5 py-2 md:gap-6",
                        div { class: "mx-auto w-full max-w-[min(100%,40dvh)] md:max-w-[min(100%,52vh)]",
                            ontouchstart: on_start, ontouchmove: on_move, ontouchend: on_end,
                            div { class: if paused { "np-cover paused" } else { "np-cover" },
                                Cover { src: art.clone(), class: "aspect-square w-full rounded-2xl shadow-2xl", eager: true }
                            }
                        }
                        div { class: "flex items-center gap-3",
                            div { class: "min-w-0 flex-1", NowTitle { track: t.clone(), large: true } }
                            Heart { track: t.clone(), class: "size-10 [&_svg]:size-5" }
                        }
                        SeekBar { large: true }
                        Controls { large: true }
                        div { class: "flex items-center justify-between gap-2",
                            Volume { class: "max-md:hidden" }
                            div { class: "flex items-center gap-1 md:ml-auto",
                                button { class: btn(&format!("{BTN_GHOST} {BTN_SM}")), onclick: move |_| pick_playlist(vec![t_add.to_ref()]),
                                    icons::ListPlus {} {tr("Zur Playlist")}
                                }
                                if matches!(t, PlayableTrack::Youtube(_)) {
                                    button { class: btn(&format!("{BTN_GHOST} {BTN_SM}")), onclick: move |_| player::start_radio(t_radio.clone()),
                                        icons::Radio {} {tr("Radio starten")}
                                    }
                                }
                            }
                        }
                    }
                }
                // Rechts: Warteschlange
                div { class: if show_queue() { "flex min-h-0 w-full flex-col lg:w-96" } else { "hidden min-h-0 w-96 flex-col lg:flex" },
                    div { class: "min-h-0 flex-1 overflow-hidden rounded-2xl border bg-card/60 backdrop-blur",
                        QueueList {}
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Warteschlange

/// Seitenleiste rechts mit der Warteschlange (Desktop), auf dem Handy bildschirmfüllend.
#[component]
pub fn QueuePanel() -> Element {
    if !player::QUEUE_OPEN() || player::NOW_PLAYING_OPEN() {
        return rsx! {};
    }
    rsx! {
        // Klick daneben schliesst – die Spielerleiste bleibt bedienbar
        div { class: "above-dock fixed inset-x-0 top-0 bottom-[7.5rem] z-40 bg-black/40 md:bottom-20 md:bg-transparent", onclick: move |_| *player::QUEUE_OPEN.write() = false }
        aside {
            class: "above-dock pt-safe fixed top-0 right-0 bottom-[7.5rem] z-40 flex w-full flex-col border-l bg-popover shadow-2xl animate-in md:top-14 md:bottom-20 md:w-96 md:pt-0",
            onkeydown: move |e| if e.key() == Key::Escape { *player::QUEUE_OPEN.write() = false },
            QueueList { closable: true }
        }
    }
}

#[component]
pub fn QueueList(#[props(default)] closable: bool) -> Element {
    let queue = player::QUEUE();
    let index = player::INDEX();
    let playing = player::PLAYING();
    let mut save_open = use_signal(|| false);
    let mut drag_from = use_signal(|| None::<usize>);
    let mut drag_over = use_signal(|| None::<usize>);
    let mut show_past = use_signal(|| false);
    let upcoming = index.map(|i| queue.len().saturating_sub(i + 1)).unwrap_or(queue.len());
    let start = match index {
        Some(i) if !show_past() => i,
        _ => 0,
    };
    let remaining_ms: u64 = queue.iter().skip(index.map(|i| i + 1).unwrap_or(0)).filter_map(|q| q.track.duration_ms()).sum();
    rsx! {
        div { class: "flex h-full min-h-0 flex-col",
            div { class: "flex items-center gap-2 border-b px-4 py-3",
                div { class: "min-w-0 flex-1",
                    h2 { class: "font-semibold leading-tight", {tr("Warteschlange")} }
                    p { class: "text-xs text-muted-foreground",
                        {trf("{0} als Nächstes", &[&upcoming])}
                        if remaining_ms > 0 { " · {util::long_duration(remaining_ms)}" }
                    }
                }
                Dropdown { align: Align::End, title: tr("Mehr"),
                    trigger_class: ICON_BTN,
                    trigger: rsx! { icons::Ellipsis { class: "size-4" } },
                    MenuItem { disabled: queue.is_empty(), onclick: move |_| save_open.set(true), icons::Save {} {tr("Als Playlist speichern")} }
                    MenuItem { disabled: queue.is_empty(), onclick: move |_| pick_playlist(player::QUEUE.peek().iter().map(|q| q.track.to_ref()).collect()),
                        icons::ListPlus {} {tr("Alle zu einer Playlist hinzufügen …")}
                    }
                    DownloadYoutubeItem { tracks: queue.iter().map(|q| q.track.clone()).collect::<Vec<_>>() }
                    MenuItem { onclick: move |_| show_past.toggle(), icons::History {}
                        if show_past() { {tr("Gespielte ausblenden")} } else { {tr("Gespielte zeigen")} }
                    }
                    MenuSeparator {}
                    MenuItem { destructive: true, disabled: upcoming == 0, onclick: move |_| player::clear_queue(), icons::ListX {} {tr("Warteschlange leeren")} }
                }
                if closable {
                    button { class: ICON_BTN, title: tr("Schliessen"), onclick: move |_| *player::QUEUE_OPEN.write() = false, icons::X { class: "size-4" } }
                }
            }
            div { class: "min-h-0 flex-1 overflow-y-auto p-2",
                if queue.is_empty() {
                    p { class: "px-3 py-10 text-center text-sm text-muted-foreground", {tr("Die Warteschlange ist leer.")} }
                }
                for (i , item) in queue.iter().cloned().enumerate().skip(start) {
                    {
                        let is_cur = Some(i) == index;
                        let past = index.is_some_and(|c| i < c);
                        let label = if is_cur { Some(tr("Läuft gerade")) } else if index.is_some_and(|c| i == c + 1) { Some(tr("Nächste Titel")) } else { None };
                        let over = drag_over() == Some(i) && drag_from().is_some_and(|f| f != i);
                        let len = queue.len();
                        let t_menu = item.track.clone();
                        let uid = item.uid;
                        rsx! {
                            div { key: "{item.uid}",
                            if let Some(l) = label {
                                p { class: "px-2 pt-3 pb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground", "{l}" }
                            }
                            div {
                                class: format!(
                                    "group flex cursor-default select-none items-center gap-3 rounded-lg px-2 py-1.5 hover:bg-accent/50 {} {} {}",
                                    if is_cur { "bg-accent/50" } else { "" },
                                    if past { "opacity-50" } else { "" },
                                    if over { "ring-2 ring-foreground/40" } else { "" },
                                ),
                                draggable: "true",
                                onclick: move |_| if !is_cur { player::jump(i) } else { player::toggle() },
                                oncontextmenu: move |e: MouseEvent| {
                                    e.prevent_default();
                                    let (x, y) = client_xy(&e);
                                    open_menu(x, y, t_menu.clone(), vec![MenuExtra::RemoveFromQueue(uid)]);
                                },
                                ondragstart: move |e: DragEvent| {
                                    if let Some(dt) = e.data().downcast::<web_sys::DragEvent>().and_then(|d| d.data_transfer()) {
                                        let _ = dt.set_data("text/plain", &i.to_string());
                                        dt.set_effect_allowed("move");
                                    }
                                    drag_from.set(Some(i));
                                },
                                ondragover: move |e: DragEvent| {
                                    if drag_from().is_some() {
                                        e.prevent_default();
                                        if drag_over() != Some(i) { drag_over.set(Some(i)); }
                                    }
                                },
                                ondrop: move |e: DragEvent| {
                                    if let Some(from) = drag_from() {
                                        e.prevent_default();
                                        e.stop_propagation();
                                        player::move_item(from, i);
                                    }
                                    drag_from.set(None);
                                    drag_over.set(None);
                                },
                                ondragend: move |_| { drag_from.set(None); drag_over.set(None); },
                                icons::GripVertical { class: "size-4 shrink-0 cursor-grab text-muted-foreground/60 max-md:hidden" }
                                div { class: "relative",
                                    Cover { src: item.track.artwork_url(96), class: "size-10" }
                                    if is_cur {
                                        div { class: "absolute inset-0 grid place-items-center rounded-md bg-black/50", EqBars { playing } }
                                    }
                                }
                                div { class: "min-w-0 flex-1",
                                    p { class: if is_cur { "truncate text-sm font-semibold" } else { "truncate text-sm" }, "{item.track.title()}" }
                                    div { class: "flex min-w-0 items-center gap-1 text-xs text-muted-foreground",
                                        if let PlayableTrack::Youtube(y) = &item.track {
                                            DownloadBadge { video_id: y.video_id.clone() }
                                        }
                                        span { class: "truncate", "{item.track.artist()}" }
                                    }
                                }
                                div { class: "flex items-center opacity-100 md:opacity-0 md:group-hover:opacity-100",
                                    button { class: "{ICON_BTN} size-7 max-md:hidden", title: tr("Nach oben"), disabled: i == 0,
                                        onclick: move |e: MouseEvent| { e.stop_propagation(); player::move_item(i, i - 1); },
                                        icons::ArrowUp { class: "size-3.5" }
                                    }
                                    button { class: "{ICON_BTN} size-7 max-md:hidden", title: tr("Nach unten"), disabled: i + 1 >= len,
                                        onclick: move |e: MouseEvent| { e.stop_propagation(); player::move_item(i, i + 1); },
                                        icons::ArrowDown { class: "size-3.5" }
                                    }
                                    button { class: "{ICON_BTN} size-7", title: tr("Entfernen"),
                                        onclick: move |e: MouseEvent| { e.stop_propagation(); player::remove(uid); },
                                        icons::X { class: "size-3.5" }
                                    }
                                }
                            }
                            }
                        }
                    }
                }
                if index.is_some() && player::FEED_MODE() {
                    p { class: "px-3 py-4 text-center text-xs text-muted-foreground", {tr("Läuft aus „Für dich“ – es kommen laufend neue Titel dazu.")} }
                } else if index.is_some() && upcoming == 0 && player::AUTOPLAY_RADIO() {
                    p { class: "px-3 py-4 text-center text-xs text-muted-foreground",
                        if matches!(player::current(), Some(PlayableTrack::Youtube(_))) { {tr("Danach geht es mit ähnlichen Titeln weiter (Radio).")} } else { {tr("Danach geht es mit Titeln für dich weiter.")} }
                    }
                }
            }
        }
        SaveQueueDialog { open: save_open }
    }
}

#[component]
fn SaveQueueDialog(open: Signal<bool>) -> Element {
    let mut name = use_signal(String::new);
    use_effect(move || {
        if open() {
            name.set(trf("Warteschlange vom {0}", &[&util::date(js_sys::Date::now() as i64 / 1000)]));
        }
    });
    let mut open2 = open;
    rsx! {
        Modal { open, title: tr("Als Playlist speichern"), description: trf("{0} Titel aus der Warteschlange", &[&player::QUEUE.read().len()]),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    let n = name().trim().to_string();
                    if !n.is_empty() {
                        player::save_queue_as_playlist(n);
                        open2.set(false);
                    }
                },
                Field { label: tr("Name"),
                    input { class: INPUT, value: "{name}", oninput: move |e| name.set(e.value()) }
                }
                div { class: "flex justify-end gap-2",
                    button { r#type: "button", class: btn(BTN_OUTLINE), onclick: move |_| open2.set(false), {tr("Abbrechen")} }
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: name().trim().is_empty(), {tr("Speichern")} }
                }
            }
        }
    }
}
