//! Downloads: Aufträge mit Fortschritt (abbrechen, erneut versuchen, aus der Liste nehmen,
//! zum lokalen Album), Stand von yt-dlp/ffmpeg/Deno und "yt-dlp aktualisieren".

use crate::api;
use crate::components::common::*;
use crate::components::download::{ProgressRing, percent};
use crate::components::overlay::{toast, toast_error};
use crate::components::tracks::{client_xy, open_menu};
use crate::downloads::{self, JOBS, LOADED, OFF};
use crate::i18n::{tr, trf};
use crate::pages::{LibraryTabs, LoadError};
use crate::{Route, player, util};
use dioxus::prelude::*;
use nrzmusic_types::{DownloadJob, DownloadStatus, PlayableTrack, Track, YtDlpInfo};

#[component]
pub fn Downloads() -> Element {
    // Beim Öffnen frisch laden (Live-Ereignisse halten die Liste danach aktuell)
    use_future(downloads::load);
    let mut tool = use_resource(|| async { api::get::<YtDlpInfo>("/downloads/tool").await });
    let mut updating = use_signal(|| false);
    let jobs = JOBS.read().clone();
    let off = OFF.read().clone();
    let running = jobs.iter().filter(|j| j.status == DownloadStatus::Running).count();
    let queued = jobs.iter().filter(|j| j.status == DownloadStatus::Queued).count();
    let finished = jobs.len() - running - queued;
    let description = if running + queued > 0 {
        trf("{0} laufen · {1} warten", &[&running, &queued])
    } else if jobs.is_empty() {
        String::new()
    } else {
        tr("Alles erledigt.")
    };

    let update_tool = move |_| {
        spawn(async move {
            updating.set(true);
            match api::action_json::<YtDlpInfo>("/downloads/tool/update").await {
                Ok(info) => {
                    toast(info.message.as_deref().filter(|m| !m.is_empty()).map(tr).unwrap_or_else(|| tr("yt-dlp ist aktuell.")));
                    tool.restart();
                }
                Err(e) => toast_error(e),
            }
            updating.set(false);
        });
    };

    rsx! {
        LibraryTabs {}
        div { class: "mx-auto flex max-w-4xl flex-col gap-6",
            // Leere Beschreibung und Knopfleiste nicht ausgeben (sonst bleibt eine Lücke)
            match (description.is_empty(), finished > 0) {
                (true, false) => rsx! { PageHeader { title: tr("Downloads") } },
                (false, false) => rsx! { PageHeader { title: tr("Downloads"), description: description.clone() } },
                (empty, true) => rsx! {
                    PageHeader { title: tr("Downloads"), description: if empty { None } else { Some(description.clone()) },
                        actions: rsx! {
                            button { class: btn(BTN_OUTLINE), onclick: move |_| downloads::clear_finished(), icons::ListChecks {} {tr("Erledigte entfernen")} }
                        }
                    }
                },
            }
            if let Some(msg) = off {
                EmptyState {
                    title: tr("Downloads sind ausgeschaltet"),
                    description: msg,
                    icon: rsx! { icons::Download {} },
                    Link { class: btn(BTN_OUTLINE), to: Route::SettingsPage {}, icons::Settings {} {tr("Zu den Einstellungen")} }
                }
            } else {
                Callout { text: tr("Heruntergeladene Titel landen im Musikordner unter „YouTube/Künstler/Album“ und erscheinen wie jede andere Datei in der Bibliothek. Liegt ein YouTube-Titel schon auf dem Server, spielt nrzMusic die Datei.") }
                if !LOADED() {
                    Loading {}
                } else if jobs.is_empty() {
                    EmptyState {
                        title: tr("Keine Downloads"),
                        description: tr("„Herunterladen“ gibt es im Menü „…“ jedes YouTube-Titels und auf Album- und Playlist-Seiten."),
                        icon: rsx! { icons::Download {} },
                    }
                } else {
                    div { class: "flex flex-col divide-y overflow-hidden rounded-xl border bg-card",
                        for j in jobs.iter().cloned() {
                            JobRow { key: "{j.id}", job: j }
                        }
                    }
                }
            }
            Section { title: tr("yt-dlp"),
                description: tr("Lädt die Titel von YouTube. ffmpeg wandelt um, eine JavaScript-Laufzeit (Deno) löst YouTubes Prüfungen."),
                actions: rsx! {
                    button { class: "{btn(BTN_OUTLINE)} {BTN_SM}", disabled: updating(), onclick: update_tool,
                        icons::RefreshCw { class: if updating() { "size-4 animate-spin" } else { "size-4" } }
                        {tr("yt-dlp aktualisieren")}
                    }
                },
                match &*tool.read() {
                    None => rsx! { Loading {} },
                    Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
                    Some(Ok(info)) => rsx! { ToolInfo { info: info.clone() } },
                }
            }
        }
    }
}

