//! Gemeinsame Bausteine im Stil von rust-ui: Klassen für Knöpfe und Felder, Seitenkopf,
//! leerer Zustand, Ladeanzeige, Abschnitt, Schalter – und für Musik: Cover mit Platzhalter,
//! Collage, Herz, Equalizer-Balken.

use crate::i18n::tr;
use crate::state;
use dioxus::prelude::*;
use nrzmusic_types::PlayableTrack;
use tw_merge::tw_merge;

pub const INPUT: &str = "flex h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-base shadow-xs outline-none transition-[color,box-shadow] placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 md:text-sm dark:bg-input/30";
pub const TEXTAREA: &str = "flex min-h-16 w-full rounded-md border border-input bg-transparent px-3 py-2 text-base shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 md:text-sm dark:bg-input/30";
pub const SELECT: &str = "flex h-9 w-full items-center rounded-md border border-input bg-transparent px-3 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 dark:bg-input/30 [&>option]:bg-popover";
pub const BTN: &str = "inline-flex shrink-0 cursor-pointer items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-all outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 active:scale-[0.98] [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";
pub const BTN_PRIMARY: &str = "bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 h-9 px-4 py-2";
pub const BTN_OUTLINE: &str = "border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground dark:bg-input/30 dark:border-input dark:hover:bg-input/50 h-9 px-4 py-2";
pub const BTN_GHOST: &str = "hover:bg-accent hover:text-accent-foreground dark:hover:bg-accent/50 h-9 px-3";
pub const BTN_SM: &str = "h-8 px-3 text-[13px]";
pub const BTN_ICON: &str = "size-9 p-0";
/// Runder Symbolknopf (Spielerleiste, Zeilen)
pub const ICON_BTN: &str = "grid size-8 shrink-0 cursor-pointer place-items-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40 [&_svg]:pointer-events-none";

/// Klassen für einen Knopf, z. B. `btn(BTN_PRIMARY)`.
pub fn btn(variant: &str) -> String {
    format!("{BTN} {variant}")
}

#[component]
pub fn Field(#[props(into)] label: String, #[props(into, optional)] hint: Option<String>, #[props(into, optional)] class: Option<String>, children: Element) -> Element {
    let c = tw_merge!("grid gap-1.5", class.as_deref().unwrap_or(""));
    rsx! {
        label { class: "{c}",
            span { class: "text-sm font-medium leading-none", "{label}" }
            {children}
            if let Some(h) = hint {
                span { class: "text-xs text-muted-foreground", "{h}" }
            }
        }
    }
}

