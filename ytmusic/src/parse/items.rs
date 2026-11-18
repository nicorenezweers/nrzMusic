//! Einzelne Einträge: Listenzeilen (`musicResponsiveListItemRenderer`, in Suche, Alben,
//! Playlists, Top-Songs) und Kacheln (`musicTwoRowItemRenderer`, in Karussells).

use nrzmusic_types::{
    YtAlbumSummary, YtArtistRef, YtArtistSummary, YtItem, YtPlaylistSummary, YtTrack,
};
use serde_json::Value;

use super::common::*;
use crate::json::{runs_at, JsonExt, Run};

const PLAY_VIDEO_ID: &str =
    "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId";
const PLAY_VIDEO_TYPE: &str = "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
const WATCH_VIDEO_TYPE: &str =
    "/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
const NAV_BROWSE_ID: &str = "/navigationEndpoint/browseEndpoint/browseId";
const NAV_PAGE_TYPE: &str =
    "/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType";
const GREY_OUT: &str = "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT";

/// Art eines Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemKind {
    Track,
    Album,
    Artist,
    Playlist,
    /// Podcasts, Folgen, Profile … – wird übersprungen
    Other,
}

/// Art eines Browse-Ziels nach Seitentyp, sonst nach Präfix der ID.
pub(crate) fn kind_from_browse(browse_id: &str, page_type: Option<&str>) -> ItemKind {
    match page_type {
        Some(PAGE_ALBUM | PAGE_AUDIOBOOK) => ItemKind::Album,
        Some(PAGE_ARTIST) => ItemKind::Artist,
        Some(PAGE_PLAYLIST) => ItemKind::Playlist,
        Some(_) => ItemKind::Other,
        None if browse_id.starts_with("MPRE") => ItemKind::Album,
        None if browse_id.starts_with("UC") || browse_id.starts_with("MPLA") => ItemKind::Artist,
        None if ["VL", "PL", "RDCLAK", "OLAK"]
            .iter()
            .any(|p| browse_id.starts_with(p)) =>
        {
            ItemKind::Playlist
        }
        None => ItemKind::Other,
    }
}

