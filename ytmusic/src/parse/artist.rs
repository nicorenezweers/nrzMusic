//! Künstlerseite (`browse` mit `UC…`).

use nrzmusic_types::{YtAlbumSummary, YtArtist, YtArtistRef, YtArtistSummary, YtItem};
use serde_json::Value;

use super::common::*;
use super::items::*;
use crate::json::JsonExt;
use crate::{Error, Result};

const SINGLE_COLUMN: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";
const TWO_COLUMN: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";
const PAGE_DISCOGRAPHY: &str = "MUSIC_PAGE_TYPE_ARTIST_DISCOGRAPHY";

/// Überschriften der Album- bzw. Single-Karussells, falls sie keinen "Mehr"-Link haben.
const ALBUM_TITLES: &[&str] = &["alben", "albums", "álbumes", "álbuns", "album"];
const SINGLE_TITLES: &[&str] = &["single", "sencillo", "singoli"];

pub(crate) fn parse_artist(resp: &Value, channel_id: &str) -> Result<YtArtist> {
    let header = [
        "/header/musicImmersiveHeaderRenderer",
        "/header/musicVisualHeaderRenderer",
        "/header/musicHeaderRenderer",
    ]
    .iter()
    .find_map(|p| resp.at(p));
    let sections = resp
        .at(SINGLE_COLUMN)
        .or_else(|| resp.at(TWO_COLUMN))
        .and_then(Value::as_array);
    let (Some(header), Some(sections)) = (header, sections) else {
        return Err(if resp.get("contents").is_none() {
            Error::NotFound
        } else {
            Error::Parse("Künstlerseite ohne Kopf oder Inhalt".into())
        });
    };

    let name = header.text_at("/title").unwrap_or_default();
    let subscribers = header
        .text_at("/subscriptionButton/subscribeButtonRenderer/longSubscriberCountText")
        .or_else(|| {
            header.text_at("/subscriptionButton/subscribeButtonRenderer/subscriberCountText")
        });
    let description = header.text_at("/description").or_else(|| {
        sections
            .iter()
            .find_map(|s| s.get("musicDescriptionShelfRenderer"))
            .and_then(|d| d.text_at("/description"))
    });
    let artist_ref = YtArtistRef {
        name: name.clone(),
        id: Some(channel_id.to_string()),
    };

    // Top-Songs: erstes Listen-Regal; sein Titel führt zur Playlist mit allen Songs
    let mut top_songs = Vec::new();
    let mut songs_playlist_id = None;
    if let Some(shelf) = sections.iter().find_map(|s| s.get("musicShelfRenderer")) {
        top_songs = shelf
            .arr_at("/contents")
            .iter()
            .filter_map(|row| row.get("musicResponsiveListItemRenderer"))
            .filter_map(|row| track_from_list_item(row, false))
            .map(|mut t| {
                fill_artists(&mut t, std::slice::from_ref(&artist_ref));
                t
            })
            .collect();
        songs_playlist_id = shelf
            .str_at("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
            .or_else(|| shelf.str_at("/bottomEndpoint/browseEndpoint/browseId"))
            .filter(|id| id.starts_with("VL") || id.starts_with("PL") || id.starts_with("OLAK"))
            .map(playlist_id_from_browse);
    }

    // Alben und Singles: Karussells mit Link zur Diskografie
    let mut albums: Vec<YtAlbumSummary> = Vec::new();
    let mut singles: Vec<YtAlbumSummary> = Vec::new();
    let mut discography_seen = 0;
    for carousel in sections
        .iter()
        .filter_map(|s| s.get("musicCarouselShelfRenderer"))
    {
        let title_run = carousel.at("/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0");
        let title = carousel
            .text_at("/header/musicCarouselShelfBasicHeaderRenderer/title")
            .unwrap_or_default()
            .to_lowercase();
        let browse = title_run.and_then(|r| r.at("/navigationEndpoint/browseEndpoint"));
        let browse_id = browse.and_then(|b| b.str_at("/browseId")).unwrap_or("");
        let page = browse.and_then(|b| {
            b.str_at(
                "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
            )
        });
        let params = browse.and_then(|b| b.str_at("/params")).unwrap_or("");

        let is_discography = browse_id.starts_with("MPAD") || page == Some(PAGE_DISCOGRAPHY);
        let singles_by_title = SINGLE_TITLES.iter().any(|w| title.contains(w));
        let albums_by_title = ALBUM_TITLES.iter().any(|w| title == *w);
        let into_singles = if is_discography {
            // Die params unterscheiden Alben (…egYIAR…) von Singles (…egYIAh…)
            if params.contains("egYIAh") {
                true
            } else if params.contains("egYIAR") {
                false
            } else {
                singles_by_title || discography_seen > 0
            }
        } else if singles_by_title {
            true
        } else if albums_by_title {
            false
        } else {
            continue;
        };
        if is_discography {
            discography_seen += 1;
        }

        let default_kind = if into_singles { "Single" } else { "Album" };
        let target = if into_singles {
            &mut singles
        } else {
            &mut albums
        };
        for entry in carousel
            .arr_at("/contents")
            .iter()
            .filter_map(|c| c.get("musicTwoRowItemRenderer"))
        {
            if let Some(YtItem::Album(mut album)) = item_from_two_row(entry, default_kind) {
                if album.artists.is_empty() {
                    album.artists.push(artist_ref.clone());
                }
                if !target.iter().any(|a| a.browse_id == album.browse_id) {
                    target.push(album);
                }
            }
        }
    }

    Ok(YtArtist {
        summary: YtArtistSummary {
            browse_id: channel_id.to_string(),
            name,
            subscribers,
            // Kanäle ohne Künstlerseite: Profilbild im Vordergrund statt Banner
            thumbnail: best_thumbnail(
                header.arr_at("/foregroundThumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
            )
            .or_else(|| thumbnail_of(header)),
        },
        description,
        top_songs,
        albums,
        singles,
        songs_playlist_id,
    })
}
