//! Umbrel-Paket ohne Registry und ohne Login (wie nrzMedia): `cargo xtask umbrel-release 1.1.0`
//!
//! Baut das Programm für linux/amd64 und linux/arm64 (Stufe `dist` des Dockerfiles, mit
//! eingebetteter Oberfläche und Android-App aus static/), holt ein statisches ffmpeg/ffprobe
//! (gepinntes Image mwader/static-ffmpeg) sowie yt-dlp und Deno (Versionen und Prüfsummen aus
//! docker.rs) und legt alles als Commit auf den Zweig `umbrel` mit dem Tag `umbrel-<version>`:
//! `nrzmusic-<arch>.gz  ffmpeg-<arch>.gz  ffprobe-<arch>.gz  yt-dlp-<arch>.gz  deno-<arch>.gz
//! SHA256SUMS`. Das `pre-start`-Skript der App `nrz-music` im nrzStore lädt diese Dateien auf
//! dem Gerät von http://localhost:8390/nicorenezweers/nrzMusic/raw/tag/umbrel-<version>/…
//! (nrzGit) und startet sie in einem gepinnten debian:trixie-slim.
//!
//! Vorher: `cargo xtask android` (die APK liegt dann in static/). Gepusht wird auf den Remote
//! aus [`crate::remote`] ($REMOTE, sonst nrz bzw. origin); Docker muss laufen. Die Oberfläche
//! baut der Docker-Build selbst.

use crate::docker::{self, DENO_SHA256_AMD64, DENO_SHA256_ARM64, DENO_VERSION};
use crate::docker::{YTDLP_SHA256_AMD64, YTDLP_SHA256_ARM64, YTDLP_VERSION};
use crate::{git, output, remote, root, run, succeeds, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const FFMPEG_IMAGE: &str =
    "mwader/static-ffmpeg:7.1@sha256:a8090df5f5608daef387e1b2e93b98aaacb4d92153ad904e7d715c725724fca4";
const ARCHES: [&str; 2] = ["amd64", "arm64"];
const TOOLS: [&str; 5] = ["nrzmusic", "ffmpeg", "ffprobe", "yt-dlp", "deno"];

pub fn release(version: &str) -> Result {
    let root = root();
    let remote = remote();
    let tag = format!("umbrel-{version}");
    let out = root.join("dist/umbrel").join(version);
    let builder = env::var("BUILDER").unwrap_or_else(|_| "nrzmusic-builder".into());

    if succeeds(git().args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])) {
        return Err(format!("Tag {tag} gibt es schon – neue Version wählen.").into());
    }
    let apk = root.join("static/nrzMusic.apk");
    if !apk.is_file() {
        return Err("static/nrzMusic.apk fehlt – erst cargo xtask android ausführen.".into());
    }

    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(out.join("bin"))?;
    fs::create_dir_all(out.join("dl"))?;

    println!("==> Programm (linux/amd64, linux/arm64)");
    if !succeeds(Command::new("docker").args(["buildx", "inspect", &builder])) {
        run(Command::new("docker")
            .args([
                "buildx",
                "create",
                "--name",
                &builder,
                "--driver",
                "docker-container",
            ])
            .stdout(std::process::Stdio::null()))?;
    }
    let mut build = docker::command(&["buildx", "build"]);
    build
        .args([
            "--builder",
            &builder,
            "--platform",
            "linux/amd64,linux/arm64",
            "--target",
            "dist",
        ])
        .arg("--output")
        .arg(format!("type=local,dest={}", out.join("bin").display()));
    docker::run(build)?;

    println!("==> ffmpeg, yt-dlp {YTDLP_VERSION}, Deno {DENO_VERSION}");
    for arch in ARCHES {
        let (ytdlp, ytdlp_sha, deno, deno_sha) = match arch {
            "amd64" => (
                "yt-dlp_linux",
                YTDLP_SHA256_AMD64,
                "x86_64",
                DENO_SHA256_AMD64,
            ),
            _ => (
                "yt-dlp_linux_aarch64",
                YTDLP_SHA256_ARM64,
                "aarch64",
                DENO_SHA256_ARM64,
            ),
        };
        let bin = out.join(format!("bin/linux_{arch}"));
        let platform = format!("linux/{arch}");

        run(Command::new("docker")
            .args(["pull", "-q", "--platform", &platform, FFMPEG_IMAGE])
            .stdout(std::process::Stdio::null()))?;
        let container =
            output(Command::new("docker").args(["create", "--platform", &platform, FFMPEG_IMAGE]))?;
        let copied = ["ffmpeg", "ffprobe"].iter().try_for_each(|tool| {
            run(Command::new("docker")
                .arg("cp")
                .arg(format!("{container}:/{tool}"))
                .arg(bin.join(tool)))
        });
        let _ = Command::new("docker").args(["rm", &container]).output();
        copied?;

        let ytdlp_file = out.join(format!("dl/yt-dlp-{arch}"));
        let deno_zip = out.join(format!("dl/deno-{arch}.zip"));
        download(
            &format!("https://github.com/yt-dlp/yt-dlp/releases/download/{YTDLP_VERSION}/{ytdlp}"),
            &ytdlp_file,
            ytdlp_sha,
        )?;
        download(
            &format!(
                "https://github.com/denoland/deno/releases/download/v{DENO_VERSION}/deno-{deno}-unknown-linux-gnu.zip"
            ),
            &deno_zip,
            deno_sha,
        )?;
        let deno_dir = out.join(format!("dl/deno-{arch}"));
        run(Command::new("unzip")
            .arg("-q")
            .arg("-o")
            .arg(&deno_zip)
            .arg("deno")
            .arg("-d")
            .arg(&deno_dir))?;

        for (tool, source) in [
            ("nrzmusic", bin.join("nrzmusic")),
            ("ffmpeg", bin.join("ffmpeg")),
            ("ffprobe", bin.join("ffprobe")),
            ("yt-dlp", ytdlp_file),
            ("deno", deno_dir.join("deno")),
        ] {
            gzip(&source, &out.join(format!("{tool}-{arch}.gz")))?;
        }
    }

    let mut files: Vec<String> = TOOLS
        .iter()
        .flat_map(|tool| ARCHES.map(|arch| format!("{tool}-{arch}.gz")))
        .collect();
    fs::copy(root.join("Rocket.toml"), out.join("Rocket.toml"))?;
    files.push("Rocket.toml".into());
    let mut sums = String::new();
    for file in &files {
        sums += &format!("{}  {file}\n", sha256_file(&out.join(file))?);
    }
    fs::write(out.join("SHA256SUMS"), &sums)?;
    files.push("SHA256SUMS".into());

    println!("==> Zweig umbrel, Tag {tag}");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let worktree = Worktree(env::temp_dir().join(format!("nrzmusic-umbrel-{stamp}")));
    let _ = git().args(["fetch", "-q", &remote, "umbrel"]).output();
    let tracking = format!("refs/remotes/{remote}/umbrel");
    if succeeds(git().args(["rev-parse", "-q", "--verify", &tracking])) {
        run(git()
            .args(["worktree", "add", "-q", "-B", "umbrel"])
            .arg(&worktree.0)
            .arg(format!("{remote}/umbrel")))?;
    } else {
        run(git()
            .args(["worktree", "add", "-q", "--orphan", "-b", "umbrel"])
            .arg(&worktree.0))?;
    }
    for entry in fs::read_dir(&worktree.0)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".gz") || name == "SHA256SUMS" || name == "Rocket.toml" {
            fs::remove_file(entry.path())?;
        }
    }
    for file in &files {
        fs::copy(out.join(file), worktree.0.join(file))?;
    }
    fs::copy(&apk, worktree.0.join("nrzMusic.apk"))?;
    let head = output(git().args(["rev-parse", "--short", "HEAD"]))?;
    fs::write(worktree.0.join("README.md"), readme(version, &head))?;

    let name = output(git().args(["config", "user.name"])).unwrap_or_else(|_| "nrz".into());
    let email =
        output(git().args(["config", "user.email"])).unwrap_or_else(|_| "nrz@localhost".into());
    let in_tree = |args: &[&str]| -> Result {
        run(Command::new("git").arg("-C").arg(&worktree.0).args(args))
    };
    in_tree(&["add", "-A"])?;
    in_tree(&[
        "-c",
        &format!("user.name={name}"),
        "-c",
        &format!("user.email={email}"),
        "commit",
        "-q",
        "-m",
        &format!("Umbrel-Paket nrzMusic {version}"),
    ])?;
    in_tree(&[
        "tag",
        "-a",
        &tag,
        "-m",
        &format!("Umbrel-Paket nrzMusic {version}"),
    ])?;
    run(git().args(["push", "-q", &remote, "umbrel", &format!("refs/tags/{tag}")]))?;
    println!("Fertig: {tag} auf {remote} (Zweig umbrel).");
    print!("{sums}");
    Ok(())
}