#[component]
pub fn PageHeader(
    #[props(into)] title: String,
    #[props(into, optional)] description: Option<String>,
    #[props(optional)] actions: Option<Element>,
    #[props(into, optional)] eyebrow: Option<String>,
) -> Element {
    rsx! {
        div { class: "mb-6 flex flex-wrap items-end justify-between gap-4",
            div { class: "min-w-0",
                if let Some(e) = eyebrow {
                    p { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", "{e}" }
                }
                h1 { class: "truncate text-2xl font-semibold tracking-tight md:text-3xl", "{title}" }
                if let Some(d) = description {
                    p { class: "mt-1 text-sm text-muted-foreground", "{d}" }
                }
            }
            if let Some(a) = actions {
                div { class: "flex flex-wrap items-center gap-2", {a} }
            }
        }
    }
}

#[component]
pub fn EmptyState(
    #[props(into)] title: String,
    #[props(into, optional)] description: Option<String>,
    #[props(optional)] icon: Option<Element>,
    #[props(optional)] children: Option<Element>,
) -> Element {
    rsx! {
        div { class: "flex min-h-[14rem] flex-col items-center justify-center gap-3 rounded-xl border border-dashed p-10 text-center",
            if let Some(i) = icon {
                div { class: "grid size-12 place-items-center rounded-xl bg-muted text-muted-foreground [&_svg]:size-6", {i} }
            }
            div {
                p { class: "font-medium", "{title}" }
                if let Some(d) = description {
                    p { class: "mt-1 max-w-md text-sm text-muted-foreground", "{d}" }
                }
            }
            if let Some(c) = children {
                {c}
            }
        }
    }
}

#[component]
pub fn Loading(#[props(into, optional)] text: Option<String>) -> Element {
    let text = text.unwrap_or_else(|| tr("Lade …"));
    rsx! {
        div { class: "flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground",
            icons::LoaderCircle { class: "size-4 animate-spin" }
            "{text}"
        }
    }
}

#[component]
pub fn Callout(#[props(into)] text: String, #[props(default)] error: bool, #[props(optional)] children: Option<Element>) -> Element {
    let c = if error { "border-destructive/40 bg-destructive/10" } else { "bg-muted/50" };
    rsx! {
        div { class: "flex items-start gap-2 rounded-lg border px-3 py-2.5 text-sm {c}",
            if error {
                icons::CircleAlert { class: "mt-0.5 size-4 shrink-0 text-destructive" }
            } else {
                icons::Info { class: "mt-0.5 size-4 shrink-0 text-muted-foreground" }
            }
            div { class: "flex flex-1 flex-wrap items-center justify-between gap-2",
                span { "{text}" }
                if let Some(c) = children {
                    {c}
                }
            }
        }
    }
}

/// Abschnitt einer Einstellungsseite (rust-ui Card).
#[component]
pub fn Section(#[props(into)] title: String, #[props(into, optional)] description: Option<String>, #[props(optional)] actions: Option<Element>, children: Element) -> Element {
    rsx! {
        section { class: "rounded-xl border bg-card text-card-foreground shadow-sm",
            div { class: "flex flex-wrap items-start justify-between gap-3 border-b px-5 py-4 md:px-6",
                div {
                    h2 { class: "font-semibold leading-none", "{title}" }
                    if let Some(d) = description {
                        p { class: "mt-1.5 text-sm text-muted-foreground", "{d}" }
                    }
                }
                if let Some(a) = actions {
                    div { class: "flex items-center gap-2", {a} }
                }
            }
            div { class: "px-5 py-5 md:px-6", {children} }
        }
    }
}

/// rust-ui Switch als kontrollierte Komponente.
#[component]
pub fn Toggle(checked: bool, onchange: EventHandler<bool>, #[props(into, optional)] label: Option<String>, #[props(default)] disabled: bool) -> Element {
    let state = if checked { "bg-primary" } else { "bg-input dark:bg-input/80" };
    let thumb = if checked { "translate-x-[calc(100%-2px)] dark:bg-primary-foreground" } else { "translate-x-0 dark:bg-foreground" };
    rsx! {
        label { class: "inline-flex cursor-pointer items-center gap-2.5 text-sm",
            button {
                r#type: "button",
                role: "switch",
                disabled,
                "aria-checked": "{checked}",
                class: "peer inline-flex h-[1.15rem] w-8 shrink-0 items-center rounded-full border border-transparent shadow-xs transition-all outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 {state}",
                onclick: move |_| onchange.call(!checked),
                span { class: "pointer-events-none block size-4 rounded-full bg-background ring-0 transition-transform {thumb}" }
            }
            if let Some(l) = label {
                span { "{l}" }
            }
        }
    }
}

// ------------------------------------------------------------------ Musik

/// Cover mit neutralem Platzhalter (Musiknote auf grauer Kachel), lädt erst bei Sichtbarkeit.
#[component]
pub fn Cover(
    src: Option<String>,
    #[props(into, default)] class: String,
    #[props(default)] round: bool,
    #[props(default)] eager: bool,
    #[props(default)] icon: Option<Element>,
) -> Element {
    let mut failed = use_signal(|| None::<String>);
    let shape = if round { "rounded-full" } else { "rounded-md" };
    let c = tw_merge!("relative shrink-0 overflow-hidden bg-muted", shape, class);
    let usable = src.filter(|s| failed.read().as_deref() != Some(s.as_str()));
    rsx! {
        div { class: "{c}",
            if let Some(u) = usable {
                img {
                    src: "{u}",
                    alt: "",
                    loading: if eager { "eager" } else { "lazy" },
                    decoding: "async",
                    draggable: "false",
                    referrerpolicy: "no-referrer",
                    class: "size-full object-cover",
                    onerror: move |_| failed.set(Some(u.clone())),
                }
            } else {
                div { class: "grid size-full place-items-center bg-gradient-to-br from-muted to-secondary text-muted-foreground/70 [&_svg]:size-[38%] [&_svg]:max-h-16 [&_svg]:max-w-16",
                    if let Some(i) = icon { {i} } else { icons::Music {} }
                }
            }
        }
    }
}

/// Collage aus bis zu vier Bildern (Playlists).
#[component]
pub fn Collage(artwork: Vec<String>, #[props(into, default)] class: String) -> Element {
    let c = tw_merge!("relative shrink-0 overflow-hidden rounded-md bg-muted", class.clone());
    if artwork.len() < 4 {
        return rsx! {
            Cover { src: artwork.first().cloned(), class, icon: rsx! { icons::ListMusic {} } }
        };
    }
    rsx! {
        div { class: "{c} grid grid-cols-2 grid-rows-2",
            for (i , a) in artwork.iter().take(4).enumerate() {
                img { key: "{i}", src: "{a}", alt: "", loading: "lazy", draggable: "false", referrerpolicy: "no-referrer", class: "size-full object-cover" }
            }
        }
    }
}

/// Herz zum Umschalten des Favoriten.
#[component]
pub fn Heart(track: PlayableTrack, #[props(into, default)] class: String) -> Element {
    let on = state::FAVORITES.read().contains(&track.key());
    let color = if on { "text-heart hover:text-heart" } else { "" };
    let c = tw_merge!(ICON_BTN, color, class);
    rsx! {
        button {
            class: "{c}",
            title: if on { tr("Aus den Favoriten entfernen") } else { tr("Zu den Favoriten") },
            "aria-pressed": "{on}",
            onclick: move |e| {
                e.stop_propagation();
                state::toggle_favorite(track.clone());
            },
            if on {
                icons::Heart { class: "size-4 fill-current" }
            } else {
                icons::Heart { class: "size-4" }
            }
        }
    }
}

/// Drei Balken beim laufenden Titel.
#[component]
pub fn EqBars(playing: bool) -> Element {
    let anim = if playing { "eq-bar" } else { "" };
    rsx! {
        span { class: "inline-flex h-3.5 items-end gap-[2px]", "aria-hidden": "true",
            span { class: "w-[3px] h-full rounded-sm bg-foreground {anim}" }
            span { class: "w-[3px] h-2/3 rounded-sm bg-foreground {anim}" }
            span { class: "w-[3px] h-1/2 rounded-sm bg-foreground {anim}" }
        }
    }
}

/// Kleine Plakette (z. B. "E" für explizit, "Video").
#[component]
pub fn Tag(#[props(into)] text: String) -> Element {
    rsx! {
        span { class: "inline-flex h-4 shrink-0 items-center rounded-[4px] bg-muted px-1 text-[10px] font-semibold uppercase leading-none text-muted-foreground", "{text}" }
    }
}

/// Knöpfe "Abspielen" und "Zufall" für Detailseiten.
#[component]
pub fn PlayButtons(tracks: Vec<PlayableTrack>) -> Element {
    let t2 = tracks.clone();
    let t3 = tracks.clone();
    let empty = tracks.is_empty();
    rsx! {
        button { class: "{btn(BTN_PRIMARY)} rounded-full px-5", disabled: empty,
            onclick: move |_| crate::player::play_now(tracks.clone(), 0),
            icons::Play { class: "size-4 fill-current" }
            {tr("Abspielen")}
        }
        button { class: "{btn(BTN_OUTLINE)} rounded-full", disabled: empty,
            onclick: move |_| crate::player::play_shuffled(t2.clone()),
            icons::Shuffle { class: "size-4" }
            {tr("Zufall")}
        }
        if crate::android::available() {
            button { class: "{btn(BTN_OUTLINE)} rounded-full", disabled: empty, title: tr("Alle Titel aufs Handy laden – sie spielen dann auch ohne Verbindung."),
                onclick: move |_| crate::android::save(t3.clone()),
                icons::Smartphone { class: "size-4" }
                {tr("Aufs Handy")}
            }
        }
    }
}

/// Waagrechte Reihe mit Überschrift (Startseite, YouTube-Regale).
#[component]
pub fn Shelf(#[props(into)] title: String, #[props(optional)] action: Option<Element>, children: Element) -> Element {
    rsx! {
        section { class: "min-w-0",
            div { class: "mb-3 flex items-end justify-between gap-3",
                h2 { class: "text-lg font-semibold tracking-tight", "{title}" }
                if let Some(a) = action {
                    {a}
                }
            }
            div { class: "shelf -mx-4 flex gap-4 overflow-x-auto px-4 pb-2 md:-mx-6 md:px-6", {children} }
        }
    }
}

/// Grosser Kopf für Album, Künstler, Playlist: Bild links, Text und Knöpfe rechts.
#[component]
pub fn DetailHeader(
    image: Element,
    #[props(into)] kind: String,
    #[props(into)] title: String,
    #[props(optional)] subtitle: Option<Element>,
    #[props(into, optional)] meta: Option<String>,
    #[props(optional)] backdrop: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        div { class: "relative isolate -mx-4 -mt-4 mb-6 overflow-hidden px-4 pt-6 pb-6 md:-mx-6 md:-mt-6 md:px-6 md:pt-10",
            // Die Farbe kommt vom Cover: stark verschwommen im Hintergrund
            if let Some(b) = backdrop {
                img { class: "pointer-events-none absolute inset-0 -z-10 size-full scale-125 object-cover opacity-30 blur-3xl", src: "{b}", alt: "", referrerpolicy: "no-referrer" }
                div { class: "pointer-events-none absolute inset-0 -z-10 bg-gradient-to-b from-transparent to-background" }
            }
            div { class: "flex flex-col items-center gap-5 text-center sm:flex-row sm:items-end sm:text-left md:gap-7",
                div { class: "w-48 shrink-0 shadow-2xl sm:w-52 md:w-60", {image} }
                div { class: "flex min-w-0 flex-col gap-2",
                    p { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", "{kind}" }
                    h1 { class: "text-2xl font-bold tracking-tight break-words sm:text-3xl md:text-5xl", "{title}" }
                    if let Some(s) = subtitle {
                        div { class: "text-sm", {s} }
                    }
                    if let Some(m) = meta {
                        p { class: "text-sm text-muted-foreground", "{m}" }
                    }
                    div { class: "mt-3 flex flex-wrap items-center justify-center gap-2 sm:justify-start", {children} }
                }
            }
        }
    }
}

/// Spotify-Zeichen im Strichstil der übrigen Symbole (lucide hat keines).
#[component]
pub fn SpotifyIcon(#[props(into, default)] class: String) -> Element {
    rsx! {
        svg {
            xmlns: "http://www.w3.org/2000/svg",
            width: "24",
            height: "24",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            "stroke-width": "2",
            "stroke-linecap": "round",
            "stroke-linejoin": "round",
            "aria-hidden": "true",
            class: "{class}",
            circle { cx: "12", cy: "12", r: "10" }
            path { d: "M7 9.6c3.3-1 6.9-.7 9.8.9" }
            path { d: "M7.7 12.8c2.6-.7 5.4-.5 7.6.7" }
            path { d: "M8.4 15.9c2-.5 4-.3 5.6.5" }
        }
    }
}
