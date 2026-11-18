//! Geteilte Links (`/s/<token>`): angemeldet als normale Seite mit Abspielen und "Als Playlist
//! speichern"; ohne Anmeldung eine eigene schlanke Seite mit kleinem Player, der die Titel
//! über den Link abspielt (`/api/v1/public/share/<token>/stream/<key>`).

use crate::api;
use crate::components::common::*;
use crate::components::tracks::TrackList;
use crate::i18n::{tr, trf};
use crate::pages::LoadError;
use crate::{ICON, state, util};
use dioxus::prelude::*;
use nrzmusic_types::{PlayableTrack, ShareKind, SharedView};

fn heading(v: &SharedView) -> String {
    match v.kind {
        ShareKind::Favorites => trf("Likes von {0}", &[&v.owner]),
        _ => v.title.clone(),
    }
}

fn kind_label(v: &SharedView) -> String {
    let what = match v.kind {
        ShareKind::Track => tr("Titel"),
        ShareKind::Favorites => tr("Likes"),
        ShareKind::Playlist => tr("Playlist"),
    };
    trf("{0} · geteilt von {1}", &[&what, &v.owner])
}

async fn load(token: &str) -> api::ApiResult<SharedView> {
    api::get::<SharedView>(&format!("/public/share/{}", api::enc(token))).await
}

/// Angemeldet: im Rahmen der App.
#[component]
pub fn SharedPage(token: String) -> Element {
    let t = token.clone();
    let view = use_resource(move || {
        let t = t.clone();
        async move { load(&t).await }
    });
    match &*view.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) if e.status == 404 => rsx! {
            EmptyState { title: tr("Link nicht gefunden"), description: tr("Der Link wurde gelöscht oder ist falsch."), icon: rsx! { icons::Link2 {} } }
        },
        Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
        Some(Ok(v)) => {
            let tracks = v.tracks.clone();
            let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
            let name = heading(v);
            let desc = trf("Geteilt von {0}", &[&v.owner]);
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: v.artwork.clone(), class: "aspect-square w-full rounded-xl", eager: true } },
                    backdrop: v.artwork.clone(),
                    kind: kind_label(v),
                    title: heading(v),
                    subtitle: rsx! {
                        if !v.description.is_empty() {
                            p { class: "max-w-2xl whitespace-pre-line text-muted-foreground", "{v.description}" }
                        }
                    },
                    meta: util::tracks_count(tracks.len() as u64),
                    PlayButtons { tracks: tracks.clone() }
                    if tracks.len() > 1 {
                        button { class: "{btn(BTN_OUTLINE)} rounded-full", title: tr("Als eigene Playlist speichern"),
                            onclick: move |_| {
                                let (name, desc, refs) = (name.clone(), desc.clone(), refs.clone());
                                spawn(async move { let _ = state::create_playlist(name, desc, refs).await; });
                            },
                            icons::ListPlus { class: "size-4" }
                            {tr("Speichern")}
                        }
                    }
                }
                TrackList { tracks: tracks.clone(), lead: crate::components::tracks::Lead::Cover }
            }
        }
    }
}