#[component]
fn ToolInfo(info: YtDlpInfo) -> Element {
    rsx! {
        div { class: "grid gap-4",
            if !info.available {
                Callout { error: true, text: tr("yt-dlp wurde nicht gefunden. Installiere es auf dem Server oder gib den Pfad über NRZMUSIC_YTDLP an.") }
            }
            dl { class: "grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm",
                dt { class: "text-muted-foreground", "yt-dlp" }
                dd { class: "font-mono break-all", {info.version.clone().unwrap_or_else(|| tr("nicht gefunden"))} }
                dt { class: "text-muted-foreground", "ffmpeg" }
                dd { Check { ok: info.ffmpeg, yes: tr("vorhanden"), no: tr("fehlt – MP3 und das Umverpacken gehen nicht") } }
                dt { class: "text-muted-foreground", {tr("JavaScript-Laufzeit")} }
                dd {
                    match info.js_runtime.clone() {
                        Some(r) => rsx! { Check { ok: true, yes: r, no: String::new() } },
                        None => rsx! { Check { ok: false, yes: String::new(), no: tr("keine – manche Titel lassen sich dann nicht laden") } },
                    }
                }
                dt { class: "text-muted-foreground", {tr("Cookies")} }
                dd {
                    if info.cookies {
                        Check { ok: true, yes: tr("data/youtube-cookies.txt vorhanden"), no: String::new() }
                    } else {
                        span { class: "text-muted-foreground", {tr("keine (optional: data/youtube-cookies.txt – hilft, wenn YouTube „Sign in to confirm you’re not a bot“ meldet)")} }
                    }
                }
                dt { class: "text-muted-foreground", {tr("Zuletzt aktualisiert")} }
                dd { {info.last_update.map(util::datetime).unwrap_or_else(|| "–".into())} }
            }
            if let Some(m) = info.message.as_deref().filter(|m| !m.trim().is_empty()).map(tr) {
                pre { class: "max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/50 px-3 py-2 font-mono text-xs text-muted-foreground", "{m}" }
            }
        }
    }
}

#[component]
fn Check(ok: bool, #[props(into)] yes: String, #[props(into)] no: String) -> Element {
    rsx! {
        span { class: "inline-flex items-center gap-1.5",
            if ok {
                icons::CircleCheck { class: "size-4 shrink-0 text-success" }
                span { "{yes}" }
            } else {
                icons::CircleX { class: "size-4 shrink-0 text-destructive" }
                span { class: "text-muted-foreground", "{no}" }
            }
        }
    }
}

/// Zum lokalen Album (bzw. Künstler) des heruntergeladenen Titels.
async fn open_local(nav: dioxus::router::Navigator, track_id: i64) {
    match api::get::<Track>(&format!("/tracks/{track_id}")).await {
        Ok(t) if !t.album_id.is_empty() && !t.album.is_empty() => {
            nav.push(Route::AlbumPage { id: t.album_id });
        }
        Ok(t) if !t.artist_id.is_empty() => {
            nav.push(Route::ArtistPage { id: t.artist_id });
        }
        Ok(_) => {
            nav.push(Route::Songs {});
        }
        Err(e) => toast_error(e),
    }
}

