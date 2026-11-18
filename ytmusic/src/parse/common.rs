//! Bausteine, die alle Seiten gemeinsam haben: Dauer, Vorschaubilder, Badges und das
//! Zerlegen der Textzeilen ("Titel • Daft Punk • Discovery • 3:45") in Künstler, Album,
//! Dauer und Jahr – nach dem Vorbild von ytmusicapi (`parse_song_runs`).

use nrzmusic_types::{YtAlbumRef, YtArtistRef};
use serde_json::Value;

use crate::json::{JsonExt, Run};

pub(crate) const VIDEO_TYPE_SONG: &str = "MUSIC_VIDEO_TYPE_ATV";
pub(crate) const VIDEO_TYPE_UPLOAD: &str = "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK";
pub(crate) const VIDEO_TYPE_EPISODE: &str = "MUSIC_VIDEO_TYPE_PODCAST_EPISODE";

pub(crate) const PAGE_ALBUM: &str = "MUSIC_PAGE_TYPE_ALBUM";
pub(crate) const PAGE_AUDIOBOOK: &str = "MUSIC_PAGE_TYPE_AUDIOBOOK";
pub(crate) const PAGE_ARTIST: &str = "MUSIC_PAGE_TYPE_ARTIST";
pub(crate) const PAGE_PLAYLIST: &str = "MUSIC_PAGE_TYPE_PLAYLIST";
pub(crate) const PAGE_EPISODE: &str = "MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE";

/// Grösse, auf die Bilder von Googles Bildserver hochgerechnet werden.
const THUMB_EDGE: u32 = 544;

// ---------------------------------------------------------------- Dauer, Jahr, Aufrufe

/// "3:45" → 225000, "1:02:03" → 3723000 (Millisekunden). Alles andere → `None`.
pub fn parse_duration_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() < 2 || parts.len() > 4 {
        return None;
    }
    if parts
        .iter()
        .any(|p| p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let mut seconds: u64 = 0;
    for (i, part) in parts.iter().rev().enumerate() {
        let n: u64 = part.parse().ok()?;
        let factor = match i {
            0 => 1,
            1 => 60,
            2 => 3600,
            _ => 86_400,
        };
        seconds += n * factor;
    }
    Some(seconds * 1000)
}

fn is_year(text: &str) -> bool {
    let t = text.trim();
    t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit())
}

/// Erkennt Angaben wie "55 Mio. Wiedergaben", "76M plays", "2394 Aufrufe" oder
/// "조회수 17억회". Gibt die Angabe zurück, `None` wenn es eher ein Name ist ("2Pac").
pub(crate) fn parse_views(text: &str) -> Option<String> {
    let mut t = text.trim();
    let mut prefixed = false;
    // Nicht-lateinische Schriften stellen das Wort voran ("播放次數：4505")
    if !t.chars().any(|c| c.is_ascii_alphabetic()) {
        if let Some(pos) = t.find(|c: char| c.is_ascii_digit()) {
            let prefix = &t[..pos];
            if prefix
                .chars()
                .any(|c| c.is_whitespace() || matches!(c, ':' | '\u{ff1a}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}'))
            {
                t = &t[pos..];
                prefixed = true;
            }
        }
    }
    if !t.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if !prefixed && t.is_ascii() && !t.contains(' ') {
        return None;
    }
    Some(t.to_string())
}

/// Wörter, an denen eine Titelanzahl ("35 Songs", "19 Titel") zu erkennen ist – im
/// Gegensatz zu Aufrufen ("2394 Aufrufe"). Bewusst kurz gehalten: de, en, fr, es, it, pt, nl.
const TRACK_WORDS: &[&str] = &[
    "song",
    "songs",
    "titel",
    "track",
    "tracks",
    "titre",
    "titres",
    "canción",
    "canciones",
    "brano",
    "brani",
    "faixa",
    "faixas",
    "música",
    "músicas",
    "nummer",
    "nummers",
    "utwór",
    "utwory",
    "utworów",
];

/// "35 Songs" → `true`, "2394 Aufrufe" → `false`.
pub(crate) fn is_track_count(text: &str) -> bool {
    let t = text.trim();
    if !t.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    t.split_whitespace().skip(1).any(|w| {
        TRACK_WORDS.contains(
            &w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric()),
        )
    })
}

// ---------------------------------------------------------------- Vorschaubilder

