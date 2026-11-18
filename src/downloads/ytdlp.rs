//! Aufruf von yt-dlp für einen Titel: Argumentliste (keine Shell), Fortschritt aus der
//! eigenen `--progress-template`-Zeile und eine lesbare Fehlermeldung aus stderr.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Präfix der Fortschrittszeilen beim Herunterladen.
const PROGRESS_PREFIX: &str = "[nrz]";
/// Präfix der Fortschrittszeilen der Nachbearbeitung (ffmpeg).
const POSTPROCESS_PREFIX: &str = "[nrz-pp]";

/// Anteil des Herunterladens am Gesamtfortschritt; der Rest ist Umwandeln, Taggen, Ablegen.
pub const DOWNLOAD_SHARE: f32 = 0.9;

/// Nur echte YouTube-Video-IDs: genau 11 Zeichen aus `[A-Za-z0-9_-]`.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Adresse, die yt-dlp bekommt.
pub fn watch_url(video_id: &str) -> String {
    format!("https://music.youtube.com/watch?v={video_id}")
}

/// Was für einen Aufruf nötig ist.
pub struct Invocation<'a> {
    pub video_id: &'a str,
    /// "mp3", "m4a" oder "opus"
    pub format: &'a str,
    pub ffmpeg: Option<&'a Path>,
    /// Wert für `--js-runtimes` (Node/Bun), `None` = yt-dlps Standard (Deno)
    pub js_runtime: Option<String>,
    pub cookies: Option<PathBuf>,
    pub cache_dir: &'a Path,
    /// Arbeitsordner des Auftrags (Ausgabe landet hier)
    pub out_dir: &'a Path,
}

/// Formatauswahl: für M4A und Opus die passende Tonspur nehmen (ohne Umwandlung),
/// sonst die beste.
fn format_selector(format: &str) -> &'static str {
    match format {
        "m4a" => "bestaudio[ext=m4a]/bestaudio/best",
        "opus" => "bestaudio[acodec=opus]/bestaudio/best",
        _ => "bestaudio/best",
    }
}

/// Argumente für yt-dlp (ohne Programmnamen).
pub fn args(inv: &Invocation<'_>) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let mut push = |s: &str| a.push(OsString::from(s));
    for s in [
        "--ignore-config",
        "--no-playlist",
        "--newline",
        "--no-colors",
        "--no-mtime",
        "--progress-template",
        "download:[nrz] %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
        "--progress-template",
        "postprocess:[nrz-pp] %(progress.status)s %(progress.postprocessor)s",
        "-f",
        format_selector(inv.format),
        "-x",
        "--audio-format",
        inv.format,
    ] {
        push(s);
    }
    if inv.format == "mp3" {
        // VBR V0
        push("--audio-quality");
        push("0");
    }
    if let Some(js) = &inv.js_runtime {
        push("--js-runtimes");
        push(js);
    }
    if let Some(ffmpeg) = inv.ffmpeg {
        a.push("--ffmpeg-location".into());
        a.push(ffmpeg.as_os_str().to_owned());
    }
    if let Some(cookies) = &inv.cookies {
        a.push("--cookies".into());
        a.push(cookies.as_os_str().to_owned());
    }
    a.push("--cache-dir".into());
    a.push(inv.cache_dir.as_os_str().to_owned());
    a.push("-P".into());
    a.push(inv.out_dir.as_os_str().to_owned());
    a.push("-o".into());
    a.push("%(id)s.%(ext)s".into());
    a.push("--".into());
    a.push(watch_url(inv.video_id).into());
    a
}

/// Ausgewertete Fortschrittszeile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// Herunterladen, Anteil 0.0 – 1.0
    Download(f32),
    /// Nachbearbeitung hat begonnen bzw. ist fertig
    Postprocess { finished: bool },
}

fn number(s: Option<&str>) -> Option<f64> {
    s.and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// Eine Zeile aus stdout auswerten.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix(POSTPROCESS_PREFIX) {
        let status = rest.split_whitespace().next().unwrap_or("");
        return Some(Progress::Postprocess {
            finished: status == "finished",
        });
    }
    let rest = line.strip_prefix(PROGRESS_PREFIX)?;
    let mut parts = rest.split_whitespace();
    let downloaded = number(parts.next()).unwrap_or(0.0);
    let total = number(parts.next()).or_else(|| number(parts.next()))?;
    Some(Progress::Download(
        (downloaded / total).clamp(0.0, 1.0) as f32
    ))
}

/// Gesamtfortschritt (0.0 – 1.0) nach einer Zeile.
pub fn overall(current: f32, p: Progress) -> f32 {
    let next = match p {
        Progress::Download(f) => f * DOWNLOAD_SHARE,
        Progress::Postprocess { finished: false } => DOWNLOAD_SHARE,
        Progress::Postprocess { finished: true } => DOWNLOAD_SHARE + 0.03,
    };
    next.max(current).min(0.99)
}

