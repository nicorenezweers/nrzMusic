//! Statistik: Hörzeit, meistgehörte Künstler, Titel und Genres, Tageszeit, Wochentag und
//! Verlauf – aus den gemeldeten Wiedergaben (`/api/v1/stats`). Eine Farbe (Vordergrund),
//! dünne Balken mit runden Enden, Werte beim Darüberfahren.

use crate::api;
use crate::components::common::*;
use crate::i18n::{tr, trf};
use crate::pages::{LibraryTabs, LoadError};
use crate::state::STATS_VERSION;
use crate::{player, util};
use dioxus::prelude::*;
use nrzmusic_types::{StatEntry, Stats};

const RANGES: &[(u32, &str)] = &[(7, "7 Tage"), (30, "4 Wochen"), (365, "12 Monate"), (0, "Gesamt")];

/// Hörzeit lesbar: "3 h 12 min", "45 min", "20 s".
fn listen_time(ms: u64) -> String {
    let mins = ms / 60_000;
    if mins >= 60 {
        let h = mins / 60;
        if h >= 100 { trf("{0} h", &[&util::number(h)]) } else { trf("{0} h {1} min", &[&h, &(mins % 60)]) }
    } else if mins > 0 {
        trf("{0} min", &[&mins])
    } else {
        trf("{0} s", &[&(ms / 1000)])
    }
}

fn tz_offset() -> i64 {
    js_sys::Date::new_0().get_timezone_offset() as i64
}

#[component]
pub fn StatsPage() -> Element {
    let mut days = use_signal(|| util::storage_get("nrzmusic-stats-days").and_then(|v| v.parse().ok()).unwrap_or(30u32));
    let data = use_resource(move || async move {
        let _ = STATS_VERSION();
        api::get::<Stats>(&format!("/stats?days={}&tz={}", days(), tz_offset())).await
    });
    rsx! {
        LibraryTabs {}
        PageHeader { title: tr("Statistiken"), description: tr("Was du wie lange gehört hast – lokal und YouTube Music."),
            actions: rsx! {
                div { class: "flex rounded-full bg-muted p-1", role: "tablist",
                    for (d , label) in RANGES.iter().copied() {
                        button {
                            role: "tab",
                            "aria-selected": "{days() == d}",
                            class: if days() == d { "rounded-full bg-background px-3 py-1 text-sm font-medium shadow-sm" } else { "rounded-full px-3 py-1 text-sm text-muted-foreground hover:text-foreground" },
                            onclick: move |_| {
                                days.set(d);
                                util::storage_set("nrzmusic-stats-days", &d.to_string());
                            },
                            {tr(label)}
                        }
                    }
                }
            }
        }
        match &*data.read() {
            None => rsx! { Loading {} },
            Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
            Some(Ok(s)) if s.total_ms == 0 => rsx! {
                EmptyState { title: tr("Noch keine Hörzeit"), description: tr("Sobald du Musik hörst, entsteht hier deine Statistik – wie lange, welche Künstler, welche Genres."), icon: rsx! { icons::ChartColumn {} } }
            },
            Some(Ok(s)) => rsx! { StatsBody { stats: s.clone() } },
        }
    }
}

#[component]
fn StatsBody(stats: Stats) -> Element {
    let s = stats;
    let skip_rate = if s.plays + s.skips > 0 { (s.skips as f64 / (s.plays + s.skips).max(1) as f64 * 100.0).round() as u64 } else { 0 };
    let avg_day = if s.days > 0 { s.total_ms / s.days as u64 } else { 0 };
    rsx! {
        // Kennzahlen
        div { class: "mb-8 grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5",
            Tile { label: tr("Hörzeit"), value: listen_time(s.total_ms),
                hint: if s.days > 0 { trf("ø {0} pro Tag", &[&listen_time(avg_day)]) } else { s.since.map(|t| trf("seit {0}", &[&util::date(t)])).unwrap_or_default() } }
            Tile { label: tr("Wiedergaben"), value: util::number(s.plays), hint: tr("ab 30 Sekunden") }
            Tile { label: tr("Titel"), value: util::number(s.tracks), hint: tr("verschiedene") }
            Tile { label: tr("Künstler"), value: util::number(s.artists), hint: tr("verschiedene") }
            Tile { label: tr("Übersprungen"), value: format!("{skip_rate} %"), hint: trf("{0} Mal", &[&util::number(s.skips)]) }
        }

        // Verlauf
        section { class: "mb-8",
            h2 { class: "mb-3 text-lg font-semibold tracking-tight", if s.bucket == "day" { {tr("Hörzeit pro Tag")} } else { {tr("Hörzeit pro Monat")} } }
            Columns {
                values: s.timeline.iter().map(|b| b.ms).collect::<Vec<_>>(),
                labels: s.timeline.iter().map(|b| bucket_label(b.start, &s.bucket)).collect::<Vec<_>>(),
                tick_every: if s.timeline.len() > 14 { (s.timeline.len() / 7).max(1) } else { 1 },
                height: "h-40",
            }
        }

        div { class: "grid gap-8 lg:grid-cols-2",
            section {
                h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Top-Künstler")} }
                Ranking { entries: s.top_artists.clone(), round: true }
            }
            section {
                h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Top-Genres")} }
                if s.top_genres.is_empty() {
                    p { class: "text-sm text-muted-foreground", {tr("Noch keine Genres bekannt – lokale Titel bringen sie in den Tags mit, für YouTube-Künstler werden sie im Hintergrund nachgeschlagen.")} }
                } else {
                    Ranking { entries: s.top_genres.clone(), round: false }
                }
            }
            section { class: "lg:col-span-2",
                div { class: "mb-3 flex items-end justify-between gap-3",
                    h2 { class: "text-lg font-semibold tracking-tight", {tr("Top-Titel")} }
                    if !s.top_tracks.is_empty() {
                        button { class: "{btn(BTN_OUTLINE)} {BTN_SM} rounded-full",
                            onclick: {
                                let t: Vec<_> = s.top_tracks.iter().map(|x| x.track.clone()).collect();
                                move |_| player::play_now(t.clone(), 0)
                            },
                            icons::Play { class: "size-3.5 fill-current" }
                            {tr("Abspielen")}
                        }
                    }
                }
                div { class: "grid gap-1 md:grid-cols-2 md:gap-x-6",
                    for (i , t) in s.top_tracks.iter().cloned().enumerate() {
                        div { key: "{t.track.key()}", class: "flex items-center gap-3 rounded-lg p-1.5 hover:bg-accent/50",
                            ondoubleclick: {
                                let all: Vec<_> = s.top_tracks.iter().map(|x| x.track.clone()).collect();
                                move |_| player::play_now(all.clone(), i)
                            },
                            span { class: "w-5 text-right text-sm tabular-nums text-muted-foreground", "{i + 1}" }
                            Cover { src: t.track.artwork_url(96), class: "size-10" }
                            div { class: "min-w-0 flex-1",
                                p { class: "truncate text-sm font-medium", "{t.track.title()}" }
                                p { class: "truncate text-xs text-muted-foreground", "{t.track.artist()}" }
                            }
                            div { class: "text-right",
                                p { class: "text-sm tabular-nums", {listen_time(t.ms)} }
                                p { class: "text-xs tabular-nums text-muted-foreground", {trf("{0}× gespielt", &[&t.plays])} }
                            }
                        }
                    }
                }
            }
            section {
                h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Tageszeit")} }
                Columns {
                    values: s.by_hour.clone(),
                    labels: (0..24).map(|h| format!("{h:02}:00")).collect::<Vec<_>>(),
                    tick_every: 6,
                    height: "h-32",
                }
            }
            section {
                h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Wochentag")} }
                Columns {
                    values: s.by_weekday.clone(),
                    labels: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"].iter().map(|d| tr(d)).collect::<Vec<_>>(),
                    tick_every: 1,
                    height: "h-32",
                }
            }
        }
    }
}

