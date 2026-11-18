//! Albumseite (`browse` mit `MPREb_…`).

use nrzmusic_types::{YtAlbum, YtAlbumRef, YtAlbumSummary};
use serde_json::Value;

use super::common::*;
use super::items::*;
use crate::json::{runs_at, JsonExt};
use crate::{Error, Result};

const HEADER_2024: &str =
    "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";
const HEADER_OLD: &str = "/header/musicDetailHeaderRenderer";
const SECONDARY: &str =
    "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents";
const SINGLE_COLUMN: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";

pub(crate) fn parse_album(resp: &Value, browse_id: &str) -> Result<YtAlbum> {
    let Some(header) = resp.at(HEADER_2024).or_else(|| resp.at(HEADER_OLD)) else {
        return Err(if resp.get("contents").is_none() {
            Error::NotFound
        } else {
            Error::Parse("Albumseite ohne Kopfbereich".into())
        });
    };

    let title = header.text_at("/title").unwrap_or_default();
    let sub = runs_at(header, "/subtitle/runs");
    let (rest, label) = strip_type_label(&sub);
    let sub_info = parse_song_runs(rest, false);
    let kind = label
        .map(str::to_string)
        .or_else(|| {
            sub.first()
                .map(|r| r.text().trim().to_string())
                .filter(|t| !t.is_empty() && sub.len() > 1)
        })
        .unwrap_or_else(|| "Album".into());

    // Künstler stehen (seit 2024) in der Strapline, früher im Untertitel
    let strapline = runs_at(header, "/straplineTextOne/runs");
    let mut artists = parse_song_runs(&strapline, false).artists;
    if artists.is_empty() {
        artists = sub_info.artists.clone();
    }

    let description = header
        .text_at("/description/musicDescriptionShelfRenderer/description")
        .or_else(|| header.text_at("/description"));
    let thumbnail = thumbnail_of(header);
    let playlist_id = audio_playlist_id(header);

    // Titelliste: erstes musicShelfRenderer (zweispaltig, sonst einspaltig)
    let sections = if resp.at(SECONDARY).is_some() {
        resp.arr_at(SECONDARY)
    } else {
        resp.arr_at(SINGLE_COLUMN)
    };
    let shelf = sections.iter().find_map(|s| s.get("musicShelfRenderer"));
    let Some(shelf) = shelf else {
        return Err(Error::Parse("Albumseite ohne Titelliste".into()));
    };

    let album_ref = YtAlbumRef {
        title: title.clone(),
        id: Some(browse_id.to_string()),
    };
    let tracks = shelf
        .arr_at("/contents")
        .iter()
        .filter_map(|row| row.get("musicResponsiveListItemRenderer"))
        .filter_map(|row| track_from_list_item(row, false))
        .map(|mut t| {
            fill_artists(&mut t, &artists);
            if t.album.is_none() {
                t.album = Some(album_ref.clone());
            }
            if t.thumbnail.is_none() {
                t.thumbnail = thumbnail.clone();
            }
            t
        })
        .collect();

    Ok(YtAlbum {
        summary: YtAlbumSummary {
            browse_id: browse_id.to_string(),
            title,
            artists,
            year: sub_info.year,
            kind,
            thumbnail,
        },
        description,
        tracks,
        playlist_id,
    })
}

/// Audio-Playlist des Albums ("OLAK5uy_…") aus dem Abspielknopf bzw. dem Menü.
fn audio_playlist_id(header: &Value) -> Option<String> {
    let from_button = header.arr_at("/buttons").iter().find_map(|b| {
        let nav = b.at("/musicPlayButtonRenderer/playNavigationEndpoint")?;
        nav.str_at("/watchPlaylistEndpoint/playlistId")
            .or_else(|| nav.str_at("/watchEndpoint/playlistId"))
    });
    let from_menu = || {
        header
            .arr_at("/buttons")
            .iter()
            .flat_map(|b| b.arr_at("/menuRenderer/items"))
            .chain(header.arr_at("/menu/menuRenderer/topLevelButtons"))
            .chain(header.arr_at("/menu/menuRenderer/items"))
            .find_map(|m| {
                [
                    "/menuNavigationItemRenderer/navigationEndpoint",
                    "/buttonRenderer/navigationEndpoint",
                ]
                .iter()
                .filter_map(|p| m.at(p))
                .find_map(|n| {
                    n.str_at("/watchPlaylistEndpoint/playlistId")
                        .or_else(|| n.str_at("/watchEndpoint/playlistId"))
                })
                .filter(|id| id.starts_with("OLAK"))
            })
    };
    from_button.or_else(from_menu).map(playlist_id_from_browse)
}
