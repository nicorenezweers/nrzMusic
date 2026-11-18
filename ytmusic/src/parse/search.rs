//! Suche (`search`) und Suchvorschläge (`music/get_search_suggestions`).
//!
//! YouTube Music liefert die gemischte Suche inzwischen als eine Liste einzelner Zeilen
//! (`itemSectionRenderer`, je ein Treffer, ohne Kategorie-Überschrift), früher als
//! Regale pro Kategorie (`musicShelfRenderer`). Beides wird unterstützt: jede Zeile wird
//! nach ihrem Link-Ziel bzw. Videotyp einsortiert, nicht nach der (übersetzten) Überschrift.

use std::collections::HashSet;

use nrzmusic_types::{YtArtistRef, YtItem, YtSearchResults, YtTrack};
use serde_json::Value;

use super::common::*;
use super::items::*;
use crate::json::{runs_at, JsonExt};
use crate::{Error, Result, SearchFilter};

/// `params` der InnerTube-Suche für einen Filter (wie ytmusicapi `get_search_params`).
pub(crate) fn filter_params(filter: SearchFilter) -> &'static str {
    match filter {
        SearchFilter::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
        SearchFilter::Videos => "EgWKAQIQAWoMEA4QChADEAQQCRAF",
        SearchFilter::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
        SearchFilter::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
        SearchFilter::Playlists => "Eg-KAQwIABAAGAAgACgBMABqChAEEAMQCRAFEAo%3D",
    }
}

/// Sammelt Treffer über mehrere Seiten, ohne Doppelte, nur die gefilterte Kategorie.
#[derive(Default)]
pub(crate) struct SearchCollector {
    pub results: YtSearchResults,
    filter: Option<SearchFilter>,
    seen: HashSet<String>,
}

impl SearchCollector {
    pub fn new(filter: Option<SearchFilter>) -> Self {
        SearchCollector {
            filter,
            ..Default::default()
        }
    }

    /// Anzahl Treffer in der gefilterten Kategorie (bzw. insgesamt).
    pub fn len(&self) -> usize {
        let r = &self.results;
        match self.filter {
            Some(SearchFilter::Songs) => r.songs.len(),
            Some(SearchFilter::Videos) => r.videos.len(),
            Some(SearchFilter::Albums) => r.albums.len(),
            Some(SearchFilter::Artists) => r.artists.len(),
            Some(SearchFilter::Playlists) => r.playlists.len(),
            None => {
                r.songs.len()
                    + r.videos.len()
                    + r.albums.len()
                    + r.artists.len()
                    + r.playlists.len()
            }
        }
    }

    pub fn push(&mut self, item: YtItem) {
        use SearchFilter as F;
        let filter = self.filter;
        let allowed = |f: SearchFilter| filter.is_none() || filter == Some(f);
        match item {
            YtItem::Track(t) => {
                let category = if t.is_video { F::Videos } else { F::Songs };
                if allowed(category) && self.seen.insert(format!("v:{}", t.video_id)) {
                    if t.is_video {
                        self.results.videos.push(t);
                    } else {
                        self.results.songs.push(t);
                    }
                }
            }
            YtItem::Album(a) => {
                if allowed(F::Albums) && self.seen.insert(format!("b:{}", a.browse_id)) {
                    self.results.albums.push(a);
                }
            }
            YtItem::Artist(a) => {
                if allowed(F::Artists) && self.seen.insert(format!("b:{}", a.browse_id)) {
                    self.results.artists.push(a);
                }
            }
            YtItem::Playlist(p) => {
                if allowed(F::Playlists) && self.seen.insert(format!("p:{}", p.playlist_id)) {
                    self.results.playlists.push(p);
                }
            }
        }
    }
}

/// Fortsetzungs-Token eines Regals: alte Form (`continuations`) oder neue
/// (`continuationItemRenderer` als letzter Eintrag).
pub(crate) fn continuation_token(container: &Value) -> Option<String> {
    container
        .str_at("/continuations/0/nextContinuationData/continuation")
        .map(str::to_string)
        .or_else(|| token_from_items(container.arr_at("/contents")))
}

