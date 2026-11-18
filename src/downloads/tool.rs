//! Die externen Programme der Downloads: yt-dlp (als eigene Kopie in `data/tools/`, damit es
//! sich selbst aktualisieren kann), ffmpeg und eine JavaScript-Laufzeit, die yt-dlp für
//! YouTubes Abfragen braucht (Deno ist yt-dlps Standard; gibt es nur Node oder Bun, wird das
//! mit `--js-runtimes node:<pfad>` bzw. `bun:<pfad>` übergeben).
//!
//! Alle Programme laufen ohne Shell, mit Argumentliste und Zeitlimit.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Zeitlimit für `--version` (der eigenständige yt-dlp entpackt sich beim ersten Start).
const VERSION_TIMEOUT: Duration = Duration::from_secs(90);
/// Zeitlimit für `yt-dlp -U`.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// Gefundene JavaScript-Laufzeit.
#[derive(Debug, Clone, PartialEq)]
pub struct JsRuntime {
    /// "deno", "node" oder "bun"
    pub name: &'static str,
    pub path: PathBuf,
    /// z. B. "26.13.0"
    pub version: String,
}

impl JsRuntime {
    /// Für `YtDlpInfo.js_runtime`, z. B. "node 26.13.0".
    pub fn label(&self) -> String {
        format!("{} {}", self.name, self.version)
    }

    /// Wert für `--js-runtimes` – Deno ist schon Standard und braucht keinen.
    pub fn ytdlp_arg(&self) -> Option<String> {
        (self.name != "deno").then(|| format!("{}:{}", self.name, self.path.display()))
    }
}

/// Was beim Start (oder nach einer Aktualisierung) gefunden wurde.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    /// Das yt-dlp, das benutzt wird (die Kopie in `data/tools/`, sonst das eingestellte)
    pub ytdlp: Option<PathBuf>,
    pub version: Option<String>,
    pub ffmpeg: Option<PathBuf>,
    pub js: Option<JsRuntime>,
    /// Hinweis zur Einrichtung (z. B. warum keine Kopie möglich war)
    pub message: Option<String>,
}

#[cfg(windows)]
const YTDLP_FILE: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const YTDLP_FILE: &str = "yt-dlp";

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

/// Programm finden: ein Pfad (mit '/') wird geprüft, ein Name im PATH gesucht.
/// Liefert einen absoluten Pfad.
pub fn which(program: &str) -> Option<PathBuf> {
    let program = program.trim();
    if program.is_empty() {
        return None;
    }
    if program.contains('/') || program.contains('\\') {
        let path = std::path::absolute(program).ok()?;
        return is_executable(&path).then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(program);
        if is_executable(&candidate) {
            return Some(std::path::absolute(&candidate).unwrap_or(candidate));
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{program}.exe"));
            if is_executable(&exe) {
                return Some(exe);
            }
        }
    }
    None
}

/// Programm ausführen (ohne Shell) und Ausgabe sammeln; `None` bei Startfehler oder Zeitüberschreitung.
async fn run(program: &Path, args: &[&str], timeout: Duration) -> Option<std::process::Output> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => Some(out),
        Ok(Err(e)) => {
            tracing::debug!(program = %program.display(), "Start fehlgeschlagen: {e}");
            None
        }
        Err(_) => {
            tracing::warn!(program = %program.display(), "Keine Antwort (Zeitlimit)");
            None
        }
    }
}

/// `yt-dlp --version` → "2026.10.26" (oder `None`, wenn es nicht läuft).
pub async fn ytdlp_version(program: &Path) -> Option<String> {
    let out = run(program, &["--version"], VERSION_TIMEOUT).await?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.lines().next()?.trim().to_string();
    v.starts_with(|c: char| c.is_ascii_digit()).then_some(v)
}

/// Versionen wie "2026.10.26" oder "2026.10.26.232804" vergleichen (Teile als Zahlen).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| -> Vec<u64> {
        s.trim()
            .split(['.', '-', '+'])
            .map(|p| {
                p.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (mut a, mut b) = (parts(a), parts(b));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a.cmp(&b)
}

/// Eingestelltes yt-dlp nach `tools_dir/yt-dlp` kopieren (über eine temporäre Datei).
async fn copy_ytdlp(source: &Path, target: &Path) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!(
        ".yt-dlp-{}.tmp",
        hex::encode(rand::random::<[u8; 6]>())
    ));
    let result = async {
        tokio::fs::copy(source, &tmp).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
        }
        tokio::fs::rename(&tmp, target).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// yt-dlp einrichten: die Kopie in `tools_dir` benutzen und sie ersetzen, wenn sie fehlt,
