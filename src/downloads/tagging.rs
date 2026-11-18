//! Tags der heruntergeladenen Datei schreibt nrzMusic selbst (lofty) – aus den Metadaten von
//! YouTube Music, nicht aus dem, was yt-dlp vom Video weiss. Cover ist das quadratische
//! Vorschaubild (544 px), notfalls mittig quadratisch zugeschnitten.

use crate::routes::api::upload::{clean_component, truncate_bytes};
use image::GenericImageView;
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::Tag;
use nrzmusic_types::YtTrack;
use std::path::Path;
use std::time::Duration;

/// Kantenlänge des bevorzugten Vorschaubilds von YouTube Music.
const COVER_EDGE: u32 = 544;
/// Grösser wird kein Cover geladen.
const MAX_COVER_BYTES: usize = 10 * 1024 * 1024;

/// Was in die Datei geschrieben wird.
#[derive(Debug, Clone, PartialEq)]
pub struct TagData {
    pub title: String,
    pub artist: String,
    /// Leer, wenn YouTube kein Album kennt
    pub album: String,
    pub album_artist: String,
    pub comment: String,
}

impl TagData {
    pub fn from_track(t: &YtTrack) -> Self {
        TagData {
            title: t.title.trim().to_string(),
            artist: t.artist_line(),
            album: t
                .album
                .as_ref()
                .map(|a| a.title.trim().to_string())
                .unwrap_or_default(),
            album_artist: t
                .artists
                .first()
                .map(|a| a.name.trim().to_string())
                .unwrap_or_default(),
            comment: super::ytdlp::watch_url(&t.video_id),
        }
    }
}

/// Tags und Cover schreiben (blockierend).
pub fn write_tags(path: &Path, data: &TagData, cover: Option<Vec<u8>>) -> Result<(), String> {
    let mut file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let tag_type = file.primary_tag_type();
    if file.tag(tag_type).is_none() {
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file.tag_mut(tag_type).expect("eben eingefügt");
    tag.set_title(data.title.clone());
    if data.artist.is_empty() {
        tag.remove_artist();
    } else {
        tag.set_artist(data.artist.clone());
    }
    if data.album.is_empty() {
        tag.remove_album();
    } else {
        tag.set_album(data.album.clone());
    }
    tag.remove_key(&ItemKey::AlbumArtist);
    if !data.album_artist.is_empty() {
        tag.insert_text(ItemKey::AlbumArtist, data.album_artist.clone());
    }
    tag.set_comment(data.comment.clone());
    if let Some(cover) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Jpeg),
            None,
            cover,
        ));
    }
    file.save_to_path(path, WriteOptions::default())
        .map_err(|e| e.to_string())
}

/// Adresse des quadratischen 544-px-Bilds (Bilder von googleusercontent/ggpht lassen sich so
/// bestellen); andere Adressen bleiben, wie sie sind.
pub fn square_cover_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let google = lower.contains("googleusercontent.com/") || lower.contains("ggpht.com/");
    if google {
        if let Some(pos) = url.rfind('=') {
            let suffix = &url[pos + 1..];
            if !suffix.contains('/') && !suffix.contains('?') {
                return format!("{}=w{COVER_EDGE}-h{COVER_EDGE}-l90-rj", &url[..pos]);
            }
        }
    }
    url.to_string()
}

/// Bild quadratisch machen (mittig zuschneiden) und als JPEG liefern. Schon quadratische
/// JPEGs bleiben unverändert.
pub fn square_jpeg(data: &[u8]) -> Option<Vec<u8>> {
    let format = image::guess_format(data).ok()?;
    let img = image::load_from_memory(data).ok()?;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    if w == h && format == image::ImageFormat::Jpeg {
        return Some(data.to_vec());
    }
    let edge = w.min(h);
    let square = img
        .crop_imm((w - edge) / 2, (h - edge) / 2, edge, edge)
        .to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&square)
        .ok()?;
    Some(out)
}

