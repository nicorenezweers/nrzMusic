//! Tags und Eigenschaften einer Audiodatei mit `lofty` lesen – mit Fallbacks, damit auch
//! Dateien ohne Tags sinnvoll in der Bibliothek stehen:
//! Titel aus dem Dateinamen (ohne führende Titelnummer wie "01 - "), Album aus dem Ordnernamen,
//! Künstler aus dem Ordner darüber ("Künstler/Album/Titel"),
//! Album-Künstler = Künstler. Unbekannte Künstler bleiben leer (die Oberfläche zeigt einen
//! Platzhalter in ihrer Sprache).

use lofty::picture::PictureType;
use lofty::prelude::*;
use std::path::Path;

/// Was aus einer Datei gelesen wurde (noch ohne Cover-ID, ohne Dateigrösse/-zeit).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub bitrate_kbps: Option<u32>,
}

/// Liest eine Datei. `rel` ist der Pfad relativ zum Musikordner ('/'-getrennt).
/// Liefert die Tags samt Fallbacks und das eingebettete Cover (Vorderseite, sonst das erste Bild).
pub fn read(abs: &Path, rel: &str) -> (TagInfo, Option<Vec<u8>>) {
    let mut info = TagInfo::default();
    let mut picture = None;

    match lofty::read_from_path(abs) {
        Ok(file) => {
            let props = file.properties();
            info.duration_ms = props.duration().as_millis() as u64;
            info.bitrate_kbps = props
                .audio_bitrate()
                .or(props.overall_bitrate())
                .filter(|b| *b > 0);

            // Haupt-Tag zuerst, dann die übrigen (z. B. ID3v1 neben ID3v2)
            let mut tags: Vec<&lofty::tag::Tag> = file.primary_tag().into_iter().collect();
            tags.extend(
                file.tags()
                    .iter()
                    .filter(|t| Some(t.tag_type()) != file.primary_tag().map(|p| p.tag_type())),
            );
            for tag in &tags {
                fill(&mut info.title, tag.title().as_deref());
                fill(&mut info.artist, tag.artist().as_deref());
                fill(&mut info.album, tag.album().as_deref());
                fill(
                    &mut info.album_artist,
                    tag.get_string(&ItemKey::AlbumArtist),
                );
                info.track_no = info.track_no.or(tag.track().filter(|n| *n > 0));
                info.disc_no = info.disc_no.or(tag.disk().filter(|n| *n > 0));
                info.year = info.year.or(tag
                    .year()
                    .filter(|y| (1..=9999).contains(y))
                    .map(|y| y as i32));
                if info.genre.is_none() {
                    info.genre = tag
                        .genre()
                        .map(|g| g.trim().to_string())
                        .filter(|g| !g.is_empty());
                }
            }
            picture = tags
                .iter()
                .flat_map(|t| t.pictures())
                .find(|p| p.pic_type() == PictureType::CoverFront)
                .or_else(|| tags.iter().flat_map(|t| t.pictures()).next())
                .map(|p| p.data().to_vec())
                .filter(|d| !d.is_empty());
        }
        Err(e) => tracing::debug!(file = rel, "Tags nicht lesbar: {e}"),
    }

    apply_fallbacks(&mut info, rel);
    (info, picture)
}

fn fill(target: &mut String, value: Option<&str>) {
    if target.is_empty() {
        if let Some(v) = value.map(clean).filter(|v| !v.is_empty()) {
            *target = v;
        }
    }
}