#[component]
fn JobRow(job: DownloadJob) -> Element {
    let nav = use_navigator();
    let t = job.track.clone();
    let p = PlayableTrack::Youtube(t.clone());
    let key = p.key();
    let is_current = player::current_key().as_deref() == Some(key.as_str());
    let playing = is_current && player::PLAYING();
    let id = job.id;
    let (p1, p2, p3) = (p.clone(), p.clone(), p.clone());
    let sub = match t.album.as_ref() {
        Some(a) if !a.title.is_empty() => format!("{} · {}", t.artist_line(), a.title),
        _ => t.artist_line(),
    };
    rsx! {
        div {
            class: if is_current { "group flex items-center gap-3 bg-accent/40 px-3 py-2.5 md:px-4" } else { "group flex items-center gap-3 px-3 py-2.5 hover:bg-accent/30 md:px-4" },
            ondoubleclick: move |_| player::play_now(vec![p1.clone()], 0),
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let (x, y) = client_xy(&e);
                open_menu(x, y, p2.clone(), Vec::new());
            },
            div { class: "relative shrink-0",
                Cover { src: t.thumbnail.clone(), class: "size-11" }
                button {
                    class: "absolute inset-0 grid place-items-center rounded-md bg-black/50 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 max-md:opacity-0",
                    title: tr("Abspielen"),
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        if is_current { player::toggle() } else { player::play_now(vec![p3.clone()], 0) }
                    },
                    if playing { icons::Pause { class: "size-4 fill-current pointer-events-none" } } else { icons::Play { class: "size-4 fill-current pointer-events-none" } }
                }
            }
            div { class: "min-w-0 flex-1",
                div { class: "flex min-w-0 items-center gap-1.5",
                    p { class: "truncate text-sm font-medium", title: "{t.title}", "{t.title}" }
                    if t.explicit { Tag { text: "E" } }
                }
                p { class: "truncate text-xs text-muted-foreground", "{sub}" }
                match job.status {
                    DownloadStatus::Running => rsx! {
                        div { class: "mt-1.5 flex items-center gap-2",
                            div { class: "h-1 flex-1 overflow-hidden rounded-full bg-muted",
                                div {
                                    // Beim Umwandeln steht der Wert still – dann pulsieren
                                    class: if job.progress >= 0.9 { "h-full animate-pulse rounded-full bg-primary transition-[width]" } else { "h-full rounded-full bg-primary transition-[width]" },
                                    style: format!("width: {}%", (job.progress.clamp(0.0, 1.0) * 100.0).max(2.0)),
                                }
                            }
                            span { class: "shrink-0 text-right text-[11px] tabular-nums text-muted-foreground",
                                if job.progress >= 0.9 { {downloads::phase(job.progress)} } else { {percent(job.progress)} }
                            }
                        }
                    },
                    DownloadStatus::Queued => rsx! {
                        p { class: "mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground", icons::Clock { class: "size-3" } {tr("wartet")} }
                    },
                    DownloadStatus::Done => rsx! {
                        p { class: "mt-0.5 flex items-center gap-1 text-[11px] text-muted-foreground",
                            icons::HardDrive { class: "size-3" }
                            {match job.finished_at { Some(ts) => trf("auf dem Server · {0}", &[&util::datetime(ts)]), None => tr("auf dem Server") }}
                        }
                    },
                    DownloadStatus::Failed => rsx! {
                        p { class: "mt-0.5 line-clamp-2 break-words text-[11px] text-destructive", title: job.error.clone().unwrap_or_default(),
                            {trf("fehlgeschlagen: {0}", &[&job.error.as_deref().map(tr).unwrap_or_else(|| tr("unbekannter Fehler"))])}
                        }
                        if job.error.as_deref().is_some_and(downloads::needs_cookies) {
                            p { class: "mt-0.5 flex items-start gap-1 text-[11px] text-muted-foreground",
                                icons::Cookie { class: "mt-px size-3 shrink-0" }
                                {tr("YouTube verlangt eine Anmeldung: Cookies aus dem Browser (Netscape-Format) als data/youtube-cookies.txt ablegen, dann erneut versuchen.")}
                            }
                        }
                    },
                    DownloadStatus::Canceled => rsx! {
                        p { class: "mt-0.5 text-[11px] text-muted-foreground", {tr("abgebrochen")} }
                    },
                }
            }
            // Rechts: Zustand und Aktionen
            div { class: "flex shrink-0 items-center gap-0.5",
                match job.status {
                    DownloadStatus::Running => rsx! {
                        span { class: "mr-1 hidden text-foreground sm:inline-flex", ProgressRing { value: job.progress, class: "size-4" } }
                        button { class: ICON_BTN, title: tr("Abbrechen"), onclick: move |_| downloads::remove(id), icons::X { class: "size-4" } }
                    },
                    DownloadStatus::Queued => rsx! {
                        button { class: ICON_BTN, title: tr("Abbrechen"), onclick: move |_| downloads::remove(id), icons::X { class: "size-4" } }
                    },
                    DownloadStatus::Failed | DownloadStatus::Canceled => rsx! {
                        button { class: ICON_BTN, title: tr("Erneut versuchen"), onclick: move |_| downloads::retry(id), icons::RotateCw { class: "size-4" } }
                        button { class: ICON_BTN, title: tr("Aus der Liste entfernen"), onclick: move |_| downloads::remove(id), icons::Trash2 { class: "size-4" } }
                    },
                    DownloadStatus::Done => rsx! {
                        if let Some(tid) = job.track_id {
                            button { class: ICON_BTN, title: tr("Zum lokalen Album"), onclick: move |_| { spawn(open_local(nav, tid)); }, icons::Disc3 { class: "size-4" } }
                        }
                        button { class: ICON_BTN, title: tr("Aus der Liste entfernen (die Datei bleibt)"), onclick: move |_| downloads::remove(id), icons::Trash2 { class: "size-4" } }
                    },
                }
            }
        }
    }
}
