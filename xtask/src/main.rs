//! Bauwerkzeug von nrzMusic: `cargo xtask <befehl>` (Alias in `.cargo/config.toml`).
//! Was früher Shell-Skripte und ein Dockerfile erledigten, steht hier in Rust.
//!
//! ```text
//! cargo xtask css                    ui/tailwind.css aus ui/src/theme.rs erzeugen
//! cargo xtask ui                     Oberfläche bauen → ui/dist/public (build.rs bettet sie ein)
//! cargo xtask serve [dx-Argumente]   Oberfläche entwickeln (dx serve, API vom laufenden Server)
//! cargo xtask android                Android-App (Zweig `android`) bauen → static/nrzMusic.apk
//! cargo xtask macos [--skip-ui]      macOS-App (macos/) bauen → dist-macos/nrzMusic.app + DMG
//! cargo xtask dockerfile             Dockerfile ausgeben, z. B. `… > Dockerfile`
//! cargo xtask docker [Argumente]     docker buildx build mit diesem Dockerfile (Kontext: Repo)
//! cargo xtask umbrel-release <ver>   Umbrel-Paket bauen und auf den Zweig `umbrel` legen
//! ```

mod android;
mod docker;
mod macos;
#[path = "../../ui/src/theme.rs"]
mod theme;
mod ui;
mod umbrel;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Aufruf: cargo xtask <befehl>

  css                     ui/tailwind.css aus ui/src/theme.rs erzeugen
  ui                      Oberfläche bauen → ui/dist/public (vor dem Release-Build)
  serve [dx-Argumente]    Oberfläche entwickeln (dx serve; API vom Server auf :8350)
  android                 Android-App (Zweig android) bauen → static/nrzMusic.apk
  macos [--skip-ui]       macOS-App bauen → dist-macos/nrzMusic.app und DMG
  dockerfile              Dockerfile ausgeben
  docker [Argumente]      docker buildx build mit diesem Dockerfile, z. B. -t nrzmusic --load
  umbrel-release <ver>    Umbrel-Paket bauen, auf den Zweig umbrel legen und pushen";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("css") => ui::css(),
        Some("ui") => ui::bundle(),
        Some("serve") => ui::serve(rest),
        Some("android") => android::build(),
        Some("macos") => macos::build(rest.iter().any(|a| a == "--skip-ui")),
        Some("dockerfile") => {
            print!("{}", docker::dockerfile());
            Ok(())
        }
        Some("docker") => docker::build(rest),
        Some("umbrel-release") => match rest {
            [version] => umbrel::release(version),
            _ => Err("Aufruf: cargo xtask umbrel-release <version>   (z. B. 1.5.1)".into()),
        },
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(USAGE.into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Wurzel des Repos (xtask/ liegt direkt darin).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask liegt im Repo")
        .to_path_buf()
}

/// Befehl ausführen (Ein- und Ausgabe wie im Terminal); Fehler, wenn er scheitert.
pub fn run(cmd: &mut Command) -> Result {
    let status = cmd
        .status()
        .map_err(|e| format!("{} lässt sich nicht starten: {e}", program(cmd)))?;
    if !status.success() {
        return Err(format!("{} ist fehlgeschlagen ({status})", program(cmd)).into());
    }
    Ok(())
}

/// Befehl ausführen und seine Ausgabe (stdout) zurückgeben.
pub fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("{} lässt sich nicht starten: {e}", program(cmd)))?;
    if !out.status.success() {
        return Err(format!("{} ist fehlgeschlagen ({})", program(cmd), out.status).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Läuft der Befehl ohne Fehler durch? (Ausgabe verworfen)
pub fn succeeds(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn program(cmd: &Command) -> String {
    let args: Vec<_> = cmd
        .get_args()
        .take(2)
        .map(|a| a.to_string_lossy())
        .collect();
    format!(
        "`{} {}`",
        cmd.get_program().to_string_lossy(),
        args.join(" ")
    )
}

/// `git` im Repo.
pub fn git() -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root());
    cmd
}

/// Remote für Zweige und Tags: $REMOTE, sonst `nrz` (nrzGit), wenn es ihn gibt, sonst `origin`.
pub fn remote() -> String {
    if let Ok(r) = std::env::var("REMOTE") {
        return r;
    }
    if succeeds(git().args(["remote", "get-url", "nrz"])) {
        "nrz".into()
    } else {
        "origin".into()
    }
}

/// Grösse lesbar, z. B. "12.3 MB".
pub fn human_size(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    unreachable!()
}

/// Gesamtgrösse eines Ordners.
pub fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn human_size() {
        assert_eq!(super::human_size(512), "512 B");
        assert_eq!(super::human_size(1536), "1.5 KB");
        assert_eq!(super::human_size(12 * 1024 * 1024), "12.0 MB");
    }
}
