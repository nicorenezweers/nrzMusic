//! "Ordner & Upload": überwachter Musikordner, Scan-Status, neu einlesen, Hochladen von
//! Dateien und Ordnern mit Fortschritt pro Datei.

use crate::components::common::*;
use crate::i18n::{tr, trf};
use crate::state::{self, SCAN, STATUS};
use crate::upload::{self, UPLOADS, UpState};
use crate::util;
use dioxus::prelude::*;

/// Knöpfe "Dateien wählen" und "Ordner wählen".
#[component]
pub fn UploadButtons() -> Element {
    rsx! {
        label { class: "{btn(BTN_PRIMARY)} cursor-pointer",
            icons::Upload { class: "size-4" }
            {tr("Dateien wählen")}
            input { r#type: "file", multiple: true, class: "hidden", accept: "audio/*,.flac,.m4a,.opus,.ogg,.aiff,.ape,.wv",
                onchange: move |e| upload::enqueue(util::input_files(&e)),
            }
        }
        label { class: "{btn(BTN_OUTLINE)} cursor-pointer",
            icons::FolderOpen { class: "size-4" }
            {tr("Ordner wählen")}
            input { r#type: "file", multiple: true, class: "hidden", "webkitdirectory": "true", "directory": "true",
                onchange: move |e| upload::enqueue(util::input_files(&e)),
            }
        }
    }
}