/// Token aus dem letzten Eintrag einer Liste (`continuationItemRenderer`).
pub(crate) fn token_from_items(items: &[Value]) -> Option<String> {
    let last = items.last()?;
    if let Some(token) =
        last.str_at("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
    {
        return Some(token.to_string());
    }
    // Manchmal in einem commandExecutorCommand verschachtelt
    last.arr_at("/continuationItemRenderer/continuationEndpoint/commandExecutorCommand/commands")
        .iter()
        .find_map(|c| c.str_at("/continuationCommand/token"))
        .map(str::to_string)
}

/// Neue Fortsetzungsantworten: `onResponseReceived…/appendContinuationItemsAction`.
pub(crate) fn appended_items(resp: &Value) -> Option<&[Value]> {
    [
        "/onResponseReceivedActions",
        "/onResponseReceivedCommands",
        "/onResponseReceivedEndpoints",
    ]
    .iter()
    .flat_map(|p| resp.arr_at(p))
    .find_map(|a| a.at("/appendContinuationItemsAction/continuationItems"))
    .and_then(Value::as_array)
    .map(Vec::as_slice)
}

/// Erste Suchseite auswerten. Liefert das Fortsetzungs-Token des (gefilterten) Regals.
pub(crate) fn parse_search(
    resp: &Value,
    collector: &mut SearchCollector,
) -> Result<Option<String>> {
    // Ohne `contents` hat YouTube schlicht nichts gefunden
    let Some(contents) = resp.get("contents") else {
        return Ok(None);
    };
    let sections = contents
        .at("/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .or_else(|| contents.at("/sectionListRenderer/contents"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Parse("Suchergebnisse ohne sectionListRenderer".into()))?;

    let mut token = None;
    for section in sections {
        if let Some(card) = section.get("musicCardShelfRenderer") {
            parse_top_card(card, collector);
            continue;
        }
        let (rows, shelf) = if let Some(shelf) = section.get("musicShelfRenderer") {
            (shelf.arr_at("/contents"), Some(shelf))
        } else if let Some(sec) = section.get("itemSectionRenderer") {
            (sec.arr_at("/contents"), None)
        } else {
            continue;
        };
        // Gefilterte Regale haben keine Typangabe; gemischte Zeilen schon
        let mixed = collector.filter.is_none() || shelf.is_none();
        for row in rows {
            if let Some(item) = row
                .get("musicResponsiveListItemRenderer")
                .and_then(|r| item_from_list_item(r, mixed))
            {
                collector.push(item);
            }
        }
        if token.is_none() {
            token = shelf.and_then(continuation_token);
        }
    }
    Ok(token)
}

/// Weitere Seite einer gefilterten Suche (`{"continuation": token}`).
pub(crate) fn parse_search_continuation(
    resp: &Value,
    collector: &mut SearchCollector,
) -> Option<String> {
    if let Some(shelf) = resp.at("/continuationContents/musicShelfContinuation") {
        for row in shelf.arr_at("/contents") {
            if let Some(item) = row
                .get("musicResponsiveListItemRenderer")
                .and_then(|r| item_from_list_item(r, false))
            {
                collector.push(item);
            }
        }
        return continuation_token(shelf);
    }
    let items = appended_items(resp)?;
    for row in items {
        if let Some(item) = row
            .get("musicResponsiveListItemRenderer")
            .and_then(|r| item_from_list_item(r, false))
        {
            collector.push(item);
        }
    }
    token_from_items(items)
}

/// Die Top-Karte oben in der gemischten Suche (Künstler, Album, Playlist oder Titel)
/// samt der Titel darunter.
fn parse_top_card(card: &Value, collector: &mut SearchCollector) {
    let title_runs = runs_at(card, "/title/runs");
    let sub = runs_at(card, "/subtitle/runs");
    let thumbnail = thumbnail_of(card);
    let mut card_artists: Vec<YtArtistRef> = Vec::new();

    let title = join_text(&title_runs);
    let nav = title_runs
        .first()
        .and_then(|r| r.0.get("navigationEndpoint"))
        .or_else(|| card.get("onTap"));
    if let (Some(title), Some(nav)) = (title, nav) {
        if let Some(browse_id) = nav.str_at("/browseEndpoint/browseId") {
            let page = nav.str_at("/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType");
            let (rest, label) = strip_type_label(&sub);
            match kind_from_browse(browse_id, page) {
                ItemKind::Artist => {
                    card_artists.push(YtArtistRef {
                        name: title.clone(),
                        id: Some(browse_id.to_string()),
                    });
                    let subscribers = rest
                        .iter()
                        .rev()
                        .map(|r| r.text().trim())
                        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
                        .map(str::to_string);
                    collector.push(YtItem::Artist(nrzmusic_types::YtArtistSummary {
                        browse_id: browse_id.to_string(),
                        name: title,
                        subscribers,
                        thumbnail,
                    }));
                }
                ItemKind::Album => {
                    let info = parse_song_runs(rest, false);
                    card_artists = info.artists.clone();
                    collector.push(YtItem::Album(nrzmusic_types::YtAlbumSummary {
                        browse_id: browse_id.to_string(),
                        title,
                        artists: info.artists,
                        year: info.year,
                        kind: label.unwrap_or("Album").to_string(),
                        thumbnail,
                    }));
                }
                ItemKind::Playlist => {
                    let (author, track_count) = playlist_meta(rest);
                    collector.push(YtItem::Playlist(nrzmusic_types::YtPlaylistSummary {
                        playlist_id: playlist_id_from_browse(browse_id),
                        title,
                        author,
                        track_count,
                        thumbnail,
                    }));
                }
                ItemKind::Track | ItemKind::Other => {}
            }
        } else if let Some(video_id) = nav
            .str_at("/watchEndpoint/videoId")
            .filter(|v| !v.is_empty())
        {
            let video_type = nav
                .str_at("/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
                .or_else(|| {
                    card.str_at("/onTap/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
                });
            if video_type != Some(VIDEO_TYPE_EPISODE) {
                let info = parse_song_runs(&sub, true);
                let is_video = is_video_type(video_type)
                    .unwrap_or_else(|| looks_like_video(thumbnail.as_deref()));
                collector.push(YtItem::Track(YtTrack {
                    video_id: video_id.to_string(),
                    title,
                    artists: info.artists,
                    album: info.album,
                    duration_ms: info.duration_ms,
                    thumbnail,
                    explicit: is_explicit(card),
                    is_video,
                }));
            }
        }
    }

    // Titel unter der Karte; bei einer Künstler-Karte fehlt dort der Künstler
    for row in card.arr_at("/contents") {
        let Some(item) = row
            .get("musicResponsiveListItemRenderer")
            .and_then(|r| item_from_list_item(r, true))
        else {
            continue;
        };
        let item = match item {
            YtItem::Track(mut t) => {
                fill_artists(&mut t, &card_artists);
                YtItem::Track(t)
            }
            other => other,
        };
        collector.push(item);
    }
}

/// Suchvorschläge als einfache Texte (ohne Doppelte).
pub(crate) fn parse_suggestions(resp: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for section in resp.arr_at("/contents") {
        for entry in section.arr_at("/searchSuggestionsSectionRenderer/contents") {
            let Some(r) = entry
                .get("searchSuggestionRenderer")
                .or_else(|| entry.get("historySuggestionRenderer"))
            else {
                continue;
            };
            let text = r
                .str_at("/navigationEndpoint/searchEndpoint/query")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| r.text_at("/suggestion"));
            if let Some(text) = text {
                if !out.contains(&text) {
                    out.push(text);
                }
            }
        }
    }
    out
}