/// Steuerzeichen (z. B. NUL am Ende von ID3v1-Feldern) entfernen, Leerraum kürzen.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Fehlende Angaben aus Dateiname und Ordner ergänzen.
pub fn apply_fallbacks(info: &mut TagInfo, rel: &str) {
    let mut parts: Vec<&str> = rel.split('/').collect();
    let file_name = parts.pop().unwrap_or(rel);
    let stem = match file_name.rfind('.') {
        Some(i) if i > 0 => &file_name[..i],
        _ => file_name,
    };
    if info.title.is_empty() {
        let (title, number) = title_from_file_name(stem);
        info.title = title;
        info.track_no = info.track_no.or(number);
    }
    // Liegt die Datei nicht direkt im Musikordner, heisst das Album wie ihr Ordner.
    if info.album.is_empty() {
        if let Some(folder) = parts.last() {
            info.album = folder.trim().to_string();
        }
    }
    // Übliche Ablage "Künstler/Album/Titel": der Ordner darüber ist der Künstler.
    if info.artist.is_empty() && parts.len() >= 2 {
        info.artist = parts[parts.len() - 2].trim().to_string();
    }
    if info.album_artist.is_empty() {
        info.album_artist = info.artist.clone();
    }
}

/// Titel aus dem Dateinamen: führende Titelnummer ("01 - ", "01. ", "1_", "03) ", "07 ")
/// entfernen und als Nummer zurückgeben; Unterstriche werden zu Leerzeichen.
pub fn title_from_file_name(stem: &str) -> (String, Option<u32>) {
    let stem = stem.trim();
    let digits = stem.chars().take_while(|c| c.is_ascii_digit()).count();
    let mut title = stem;
    let mut number = None;
    if (1..=3).contains(&digits) {
        let rest = &stem[digits..];
        let trimmed = rest.trim_start();
        let after_sep = trimmed
            .strip_prefix(['-', '.', '_', ')', '–', '—'])
            .map(str::trim_start);
        // "07 Titel" nur mit führender Null – "99 Luftballons" bleibt, wie es ist
        let after_space = (trimmed.len() < rest.len() && stem.starts_with('0')).then_some(trimmed);
        if let Some(t) = after_sep.or(after_space).filter(|t| !t.is_empty()) {
            title = t;
            number = stem[..digits].parse().ok().filter(|n| *n > 0);
        }
    }
    let title = if title.contains(' ') {
        title.to_string()
    } else {
        title.replace('_', " ")
    };
    let title = clean(&title);
    if title.is_empty() {
        (clean(stem), None)
    } else {
        (title, number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_fallbacks() {
        assert_eq!(title_from_file_name("01 - Song"), ("Song".into(), Some(1)));
        assert_eq!(title_from_file_name("01. Song"), ("Song".into(), Some(1)));
        assert_eq!(
            title_from_file_name("12_My_Song"),
            ("My Song".into(), Some(12))
        );
        assert_eq!(title_from_file_name("07 Intro"), ("Intro".into(), Some(7)));
        assert_eq!(
            title_from_file_name("99 Luftballons"),
            ("99 Luftballons".into(), None)
        );
        assert_eq!(
            title_from_file_name("2Pac - Changes"),
            ("2Pac - Changes".into(), None)
        );
        assert_eq!(title_from_file_name("1999"), ("1999".into(), None));
        assert_eq!(title_from_file_name("01"), ("01".into(), None));
    }

    #[test]
    fn folder_is_album_and_artist_stays_empty() {
        let mut info = TagInfo::default();
        apply_fallbacks(&mut info, "Some Folder/03 - Track.flac");
        assert_eq!(info.title, "Track");
        assert_eq!(info.track_no, Some(3));
        assert_eq!(info.album, "Some Folder");
        assert_eq!(info.artist, "");
        assert_eq!(info.album_artist, "");

        let mut info = TagInfo {
            artist: "Band".into(),
            ..TagInfo::default()
        };
        apply_fallbacks(&mut info, "loose.mp3");
        assert_eq!(info.title, "loose");
        assert_eq!(info.album, "");
        assert_eq!(info.album_artist, "Band");

        let mut info = TagInfo::default();
        apply_fallbacks(&mut info, "Nordlicht/Polarnacht/01 - Aurora.mp3");
        assert_eq!(info.artist, "Nordlicht");
        assert_eq!(info.album_artist, "Nordlicht");
        assert_eq!(info.album, "Polarnacht");
    }
}