/// Bild-URL für die Oberfläche: Bilder von Googles Bildserver (`lh3.googleusercontent.com`,
/// `yt3.ggpht.com` …) werden auf 544 px hochgerechnet, Videobilder (`i.ytimg.com`) auf das
/// schlichte `hqdefault.jpg` zurückgeführt.
pub fn upscale_thumbnail(url: &str) -> String {
    let url = if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    };
    let Ok(parsed) = url::Url::parse(&url) else {
        return url;
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();

    if host.ends_with("ytimg.com") {
        let mut segments = parsed.path_segments().into_iter().flatten();
        while let Some(seg) = segments.next() {
            if seg == "vi" || seg == "vi_webp" {
                if let Some(id) = segments.next().filter(|id| !id.is_empty()) {
                    return format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg");
                }
            }
        }
        return url;
    }

    if host.ends_with("googleusercontent.com") || host.ends_with("ggpht.com") {
        if let Some(pos) = url.rfind('=') {
            let (base, suffix) = (&url[..pos], &url[pos + 1..]);
            if suffix.contains('/') || suffix.contains('?') {
                return url;
            }
            let mut parts = suffix.split('-');
            let first = parts.next().unwrap_or("");
            let second = parts.next().unwrap_or("");
            let num = |s: &str, prefix: char| {
                s.strip_prefix(prefix)
                    .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    .map(|n| n.parse::<u32>().unwrap_or(0))
            };
            if num(first, 'w').is_some() && num(second, 'h').is_some() {
                // "-p" = auf das Quadrat zuschneiden (Künstlerbilder, breite Kopfbilder)
                let crop = suffix.split('-').any(|p| p == "p");
                return format!(
                    "{base}=w{THUMB_EDGE}-h{THUMB_EDGE}{}-l90-rj",
                    if crop { "-p" } else { "" }
                );
            }
            if let Some(size) = num(first, 's') {
                if size < THUMB_EDGE {
                    let rest = &suffix[first.len()..];
                    return format!("{base}=s{THUMB_EDGE}{rest}");
                }
            }
        }
        return url;
    }

    url
}

/// Grösstes Bild aus einer `thumbnails`-Liste, hochgerechnet.
pub(crate) fn best_thumbnail(thumbs: &[Value]) -> Option<String> {
    thumbs
        .iter()
        .filter_map(|t| {
            let url = t.str_at("/url")?;
            let w = t.get("width").and_then(Value::as_u64).unwrap_or(0);
            Some((w, url))
        })
        .max_by_key(|(w, _)| *w)
        .map(|(_, url)| upscale_thumbnail(url))
}

/// Vorschaubild eines Renderers – probiert die üblichen Orte durch.
pub(crate) fn thumbnail_of(v: &Value) -> Option<String> {
    const PATHS: &[&str] = &[
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnailRenderer/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
    ];
    PATHS.iter().find_map(|p| best_thumbnail(v.arr_at(p)))
}

// ---------------------------------------------------------------- Badges, Typen

/// Hat das Element das "E"-Badge?
pub(crate) fn is_explicit(v: &Value) -> bool {
    ["/badges", "/subtitleBadges", "/subtitleBadge"]
        .iter()
        .any(|p| {
            v.arr_at(p).iter().any(|b| {
                b.str_at("/musicInlineBadgeRenderer/icon/iconType") == Some("MUSIC_EXPLICIT_BADGE")
            })
        })
}

/// `musicVideoType` → Musikvideo (`true`) oder Audio-Titel (`false`); `None` wenn unbekannt.
pub(crate) fn is_video_type(video_type: Option<&str>) -> Option<bool> {
    match video_type? {
        VIDEO_TYPE_SONG | VIDEO_TYPE_UPLOAD => Some(false),
        VIDEO_TYPE_EPISODE => None,
        t if t.starts_with("MUSIC_VIDEO_TYPE_") => Some(true),
        _ => None,
    }
}

/// Ohne Typangabe: Videobilder (16:9 von `i.ytimg.com`) deuten auf ein Video hin.
pub(crate) fn looks_like_video(thumbnail: Option<&str>) -> bool {
    thumbnail.is_some_and(|t| t.contains("ytimg.com/"))
}

/// Playlist-ID aus einer Browse-ID ("VLPL…" → "PL…").
pub(crate) fn playlist_id_from_browse(browse_id: &str) -> String {
    browse_id
        .strip_prefix("VL")
        .unwrap_or(browse_id)
        .to_string()
}

// ---------------------------------------------------------------- Textzeilen zerlegen

/// Was in einer Zeile wie "Daft Punk • Discovery • 2001" steckt.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RunInfo {
    pub artists: Vec<YtArtistRef>,
    pub album: Option<YtAlbumRef>,
    pub duration_ms: Option<u64>,
    pub year: Option<String>,
    pub views: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunKind {
    Album(YtAlbumRef),
    Artist(YtArtistRef),
    Duration(u64),
    Year(String),
    Views(String),
    Skip,
}