/// Ohne Anmeldung: eigene Seite mit kleinem Player.
#[component]
pub fn PublicShare(token: String) -> Element {
    let t = token.clone();
    let view = use_resource(move || {
        let t = t.clone();
        async move { load(&t).await }
    });
    let mut current = use_signal(|| None::<usize>);
    rsx! {
        div { class: "min-h-dvh bg-background pb-40",
            header { class: "pt-safe px-safe sticky top-0 z-10 border-b bg-background/80 backdrop-blur",
                div { class: "mx-auto flex h-14 max-w-3xl items-center gap-2.5 px-4",
                    img { src: ICON, alt: "", class: "size-7 rounded-lg" }
                    span { class: "text-[15px] font-semibold tracking-tight", "nrzMusic" }
                    a { class: "{btn(BTN_OUTLINE)} {BTN_SM} ml-auto rounded-full", href: "/", {tr("Anmelden")} }
                }
            }
            div { class: "mx-auto max-w-3xl px-4 pt-6",
                match &*view.read() {
                    None => rsx! { Loading {} },
                    Some(Err(e)) if e.status == 404 => rsx! {
                        EmptyState { title: tr("Link nicht gefunden"), description: tr("Der Link wurde gelöscht oder ist falsch."), icon: rsx! { icons::Link2 {} } }
                    },
                    Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
                    Some(Ok(v)) => {
                        let tracks = v.tracks.clone();
                        rsx! {
                            div { class: "mb-6 flex flex-col items-center gap-5 text-center sm:flex-row sm:items-end sm:text-left",
                                Cover { src: v.artwork.clone(), class: "size-48 rounded-2xl shadow-xl sm:size-40", eager: true }
                                div { class: "min-w-0",
                                    p { class: "text-xs font-medium uppercase tracking-wider text-muted-foreground", {kind_label(v)} }
                                    h1 { class: "mt-1 text-2xl font-semibold tracking-tight md:text-3xl", {heading(v)} }
                                    if !v.description.is_empty() {
                                        p { class: "mt-1 whitespace-pre-line text-sm text-muted-foreground", "{v.description}" }
                                    }
                                    p { class: "mt-1 text-sm text-muted-foreground", {util::tracks_count(tracks.len() as u64)} }
                                    button { class: "{btn(BTN_PRIMARY)} mt-4 rounded-full px-6", disabled: tracks.is_empty(),
                                        onclick: move |_| current.set(Some(0)),
                                        icons::Play { class: "size-4 fill-current" }
                                        {tr("Abspielen")}
                                    }
                                }
                            }
                            if tracks.is_empty() {
                                EmptyState { title: tr("Noch leer"), description: tr("Hier ist gerade nichts drin."), icon: rsx! { icons::Music {} } }
                            }
                            div { class: "flex flex-col", role: "list",
                                for (i , t) in tracks.iter().cloned().enumerate() {
                                    button {
                                        key: "{t.key()}",
                                        role: "listitem",
                                        class: if current() == Some(i) { "flex items-center gap-3 rounded-lg bg-accent/50 p-2 text-left" } else { "flex items-center gap-3 rounded-lg p-2 text-left hover:bg-accent/50" },
                                        onclick: move |_| current.set(Some(i)),
                                        Cover { src: t.artwork_url(96), class: "size-11" }
                                        div { class: "min-w-0 flex-1",
                                            p { class: "truncate text-sm font-medium", "{t.title()}" }
                                            p { class: "truncate text-xs text-muted-foreground", "{t.artist()}" }
                                        }
                                        span { class: "text-xs tabular-nums text-muted-foreground", {t.duration_ms().map(util::clock_ms).unwrap_or_default()} }
                                    }
                                }
                            }
                            if let Some(i) = current() {
                                if let Some(t) = tracks.get(i).cloned() {
                                    MiniAudio { token: v.token.clone(), track: t, index: i, count: tracks.len(), current }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Unten fest: aktueller Titel und ein schlichter Player; am Ende der nächste Titel.
#[component]
fn MiniAudio(token: String, track: PlayableTrack, index: usize, count: usize, current: Signal<Option<usize>>) -> Element {
    let src = format!("/api/v1/public/share/{}/stream/{}", api::enc(&token), api::enc(&track.key()));
    rsx! {
        div { class: "pb-safe px-safe fixed inset-x-0 bottom-0 z-20 border-t bg-background/95 backdrop-blur",
            div { class: "mx-auto flex max-w-3xl items-center gap-3 px-4 py-3",
                Cover { src: track.artwork_url(96), class: "size-12" }
                div { class: "min-w-0 flex-1",
                    p { class: "truncate text-sm font-medium", "{track.title()}" }
                    p { class: "truncate text-xs text-muted-foreground", "{track.artist()}" }
                    audio {
                        key: "{src}",
                        class: "mt-1.5 h-9 w-full",
                        src: "{src}",
                        controls: true,
                        autoplay: true,
                        preload: "auto",
                        onended: move |_| {
                            if index + 1 < count { current.set(Some(index + 1)) }
                        },
                    }
                }
                div { class: "flex flex-col gap-1",
                    button { class: "{ICON_BTN}", title: tr("Zurück"), disabled: index == 0, onclick: move |_| current.set(Some(index.saturating_sub(1))),
                        icons::SkipBack { class: "size-4" }
                    }
                    button { class: "{ICON_BTN}", title: tr("Weiter"), disabled: index + 1 >= count, onclick: move |_| current.set(Some(index + 1)),
                        icons::SkipForward { class: "size-4" }
                    }
                }
            }
        }
    }
}
