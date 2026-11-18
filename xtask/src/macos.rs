//! macOS-App (`macos/`): Oberfläche bauen (wird eingebettet), App übersetzen, Bundle
//! `nrzMusic.app` samt Info.plist (hier erzeugt) und Symbol (aus `static/icon.png`), signieren und
//! als DMG nach `dist-macos/` legen. Werkzeuge des Systems: `sips`, `iconutil`, `codesign`, `hdiutil`.
//!
//! Signatur: `MACOS_SIGN_IDENTITY` (z. B. "Developer ID Application: …"), sonst ad hoc (`-`).

use crate::{human_size, root, run, ui, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

const BUNDLE_ID: &str = "ch.zweers.nrzmusic";

pub fn build(skip_ui: bool) -> Result {
    if !cfg!(target_os = "macos") {
        return Err("Die macOS-App lässt sich nur auf einem Mac bauen".into());
    }
    let root = root();
    let version = package_version(&root.join("Cargo.toml"))?;
    let app_version = package_version(&root.join("macos/Cargo.toml"))?;
    if version != app_version {
        return Err(format!("Version der App ({app_version}) ≠ Server ({version}) – macos/Cargo.toml anpassen").into());
    }
    if !skip_ui {
        ui::bundle()?;
    }
    run(Command::new("cargo")
        .args(["build", "--release", "--locked", "--manifest-path"])
        .arg(root.join("macos/Cargo.toml")))?;

    let dist = root.join("dist-macos");
    let app = dist.join("nrzMusic.app");
    let _ = fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    fs::copy(
        root.join("macos/target/release/nrzmusic-macos"),
        contents.join("MacOS/nrzmusic-macos"),
    )?;
    fs::write(contents.join("Info.plist"), info_plist(&version))?;
    fs::write(contents.join("PkgInfo"), "APPL????")?;
    icon(&root.join("static/icon.png"), &contents.join("Resources/nrzMusic.icns"), &dist)?;

    let identity = std::env::var("MACOS_SIGN_IDENTITY").unwrap_or_else(|_| "-".into());
    let mut sign = Command::new("codesign");
    sign.args(["--force", "--deep", "--sign", &identity]);
    if identity != "-" {
        sign.args(["--options", "runtime", "--timestamp"]);
    }
    run(sign.arg(&app))?;
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(&app))?;

    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    let dmg = dist.join(format!("nrzMusic-{version}-{arch}.dmg"));
    let _ = fs::remove_file(&dmg);
    // Im DMG: die App und eine Verknüpfung zu /Programme zum Hineinziehen
    let stage = dist.join("dmg");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage)?;
    run(Command::new("cp").arg("-R").arg(&app).arg(&stage))?;
    std::os::unix::fs::symlink("/Applications", stage.join("Applications"))?;
    run(Command::new("hdiutil")
        .args(["create", "-quiet", "-volname", "nrzMusic", "-fs", "HFS+", "-format", "UDZO", "-srcfolder"])
        .arg(&stage)
        .arg(&dmg))?;
    let _ = fs::remove_dir_all(&stage);
    println!(
        "{} ({})\n{} ({})",
        app.display(),
        human_size(crate::dir_size(&app)),
        dmg.display(),
        human_size(fs::metadata(&dmg)?.len())
    );
    Ok(())
}

/// `version = "…"` aus dem Abschnitt `[package]`.
fn package_version(manifest: &Path) -> Result<String> {
    let text = fs::read_to_string(manifest)?;
    text.lines()
        .skip_while(|l| l.trim() != "[package]")
        .find_map(|l| l.trim().strip_prefix("version = \"")?.strip_suffix('"').map(str::to_string))
        .ok_or_else(|| format!("keine Version in {}", manifest.display()).into())
}

/// `static/icon.png` (512 px) → `.icns` mit allen Grössen.
fn icon(png: &Path, icns: &Path, dist: &Path) -> Result {
    let set = dist.join("nrzMusic.iconset");
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(&set)?;
    for (size, name) in [
        (16, "16x16"),
        (32, "16x16@2x"),
        (32, "32x32"),
        (64, "32x32@2x"),
        (128, "128x128"),
        (256, "128x128@2x"),
        (256, "256x256"),
        (512, "256x256@2x"),
        (512, "512x512"),
    ] {
        let out = set.join(format!("icon_{name}.png"));
        let s = size.to_string();
        let mut cmd = Command::new("sips");
        cmd.args(["-s", "format", "png", "-z", &s, &s]).arg(png).arg("--out").arg(&out);
        cmd.stdout(std::process::Stdio::null());
        run(&mut cmd)?;
    }
    run(Command::new("iconutil").args(["-c", "icns"]).arg(&set).arg("-o").arg(icns))?;
    let _ = fs::remove_dir_all(&set);
    Ok(())
}

pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>nrzMusic</string>
  <key>CFBundleDisplayName</key><string>nrzMusic</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>nrzmusic-macos</string>
  <key>CFBundleIconFile</key><string>nrzMusic</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleDevelopmentRegion</key><string>de</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSRequiresAquaSystemAppearance</key><false/>
  <key>NSAppTransportSecurity</key>
  <dict>
    <key>NSAllowsLocalNetworking</key><true/>
    <key>NSAllowsArbitraryLoads</key><true/>
    <key>NSAllowsArbitraryLoadsInWebContent</key><true/>
  </dict>
  <key>NSLocalNetworkUsageDescription</key><string>nrzMusic verbindet sich mit deinem nrzMusic-Server im Netzwerk.</string>
  <key>NSHumanReadableCopyright</key><string>AGPL-3.0-or-later</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn plist_has_version_and_executable() {
        let p = super::info_plist("1.2.3");
        assert!(p.contains("<key>CFBundleShortVersionString</key><string>1.2.3</string>"));
        assert!(p.contains("<string>nrzmusic-macos</string>"));
    }
}
