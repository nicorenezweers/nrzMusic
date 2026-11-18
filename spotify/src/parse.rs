//! Antworten der Web-API → gemeinsame Typen. Nachsichtig: fehlende Felder ergeben
//! Standardwerte, kaputte Einträge (`null`, Episoden, lokale Dateien) werden übersprungen –
//! nie eine Panik.
//!
//! Seit Februar 2026 heisst die Titelliste einer Playlist `items` (früher `tracks`) und der
//! Titel eines Eintrags `item` (früher `track`). Beide Schreibweisen werden gelesen.

use serde_json::Value;

use crate::{PlaylistEntry, SpPlaylistSummary, SpTrack, SpotifyUser};

/// Bilder dieser Kantenlänge reichen für Listen und den Player (Spotify liefert Alben in
/// 640, 300 und 64 px).
const MIN_IMAGE: u64 = 300;

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn u64_of(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

/// `(url, Kantenlänge)` aller Bilder einer `images`-Liste; die Grösse ist `None`, wenn
/// Spotify keine nennt (z. B. bei Playlist-Collagen).
fn images(v: Option<&Value>) -> Vec<(&str, Option<u64>)> {
    v.and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|img| {
                    let url = str_of(img, "url").filter(|u| !u.is_empty())?;
                    let size = match (u64_of(img, "width"), u64_of(img, "height")) {
                        (Some(w), Some(h)) => Some(w.max(h)),
                        (w, h) => w.or(h),
                    };
                    Some((url, size))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Album-Cover für einen Titel: das kleinste Bild mit mindestens 300 px (normal 300 px –
/// reicht für Listen und Player, ohne 640-px-Bilder für lange Listen zu laden); sonst das
/// grösste bekannte, sonst das erste.
pub(crate) fn track_image(v: Option<&Value>) -> Option<String> {
    let imgs = images(v);
    imgs.iter()
        .filter_map(|&(u, s)| s.filter(|&s| s >= MIN_IMAGE).map(|s| (u, s)))
        .min_by_key(|&(_, s)| s)
        .or_else(|| {
            imgs.iter()
                .filter_map(|&(u, s)| s.map(|s| (u, s)))
                .max_by_key(|&(_, s)| s)
        })
        .map(|(u, _)| u)
        .or_else(|| imgs.first().map(|&(u, _)| u))
        .map(str::to_string)
}

/// Bild einer Playlist oder eines Profils: das grösste mit bekannter Grösse (≥ 300 px
/// bevorzugt), sonst das erste (Spotify sortiert absteigend; Collagen haben keine Grösse).
pub(crate) fn largest_image(v: Option<&Value>) -> Option<String> {
    let imgs = images(v);
    imgs.iter()
        .filter_map(|&(u, s)| s.map(|s| (u, s)))
        .max_by_key(|&(_, s)| s)
        .filter(|&(_, s)| s >= MIN_IMAGE)
        .map(|(u, _)| u)
        .or_else(|| imgs.first().map(|&(u, _)| u))
        .map(str::to_string)
}

/// Ein Track-Objekt → [`SpTrack`]. `None` für Episoden, lokale Dateien, `null` und Titel
/// ohne ID.
pub(crate) fn track(v: &Value, added_at: Option<&str>) -> Option<SpTrack> {
    if !v.is_object() {
        return None;
    }
    if str_of(v, "type").is_some_and(|t| t != "track") {
        return None;
    }
    // Lokale Dateien; ältere Antworten markieren Episoden als Track mit `"episode": true`
    if v.get("is_local").and_then(Value::as_bool) == Some(true)
        || v.get("episode").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let id = non_empty(str_of(v, "id"))?;
    let artists = v
        .get("artists")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| non_empty(str_of(a, "name")))
                .collect()
        })
        .unwrap_or_default();
    let album = v.get("album");
    Some(SpTrack {
        id,
        name: str_of(v, "name").unwrap_or_default().trim().to_string(),
        artists,
        album: album
            .and_then(|a| str_of(a, "name"))
            .unwrap_or_default()
            .trim()
            .to_string(),
        duration_ms: u64_of(v, "duration_ms").unwrap_or(0),
        image: track_image(album.and_then(|a| a.get("images"))),
        isrc: non_empty(v.pointer("/external_ids/isrc").and_then(Value::as_str))
            .map(|s| s.to_ascii_uppercase()),
        explicit: v.get("explicit").and_then(Value::as_bool).unwrap_or(false),
        added_at: non_empty(added_at),
        youtube: None,
    })
}

/// Ein Listeneintrag (`/me/tracks`, `/playlists/{id}/items`): `{added_at, item | track}`.
pub(crate) fn saved_item(v: &Value) -> Option<SpTrack> {
    if v.get("is_local").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let inner = v
        .get("item")
        .filter(|i| i.is_object())
        .or_else(|| v.get("track").filter(|t| t.is_object()))?;
    track(inner, str_of(v, "added_at"))
}

/// Alle brauchbaren Titel einer Seite (`{"items": […]}`).
pub(crate) fn track_page_items(page: &Value) -> Vec<SpTrack> {
    page.get("items")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(saved_item).collect())
        .unwrap_or_default()
}