/// Vorschaubild laden (quadratisch, 544 px bevorzugt). Fehler sind nicht schlimm – dann eben
/// ohne Cover.
pub async fn fetch_cover(http: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let url = if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    };
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let mut candidates = vec![square_cover_url(&url)];
    if candidates[0] != url {
        candidates.push(url.clone());
    }
    for candidate in candidates {
        let res = match http
            .get(&candidate)
            .timeout(Duration::from_secs(20))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::debug!(url = %candidate, status = %r.status(), "Cover nicht ladbar");
                continue;
            }
            Err(e) => {
                tracing::debug!(url = %candidate, "Cover nicht ladbar: {e}");
                continue;
            }
        };
        if res
            .content_length()
            .is_some_and(|n| n as usize > MAX_COVER_BYTES)
        {
            continue;
        }
        let Ok(bytes) = res.bytes().await else {
            continue;
        };
        if bytes.len() > MAX_COVER_BYTES {
            continue;
        }
        let data = bytes.to_vec();
        if let Ok(Some(jpeg)) = tokio::task::spawn_blocking(move || square_jpeg(&data)).await {
            return Some(jpeg);
        }
    }
    None
}

/// Ein Teil des Zielpfads: keine Schrägstriche, keine Steuer- und Sonderzeichen, nicht
/// versteckt, nicht zu lang.
fn path_component(s: &str, fallback: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    let cleaned = clean_component(&replaced);
    let cleaned = cleaned.trim_start_matches(['.', ' ']).to_string();
    let cleaned = truncate_bytes(&cleaned, 120);
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Zielpfad relativ zum Musikordner: `YouTube/<Künstler>/<Album oder Singles>/<Titel>.<ext>`
/// (als Teile; der Dateiname zuletzt).
pub fn target_parts(t: &YtTrack, ext: &str) -> Vec<String> {
    let artist = t.artists.first().map(|a| a.name.as_str()).unwrap_or("");
    let album = t.album.as_ref().map(|a| a.title.as_str()).unwrap_or("");
    vec![
        "YouTube".to_string(),
        path_component(artist, "Unbekannt"),
        path_component(album, "Singles"),
        format!("{}.{ext}", path_component(&t.title, &t.video_id)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use nrzmusic_types::{YtAlbumRef, YtArtistRef};

    fn track() -> YtTrack {
        YtTrack {
            video_id: "dQw4w9WgXcQ".into(),
            title: "What/Ever: Live?".into(),
            artists: vec![
                YtArtistRef {
                    name: "AC/DC".into(),
                    id: None,
                },
                YtArtistRef {
                    name: "Guest".into(),
                    id: None,
                },
            ],
            album: None,
            duration_ms: None,
            thumbnail: None,
            explicit: false,
            is_video: false,
        }
    }

    #[test]
    fn target_paths() {
        let mut t = track();
        assert_eq!(
            target_parts(&t, "mp3"),
            vec!["YouTube", "AC_DC", "Singles", "What_Ever_ Live_.mp3"]
        );
        t.album = Some(YtAlbumRef {
            title: "..Hidden".into(),
            id: None,
        });
        t.artists.clear();
        t.title = "  ".into();
        assert_eq!(
            target_parts(&t, "opus"),
            vec!["YouTube", "Unbekannt", "Hidden", "dQw4w9WgXcQ.opus"]
        );
    }

    #[test]
    fn tag_data() {
        let d = TagData::from_track(&track());
        assert_eq!(d.artist, "AC/DC & Guest");
        assert_eq!(d.album_artist, "AC/DC");
        assert_eq!(d.album, "");
        assert_eq!(d.comment, "https://music.youtube.com/watch?v=dQw4w9WgXcQ");
    }

    #[test]
    fn cover_urls() {
        assert_eq!(
            square_cover_url("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            square_cover_url("https://i.ytimg.com/vi/x/hqdefault.jpg"),
            "https://i.ytimg.com/vi/x/hqdefault.jpg"
        );
    }

    #[test]
    fn squares() {
        let img = image::RgbImage::from_pixel(160, 90, image::Rgb([10, 20, 30]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let jpeg = square_jpeg(png.get_ref()).unwrap();
        let out = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(out.dimensions(), (90, 90));
        assert_eq!(
            image::guess_format(&jpeg).unwrap(),
            image::ImageFormat::Jpeg
        );
    }
}
