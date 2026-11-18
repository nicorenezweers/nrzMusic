//! "Für dich": unendlicher Feed aus dem Hörverhalten (Server: `src/feed.rs`). Abspielen startet
//! den Feed-Modus des Players – die Warteschlange füllt sich dann von selbst nach.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::{toast, toast_error};
use crate::components::tracks::{ArtistLinks, MenuButton, coarse_pointer};
use crate::i18n::{tr, trf};
use crate::{player, util};
use dioxus::prelude::*;
use nrzmusic_types::{DislikeRequest, FeedItem, FeedPage, FeedReason, FeedRequest, PlayableTrack};

/// Geladene Einträge (bleiben beim Seitenwechsel erhalten).
static ITEMS: GlobalSignal<Vec<FeedItem>> = Signal::global(Vec::new);
static COLD: GlobalSignal<bool> = Signal::global(|| false);
static LOADING: GlobalSignal<bool> = Signal::global(|| false);
/// Letzte Seite brachte nichts Neues – nicht weiter automatisch nachladen
static EXHAUSTED: GlobalSignal<bool> = Signal::global(|| false);
static ERROR: GlobalSignal<Option<String>> = Signal::global(|| None);

fn tracks() -> Vec<PlayableTrack> {
    ITEMS.peek().iter().map(|i| i.track.clone()).collect()
}

/// Nächste Seite holen und anhängen.
fn load_more() {
    if *LOADING.peek() {
        return;
    }
    *LOADING.write() = true;
    spawn(async move {
        let exclude: Vec<String> = ITEMS.peek().iter().map(|i| i.track.key()).collect();
        match api::post::<FeedPage, _>("/feed", &FeedRequest { exclude: exclude.clone(), count: Some(20) }).await {
            Ok(page) => {
                let known: std::collections::HashSet<String> = exclude.into_iter().collect();
                let new: Vec<FeedItem> = page.items.into_iter().filter(|i| !known.contains(&i.track.key())).collect();
                *EXHAUSTED.write() = new.is_empty();
                *COLD.write() = page.cold_start;
                *ERROR.write() = None;
                ITEMS.write().extend(new);
            }
            Err(e) => *ERROR.write() = Some(e.to_string()),
        }
        *LOADING.write() = false;
    });
}

fn reshuffle() {
    ITEMS.write().clear();
    *EXHAUSTED.write() = false;
    *ERROR.write() = None;
    load_more();
}

/// Ist das Ende der Liste fast im Bild?
fn near_end() -> bool {
    let Some(el) = util::document().and_then(|d| d.get_element_by_id("feed-end")) else { return false };
    let h = web_sys::window().and_then(|w| w.inner_height().ok()).and_then(|v| v.as_f64()).unwrap_or(800.0);
    el.get_bounding_client_rect().top() < h + 900.0
}

fn reason_text(r: &FeedReason) -> String {
    match r {
        FeedReason::Similar { to } => trf("Weil du „{0}“ magst", &[to]),
        FeedReason::Artist { name } => trf("Mehr von {0}", &[name]),
        FeedReason::Rediscover => tr("Lange nicht gehört"),
        FeedReason::Library => tr("Aus deiner Bibliothek"),
        FeedReason::Explore => tr("Neu für dich"),
        FeedReason::Trending => tr("Gerade beliebt"),
    }
}

#[component]
fn ReasonIcon(reason: FeedReason) -> Element {
    let c = "size-3 shrink-0";
    match reason {
        FeedReason::Similar { .. } => rsx! { icons::Sparkles { class: c } },
        FeedReason::Artist { .. } => rsx! { icons::MicVocal { class: c } },
        FeedReason::Rediscover => rsx! { icons::History { class: c } },
        FeedReason::Library => rsx! { icons::Library { class: c } },
        FeedReason::Explore => rsx! { icons::Compass { class: c } },
        FeedReason::Trending => rsx! { icons::TrendingUp { class: c } },
    }
}

