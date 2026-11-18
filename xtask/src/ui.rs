//! Oberfläche (Dioxus 0.7, `ui/`): Eingabe für Tailwind erzeugen, bauen, entwickeln.
//! Braucht: rustup mit Ziel wasm32-unknown-unknown und die Dioxus-CLI (`dx`, Version 0.7).

use crate::{dir_size, human_size, root, run, theme, Result};
use std::path::PathBuf;
use std::process::Command;
use std::{env, fs};

/// `ui/tailwind.css` aus `ui/src/theme.rs` schreiben (nur wenn sich etwas ändert – sonst
/// bauen `dx serve` und Tailwind unnötig neu). dx findet die Datei und lässt Tailwind laufen.
pub fn css() -> Result {
    let path = root().join("ui/tailwind.css");
    let css = theme::tailwind_input();
    if fs::read_to_string(&path).ok().as_deref() != Some(css.as_str()) {
        fs::write(&path, css)?;
    }
    Ok(())
}

/// Oberfläche bauen und nach `ui/dist/public` legen – build.rs bettet sie ins Programm ein.
/// Vor jedem Release-Build ausführen (der Docker-Build macht das selbst).
pub fn bundle() -> Result {
    css()?;
    let ui = root().join("ui");
    // `dx bundle` räumt seinen Zwischenordner (target/dx/…/public) nicht auf und kopiert alles
    // daraus mit: vorher leeren und in einen frischen Ordner bauen, sonst landen alte
    // WASM-Dateien mit im Programm.
    let _ = fs::remove_dir_all(ui.join("target/dx/nrzmusic-ui/release/web/public"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let out = ui.join(format!("target/ui-bundle-{stamp}"));
    run(dioxus(&["bundle", "--web", "--release", "--out-dir"])?.arg(&out))?;

    let dist = ui.join("dist/public");
    if !out.join("public/index.html").is_file() {
        return Err(format!("dx bundle hat keine index.html in {} gelegt", out.display()).into());
    }
    let _ = fs::remove_dir_all(&dist);
    fs::create_dir_all(ui.join("dist"))?;
    fs::rename(out.join("public"), &dist)?;
    let _ = fs::remove_dir_all(&out);
    println!(
        "Oberfläche gebaut: ui/dist/public ({})",
        human_size(dir_size(&dist))
    );
    Ok(())
}

/// `dx serve` mit frisch erzeugtem CSS. API, Audio, Cover und Schriften gehen an den laufenden
/// Server (Proxy in ui/Dioxus.toml), also vorher `cargo run`.
pub fn serve(args: &[String]) -> Result {
    css()?;
    run(dioxus(&["serve"])?.args(args))
}

/// Die Dioxus-CLI im Ordner `ui/`, mit derselben Rust-Toolchain wie dieser Aufruf.
fn dioxus(args: &[&str]) -> Result<Command> {
    let dx = find_dx()?;
    let mut cmd = Command::new(dx);
    cmd.current_dir(root().join("ui")).args(args);
    // dx ruft cargo und rustc über den PATH auf: die Toolchain von `cargo xtask` zuerst (hat
    // das Ziel wasm32), nicht etwa ein Rust aus Homebrew.
    let cargo = env::var_os("CARGO").map_or_else(|| PathBuf::from(env!("CARGO")), PathBuf::from);
    let mut paths: Vec<PathBuf> = cargo.parent().map(PathBuf::from).into_iter().collect();
    paths.push(cargo_home().join("bin"));
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    cmd.env("PATH", env::join_paths(paths)?);
    Ok(cmd)
}

fn cargo_home() -> PathBuf {
    env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".cargo"))
}

/// `dx` der Dioxus-CLI: $DX, ~/.cargo/bin/dx oder aus dem PATH. Deno bringt einen eigenen
/// `dx` mit – darum prüfen, wer antwortet.
fn find_dx() -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = env::var_os("DX").map(PathBuf::from).into_iter().collect();
    candidates.push(cargo_home().join("bin/dx"));
    candidates.push(PathBuf::from("dx"));
    for dx in candidates {
        let Ok(out) = Command::new(&dx).arg("--version").output() else {
            continue;
        };
        let version = String::from_utf8_lossy(&out.stdout);
        if out.status.success() && version.starts_with("dioxus") {
            return Ok(dx);
        }
    }
    Err("Dioxus-CLI (dx) fehlt: cargo install dioxus-cli --version 0.7.10 --locked".into())
}
