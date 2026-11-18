//! Cover: eingebettetes Bild (Vorderseite, sonst das erste) oder `cover|folder|front|album`
//! `.jpg/.jpeg/.png/.webp` im selben Ordner. Jedes Bild liegt genau einmal als Original in
//! `data/covers/<id>.orig` (ID = die ersten 16 Hex-Zeichen von SHA-256 der Bilddaten);
//! verkleinerte Fassungen entstehen bei Bedarf als `data/covers/<id>-<grösse>.jpg`.

use super::short_hash;
use image::ImageFormat;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Namen von Cover-Dateien im Ordner, in dieser Reihenfolge bevorzugt.
const COVER_STEMS: &[&str] = &["cover", "folder", "front", "album"];
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Erlaubte Kantenlängen; eine Anfrage wird auf die nächstgrössere gerundet, damit der
/// Zwischenspeicher nicht beliebig wächst.
const SIZES: &[u32] = &[
    64, 96, 128, 160, 192, 256, 300, 384, 480, 512, 600, 768, 1024, 1200,
];

/// Ist das ein Dateiname wie `cover.jpg` oder `Folder.PNG`?
pub fn is_cover_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some((stem, ext)) = lower.rsplit_once('.') else {
        return false;
    };
    COVER_STEMS.contains(&stem) && COVER_EXTENSIONS.contains(&ext)
}

/// Cover-Datei in einem Ordner suchen (Gross-/Kleinschreibung egal).
pub fn folder_cover(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_ascii_lowercase();
            let (stem, ext) = name.rsplit_once('.')?;
            let rank = COVER_STEMS.iter().position(|s| *s == stem)?;
            COVER_EXTENSIONS.contains(&ext).then(|| (rank, e.path()))
        })
        .filter(|(_, p)| p.is_file())
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, p)| p)
}

/// Gültige Cover-ID? (16 Hex-Zeichen – schützt auch vor Pfadangaben)
pub fn valid_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn original_path(covers: &Path, id: &str) -> PathBuf {
    covers.join(format!("{id}.orig"))
}

fn resized_path(covers: &Path, id: &str, size: u32) -> PathBuf {
    covers.join(format!("{id}-{size}.jpg"))
}

/// Bilddaten als Cover ablegen (nur einmal je Inhalt). `None`, wenn es kein lesbares Bild ist.
pub fn store(covers: &Path, data: &[u8]) -> Option<String> {
    let format = image::guess_format(data).ok()?;
    if !matches!(
        format,
        ImageFormat::Jpeg
            | ImageFormat::Png
            | ImageFormat::WebP
            | ImageFormat::Gif
            | ImageFormat::Bmp
    ) {
        return None;
    }
    let id = short_hash(data);
    let path = original_path(covers, &id);
    if !path.exists() {
        if let Err(e) = write_atomic(&path, data) {
            tracing::warn!("Cover {id} nicht speicherbar: {e}");
            return None;
        }
    }
    Some(id)
}

/// Erst in eine temporäre Datei schreiben, dann umbenennen – nie halbe Bilder ausliefern.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", hex::encode(rand::random::<[u8; 6]>())));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Auf eine der erlaubten Kantenlängen runden.
pub fn snap_size(requested: u32) -> u32 {
    SIZES
        .iter()
        .copied()
        .find(|s| *s >= requested)
        .unwrap_or(*SIZES.last().unwrap())
}

/// Content-Type eines Originals (nach Inhalt).
pub fn content_type(data: &[u8]) -> &'static str {
    match image::guess_format(data) {
        Ok(ImageFormat::Png) => "image/png",
        Ok(ImageFormat::WebP) => "image/webp",
        Ok(ImageFormat::Gif) => "image/gif",
        Ok(ImageFormat::Bmp) => "image/bmp",
        _ => "image/jpeg",
    }
}

/// Verkleinerte Fassung (JPEG, Seitenverhältnis bleibt) liefern; wird beim ersten Mal erzeugt.
/// Blockiert – aus async-Code über `spawn_blocking` aufrufen.
pub fn resized(covers: &Path, id: &str, size: u32) -> std::io::Result<PathBuf> {
    let target = resized_path(covers, id, size);
    if target.exists() {
        return Ok(target);
    }
    let data = std::fs::read(original_path(covers, id))?;
    let img = image::load_from_memory(&data).map_err(std::io::Error::other)?;
    let img = if img.width() > size || img.height() > size {
        img.resize(size, size, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(&rgb)
        .map_err(std::io::Error::other)?;
    write_atomic(&target, &out)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sizes() {
        assert!(is_cover_file_name("Cover.JPG"));
        assert!(is_cover_file_name("folder.webp"));
        assert!(!is_cover_file_name("back.jpg"));
        assert!(!is_cover_file_name("cover.gif"));
        assert_eq!(snap_size(96), 96);
        assert_eq!(snap_size(97), 128);
        assert_eq!(snap_size(5000), 1200);
        assert!(valid_id("0123456789abcdef"));
        assert!(!valid_id("../../etc/passwd"));
        assert!(!valid_id("0123456789ABCDEF"));
    }
}