#[component]
pub fn Library() -> Element {
    let status = STATUS.read().clone();
    let scan = *SCAN.read();
    let ups = UPLOADS.read().clone();
    let finished = ups.iter().filter(|u| u.finished()).count();
    // Status beim Öffnen frisch holen
    use_future(state::load_status);
    if crate::account::enabled() && !crate::account::is_admin() {
        return rsx! {
            div { class: "mx-auto flex max-w-4xl flex-col gap-6",
                PageHeader { title: tr("Ordner & Upload") }
                Callout { text: tr("Hochladen und neu einlesen darf nur ein Admin. Titel von YouTube Music kannst du unter „Downloads“ in den gemeinsamen Musikordner laden.") }
            }
        };
    }
    rsx! {
        div { class: "mx-auto flex max-w-4xl flex-col gap-6",
            PageHeader { title: tr("Ordner & Upload"), description: tr("Alles im Musikordner erscheint von selbst in der Bibliothek – ablegen genügt.") }

            Section { title: tr("Musikordner"),
                description: tr("Der Ordner, den nrzMusic einliest und überwacht."),
                actions: rsx! {
                    button { class: btn(BTN_OUTLINE), disabled: scan.is_some(), onclick: move |_| { spawn(state::rescan()); },
                        icons::RefreshCw { class: if scan.is_some() { "size-4 animate-spin" } else { "size-4" } }
                        {tr("Jetzt neu einlesen")}
                    }
                },
                div { class: "grid gap-4",
                    if let Some(s) = status.as_ref() {
                        div { class: "flex flex-wrap items-center gap-3",
                            code { class: "rounded-md bg-muted px-2.5 py-1.5 font-mono text-sm break-all", "{s.music_dir}" }
                            span { class: "inline-flex items-center gap-1.5 text-xs text-muted-foreground",
                                span { class: if s.watching { "size-2 rounded-full bg-success" } else { "size-2 rounded-full bg-warning" } }
                                if s.watching { {tr("wird live überwacht")} } else { {tr("nur periodischer Scan")} }
                            }
                        }
                        div { class: "grid grid-cols-2 gap-3 sm:grid-cols-4",
                            Stat { label: tr("Titel"), value: util::number(s.tracks) }
                            Stat { label: tr("Alben"), value: util::number(s.albums) }
                            Stat { label: tr("Künstler"), value: util::number(s.artists) }
                            Stat { label: tr("Grösse"), value: util::size(s.total_size) }
                        }
                        p { class: "text-xs text-muted-foreground",
                            match s.last_scan {
                                Some(ts) => trf("Zuletzt eingelesen: {0}", &[&util::datetime(ts)]),
                                None => tr("Noch nie vollständig eingelesen."),
                            }
                        }
                    } else {
                        Loading {}
                    }
                    if let Some(p) = scan {
                        div { class: "rounded-lg border bg-muted/30 p-3",
                            div { class: "flex items-center gap-2 text-sm",
                                icons::LoaderCircle { class: "size-4 animate-spin" }
                                span { class: "flex-1", {tr("Wird eingelesen …")} }
                                if p.total > 0 { span { class: "tabular-nums text-muted-foreground", {format!("{} / {}", util::number(p.done), util::number(p.total))} } }
                            }
                            div { class: "mt-2 h-1.5 overflow-hidden rounded-full bg-muted",
                                div { class: "h-full rounded-full bg-primary transition-[width]", style: format!("width: {}%", if p.total > 0 { p.done as f64 / p.total as f64 * 100.0 } else { 3.0 }) }
                            }
                        }
                    }
                }
            }

            Section { title: tr("Hochladen"), description: tr("Dateien oder ganze Ordner – die Ordnerstruktur bleibt erhalten. Ziehen geht überall in der App."),
                div {
                    class: "flex flex-col items-center gap-4 rounded-2xl border-2 border-dashed px-6 py-10 text-center",
                    div { class: "grid size-12 place-items-center rounded-xl bg-muted", icons::Upload { class: "size-6" } }
                    p { class: "text-sm text-muted-foreground", {tr("Musik hierher ziehen oder auswählen")} }
                    div { class: "flex flex-wrap justify-center gap-2", UploadButtons {} }
                }
                if !ups.is_empty() {
                    div { class: "mt-5",
                        div { class: "mb-2 flex items-center justify-between gap-2",
                            p { class: "text-sm font-medium", {trf("{0} von {1} erledigt", &[&finished, &ups.len()])} }
                            if finished > 0 {
                                button { class: btn(&format!("{BTN_GHOST} {BTN_SM}")), onclick: move |_| upload::clear_finished(), {tr("Erledigte entfernen")} }
                            }
                        }
                        div { class: "flex max-h-[28rem] flex-col divide-y overflow-y-auto rounded-lg border",
                            for u in ups.iter().cloned() {
                                div { key: "{u.id}", class: "flex items-center gap-3 px-3 py-2",
                                    match &u.state {
                                        UpState::Waiting => rsx! { icons::Clock { class: "size-4 shrink-0 text-muted-foreground" } },
                                        UpState::Sending => rsx! { icons::LoaderCircle { class: "size-4 shrink-0 animate-spin" } },
                                        UpState::Imported(_) => rsx! { icons::Check { class: "size-4 shrink-0 text-success" } },
                                        UpState::Skipped(_) => rsx! { icons::Minus { class: "size-4 shrink-0 text-muted-foreground" } },
                                        UpState::Failed(_) => rsx! { icons::CircleAlert { class: "size-4 shrink-0 text-destructive" } },
                                    }
                                    div { class: "min-w-0 flex-1",
                                        p { class: "truncate text-sm", title: "{u.name}", "{u.name}" }
                                        match &u.state {
                                            UpState::Sending => rsx! {
                                                div { class: "mt-1 h-1 overflow-hidden rounded-full bg-muted",
                                                    div { class: "h-full bg-primary transition-[width]", style: format!("width: {}%", if u.size > 0.0 { u.loaded / u.size * 100.0 } else { 0.0 }) }
                                                }
                                            },
                                            UpState::Imported(t) => rsx! { p { class: "truncate text-xs text-muted-foreground", {trf("importiert als „{0}“", &[t])} } },
                                            UpState::Skipped(r) => rsx! { p { class: "truncate text-xs text-muted-foreground", {trf("übersprungen: {0}", &[r])} } },
                                            UpState::Failed(r) => rsx! { p { class: "truncate text-xs text-destructive", "{r}" } },
                                            UpState::Waiting => rsx! { p { class: "text-xs text-muted-foreground", {tr("wartet")} } },
                                        }
                                    }
                                    span { class: "shrink-0 text-xs tabular-nums text-muted-foreground", {util::size(u.size as u64)} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Stat(#[props(into)] label: String, #[props(into)] value: String) -> Element {
    rsx! {
        div { class: "rounded-lg border bg-muted/20 px-3 py-2.5",
            p { class: "text-xs text-muted-foreground", "{label}" }
            p { class: "mt-0.5 text-lg font-semibold tabular-nums", "{value}" }
        }
    }
}
