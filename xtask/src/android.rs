//! Android-App: Kotlin/Gradle, liegt auf dem Zweig `android` (main bleibt reines Rust).
//! Gebaut wird in `android/` – einem Arbeitsverzeichnis (`git worktree`) dieses Zweigs, das main
//! ignoriert und das der erste Aufruf anlegt – oder in $NRZMUSIC_ANDROID_DIR. Gradle liest die
//! Version aus `../Cargo.toml`, darum liegt das Arbeitsverzeichnis direkt im Repo.
//!
//! Braucht: JDK 17 und das Android-SDK (Plattform 35, Build-Tools); Signierschlüssel über
//! NRZMUSIC_KEYSTORE / NRZMUSIC_KEYSTORE_PASSWORD in ~/.gradle/gradle.properties (nie im Repo).
//! Die APK landet in `static/nrzMusic.apk`; der Server bettet sie ein und bietet sie unter
//! `/static/nrzMusic.apk` zum Herunterladen an (Anmeldeseite, Konto).

use crate::{git, human_size, remote, root, run, succeeds, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const BRANCH: &str = "android";

pub fn build() -> Result {
    let root = root();
    let dir = match env::var_os("NRZMUSIC_ANDROID_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = root.join("android");
            if !dir.join("settings.gradle.kts").is_file() {
                worktree(&dir)?;
            }
            dir
        }
    };

    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    let mut gradle = Command::new(dir.join(gradlew));
    gradle
        .current_dir(&dir)
        .args(["--console=plain", "-q", "assembleRelease"]);
    let jdk = Path::new("/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home");
    if env::var_os("JAVA_HOME").is_none() && jdk.is_dir() {
        gradle.env("JAVA_HOME", jdk);
    }
    let sdk = Path::new("/opt/homebrew/share/android-commandlinetools");
    if !dir.join("local.properties").exists() && sdk.is_dir() {
        fs::write(
            dir.join("local.properties"),
            format!("sdk.dir={}\n", sdk.display()),
        )?;
    }
    run(&mut gradle)?;

    let apk = dir.join("app/build/outputs/apk/release/app-release.apk");
    if !apk.is_file() {
        return Err("Keine signierte APK – fehlt NRZMUSIC_KEYSTORE?".into());
    }
    let target = root.join("static/nrzMusic.apk");
    fs::copy(&apk, &target)?;
    let size = fs::metadata(&target)?.len();
    println!(
        "Android-App gebaut: static/nrzMusic.apk ({})",
        human_size(size)
    );
    Ok(())
}

/// `android/` als Arbeitsverzeichnis des Zweigs anlegen (den Zweig vorher vom Remote holen,
/// falls es ihn lokal noch nicht gibt).
fn worktree(dir: &Path) -> Result {
    if !succeeds(git().args([
        "rev-parse",
        "--verify",
        "-q",
        &format!("refs/heads/{BRANCH}"),
    ])) {
        let remote = remote();
        run(git().args(["fetch", "-q", &remote, &format!("{BRANCH}:{BRANCH}")]))
            .map_err(|e| format!("Zweig {BRANCH} weder lokal noch auf {remote} ({e})"))?;
    }
    run(git().args(["worktree", "add", "-q"]).arg(dir).arg(BRANCH))?;
    println!(
        "android/ ist jetzt ein Arbeitsverzeichnis des Zweigs {BRANCH} – Änderungen an der App dort committen."
    );
    Ok(())
}