/// "Nicht interessiert": aus der Liste, dem Server melden, läuft er gerade → weiter.
fn dislike(track: PlayableTrack) {
    let key = track.key();
    ITEMS.write().retain(|i| i.track.key() != key);
    if player::current_key().as_deref() == Some(key.as_str()) {
        player::next();
    }
    spawn(async move {
        match api::post::<serde_json::Value, _>("/feed/dislike", &DislikeRequest { track: track.to_ref() }).await {
            Ok(_) => toast(trf("„{0}“ kommt nicht mehr – der Feed merkt sich das.", &[&track.title()])),
            Err(e) => toast_error(e),
        }
    });
}

#[component]
pub fn Feed() -> Element {
    // Erste Seite und unendliches Nachladen beim Scrollen
    use_future(move || async move {
        if ITEMS.peek().is_empty() {
            load_more();
        }
        loop {
            gloo_timers::future::TimeoutFuture::new(400).await;
            if !*LOADING.peek() && !*EXHAUSTED.peek() && ERROR.peek().is_none() && near_end() {
                load_more();
            }
        }
    });
    let items = ITEMS.read().clone();
    let current = player::current_key();
    let playing = player::PLAYING();
    let feed_playing = player::FEED_MODE() && current.is_some();
    rsx! {
        // Kopf
        div { class: "relative mb-6 overflow-hidden rounded-2xl border bg-gradient-to-br from-primary/15 via-muted/40 to-transparent p-5 md:p-7",
            div { class: "flex flex-wrap items-end justify-between gap-4",
                div { class: "min-w-0",
                    p { class: "flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground",
                        icons::Sparkles { class: "size-3.5" }
                        {tr("Feed")}
                    }
                    h1 { class: "mt-1 text-2xl font-semibold tracking-tight md:text-3xl", {tr("Für dich")} }
                    p { class: "mt-1 max-w-xl text-sm text-muted-foreground",
                        {tr("Lernt aus dem, was du hörst: wie lange, was du überspringst, was du likest – und probiert immer wieder Neues aus.")}
                    }
                }
                div { class: "flex flex-wrap items-center gap-2",
                    button { class: "{btn(BTN_PRIMARY)} rounded-full px-5", disabled: items.is_empty(),
                        onclick: move |_| {
                            if feed_playing { player::toggle() } else { player::play_feed(tracks(), 0) }
                        },
                        if feed_playing && playing {
                            icons::Pause { class: "size-4 fill-current" }
                            {tr("Pause")}
                        } else {
                            icons::Play { class: "size-4 fill-current" }
                            {tr("Feed abspielen")}
                        }
                    }
                    button { class: "{btn(BTN_OUTLINE)} rounded-full", title: tr("Neue Empfehlungen holen"),
                        onclick: move |_| reshuffle(),
                        icons::RefreshCw { class: "size-4" }
                        {tr("Neu mischen")}
                    }
                }
            }
        }
        if *COLD.read() {
            div { class: "mb-4",
                Callout { text: tr("Der Feed kennt dich noch kaum und rät darum mehr. Hör ein paar Titel, like, was dir gefällt, und überspring, was nicht passt – er wird schnell besser.") }
            }
        }
        if let Some(e) = ERROR.read().clone() {
            Callout { text: e, error: true,
                button { class: "{btn(BTN_OUTLINE)} {BTN_SM} mt-2", onclick: move |_| { *ERROR.write() = None; load_more(); }, {tr("Erneut versuchen")} }
            }
        }
        if items.is_empty() && *LOADING.read() {
            Loading { text: tr("Empfehlungen werden zusammengestellt …") }
        } else if items.is_empty() && ERROR.read().is_none() {
            EmptyState { title: tr("Noch keine Empfehlungen"), description: tr("Hör ein paar Titel oder like etwas – dann füllt sich der Feed."), icon: rsx! { icons::Sparkles {} } }
        }
        div { class: "grid gap-1 lg:grid-cols-2 lg:gap-x-4", role: "list",
            for (i , item) in items.iter().cloned().enumerate() {
                FeedRow { key: "{item.track.key()}", index: i, item: item.clone(),
                    current: current.as_deref() == Some(item.track.key().as_str()), playing }
            }
        }
        div { id: "feed-end", class: "flex justify-center py-8",
            if *LOADING.read() && !items.is_empty() {
                Loading { text: tr("Mehr wird geladen …") }
            } else if *EXHAUSTED.read() && !items.is_empty() {
                button { class: "{btn(BTN_OUTLINE)} rounded-full", onclick: move |_| { *EXHAUSTED.write() = false; load_more(); },
                    icons::RefreshCw { class: "size-4" }
                    {tr("Mehr laden")}
                }
            }
        }
    }
}