/// Arbeitsverzeichnis des Zweigs `umbrel`; wird am Ende (auch bei Fehlern) wieder entfernt.
struct Worktree(PathBuf);

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = git()
            .args(["worktree", "remove", "--force"])
            .arg(&self.0)
            .output();
    }
}

/// Datei laden (curl) und ihre SHA-256 prüfen.
fn download(url: &str, dest: &Path, sha256: &str) -> Result {
    run(Command::new("curl")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(dest)
        .arg(url))?;
    let found = sha256_file(dest)?;
    if found != sha256 {
        return Err(format!(
            "Prüfsumme falsch: {} ({found}, erwartet {sha256})",
            dest.display()
        )
        .into());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// gzip -9
fn gzip(source: &Path, dest: &Path) -> Result {
    let mut input = fs::File::open(source)?;
    let mut encoder =
        flate2::write::GzEncoder::new(fs::File::create(dest)?, flate2::Compression::best());
    std::io::copy(&mut input, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

fn readme(version: &str, head: &str) -> String {
    format!(
        "# nrzMusic – Umbrel-Paket

Dieser Zweig enthält nur fertige Dateien für die Umbrel-App `nrz-music` (nrzStore), damit sie
ohne Container-Registry und ohne Login installiert werden kann. Der Quellcode liegt auf `main`.

| Datei | Inhalt |
|---|---|
| `nrzmusic-<arch>.gz` | Server mit eingebetteter Oberfläche und Android-App (glibc), amd64/arm64 |
| `ffmpeg-<arch>.gz`, `ffprobe-<arch>.gz` | statisches ffmpeg 7.1 aus `{FFMPEG_IMAGE}` (GPL) |
| `yt-dlp-<arch>.gz` | yt-dlp {YTDLP_VERSION} (eigenständig, Unlicense) |
| `deno-<arch>.gz` | Deno {DENO_VERSION} (MIT), JavaScript für yt-dlp |
| `Rocket.toml` | Standard-Konfiguration |
| `nrzMusic.apk` | Android-App (auch im Server unter /static/nrzMusic.apk) |
| `SHA256SUMS` | Prüfsummen |

Stand: nrzMusic {version}, gebaut aus `{head}` mit `cargo xtask umbrel-release {version}`.
"
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn readme_names_versions() {
        let text = super::readme("9.9.9", "abc1234");
        assert!(text.contains("nrzMusic 9.9.9, gebaut aus `abc1234`"));
        assert!(text.contains(super::YTDLP_VERSION));
    }
}