fn bucket_label(start: i64, bucket: &str) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(start as f64 * 1000.0));
    let (day, month, year) = (d.get_utc_date(), d.get_utc_month() + 1, d.get_utc_full_year());
    if bucket == "day" { format!("{day}.{month}.") } else { format!("{month:02}/{}", year % 100) }
}

#[component]
fn Tile(#[props(into)] label: String, #[props(into)] value: String, #[props(into)] hint: String) -> Element {
    rsx! {
        div { class: "rounded-xl border bg-card p-4",
            p { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", "{label}" }
            p { class: "mt-1 text-2xl font-semibold tabular-nums tracking-tight", "{value}" }
            if !hint.is_empty() {
                p { class: "mt-0.5 text-xs text-muted-foreground", "{hint}" }
            }
        }
    }
}

/// Rangliste mit Balken (Länge = Hörzeit relativ zum Ersten), Werte direkt daneben.
#[component]
fn Ranking(entries: Vec<StatEntry>, round: bool) -> Element {
    let max = entries.iter().map(|e| e.ms).max().unwrap_or(1).max(1);
    rsx! {
        div { class: "flex flex-col gap-2.5",
            for (i , e) in entries.iter().cloned().enumerate() {
                div { key: "{i}-{e.name}", class: "flex items-center gap-3",
                    title: "{e.name}: {listen_time(e.ms)} · {e.plays}×",
                    span { class: "w-5 text-right text-sm tabular-nums text-muted-foreground", "{i + 1}" }
                    if round {
                        Cover { src: e.image.clone(), class: "size-9", round: true, icon: rsx! { icons::MicVocal {} } }
                    }
                    div { class: "min-w-0 flex-1",
                        div { class: "flex items-baseline justify-between gap-2",
                            span { class: "truncate text-sm font-medium", "{e.name}" }
                            span { class: "shrink-0 text-xs tabular-nums text-muted-foreground", {listen_time(e.ms)} }
                        }
                        div { class: "mt-1 h-1.5 overflow-hidden rounded-full bg-muted",
                            div { class: "h-full rounded-full bg-foreground/80", style: format!("width: {:.1}%", e.ms as f64 / max as f64 * 100.0) }
                        }
                    }
                }
            }
        }
    }
}

/// Säulen (eine Reihe), Achsenbeschriftung nur jede `tick_every`-te, Wert beim Darüberfahren.
#[component]
fn Columns(values: Vec<u64>, labels: Vec<String>, tick_every: usize, #[props(into)] height: String) -> Element {
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let tick = tick_every.max(1);
    rsx! {
        div {
            div { class: "flex {height} items-end gap-[2px] border-b border-border/60",
                for (i , v) in values.iter().copied().enumerate() {
                    div { key: "{i}", class: "group relative flex h-full flex-1 items-end",
                        title: format!("{}: {}", labels.get(i).cloned().unwrap_or_default(), listen_time(v)),
                        div {
                            class: "w-full rounded-t-[4px] bg-foreground/75 transition-colors group-hover:bg-foreground",
                            style: format!("height: {}%", if v == 0 { 0.0 } else { (v as f64 / max as f64 * 100.0).max(2.0) }),
                        }
                    }
                }
            }
            div { class: "mt-1.5 flex gap-[2px]",
                for (i , l) in labels.iter().cloned().enumerate() {
                    span { key: "{i}", class: "flex-1 whitespace-nowrap text-center text-[10px] tabular-nums text-muted-foreground",
                        if i % tick == 0 { "{l}" }
                    }
                }
            }
        }
    }
}