#[component]
fn FeedRow(index: usize, item: FeedItem, current: bool, playing: bool) -> Element {
    let t = item.track.clone();
    let t_dis = t.clone();
    let play = move || {
        if current {
            player::toggle();
        } else {
            player::play_feed(tracks(), index);
        }
    };
    rsx! {
        div {
            role: "listitem",
            tabindex: "0",
            class: if current { "group grid grid-cols-[3.5rem_minmax(0,1fr)_auto] items-center gap-3 rounded-xl bg-accent/50 p-2 outline-none" } else { "group grid grid-cols-[3.5rem_minmax(0,1fr)_auto] items-center gap-3 rounded-xl p-2 outline-none transition-colors hover:bg-accent/50 focus-visible:bg-accent/60" },
            onclick: move |_| if coarse_pointer() { play() },
            ondoubleclick: move |_| play(),
            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { play() },
            div { class: "relative size-14",
                Cover { src: t.artwork_url(160), class: "size-14 rounded-lg" }
                button {
                    class: if current { "absolute inset-0 grid place-items-center rounded-lg bg-black/50 text-white" } else { "absolute inset-0 grid place-items-center rounded-lg bg-black/50 text-white opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100" },
                    title: tr("Abspielen"),
                    onclick: move |e: MouseEvent| { e.stop_propagation(); play(); },
                    if current && playing {
                        EqBars { playing: true }
                    } else {
                        icons::Play { class: "size-5 fill-current pointer-events-none" }
                    }
                }
            }
            div { class: "flex min-w-0 flex-col gap-0.5",
                div { class: "flex min-w-0 items-center gap-1.5",
                    span { class: if current { "truncate text-sm font-semibold" } else { "truncate text-sm font-medium" }, "{t.title()}" }
                    if let PlayableTrack::Youtube(y) = &t {
                        if y.explicit { Tag { text: "E" } }
                    }
                }
                div { class: "flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground",
                    if matches!(t, PlayableTrack::Youtube(_)) { icons::Youtube { class: "size-3.5 shrink-0" } }
                    ArtistLinks { track: t.clone() }
                }
                span { class: "flex min-w-0 items-center gap-1 text-[11px] text-muted-foreground/80",
                    ReasonIcon { reason: item.reason.clone() }
                    span { class: "truncate", {reason_text(&item.reason)} }
                }
            }
            div { class: "flex items-center gap-0.5",
                Heart { track: t.clone() }
                button { class: "{ICON_BTN} md:opacity-0 md:group-hover:opacity-100 md:focus-visible:opacity-100", title: tr("Nicht interessiert"),
                    onclick: move |e: MouseEvent| { e.stop_propagation(); dislike(t_dis.clone()); },
                    icons::ThumbsDown { class: "size-4" }
                }
                span { class: "hidden w-11 text-right text-xs tabular-nums text-muted-foreground sm:block",
                    {t.duration_ms().map(util::clock_ms).unwrap_or_default()}
                }
                MenuButton { track: t.clone() }
            }
        }
    }
}