/// Fehlermeldung von yt-dlp lesbar machen: "ERROR:" und "[youtube] <id>:" weg, Hinweise auf
/// Optionen und Links abschneiden, höchstens 300 Zeichen.
pub fn clean_error(line: &str) -> String {
    let mut s = line.trim();
    s = s.strip_prefix("ERROR:").unwrap_or(s).trim();
    // "[youtube] dQw4w9WgXcQ: …"
    if s.starts_with('[') {
        if let Some(end) = s.find("] ") {
            s = s[end + 2..].trim();
            if let Some((id, rest)) = s.split_once(": ") {
                if !id.contains(' ') {
                    s = rest.trim();
                }
            }
        }
    }
    let mut out = s.to_string();
    for cut in [
        " Use --cookies",
        " See  http",
        " See http",
        "; please report this issue",
        " Also see ",
    ] {
        if let Some(i) = out.find(cut) {
            out.truncate(i);
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.trim_end_matches([',', ';', ' ']).to_string();
    if out.chars().count() > 300 {
        let mut t: String = out.chars().take(299).collect();
        t.push('…');
        t
    } else if out.is_empty() {
        line.trim().to_string()
    } else {
        out
    }
}

/// Merkt sich aus stderr die letzte Fehlerzeile (sonst die letzte Zeile überhaupt).
#[derive(Debug, Default)]
pub struct ErrorCapture {
    last_error: Option<String>,
    last_line: Option<String>,
}

impl ErrorCapture {
    pub fn push(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if line.starts_with("ERROR:") {
            self.last_error = Some(line.to_string());
        } else if !line.starts_with("WARNING:") && !line.starts_with('[') {
            self.last_line = Some(line.to_string());
        }
    }

    pub fn message(&self) -> Option<String> {
        self.last_error
            .as_deref()
            .or(self.last_line.as_deref())
            .map(clean_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids() {
        assert!(valid_video_id("dQw4w9WgXcQ"));
        assert!(valid_video_id("a-b_c-d_e-f"));
        assert!(!valid_video_id("dQw4w9WgXc"));
        assert!(!valid_video_id("dQw4w9WgXcQQ"));
        assert!(!valid_video_id("../etc/pwd1"));
        assert!(!valid_video_id("--exec=rm 1"));
    }

    #[test]
    fn progress_lines() {
        assert_eq!(
            parse_progress("[nrz] 500 1000 NA"),
            Some(Progress::Download(0.5))
        );
        assert_eq!(
            parse_progress("[nrz] 250 NA 1000.0"),
            Some(Progress::Download(0.25))
        );
        assert_eq!(parse_progress("[nrz] 250 NA NA"), None);
        assert_eq!(
            parse_progress("[nrz-pp] started ExtractAudio"),
            Some(Progress::Postprocess { finished: false })
        );
        assert_eq!(parse_progress("[download] Destination: x.webm"), None);
        assert!((overall(0.0, Progress::Download(1.0)) - 0.9).abs() < 1e-6);
        assert_eq!(overall(0.5, Progress::Download(0.1)), 0.5);
    }

    #[test]
    fn errors_are_readable() {
        let bot = "ERROR: [youtube] dQw4w9WgXcQ: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies.";
        assert_eq!(clean_error(bot), "Sign in to confirm you’re not a bot.");
        assert_eq!(
            clean_error(
                "ERROR: [youtube] abcdefghijk: Video unavailable. This video is not available"
            ),
            "Video unavailable. This video is not available"
        );
        assert_eq!(
            clean_error("ERROR: Postprocessing: ffprobe and ffmpeg not found. Please install or provide the path using --ffmpeg-location"),
            "Postprocessing: ffprobe and ffmpeg not found. Please install or provide the path using --ffmpeg-location"
        );
        let mut c = ErrorCapture::default();
        c.push("WARNING: something");
        c.push("ERROR: [youtube] abcdefghijk: Private video");
        c.push("");
        assert_eq!(c.message().as_deref(), Some("Private video"));
    }

    #[test]
    fn arguments() {
        let dir = PathBuf::from("/data/downloads/job-1");
        let inv = Invocation {
            video_id: "dQw4w9WgXcQ",
            format: "mp3",
            ffmpeg: Some(Path::new("/usr/bin/ffmpeg")),
            js_runtime: Some("node:/usr/bin/node".into()),
            cookies: None,
            cache_dir: Path::new("/data/tools/cache"),
            out_dir: &dir,
        };
        let a: Vec<String> = args(&inv)
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        let has = |pair: [&str; 2]| a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has(["--audio-format", "mp3"]));
        assert!(has(["--audio-quality", "0"]));
        assert!(has(["--js-runtimes", "node:/usr/bin/node"]));
        assert!(has(["--ffmpeg-location", "/usr/bin/ffmpeg"]));
        assert!(has(["-P", "/data/downloads/job-1"]));
        assert_eq!(
            a.last().unwrap(),
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert!(a.contains(&"--no-playlist".to_string()));
        assert!(!a.contains(&"--cookies".to_string()));
    }
}
