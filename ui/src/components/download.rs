//! Downloads in Listen und Kacheln: Plakette am YouTube-Titel (auf dem Server, läuft,
//! fehlgeschlagen) und der Knopf "Alle herunterladen" für Album-, Playlist- und andere Seiten.

use crate::components::common::*;
use crate::components::overlay::MenuItem;
use crate::downloads::{self, DlState};
use crate::i18n::{tr, trf};
use dioxus::prelude::*;
use nrzmusic_types::{PlayableTrack, YtTrack};

/// Kleiner Fortschrittsring (0.0 – 1.0).
#[component]
pub fn ProgressRing(value: f32, #[props(into, default)] class: String) -> Element {
    // Umfang bei r = 6
    const C: f32 = 37.699;
    let v = value.clamp(0.0, 1.0);
    let offset = C * (1.0 - v);
    rsx! {
        svg { class: "-rotate-90 {class}", view_box: "0 0 16 16", fill: "none", "aria-hidden": "true",
            circle { cx: "8", cy: "8", r: "6", stroke: "currentColor", "stroke-opacity": "0.25", "stroke-width": "2" }
            circle { cx: "8", cy: "8", r: "6", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
                "stroke-dasharray": "{C}", "stroke-dashoffset": "{offset}",
            }
        }
    }
}

/// Prozent als Text ("42 %").
pub fn percent(v: f32) -> String {
    format!("{} %", (v.clamp(0.0, 1.0) * 100.0).round() as u32)
}

/// Plakette für einen YouTube-Titel: heruntergeladen, wartet, läuft (Ring) oder fehlgeschlagen.
#[component]
pub fn DownloadBadge(video_id: String, #[props(default)] percent_text: bool) -> Element {
    let Some(s) = downloads::state_of(&video_id) else { return rsx! {} };
    match s {
        DlState::Downloaded(_) => rsx! {
            span { class: "inline-flex shrink-0 text-muted-foreground [&_svg]:pointer-events-none", title: tr("Auf dem Server"), "aria-label": tr("Auf dem Server"),
                icons::HardDrive { class: "size-3.5" }
            }
        },
        DlState::Queued(_) => rsx! {
            span { class: "inline-flex shrink-0 text-muted-foreground [&_svg]:pointer-events-none", title: tr("Wartet auf den Download"),
                icons::Clock { class: "size-3.5" }
            }
        },
        DlState::Running(_, p) => rsx! {
            span { class: "inline-flex shrink-0 items-center gap-1 text-foreground [&_svg]:pointer-events-none",
                title: if p >= 0.9 { downloads::phase(p) } else { trf("Wird heruntergeladen … {0}", &[&percent(p)]) },
                ProgressRing { value: p, class: "size-3.5" }
                if percent_text {
                    span { class: "text-[10px] tabular-nums", {percent(p)} }
                }
            }
        },
        DlState::Failed(_, err) => rsx! {
            span { class: "inline-flex shrink-0 text-warning [&_svg]:pointer-events-none", title: trf("Download fehlgeschlagen: {0}", &[&tr(&err)]),
                icons::TriangleAlert { class: "size-3.5" }
            }
        },
    }
}

/// Wie viele der Titel auf dem Server liegen bzw. gerade laden (reaktiv).
fn counts(tracks: &[YtTrack]) -> (usize, usize) {
    let mut done = 0;
    let mut open = 0;
    for t in tracks {
        match downloads::state_of(&t.video_id) {
            Some(DlState::Downloaded(_)) => done += 1,
            Some(DlState::Queued(_) | DlState::Running(..)) => open += 1,
            _ => {}
        }
    }
    (done, open)
}

/// Knopf "Album herunterladen" / "Alle herunterladen" mit Stand (alles da, läuft gerade).
#[component]
pub fn DownloadAllButton(tracks: Vec<YtTrack>, #[props(into)] label: String) -> Element {
    if tracks.is_empty() || !downloads::available() {
        return rsx! {};
    }
    let total = tracks.len();
    let (done, open) = counts(&tracks);
    let pending = *downloads::PENDING.read() > 0;
    if done == total {
        return rsx! {
            span { class: "inline-flex h-9 items-center gap-2 rounded-md px-3 text-sm text-muted-foreground [&_svg]:pointer-events-none", title: tr("Alle Titel liegen als Datei im Musikordner."),
                icons::HardDrive { class: "size-4" }
                {tr("Auf dem Server")}
            }
        };
    }
    rsx! {
        button { class: btn(BTN_OUTLINE),
            title: if done > 0 { trf("{0} von {1} schon auf dem Server", &[&done, &total]) } else { String::new() },
            disabled: pending,
            onclick: move |_| downloads::download(tracks.clone()),
            if pending {
                icons::LoaderCircle { class: "size-4 animate-spin" }
                {tr("Wird eingereiht …")}
            } else if open > 0 {
                icons::LoaderCircle { class: "size-4 animate-spin" }
                {trf("Wird heruntergeladen … {0}/{1}", &[&done, &total])}
            } else {
                icons::Download { class: "size-4" }
                "{label}"
            }
        }
    }
}

/// Menüeintrag "YouTube-Titel herunterladen" für gemischte Listen (nur wenn es welche gibt).
#[component]
pub fn DownloadYoutubeItem(tracks: Vec<PlayableTrack>) -> Element {
    let yt = downloads::youtube_of(&tracks);
    if yt.is_empty() || !downloads::available() {
        return rsx! {};
    }
    let (done, _) = counts(&yt);
    let all = done == yt.len();
    rsx! {
        MenuItem { disabled: all, onclick: move |_| downloads::download(yt.clone()),
            if all { icons::HardDrive {} } else { icons::Download {} }
            if all { {tr("YouTube-Titel schon auf dem Server")} } else { {tr("YouTube-Titel herunterladen")} }
        }
    }
}