/// Bedeutung eines einzelnen Runs.
pub(crate) fn classify_run(run: Run<'_>) -> RunKind {
    let text = run.text().trim();
    if run.has_endpoint() {
        if text.is_empty() {
            return RunKind::Skip;
        }
        let id = run.browse_id();
        let page = run.page_type();
        if id.is_some_and(|id| id.starts_with("MPRE") || id.contains("release_detail"))
            || matches!(page, Some(PAGE_ALBUM | PAGE_AUDIOBOOK))
        {
            return RunKind::Album(YtAlbumRef {
                title: text.to_string(),
                id: id.map(str::to_string),
            });
        }
        if run.video_id().is_some() || matches!(page, Some(PAGE_PLAYLIST)) {
            return RunKind::Skip;
        }
        return RunKind::Artist(YtArtistRef {
            name: text.to_string(),
            id: id.filter(|id| !id.is_empty()).map(str::to_string),
        });
    }
    if text.is_empty() || run.is_separator() {
        return RunKind::Skip;
    }
    if let Some(ms) = parse_duration_ms(text) {
        return RunKind::Duration(ms);
    }
    if is_year(text) {
        return RunKind::Year(text.to_string());
    }
    if let Some(views) = parse_views(text) {
        return RunKind::Views(views);
    }
    RunKind::Artist(YtArtistRef {
        name: text.to_string(),
        id: None,
    })
}

/// Lässt eine vorangestellte Typangabe ("Titel • …", "Video • …", "Album • …") weg.
/// Wie bei ytmusicapi: erster Run ohne Link, dann " • ", dann etwas Sinnvolles.
pub(crate) fn strip_type_label<'a, 'b>(runs: &'b [Run<'a>]) -> (&'b [Run<'a>], Option<&'a str>) {
    if runs.len() > 2
        && !runs[0].has_endpoint()
        && matches!(classify_run(runs[0]), RunKind::Artist(_))
        && runs[1].is_dot()
        && matches!(
            classify_run(runs[2]),
            RunKind::Artist(_) | RunKind::Duration(_) | RunKind::Views(_) | RunKind::Year(_)
        )
    {
        (&runs[2..], Some(runs[0].text().trim()))
    } else {
        (runs, None)
    }
}

