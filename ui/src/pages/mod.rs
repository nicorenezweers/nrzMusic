pub mod albums;
pub mod artists;
pub mod downloads;
pub mod favorites;
pub mod feed;
pub mod home;
pub mod layout;
pub mod library;
pub mod playlists;
pub mod search;
pub mod settings;
pub mod shared;
pub mod songs;
pub mod spotify;
pub mod stats;
pub mod youtube;

use crate::Route;
use crate::components::common::*;
use crate::i18n::tr;
use dioxus::prelude::*;

/// Reiter der Bibliothek (vor allem für das Handy, wo die Seitenleiste eingeklappt ist).
#[component]
pub fn LibraryTabs() -> Element {
    let route = use_route::<Route>();
    let tabs = [
        (Route::Songs {}, tr("Songs")),
        (Route::Albums {}, tr("Alben")),
        (Route::Artists {}, tr("Künstler")),
        (Route::Playlists {}, tr("Playlists")),
        (Route::Favorites {}, tr("Favoriten")),
        (Route::History {}, tr("Verlauf")),
        (Route::StatsPage {}, tr("Statistiken")),
        (Route::Downloads {}, tr("Downloads")),
    ];
    rsx! {
        div { class: "shelf -mx-4 mb-5 flex gap-2 overflow-x-auto px-4 md:hidden",
            for (r , label) in tabs {
                Link {
                    class: if std::mem::discriminant(&r) == std::mem::discriminant(&route) { "shrink-0 rounded-full bg-primary px-3.5 py-1.5 text-sm font-medium text-primary-foreground" } else { "shrink-0 rounded-full bg-muted px-3.5 py-1.5 text-sm text-muted-foreground" },
                    to: r.clone(),
                    "{label}"
                }
            }
        }
    }
}

/// Hinweis, wenn YouTube Music auf dem Server ausgeschaltet ist.
#[component]
pub fn YtOff() -> Element {
    rsx! {
        EmptyState {
            title: tr("YouTube Music ist ausgeschaltet"),
            description: tr("In den Einstellungen lässt es sich einschalten – oder über NRZMUSIC_YOUTUBE=true beim Start."),
            icon: rsx! { icons::Youtube {} },
            Link { class: btn(BTN_OUTLINE), to: Route::SettingsPage {}, icons::Settings {} {tr("Zu den Einstellungen")} }
        }
    }
}

/// Fehler beim Laden einer Seite.
#[component]
pub fn LoadError(#[props(into)] message: String) -> Element {
    rsx! { Callout { text: message, error: true } }
}
