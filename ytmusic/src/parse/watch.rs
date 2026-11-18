//! Warteschlange "Als Nächstes" (`next`) – für das Radio zu einem Titel und für Mixe
//! ("RD…"), die sich nicht als Playlist-Seite öffnen lassen.

use std::collections::HashSet;

use nrzmusic_types::YtTrack;
use serde_json::Value;

use super::common::*;
use crate::json::{runs_at, JsonExt};
use crate::{Error, Result};

const PANEL: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer";

/// Titel der Warteschlange ohne `skip_video_id` (den Ausgangstitel) und ohne Doppelte.
/// Zusätzlich der Titel der Liste, falls YouTube einen liefert.
pub(crate) fn parse_watch(
    resp: &Value,
    skip_video_id: Option<&str>,
) -> Result<(Vec<YtTrack>, Option<String>)> {
    let Some(panel) = resp.at(PANEL) else {
        return Err(if resp.get("contents").is_none() || resp.at(
            "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer",
        ).is_some()
        {
            Error::NotFound
        } else {
            Error::Parse("Warteschlange ohne playlistPanelRenderer".into())
        });
    };
    let mut seen = HashSet::new();
    let tracks = panel
        .arr_at("/contents")
        .iter()
        .filter_map(|entry| {
            entry
                .at("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
                .or_else(|| entry.get("playlistPanelVideoRenderer"))
        })
        .filter_map(track_from_panel)
        .filter(|t| Some(t.video_id.as_str()) != skip_video_id)
        .filter(|t| seen.insert(t.video_id.clone()))
        .collect();
    Ok((tracks, panel.text_at("/title")))
}

fn track_from_panel(v: &Value) -> Option<YtTrack> {
    if v.get("unplayableText").is_some() {
        return None;
    }
    let video_id = v
        .str_at("/videoId")
        .or_else(|| v.str_at("/navigationEndpoint/watchEndpoint/videoId"))?;
    if video_id.is_empty() {
        return None;
    }
    let title = v.text_at("/title")?;
    let video_type =
        v.str_at("/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType");
    if video_type == Some(VIDEO_TYPE_EPISODE) {
        return None;
    }
    let info = parse_song_runs(&runs_at(v, "/longBylineText/runs"), false);
    let duration_ms = v
        .text_at("/lengthText")
        .and_then(|t| parse_duration_ms(&t))
        .or(info.duration_ms);
    let thumbnail = thumbnail_of(v);
    let is_video =
        is_video_type(video_type).unwrap_or_else(|| looks_like_video(thumbnail.as_deref()));
    Some(YtTrack {
        video_id: video_id.to_string(),
        title,
        artists: info.artists,
        album: info.album,
        duration_ms,
        thumbnail,
        explicit: is_explicit(v),
        is_video,
    })
}