/// Text aller Runs, getrimmt; leer → `None`.
pub(crate) fn join_text(runs: &[Run<'_>]) -> Option<String> {
    let s: String = runs.iter().map(|r| r.text()).collect();
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Spalten einer Listenzeile als Runs (fehlende Spalten → leere Liste).
pub(crate) fn flex_columns(item: &Value) -> Vec<Vec<Run<'_>>> {
    item.arr_at("/flexColumns")
        .iter()
        .map(|c| runs_at(c, "/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .collect()
}

fn list_item_video_id<'a>(item: &'a Value, cols: &[Vec<Run<'a>>]) -> Option<&'a str> {
    item.str_at("/playlistItemData/videoId")
        .or_else(|| item.str_at(PLAY_VIDEO_ID))
        .or_else(|| {
            cols.first()
                .and_then(|c| c.first())
                .and_then(|r| r.video_id())
        })
        .filter(|id| !id.is_empty())
}

fn list_item_video_type<'a>(item: &'a Value, cols: &[Vec<Run<'a>>]) -> Option<&'a str> {
    item.str_at(PLAY_VIDEO_TYPE)
        .or_else(|| {
            cols.first()
                .and_then(|c| c.first())
                .and_then(|r| r.video_type())
        })
        .or_else(|| {
            item.arr_at("/menu/menuRenderer/items")
                .iter()
                .find_map(|m| {
                    m.at("/menuNavigationItemRenderer/navigationEndpoint")
                        .and_then(|n| n.str_at(WATCH_VIDEO_TYPE))
                })
        })
}

/// Art einer Listenzeile (nach Link-Ziel bzw. Videotyp).
pub(crate) fn list_item_kind(item: &Value) -> ItemKind {
    if let Some(browse_id) = item.str_at(NAV_BROWSE_ID) {
        return kind_from_browse(browse_id, item.str_at(NAV_PAGE_TYPE));
    }
    let cols = flex_columns(item);
    if list_item_video_type(item, &cols) == Some(VIDEO_TYPE_EPISODE)
        || cols
            .first()
            .and_then(|c| c.first())
            .and_then(|r| r.page_type())
            == Some(PAGE_EPISODE)
    {
        return ItemKind::Other;
    }
    if list_item_video_id(item, &cols).is_some() {
        ItemKind::Track
    } else {
        ItemKind::Other
    }
}

/// Titel aus einer Listenzeile. `mixed`: die zweite Spalte beginnt mit einer Typangabe
/// ("Titel • …", gemischte Suchergebnisse). Ausgegraute (nicht abspielbare) Titel → `None`.
pub(crate) fn track_from_list_item(item: &Value, mixed: bool) -> Option<YtTrack> {
    if item.str_at("/musicItemRendererDisplayPolicy") == Some(GREY_OUT) {
        return None;
    }
    let cols = flex_columns(item);
    let title = cols.first().and_then(|c| join_text(c))?;
    let video_id = list_item_video_id(item, &cols)?.to_string();
    let video_type = list_item_video_type(item, &cols);
    if video_type == Some(VIDEO_TYPE_EPISODE) {
        return None;
    }

    let mut info = cols
        .get(1)
        .map(|c| parse_song_runs(c, mixed))
        .unwrap_or_default();
    // Weitere Spalten: Album (Playlists, Top-Songs), Künstler, Dauer (gemeinsame Playlists).
    // Unverlinkter Text dort sind meist Aufrufe – ignorieren.
    let artists_from_second = !info.artists.is_empty();
    for col in cols.iter().skip(2) {
        for run in col {
            match classify_run(*run) {
                RunKind::Album(a) if info.album.is_none() => info.album = Some(a),
                RunKind::Artist(a) if run.has_endpoint() && !artists_from_second => {
                    if !info.artists.contains(&a) {
                        info.artists.push(a);
                    }
                }
                RunKind::Duration(ms) if info.duration_ms.is_none() => info.duration_ms = Some(ms),
                _ => {}
            }
        }
    }
    if info.duration_ms.is_none() {
        info.duration_ms = item.arr_at("/fixedColumns").iter().find_map(|c| {
            c.text_at("/musicResponsiveListItemFixedColumnRenderer/text")
                .and_then(|t| parse_duration_ms(&t))
        });
    }

    let thumbnail = thumbnail_of(item);
    let is_video =
        is_video_type(video_type).unwrap_or_else(|| looks_like_video(thumbnail.as_deref()));
    Some(YtTrack {
        video_id,
        title,
        artists: info.artists,
        album: info.album,
        duration_ms: info.duration_ms,
        thumbnail,
        explicit: is_explicit(item),
        is_video,
    })
}

/// Album aus einer Listenzeile ("Album • Daft Punk • 2013").
pub(crate) fn album_from_list_item(item: &Value) -> Option<YtAlbumSummary> {
    let browse_id = item.str_at(NAV_BROWSE_ID)?.to_string();
    let cols = flex_columns(item);
    let title = cols.first().and_then(|c| join_text(c))?;
    let second = cols.get(1).map(Vec::as_slice).unwrap_or(&[]);
    let (rest, label) = strip_type_label(second);
    let info = parse_song_runs(rest, false);
    Some(YtAlbumSummary {
        browse_id,
        title,
        artists: info.artists,
        year: info.year,
        kind: label.unwrap_or("Album").to_string(),
        thumbnail: thumbnail_of(item),
    })
}

/// Künstler aus einer Listenzeile ("Künstler • 68,3 Mio. Zuschauer/Monat").
pub(crate) fn artist_from_list_item(item: &Value) -> Option<YtArtistSummary> {
    let browse_id = item.str_at(NAV_BROWSE_ID)?.to_string();
    let cols = flex_columns(item);
    let name = cols.first().and_then(|c| join_text(c))?;
    let subscribers = cols.get(1).and_then(|c| count_text(c));
    Some(YtArtistSummary {
        browse_id,
        name,
        subscribers,
        thumbnail: thumbnail_of(item),
    })
}

/// Letzter Run, der mit einer Ziffer beginnt (Abonnenten, Zuschauer, Aufrufe).
fn count_text(runs: &[Run<'_>]) -> Option<String> {
    runs.iter()
        .rev()
        .map(|r| r.text().trim())
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

/// Autor und Titelanzahl aus einer Zeile wie "YouTube Music • 35 Songs".
pub(crate) fn playlist_meta(runs: &[Run<'_>]) -> (Option<String>, Option<String>) {
    let mut author = None;
    let mut count = None;
    for (i, run) in runs.iter().enumerate() {
        if (i % 2 == 1 && !run.has_endpoint()) || run.is_separator() {
            continue;
        }
        let text = run.text().trim();
        if text.is_empty() {
            continue;
        }
        if is_track_count(text) {
            count.get_or_insert_with(|| text.to_string());
        } else if !text.starts_with(|c: char| c.is_ascii_digit()) && author.is_none() {
            author = Some(text.to_string());
        }
    }
    (author, count)
}

/// Playlist aus einer Listenzeile. `mixed`: mit Typangabe ("Playlist • …").
pub(crate) fn playlist_from_list_item(item: &Value, mixed: bool) -> Option<YtPlaylistSummary> {
    let browse_id = item.str_at(NAV_BROWSE_ID)?;
    let cols = flex_columns(item);
    let title = cols.first().and_then(|c| join_text(c))?;
    let second = cols.get(1).map(Vec::as_slice).unwrap_or(&[]);
    let rest = if mixed {
        strip_type_label(second).0
    } else {
        second
    };
    let (author, mut track_count) = playlist_meta(rest);
    if track_count.is_none() {
        track_count = cols.iter().skip(2).find_map(|c| playlist_meta(c).1);
    }
    Some(YtPlaylistSummary {
        playlist_id: playlist_id_from_browse(browse_id),
        title,
        author,
        track_count,
        thumbnail: thumbnail_of(item),
    })
}

/// Beliebige Listenzeile → Eintrag; Podcasts, Folgen und Profile → `None`.
pub(crate) fn item_from_list_item(item: &Value, mixed: bool) -> Option<YtItem> {
    match list_item_kind(item) {
        ItemKind::Track => track_from_list_item(item, mixed).map(YtItem::Track),
        ItemKind::Album => album_from_list_item(item).map(YtItem::Album),
        ItemKind::Artist => artist_from_list_item(item).map(YtItem::Artist),
        ItemKind::Playlist => playlist_from_list_item(item, mixed).map(YtItem::Playlist),
        ItemKind::Other => None,
    }
}

// ---------------------------------------------------------------- Kacheln

/// Album aus einer Kachel ("Single • 2023", "Album • Daft Punk" oder nur "2013").
pub(crate) fn album_from_two_row(
    item: &Value,
    browse_id: &str,
    title: String,
    default_kind: &str,
) -> YtAlbumSummary {
    let sub = runs_at(item, "/subtitle/runs");
    let (rest, label) = strip_type_label(&sub);
    let info = parse_song_runs(rest, false);
    YtAlbumSummary {
        browse_id: browse_id.to_string(),
        title,
        artists: info.artists,
        year: info.year,
        kind: label.unwrap_or(default_kind).to_string(),
        thumbnail: thumbnail_of(item),
    }
}

/// Eine Kachel → Eintrag. `default_album_kind` gilt für Alben ohne Typangabe.
pub(crate) fn item_from_two_row(item: &Value, default_album_kind: &str) -> Option<YtItem> {
    let title_runs = runs_at(item, "/title/runs");
    let title = join_text(&title_runs)?;
    let nav = item.get("navigationEndpoint").or_else(|| {
        title_runs
            .first()
            .and_then(|r| r.0.get("navigationEndpoint"))
    })?;

    if let Some(browse_id) = nav.str_at("/browseEndpoint/browseId") {
        let page = nav.str_at("/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType");
        let sub = runs_at(item, "/subtitle/runs");
        return match kind_from_browse(browse_id, page) {
            ItemKind::Album => Some(YtItem::Album(album_from_two_row(
                item,
                browse_id,
                title,
                default_album_kind,
            ))),
            ItemKind::Artist => Some(YtItem::Artist(YtArtistSummary {
                browse_id: browse_id.to_string(),
                name: title,
                subscribers: join_text(&sub),
                thumbnail: thumbnail_of(item),
            })),
            ItemKind::Playlist => {
                let (rest, label) = strip_type_label(&sub);
                // Ohne Typangabe ist die Zeile eine Künstlerliste, kein Autor
                let (author, count) = playlist_meta(rest);
                Some(YtItem::Playlist(YtPlaylistSummary {
                    playlist_id: playlist_id_from_browse(browse_id),
                    title,
                    author: if label.is_some() { author } else { None },
                    track_count: count,
                    thumbnail: thumbnail_of(item),
                }))
            }
            ItemKind::Track | ItemKind::Other => None,
        };
    }

    if let Some(video_id) = nav
        .str_at("/watchEndpoint/videoId")
        .filter(|v| !v.is_empty())
    {
        let video_type = nav.str_at(WATCH_VIDEO_TYPE);
        if video_type == Some(VIDEO_TYPE_EPISODE) {
            return None;
        }
        let info = parse_song_runs(&runs_at(item, "/subtitle/runs"), true);
        let thumbnail = thumbnail_of(item);
        let is_video =
            is_video_type(video_type).unwrap_or_else(|| looks_like_video(thumbnail.as_deref()));
        return Some(YtItem::Track(YtTrack {
            video_id: video_id.to_string(),
            title,
            artists: info.artists,
            album: info.album,
            duration_ms: info.duration_ms,
            thumbnail,
            explicit: is_explicit(item),
            is_video,
        }));
    }

    if let Some(playlist_id) = nav
        .str_at("/watchPlaylistEndpoint/playlistId")
        .filter(|p| !p.is_empty())
    {
        return Some(YtItem::Playlist(YtPlaylistSummary {
            playlist_id: playlist_id_from_browse(playlist_id),
            title,
            author: None,
            track_count: None,
            thumbnail: thumbnail_of(item),
        }));
    }
    None
}

/// Fehlende Künstler eines Titels ergänzen (z. B. aus dem Album oder der Top-Karte).
pub(crate) fn fill_artists(track: &mut YtTrack, artists: &[YtArtistRef]) {
    if track.artists.is_empty() {
        track.artists = artists.to_vec();
    }
}