/// Zerlegt eine Textzeile. Trenner stehen an ungeraden Stellen; Runs mit Link werden
/// immer ausgewertet, auch wenn die Abwechslung einmal nicht stimmt.
pub(crate) fn parse_song_runs(runs: &[Run<'_>], skip_type_label: bool) -> RunInfo {
    let runs = if skip_type_label {
        strip_type_label(runs).0
    } else {
        runs
    };
    let mut info = RunInfo::default();
    for (i, run) in runs.iter().enumerate() {
        if i % 2 == 1 && !run.has_endpoint() {
            continue;
        }
        match classify_run(*run) {
            RunKind::Album(a) => {
                if info.album.is_none() {
                    info.album = Some(a);
                }
            }
            RunKind::Artist(a) => {
                if !info.artists.contains(&a) {
                    info.artists.push(a);
                }
            }
            RunKind::Duration(ms) => {
                info.duration_ms.get_or_insert(ms);
            }
            RunKind::Year(y) => {
                info.year.get_or_insert(y);
            }
            RunKind::Views(v) => {
                info.views.get_or_insert(v);
            }
            RunKind::Skip => {}
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("3:45"), Some(225_000));
        assert_eq!(parse_duration_ms("0:07"), Some(7_000));
        assert_eq!(parse_duration_ms("1:02:03"), Some(3_723_000));
        assert_eq!(parse_duration_ms(" 10:01 "), Some(601_000));
        assert_eq!(parse_duration_ms("2013"), None);
        assert_eq!(parse_duration_ms("2,343"), None);
        assert_eq!(parse_duration_ms("3:"), None);
        assert_eq!(parse_duration_ms(":45"), None);
        assert_eq!(parse_duration_ms(""), None);
        assert_eq!(parse_duration_ms("a:bc"), None);
    }

    #[test]
    fn views() {
        assert_eq!(
            parse_views("55 Mio. Wiedergaben").as_deref(),
            Some("55 Mio. Wiedergaben")
        );
        assert!(parse_views("76M plays").is_some());
        assert!(parse_views("2394 Aufrufe").is_some());
        assert!(parse_views("1,9\u{a0}Mrd. Wiedergaben").is_some());
        assert!(parse_views("조회수 17억회").is_some());
        assert_eq!(parse_views("2Pac"), None);
        assert_eq!(parse_views("Daft Punk"), None);
        assert!(is_track_count("35 Songs"));
        assert!(is_track_count("19 Titel"));
        assert!(!is_track_count("2394 Aufrufe"));
        assert!(!is_track_count("Songs"));
    }

    #[test]
    fn thumbnails() {
        assert_eq!(
            upscale_thumbnail("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            upscale_thumbnail("https://lh3.googleusercontent.com/abc=w1080-h450-p-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-p-l90-rj"
        );
        assert_eq!(
            upscale_thumbnail("https://yt3.ggpht.com/xyz=s48-c-k-c0x00000000-no-cc-rj-rp"),
            "https://yt3.ggpht.com/xyz=s544-c-k-c0x00000000-no-cc-rj-rp"
        );
        assert_eq!(
            upscale_thumbnail("https://yt3.googleusercontent.com/q=s576"),
            "https://yt3.googleusercontent.com/q=s576"
        );
        assert_eq!(
            upscale_thumbnail(
                "https://i.ytimg.com/vi/MV_3Dpw-BRY/hqdefault.jpg?sqp=-oaymwEW&rs=AMz"
            ),
            "https://i.ytimg.com/vi/MV_3Dpw-BRY/hqdefault.jpg"
        );
        assert_eq!(
            upscale_thumbnail("//i.ytimg.com/vi_webp/abc/sddefault.webp"),
            "https://i.ytimg.com/vi/abc/hqdefault.jpg"
        );
        assert_eq!(upscale_thumbnail("not a url"), "not a url");
        let thumbs = json!([
            {"url": "https://lh3.googleusercontent.com/a=w60-h60-l90-rj", "width": 60, "height": 60},
            {"url": "https://lh3.googleusercontent.com/b=w120-h120-l90-rj", "width": 120, "height": 120}
        ]);
        assert_eq!(
            best_thumbnail(thumbs.as_array().unwrap()).as_deref(),
            Some("https://lh3.googleusercontent.com/b=w544-h544-l90-rj")
        );
        assert_eq!(best_thumbnail(&[]), None);
    }

    fn runs(v: &Value) -> Vec<Run<'_>> {
        v.as_array().unwrap().iter().map(Run).collect()
    }

    #[test]
    fn song_runs() {
        let artist = |name: &str, id: &str| {
            json!({"text": name, "navigationEndpoint": {"browseEndpoint": {"browseId": id,
                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": PAGE_ARTIST}}}}})
        };
        let album = json!({"text": "Discovery", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x"}}});
        let v = json!([
            {"text": "Titel"}, {"text": " • "}, artist("Daft Punk", "UC1"), {"text": " und "}, artist("Pharrell", "UC2"),
            {"text": " • "}, album, {"text": " • "}, {"text": "3:45"}, {"text": " • "}, {"text": "2001"}
        ]);
        let info = parse_song_runs(&runs(&v), true);
        assert_eq!(info.artists.len(), 2);
        assert_eq!(
            info.artists[0],
            YtArtistRef {
                name: "Daft Punk".into(),
                id: Some("UC1".into())
            }
        );
        assert_eq!(
            info.album.as_ref().map(|a| a.title.as_str()),
            Some("Discovery")
        );
        assert_eq!(info.duration_ms, Some(225_000));
        assert_eq!(info.year.as_deref(), Some("2001"));

        // Ohne Typangabe bleibt ein unverlinkter Künstler erhalten
        let v = json!([{"text": "Sardi"}, {"text": " • "}, {"text": "5,5 Mio. Wiedergaben"}]);
        let info = parse_song_runs(&runs(&v), false);
        assert_eq!(
            info.artists,
            vec![YtArtistRef {
                name: "Sardi".into(),
                id: None
            }]
        );
        assert!(info.views.is_some());

        // Typangabe vor einer Dauer ("Titel • 1:45")
        let v = json!([{"text": "Titel"}, {"text": " • "}, {"text": "1:45"}]);
        let info = parse_song_runs(&runs(&v), true);
        assert!(info.artists.is_empty());
        assert_eq!(info.duration_ms, Some(105_000));

        // Kaputte Abwechslung: Trenner an gerader Stelle werden übersprungen
        let v = json!([{"text": ", "}, artist("A", "UC1")]);
        let info = parse_song_runs(&runs(&v), false);
        assert_eq!(info.artists.len(), 1);
        assert_eq!(parse_song_runs(&[], true), RunInfo::default());
    }
}