/// nicht läuft oder älter ist als das eingestellte Programm.
async fn setup_ytdlp(
    configured: &str,
    tools_dir: &Path,
) -> (Option<PathBuf>, Option<String>, Option<String>) {
    let copy = tools_dir.join(YTDLP_FILE);
    let source = which(configured);
    let source_version = match &source {
        Some(p) => ytdlp_version(p).await,
        None => None,
    };
    let same_file = source.as_ref().is_some_and(|s| {
        std::fs::canonicalize(s).ok() == std::fs::canonicalize(&copy).ok() && copy.exists()
    });
    let copy_version = if copy.is_file() && !same_file {
        ytdlp_version(&copy).await
    } else {
        None
    };
    if same_file {
        return (source, source_version, None);
    }
    match (source.as_ref().zip(source_version.as_ref()), copy_version) {
        (Some((_, sv)), Some(cv)) if compare_versions(&cv, sv) != Ordering::Less => {
            (Some(copy), Some(cv), None)
        }
        (Some((src, sv)), _) => match copy_ytdlp(src, &copy).await {
            Ok(()) => match ytdlp_version(&copy).await {
                Some(v) => {
                    tracing::info!(version = %v, "yt-dlp nach {} kopiert", copy.display());
                    (Some(copy), Some(v), None)
                }
                None => {
                    let _ = tokio::fs::remove_file(&copy).await;
                    (
                        Some(src.clone()),
                        Some(sv.clone()),
                        Some("Die Kopie von yt-dlp läuft nicht – das eingestellte Programm wird benutzt.".into()),
                    )
                }
            },
            Err(e) => {
                tracing::warn!("yt-dlp nicht kopierbar nach {}: {e}", copy.display());
                (
                    Some(src.clone()),
                    Some(sv.clone()),
                    Some(format!(
                        "yt-dlp liess sich nicht nach {} kopieren.",
                        copy.display()
                    )),
                )
            }
        },
        (None, Some(cv)) => (Some(copy), Some(cv), None),
        (None, None) => (
            None,
            None,
            Some(format!(
                "yt-dlp nicht gefunden („{configured}“) – installieren oder NRZMUSIC_YTDLP setzen."
            )),
        ),
    }
}

/// ffmpeg finden und prüfen.
async fn find_ffmpeg(configured: &str) -> Option<PathBuf> {
    let path = which(configured)?;
    let out = run(&path, &["-version"], VERSION_TIMEOUT).await?;
    out.status.success().then_some(path)
}

/// JavaScript-Laufzeit suchen: Deno, sonst Node, sonst Bun.
async fn find_js_runtime() -> Option<JsRuntime> {
    for name in ["deno", "node", "bun"] {
        let Some(path) = which(name) else {
            continue;
        };
        let Some(out) = run(&path, &["--version"], VERSION_TIMEOUT).await else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().next().unwrap_or("").trim();
        // "deno 2.5.4 (stable, …)", "v26.13.0", "1.2.3"
        let version = first
            .split_whitespace()
            .find(|w| {
                w.trim_start_matches('v')
                    .starts_with(|c: char| c.is_ascii_digit())
            })
            .unwrap_or(first)
            .trim_start_matches('v')
            .to_string();
        return Some(JsRuntime {
            name,
            path,
            version,
        });
    }
    None
}

/// Alles einrichten bzw. neu erkennen.
pub async fn setup(configured_ytdlp: &str, configured_ffmpeg: &str, tools_dir: &Path) -> Tools {
    let (ytdlp, version, message) = setup_ytdlp(configured_ytdlp, tools_dir).await;
    let ffmpeg = find_ffmpeg(configured_ffmpeg).await;
    let js = find_js_runtime().await;
    Tools {
        ytdlp,
        version,
        ffmpeg,
        js,
        message,
    }
}

/// Lesbare letzte Zeile einer yt-dlp-Ausgabe ("ERROR: " entfernt).
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
}

/// `yt-dlp -U`. Liefert die Meldung von yt-dlp; ein abgelehntes Update (Homebrew, pip) ist
/// kein Fehler – die Meldung sagt, warum.
pub async fn self_update(program: &Path) -> String {
    match run(program, &["-U"], UPDATE_TIMEOUT).await {
        Some(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Fehlermeldungen (stderr) haben Vorrang vor der letzten Statuszeile
            let message = if out.status.success() {
                last_line(&stdout).or_else(|| last_line(&stderr))
            } else {
                last_line(&stderr).or_else(|| last_line(&stdout))
            };
            message.unwrap_or_else(|| format!("yt-dlp -U beendet mit {}", out.status))
        }
        None => "yt-dlp -U liess sich nicht ausführen.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            compare_versions("2026.10.26", "2026.10.26"),
            Ordering::Equal
        );
        assert_eq!(
            compare_versions("2026.10.26", "2026.9.30"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("2025.12.01", "2026.01.02"), Ordering::Less);
        assert_eq!(
            compare_versions("2026.10.26.232804", "2026.10.26"),
            Ordering::Greater
        );
    }

    #[test]
    fn js_arg() {
        let node = JsRuntime {
            name: "node",
            path: PathBuf::from("/usr/bin/node"),
            version: "26.13.0".into(),
        };
        assert_eq!(node.label(), "node 26.13.0");
        assert_eq!(node.ytdlp_arg().as_deref(), Some("node:/usr/bin/node"));
        let deno = JsRuntime {
            name: "deno",
            ..node
        };
        assert_eq!(deno.ytdlp_arg(), None);
    }

    #[test]
    fn update_messages() {
        assert_eq!(
            last_line(
                "Latest version: stable@2026.10.26\nyt-dlp is up to date (stable@2026.10.26)\n"
            )
            .as_deref(),
            Some("yt-dlp is up to date (stable@2026.10.26)")
        );
        assert_eq!(
            last_line("ERROR: You installed yt-dlp with pip; Use that to update\n").as_deref(),
            Some("You installed yt-dlp with pip; Use that to update")
        );
    }
}