/// Kopfdaten einer Seite: `total`, `offset`, `limit`, `next`.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct PageInfo {
    pub total: Option<u64>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    /// Anzahl Einträge auf dieser Seite (inklusive übersprungener)
    pub count: usize,
    pub next: Option<String>,
}

pub(crate) fn page_info(page: &Value) -> PageInfo {
    PageInfo {
        total: u64_of(page, "total"),
        offset: u64_of(page, "offset"),
        limit: u64_of(page, "limit"),
        count: page
            .get("items")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        next: non_empty(str_of(page, "next")),
    }
}

/// Die Titelliste eines Playlist-Objekts mit ihrem Namen: `items` (neu) oder `tracks`
/// (alt), nur wenn sie eine Liste von Einträgen enthält. Spotify lässt sie für fremde
/// Playlists weg.
pub(crate) fn playlist_track_page(v: &Value) -> Option<(&'static str, &Value)> {
    ["items", "tracks"].into_iter().find_map(|k| {
        v.get(k)
            .filter(|p| p.get("items").is_some_and(Value::is_array))
            .map(|p| (k, p))
    })
}

/// Anzahl Titel einer Playlist: `items.total` (neu) oder `tracks.total` (alt).
fn playlist_total(v: &Value) -> Option<u64> {
    v.pointer("/items/total")
        .and_then(Value::as_u64)
        .or_else(|| v.pointer("/tracks/total").and_then(Value::as_u64))
}

