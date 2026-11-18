//! Alle Songs: sortierbar, lädt beim Scrollen nach (je 200).

use crate::api;
use crate::components::common::*;
use crate::components::overlay::toast_error;
use crate::components::tracks::TrackList;
use crate::i18n::{tr, trf};
use crate::pages::LibraryTabs;
use crate::state::LIBRARY_VERSION;
use crate::{player, util};
use dioxus::prelude::*;
use nrzmusic_types::{Page, PlayableTrack, Track};

const PAGE: usize = 200;

#[component]
pub fn Songs() -> Element {
    let mut sort = use_signal(|| util::storage_get("nrzmusic-songs-sort").unwrap_or_else(|| "added".into()));
    let mut order = use_signal(|| util::storage_get("nrzmusic-songs-order").unwrap_or_else(|| "desc".into()));
    let items = use_signal(Vec::<Track>::new);
    let total = use_signal(|| 0u64);
    let loading = use_signal(|| false);
    let first = use_signal(|| true);
    let generation = use_signal(|| 0u64);

    let load = move |reset: bool| {
        let (mut generation, mut loading, mut items, mut total, mut first) = (generation, loading, items, total, first);
        // Nur peek: wird auch aus dem Effekt aufgerufen und soll ihn nicht erneut auslösen
        if !reset && (*loading.peek() || items.peek().len() as u64 >= *total.peek()) {
            return;
        }
        let g = if reset { *generation.peek() + 1 } else { *generation.peek() };
        if reset {
            generation.set(g);
        }
        let offset = if reset { 0 } else { items.peek().len() };
        let url = format!("/tracks?sort={}&order={}&offset={offset}&limit={PAGE}", sort.peek(), order.peek());
        loading.set(true);
        spawn(async move {
            let res = api::get::<Page<Track>>(&url).await;
            // Inzwischen neu sortiert: Ergebnis verwerfen
            if *generation.peek() != g {
                return;
            }
            match res {
                Ok(p) => {
                    if reset {
                        items.set(p.items);
                    } else {
                        items.write().extend(p.items);
                    }
                    total.set(p.total);
                }
                Err(e) => toast_error(e),
            }
            loading.set(false);
            first.set(false);
        });
    };
    use_effect(move || {
        let _ = (sort(), order(), LIBRARY_VERSION());
        load(true);
    });

    let tracks: Vec<PlayableTrack> = items.read().iter().cloned().map(PlayableTrack::Local).collect();
    let extra: Vec<String> = match sort().as_str() {
        "played" => items.read().iter().map(|t| trf("{0}×", &[&t.play_count])).collect(),
        "added" => items.read().iter().map(|t| util::date(t.added_at)).collect(),
        "year" => items.read().iter().map(|t| t.year.map(|y| y.to_string()).unwrap_or_default()).collect(),
        _ => Vec::new(),
    };
    let shown = items.read().len() as u64;
    let t_play = tracks.clone();
    let t_shuffle = tracks.clone();
    rsx! {
        LibraryTabs {}
        PageHeader {
            title: tr("Songs"),
            description: if total() > 0 { util::tracks_count(total()) } else { String::new() },
            actions: rsx! {
                button { class: "{btn(BTN_PRIMARY)} rounded-full", disabled: shown == 0, onclick: move |_| player::play_now(t_play.clone(), 0),
                    icons::Play { class: "size-4 fill-current" } {tr("Abspielen")}
                }
                button { class: "{btn(BTN_OUTLINE)} rounded-full", disabled: shown == 0, onclick: move |_| player::play_shuffled(t_shuffle.clone()),
                    icons::Shuffle { class: "size-4" } {tr("Zufall")}
                }
            }
        }
        div { class: "mb-4 flex flex-wrap items-center gap-2",
            span { class: "text-sm text-muted-foreground", {tr("Sortieren:")} }
            select { class: tw_merge::tw_merge!(SELECT, "w-auto"),
                value: "{sort}",
                onchange: move |e| {
                    let v = e.value();
                    util::storage_set("nrzmusic-songs-sort", &v);
                    sort.set(v);
                },
                for (v , l) in [("added", tr("Hinzugefügt")), ("title", tr("Titel")), ("artist", tr("Künstler")), ("album", tr("Album")), ("played", tr("Wiedergaben")), ("duration", tr("Dauer"))] {
                    option { value: v, selected: sort() == v, "{l}" }
                }
            }
            button { class: "{btn(BTN_OUTLINE)} {BTN_ICON}",
                title: if order() == "asc" { tr("Aufsteigend") } else { tr("Absteigend") },
                onclick: move |_| {
                    let v = if order() == "asc" { "desc" } else { "asc" };
                    util::storage_set("nrzmusic-songs-order", v);
                    order.set(v.into());
                },
                if order() == "asc" { icons::ArrowUp { class: "size-4" } } else { icons::ArrowDown { class: "size-4" } }
            }
        }
        if first() && loading() {
            Loading {}
        } else if tracks.is_empty() {
            EmptyState { title: tr("Noch keine Songs"), description: tr("Lege Musik in den Musikordner oder lade sie unter „Ordner & Upload“ hoch."), icon: rsx! { icons::Music {} } }
        } else {
            TrackList { tracks, extra }
            if shown < total() {
                div {
                    class: "flex justify-center py-6",
                    // Nachladen, sobald das Ende sichtbar wird
                    onvisible: move |e| {
                        if e.is_intersecting().unwrap_or(false) {
                            load(false);
                        }
                    },
                    button { class: btn(BTN_OUTLINE), disabled: loading(), onclick: move |_| load(false),
                        if loading() { icons::LoaderCircle { class: "size-4 animate-spin" } }
                        {trf("Mehr laden ({0} von {1})", &[&shown, &total()])}
                    }
                }
            }
        }
    }
}
