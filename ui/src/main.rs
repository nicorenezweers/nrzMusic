//! nrzMusic – Oberfläche (Dioxus 0.7, WebAssembly) mit Komponenten von rust-ui.
//! Spricht nur mit der JSON-API unter `/api/v1`; alles spielt über ein `<audio>`-Element:
//! lokale Titel als Datei, YouTube-Titel als Tonspur, die der Server mit yt-dlp holt.
//! Mit Konten zeigt die App ohne Anmeldung nur die Anmeldeseite ([`account`]).

mod account;
mod android;
mod api;
mod components;
mod downloads;
mod events;
mod i18n;
mod pages;
mod player;
mod share;
mod spotify;
mod state;
mod ui;
mod upload;
mod util;

use components::overlay::{ConfirmHost, Toaster};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use pages::albums::{AlbumPage, Albums};
use pages::artists::{ArtistPage, Artists};
use pages::downloads::Downloads;
use pages::favorites::{Favorites, History};
use pages::feed::Feed;
use pages::stats::StatsPage;
use pages::home::{Home, NotFound};
use pages::layout::Layout;
use pages::library::Library;
use pages::playlists::{PlaylistPage, Playlists};
use pages::search::Search;
use pages::settings::SettingsPage;
use pages::shared::{PublicShare, SharedPage};
use pages::songs::Songs;
use pages::spotify::{Spotify, SpotifyLiked, SpotifyPlaylistPage, SpotifyPlaylists};
use pages::youtube::{YouTube, YtAlbumPage, YtArtistPage, YtPlaylistPage};
use account::AccountPage;

static CSS: Asset = asset!("/assets/tailwind.css");
pub static ICON: Asset = asset!("/assets/icon.png");
static FAVICON: Asset = asset!("/assets/favicon-32.png");

#[derive(Routable, Clone, PartialEq, Debug)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/feed")]
        Feed {},
        #[route("/search?:q")]
        Search { q: String },
        #[route("/songs")]
        Songs {},
        #[route("/albums")]
        Albums {},
        #[route("/albums/:id")]
        AlbumPage { id: String },
        #[route("/artists")]
        Artists {},
        #[route("/artists/:id")]
        ArtistPage { id: String },
        #[route("/playlists")]
        Playlists {},
        #[route("/playlists/:id")]
        PlaylistPage { id: i64 },
        #[route("/favorites")]
        Favorites {},
        #[route("/history")]
        History {},
        #[route("/stats")]
        StatsPage {},
        #[route("/youtube?:q&:tab")]
        YouTube { q: String, tab: String },
        #[route("/youtube/album/:id")]
        YtAlbumPage { id: String },
        #[route("/youtube/artist/:id")]
        YtArtistPage { id: String },
        #[route("/youtube/playlist/:id")]
        YtPlaylistPage { id: String },
        #[route("/library")]
        Library {},
        #[route("/downloads")]
        Downloads {},
        #[route("/spotify?:connected&:error")]
        Spotify { connected: String, error: String },
        #[route("/spotify/liked")]
        SpotifyLiked {},
        #[route("/spotify/playlists")]
        SpotifyPlaylists {},
        #[route("/spotify/playlists/:id")]
        SpotifyPlaylistPage { id: String },
        #[route("/s/:token")]
        SharedPage { token: String },
        #[route("/settings")]
        SettingsPage {},
        #[route("/account")]
        AccountPage {},
        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // Sprache vor dem ersten Zeichnen
    use_hook(i18n::init);
    // Erst klären, wer angemeldet ist
    use_hook(|| spawn_forever(account::load()));
    let head = rsx! {
        document::Stylesheet { href: CSS }
        document::Link { rel: "icon", r#type: "image/png", href: FAVICON }
    };
    if account::AUTH.read().is_none() {
        return rsx! { {head} };
    }
    if account::needs_login() {
        // Geteilte Links gehen auch ohne Konto
        if let Some(token) = public_share_token() {
            return rsx! { {head} PublicShare { token } Toaster {} };
        }
        return rsx! { {head} account::LoginScreen {} Toaster {} };
    }
    rsx! { {head} Main {} }
}

/// `/s/<token>` in der Adresse?
fn public_share_token() -> Option<String> {
    let path = web_sys::window()?.location().pathname().ok()?;
    let token = path.strip_prefix("/s/")?.trim_end_matches('/');
    (!token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric())).then(|| token.to_string())
}

/// Die App nach der Anmeldung.
#[component]
fn Main() -> Element {
    // Einmal: Player, Live-Ereignisse, Grunddaten, Android-Brücke
    use_hook(|| {
        android::init();
        spawn_forever(player::run());
        spawn_forever(events::run());
        spawn_forever(async {
            futures_util::future::join4(state::load_status(), state::load_playlists(), state::load_favorites(), state::load_prefs()).await;
        });
        spawn_forever(async {
            futures_util::future::join(downloads::load(), spotify::load_status()).await;
        });
    });
    rsx! {
        Router::<Route> {}
        ConfirmHost {}
        Toaster {}
    }
}
