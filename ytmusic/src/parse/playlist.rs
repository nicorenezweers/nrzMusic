//! Playlist-Seite (`browse` mit `VL<id>`) und ihre Fortsetzungen.

use nrzmusic_types::{YtPlaylist, YtPlaylistSummary, YtTrack};
use serde_json::Value;

use super::common::{is_track_count, thumbnail_of};
use super::items::*;
use super::search::{appended_items, token_from_items};
use crate::json::{runs_at, JsonExt};
use crate::{Error, Result};

const TWO_COLUMN: &str = "/contents/twoColumnBrowseResultsRenderer";
const HEADER_IN_TAB: &str = "/tabs/0/tabRenderer/content/sectionListRenderer/contents/0";

/// Erste Seite: Playlist mit den ersten (bis zu 100) Titeln und dem Fortsetzungs-Token.
pub(crate) fn parse_playlist(
    resp: &Value,
    playlist_id: &str,
) -> Result<(YtPlaylist, Option<String>)> {
    let Some(two) = resp.at(TWO_COLUMN) else {
        return Err(if resp.get("contents").is_none() {
            Error::NotFound
        } else {
            Error::Parse("Playlist-Seite ohne twoColumnBrowseResultsRenderer".into())
        });
    };

    let header_item = two.at(HEADER_IN_TAB);
    let header = header_item
        .and_then(|h| {
            h.get("musicResponsiveHeaderRenderer")
                .or_else(|| h.at("/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer"))
                .or_else(|| h.at("/musicEditablePlaylistDetailHeaderRenderer/header/musicDetailHeaderRenderer"))
        })
        .or_else(|| resp.at("/header/musicDetailHeaderRenderer"));

    let shelf = two
        .arr_at("/secondaryContents/sectionListRenderer/contents")
        .iter()
        .find_map(|s| s.get("musicPlaylistShelfRenderer"));

    if header.is_none() && shelf.is_none() {
        return Err(Error::Parse(
            "Playlist-Seite ohne Kopf und ohne Titelliste".into(),
        ));
    }

    let rows = shelf.map(|s| s.arr_at("/contents")).unwrap_or(&[]);
    let tracks = tracks_from_rows(rows);
    let token = shelf.and_then(|s| {
        token_from_items(rows).or_else(|| {
            s.str_at("/continuations/0/nextContinuationData/continuation")
                .map(str::to_string)
        })
    });

    let mut summary = YtPlaylistSummary {
        playlist_id: playlist_id.to_string(),
        title: String::new(),
        author: None,
        track_count: None,
        thumbnail: None,
    };
    let mut description = None;
    if let Some(h) = header {
        summary.title = h.text_at("/title").unwrap_or_default();
        summary.author = h
            .str_at("/facepile/avatarStackViewModel/text/content")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| h.text_at("/straplineTextOne"))
            .or_else(|| {
                // Alter Kopf: "Playlist • Autor • 2024"
                let sub = runs_at(h, "/subtitle/runs");
                sub.get(2)
                    .map(|r| r.text().trim().to_string())
                    .filter(|t| !t.is_empty())
            });
        summary.track_count = track_count(&runs_at(h, "/secondSubtitle/runs"));
        summary.thumbnail = thumbnail_of(h);
        description = h
            .text_at("/description/musicDescriptionShelfRenderer/description")
            .or_else(|| h.text_at("/description"));
    }
    if summary.title.is_empty() {
        // Album-Playlists ("OLAK5uy_…") kommen manchmal ohne Kopf: Albumtitel nehmen
        summary.title = tracks
            .iter()
            .find_map(|t| t.album.as_ref().map(|a| a.title.clone()))
            .unwrap_or_default();
    }
    if summary.thumbnail.is_none() {
        summary.thumbnail = tracks.first().and_then(|t| t.thumbnail.clone());
    }

    Ok((
        YtPlaylist {
            summary,
            description,
            tracks,
        },
        token,
    ))
}

/// "63 Songs • 4 Stunden" bzw. "12.399 Aufrufe • 19 Titel • 1 Stunde" → Titelanzahl.
fn track_count(runs: &[crate::json::Run<'_>]) -> Option<String> {
    if let Some(t) = runs
        .iter()
        .step_by(2)
        .map(|r| r.text().trim())
        .find(|t| is_track_count(t))
    {
        return Some(t.to_string());
    }
    // Unbekannte Sprache: Aufbau wie bei ytmusicapi ([Aufrufe •] Anzahl • Dauer)
    let index = if runs.len() > 3 { 2 } else { 0 };
    runs.get(index)
        .map(|r| r.text().trim().to_string())
        .filter(|t| t.starts_with(|c: char| c.is_ascii_digit()))
}

/// Weitere Titel aus einer Fortsetzungsantwort und das nächste Token.
pub(crate) fn parse_playlist_continuation(resp: &Value) -> (Vec<YtTrack>, Option<String>) {
    if let Some(items) = appended_items(resp) {
        return (tracks_from_rows(items), token_from_items(items));
    }
    if let Some(shelf) = resp.at("/continuationContents/musicPlaylistShelfContinuation") {
        let rows = shelf.arr_at("/contents");
        let token = token_from_items(rows).or_else(|| {
            shelf
                .str_at("/continuations/0/nextContinuationData/continuation")
                .map(str::to_string)
        });
        return (tracks_from_rows(rows), token);
    }
    (Vec::new(), None)
}

fn tracks_from_rows(rows: &[Value]) -> Vec<YtTrack> {
    rows.iter()
        .filter_map(|row| row.get("musicResponsiveListItemRenderer"))
        .filter_map(|row| track_from_list_item(row, false))
        .collect()
}
