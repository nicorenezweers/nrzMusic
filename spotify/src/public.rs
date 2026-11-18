//! Öffentliche Playlists ohne Anmeldung: die Einbettungsseite
//! `https://open.spotify.com/embed/playlist/<id>` enthält Name, Bild und die Titelliste
//! (Titel, Künstler, Dauer) als JSON in `<script id="__NEXT_DATA__">`. So lassen sich auch
//! Playlists anderer übernehmen, deren Titel die Web-API im Entwicklungsmodus nicht
//! herausgibt. Spotify zeigt dort höchstens die ersten 100 Titel.

use crate::{Error, Result};
use nrzmusic_types::{SpPlaylist, SpPlaylistSummary, SpTrack};
use serde_json::Value;

/// `__NEXT_DATA__` aus der HTML-Seite.
fn next_data(html: &str) -> Option<Value> {
    let marker = "id=\"__NEXT_DATA__\"";
    let start = html.find(marker)?;
    let rest = &html[start..];
    let open = rest.find('>')? + 1;
    let close = rest[open..].find("</script>")?;
    serde_json::from_str(&rest[open..open + close]).ok()
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Künstler aus "A, B" (die Einbettung nennt sie als eine Zeile).
fn artists(line: &str) -> Vec<String> {
    line.split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect()
}

/// Einbettungsseite → Playlist (ohne Album und ISRC – die nennt Spotify dort nicht).
pub fn parse_embed(html: &str, id: &str) -> Result<SpPlaylist> {
    let data = next_data(html).ok_or_else(|| Error::Parse("Einbettung ohne Daten".into()))?;
    let entity = data
        .pointer("/props/pageProps/state/data/entity")
        .ok_or_else(|| Error::Api {
            status: 404,
            message: "Playlist nicht gefunden oder nicht öffentlich".into(),
        })?;
    let image = entity
        .pointer("/coverArt/sources")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .max_by_key(|x| x.get("width").and_then(Value::as_u64).unwrap_or(0))
        })
        .and_then(|x| s(x, "url"));
    let tracks: Vec<SpTrack> = entity
        .get("trackList")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|t| {
                    let uri = s(t, "uri")?;
                    let tid = uri.strip_prefix("spotify:track:")?.to_string();
                    Some(SpTrack {
                        id: tid,
                        name: s(t, "title")?,
                        artists: artists(&s(t, "subtitle").unwrap_or_default()),
                        album: String::new(),
                        duration_ms: t.get("duration").and_then(Value::as_u64).unwrap_or(0),
                        image: image.clone(),
                        isrc: None,
                        explicit: t.get("isExplicit").and_then(Value::as_bool).unwrap_or(false),
                        added_at: None,
                        youtube: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(SpPlaylist {
        summary: SpPlaylistSummary {
            id: id.to_string(),
            name: s(entity, "name")
                .or_else(|| s(entity, "title"))
                .unwrap_or_else(|| "Playlist".into()),
            owner: s(entity, "subtitle"),
            track_count: tracks.len() as u32,
            image,
            readable: true,
        },
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_page() {
        let html = r#"<html><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{"name":"Top","subtitle":"Spotify","coverArt":{"sources":[{"url":"https://i.scdn.co/a","width":60},{"url":"https://i.scdn.co/b","width":300}]},"trackList":[{"uri":"spotify:track:7bxaFZ1O3cHkgLKMsdC3xR","title":"Loser","subtitle":"Tame Impala","duration":223069,"isExplicit":true},{"uri":"spotify:episode:x","title":"Pod"}]}}}}}}</script></html>"#;
        let p = parse_embed(html, "abc").unwrap();
        assert_eq!(p.summary.name, "Top");
        assert_eq!(p.summary.image.as_deref(), Some("https://i.scdn.co/b"));
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.tracks[0].artists, vec!["Tame Impala"]);
        assert_eq!(p.tracks[0].duration_ms, 223069);
        assert!(parse_embed("<html></html>", "x").is_err());
    }
}