/// Ein (vereinfachtes) Playlist-Objekt → [`PlaylistEntry`]. `None` für `null` und
/// Einträge ohne ID.
pub(crate) fn playlist_entry(v: &Value) -> Option<PlaylistEntry> {
    if !v.is_object() {
        return None;
    }
    let id = non_empty(str_of(v, "id"))?;
    let owner = v.get("owner");
    let owner_id = owner.and_then(|o| non_empty(str_of(o, "id")));
    let owner_name = owner
        .and_then(|o| non_empty(str_of(o, "display_name")))
        .or_else(|| owner_id.clone());
    Some(PlaylistEntry {
        summary: SpPlaylistSummary {
            id,
            name: str_of(v, "name").unwrap_or_default().trim().to_string(),
            owner: owner_name,
            track_count: playlist_total(v)
                .map(|t| t.min(u32::MAX as u64) as u32)
                .unwrap_or(0),
            image: largest_image(v.get("images")),
            // Hängt vom angemeldeten Nutzer ab – der Server setzt es mit `items_readable`.
            readable: true,
        },
        owner_id,
        collaborative: v
            .get("collaborative")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// `GET /me` → [`SpotifyUser`]. `None` ohne ID.
pub(crate) fn user(v: &Value) -> Option<SpotifyUser> {
    Some(SpotifyUser {
        id: non_empty(str_of(v, "id"))?,
        display_name: non_empty(str_of(v, "display_name")),
        image: largest_image(v.get("images")),
        product: non_empty(str_of(v, "product")),
    })
}

/// Meldung aus einer Fehlerantwort: `{"error": {"status", "message", "reason"}}` (API) oder
/// `{"error": "…", "error_description": "…"}` (Anmeldung, RFC 6749).
pub(crate) fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    if let Some(obj) = err.as_object() {
        let message = obj.get("message").and_then(Value::as_str).map(str::trim);
        let reason = obj.get("reason").and_then(Value::as_str).map(str::trim);
        return match (
            message.filter(|m| !m.is_empty()),
            reason.filter(|r| !r.is_empty()),
        ) {
            (Some(m), Some(r)) => Some(format!("{m} ({r})")),
            (Some(m), None) => Some(m.to_string()),
            (None, Some(r)) => Some(r.to_string()),
            (None, None) => None,
        };
    }
    let code = err.as_str().map(str::trim).filter(|s| !s.is_empty());
    let desc = v
        .get("error_description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (code, desc) {
        (Some(c), Some(d)) => Some(format!("{d} ({c})")),
        (Some(c), None) => Some(c.to_string()),
        (None, Some(d)) => Some(d.to_string()),
        (None, None) => None,
    }
}

/// Fehlercode einer Anmelde-Fehlerantwort, z. B. `invalid_grant`.
pub(crate) fn auth_error_code(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("error").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn saved_tracks_page() {
        let page = fixture("saved_tracks.json");
        let tracks = track_page_items(&page);
        assert_eq!(tracks.len(), 2);
        let t = &tracks[0];
        assert_eq!(t.id, "4u7EnebtmKWzUH433cf5Qv");
        assert_eq!(t.name, "Bohemian Rhapsody - Remastered 2011");
        assert_eq!(t.artists, vec!["Queen"]);
        assert_eq!(t.album, "A Night At The Opera (2011 Remaster)");
        assert_eq!(t.duration_ms, 354_320);
        assert_eq!(
            t.image.as_deref(),
            Some("https://i.scdn.co/image/ab67616d00001e02ce4f1737bc8a646c8c4bd25a")
        );
        assert_eq!(t.isrc.as_deref(), Some("GBUM71029604"));
        assert!(!t.explicit);
        assert_eq!(t.added_at.as_deref(), Some("2026-10-30T19:12:45Z"));
        assert_eq!(t.youtube, None);

        let t = &tracks[1];
        assert_eq!(
            t.artists,
            vec!["Daft Punk", "Pharrell Williams", "Nile Rodgers"]
        );
        assert!(t.explicit);
        assert_eq!(t.isrc.as_deref(), Some("USQX91300108"));

        let info = page_info(&page);
        assert_eq!(info.total, Some(3));
        assert_eq!(info.offset, Some(0));
        assert_eq!(info.limit, Some(2));
        assert_eq!(info.count, 2);
        assert_eq!(
            info.next.as_deref(),
            Some("https://api.spotify.com/v1/me/tracks?offset=2&limit=2")
        );
        assert_eq!(page_info(&fixture("saved_tracks_page2.json")).next, None);
    }

    #[test]
    fn playlists_page() {
        let page = fixture("playlists.json");
        let entries: Vec<_> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(playlist_entry)
            .collect();
        assert_eq!(entries.len(), 3);

        let own = &entries[0];
        assert_eq!(own.summary.id, "3cEYpjA9oz9GiPac4AsH4n");
        assert_eq!(own.summary.name, "Roadtrip");
        assert_eq!(own.summary.owner.as_deref(), Some("Nico"));
        assert_eq!(own.summary.track_count, 5);
        // Collage ohne Grössenangabe → erstes Bild
        assert_eq!(
            own.summary.image.as_deref(),
            Some("https://mosaic.scdn.co/640/ab67616d0000b2731")
        );
        assert_eq!(own.owner_id.as_deref(), Some("nico.r"));
        assert!(own.items_readable("nico.r"));
        assert!(!own.items_readable("someone-else"));

        // Kollaborative Playlist eines anderen: Titel lesbar
        let collab = &entries[1];
        assert!(collab.collaborative);
        assert!(collab.items_readable("nico.r"));
        // Nur die alte Schreibweise `tracks.total`
        assert_eq!(collab.summary.track_count, 12);
        // Ohne display_name → Nutzer-ID
        assert_eq!(collab.summary.owner.as_deref(), Some("friend42"));
        assert_eq!(collab.summary.image, None);

        // Gefolgte Spotify-Playlist: Titel nicht lesbar
        let editorial = &entries[2];
        assert_eq!(editorial.owner_id.as_deref(), Some("spotify"));
        assert!(!editorial.items_readable("nico.r"));
        assert_eq!(
            editorial.summary.image.as_deref(),
            Some("https://i.scdn.co/image/ab67706f00000002ca5a7517156021292e5663a6")
        );
    }

    #[test]
    fn playlist_with_items() {
        let pl = fixture("playlist.json");
        let entry = playlist_entry(&pl).unwrap();
        assert_eq!(entry.summary.name, "Roadtrip");
        assert_eq!(entry.summary.track_count, 5);
        assert_eq!(
            entry.summary.image.as_deref(),
            Some("https://mosaic.scdn.co/640/ab67616d0000b2731")
        );
        let (key, page) = playlist_track_page(&pl).unwrap();
        assert_eq!(key, "items");
        let tracks = track_page_items(page);
        // Episode, null-Titel und lokale Datei fallen weg
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, "2takcwOaAZWiXQijPHIx7B");
        assert_eq!(tracks[0].name, "Time to Pretend");
        assert_eq!(tracks[0].added_at.as_deref(), Some("2026-08-01T10:00:00Z"));
        let info = page_info(page);
        assert_eq!(info.count, 4);
        assert_eq!(info.total, Some(5));
        assert!(info.next.is_some());

        let page2 = fixture("playlist_items_page2.json");
        let tracks = track_page_items(&page2);
        assert_eq!(tracks.len(), 1);
        // Alte Schreibweise `track` statt `item`
        assert_eq!(tracks[0].name, "Mädchen aus Ostberlin");
        assert_eq!(tracks[0].isrc.as_deref(), Some("DEA628100029"));
        assert_eq!(page_info(&page2).next, None);
    }

    #[test]
    fn foreign_playlist_without_items() {
        let pl = json!({
            "id": "37i9dQZF1DXcBWIGoYBM5M",
            "name": "Today's Top Hits",
            "owner": {"id": "spotify", "display_name": "Spotify"},
            "images": [],
            "collaborative": false
        });
        assert!(playlist_track_page(&pl).is_none());
        let e = playlist_entry(&pl).unwrap();
        assert_eq!(e.summary.track_count, 0);
        assert_eq!(e.summary.image, None);
        // Nur Metadaten (`items` ohne Liste) zählt auch nicht als Titelliste
        let pl = json!({"id": "x", "items": {"href": "h", "total": 50}});
        assert!(playlist_track_page(&pl).is_none());
        assert_eq!(playlist_entry(&pl).unwrap().summary.track_count, 50);
    }

    #[test]
    fn user_profile() {
        let u = user(&fixture("me.json")).unwrap();
        assert_eq!(u.id, "nico.r");
        assert_eq!(u.display_name.as_deref(), Some("Nico"));
        assert_eq!(
            u.image.as_deref(),
            Some("https://i.scdn.co/image/ab6775700000ee85-large")
        );
        assert_eq!(u.product.as_deref(), Some("premium"));

        // Entwicklungsmodus seit 2026: ohne product, ohne Bild, display_name null
        let u = user(&json!({"id": "x", "display_name": null, "images": []})).unwrap();
        assert_eq!(u.display_name, None);
        assert_eq!(u.image, None);
        assert_eq!(u.product, None);
        assert_eq!(user(&json!({"display_name": "kein id"})), None);
    }

    #[test]
    fn broken_entries_are_skipped() {
        assert_eq!(track(&json!(null), None), None);
        assert_eq!(track(&json!({"type": "episode", "id": "e"}), None), None);
        assert_eq!(
            track(
                &json!({"type": "track", "id": null, "is_local": true}),
                None
            ),
            None
        );
        assert_eq!(
            track(&json!({"type": "track", "name": "ohne id"}), None),
            None
        );
        assert_eq!(
            track(&json!({"type": "track", "id": "e", "episode": true}), None),
            None
        );
        assert_eq!(saved_item(&json!({"added_at": "x", "track": null})), None);
        assert_eq!(
            saved_item(&json!({"is_local": true, "track": {"id": "a"}})),
            None
        );
        assert!(track_page_items(&json!({"items": "kaputt"})).is_empty());
        assert!(track_page_items(&json!({})).is_empty());
        assert_eq!(playlist_entry(&json!(null)), None);

        // Minimaler Titel: alles andere mit Standardwerten
        let t = track(&json!({"id": "m"}), Some("")).unwrap();
        assert_eq!(t.name, "");
        assert!(t.artists.is_empty());
        assert_eq!(t.duration_ms, 0);
        assert_eq!(t.image, None);
        assert_eq!(t.isrc, None);
        assert_eq!(t.added_at, None);
    }

    #[test]
    fn image_choice() {
        let album = json!([
            {"url": "big", "width": 640, "height": 640},
            {"url": "mid", "width": 300, "height": 300},
            {"url": "small", "width": 64, "height": 64}
        ]);
        assert_eq!(track_image(Some(&album)).as_deref(), Some("mid"));
        assert_eq!(largest_image(Some(&album)).as_deref(), Some("big"));

        let small_only = json!([
            {"url": "s1", "width": 64, "height": 64},
            {"url": "s2", "width": 120, "height": 120}
        ]);
        assert_eq!(track_image(Some(&small_only)).as_deref(), Some("s2"));
        // Playlist/Profil: kein Bild ≥ 300 px mit Grösse → erstes
        assert_eq!(largest_image(Some(&small_only)).as_deref(), Some("s1"));

        let no_size = json!([{"url": "u1", "width": null, "height": null}, {"url": "u2"}]);
        assert_eq!(track_image(Some(&no_size)).as_deref(), Some("u1"));
        assert_eq!(largest_image(Some(&no_size)).as_deref(), Some("u1"));
        assert_eq!(track_image(Some(&json!([]))), None);
        assert_eq!(track_image(None), None);
        assert_eq!(largest_image(Some(&json!([{"url": ""}]))), None);
    }

    #[test]
    fn error_messages() {
        let api = fixture("error_unauthorized.json").to_string();
        assert_eq!(
            error_message(&api).as_deref(),
            Some("The access token expired")
        );
        let quota = fixture("error_quota.json").to_string();
        assert_eq!(
            error_message(&quota).as_deref(),
            Some("Too many requests (QUOTA_EXCEEDED)")
        );
        let auth = fixture("error_invalid_grant.json").to_string();
        assert_eq!(
            error_message(&auth).as_deref(),
            Some("Invalid refresh token (invalid_grant)")
        );
        assert_eq!(auth_error_code(&auth).as_deref(), Some("invalid_grant"));
        assert_eq!(auth_error_code(&api), None);
        assert_eq!(error_message("<html>Bad Gateway</html>"), None);
        assert_eq!(error_message(r#"{"error": {}}"#), None);
    }
}
