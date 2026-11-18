//! Docker: Das Dockerfile entsteht hier (früher eine eigene Datei im Repo).
//! `cargo xtask dockerfile` gibt es aus, `cargo xtask docker …` baut direkt damit
//! (`docker buildx build -f - … .` – Kontext ist das Repo, `.dockerignore` gilt).

use crate::{root, theme, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Basis-Images, gepinnt.
const DEBIAN: &str =
    "debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8";
const RUST: &str =
    "rust:1.98-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e";
/// Dioxus-CLI für `dx bundle` (passend zu dioxus 0.7 in ui/Cargo.toml).
const DIOXUS_CLI: &str = "0.7.10";

// yt-dlp als eigenständiges Programm und Deno (JavaScript für yt-dlp). Neue Version: Version und
// beide Prüfsummen zusammen ändern – yt-dlp: SHA2-256SUMS des Releases, Deno:
// deno-<arch>-unknown-linux-gnu.zip.sha256sum des Releases. Gilt für Image und Umbrel-Paket.
pub const YTDLP_VERSION: &str = "2026.10.26";
pub const YTDLP_SHA256_AMD64: &str =
    "767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795";
pub const YTDLP_SHA256_ARM64: &str =
    "e87e9533e89fe5889abaa21f5114dfe7756d6aa6219b83551add04d48b18e483";
pub const DENO_VERSION: &str = "2.9.8";
pub const DENO_SHA256_AMD64: &str =
    "e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609";
pub const DENO_SHA256_ARM64: &str =
    "d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905";

/// Das Dockerfile. Platzhalter `{{NAME}}` füllt [`dockerfile`].
const TEMPLATE: &str = r##"# syntax=docker/dockerfile:1.10
# nrzMusic – Multi-Stage-Build für amd64 und arm64: Oberfläche (Dioxus/WebAssembly) und Programm
# (Rust), dazu yt-dlp, Deno und ffmpeg für die Downloads.
# Erzeugt von `cargo xtask dockerfile` (xtask/src/docker.rs) – dort ändern, nicht hier.
#   cargo xtask docker -t nrzmusic --load
#   docker run -p 8350:8350 -v nrzmusic-data:/data -v /pfad/zur/musik:/music nrzmusic
# Beide Architekturen (Rust wird quer-kompiliert, nicht emuliert):
#   cargo xtask docker --platform linux/amd64,linux/arm64 -t <registry>/nrz-music:<version> --push
# Nur das Programm (für die Release-Assets, landet in dist/linux_<arch>/):
#   cargo xtask docker --platform linux/amd64,linux/arm64 --target dist --output type=local,dest=dist

ARG DEBIAN={{DEBIAN}}

ARG RUST={{RUST}}

# ---------------------------------------------------------------- Oberfläche
# WebAssembly ist überall gleich – einmal auf dem Build-Rechner bauen, für alle Architekturen.
FROM --platform=$BUILDPLATFORM ${RUST} AS ui
RUN rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo install dioxus-cli --version {{DIOXUS_CLI}} --locked --root /opt/dx
WORKDIR /src
# Die Oberfläche hängt an ../types (gemeinsame Typen der JSON-API)
COPY types types
COPY ui ui
# Eingabe für Tailwind aus ui/src/theme.rs (dasselbe wie `cargo xtask css`)
COPY <<"TAILWIND" ui/tailwind.css
{{TAILWIND}}TAILWIND
# `dx bundle` räumt seinen Zwischenordner nicht auf – vorher leeren, sonst kommen alte
# WASM-Dateien aus dem Cache mit.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/ui/target,id=nrzmusic-ui-target \
    cd ui && rm -rf target/dx/nrzmusic-ui/release/web/public \
 && PATH=/opt/dx/bin:$PATH dx bundle --web --release --out-dir /tmp/ui \
 && mkdir -p /out && cp -r /tmp/ui/public /out/public && test -f /out/public/index.html

# ---------------------------------------------------------------- Programm
FROM --platform=$BUILDPLATFORM ${RUST} AS build
ARG TARGETARCH
ARG BUILDARCH
# Querübersetzer, wenn Ziel und Build-Rechner verschieden sind (SQLite und ring sind C-Code
# und werden mit dem C-Compiler des Ziels gebaut).
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) t=x86_64-unknown-linux-gnu; p=x86_64-linux-gnu; pkg="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) t=aarch64-unknown-linux-gnu; p=aarch64-linux-gnu; pkg="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "Architektur $TARGETARCH nicht unterstützt" >&2; exit 1 ;; \
    esac; \
    echo "$t" > /rust-target; \
    if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
      apt-get update && apt-get install -y --no-install-recommends $pkg && rm -rf /var/lib/apt/lists/*; \
      rustup target add "$t"; \
      printf '[target.%s]\nlinker = "%s-gcc"\n' "$t" "$p" >> /usr/local/cargo/config.toml; \
      u=$(echo "$t" | tr - _); \
      printf 'CC_%s=%s-gcc\nAR_%s=%s-ar\n' "$u" "$p" "$u" "$p" > /cross-env; \
    else \
      touch /cross-env; \
    fi
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs Rocket.toml ./
COPY types types
COPY ytmusic ytmusic
COPY spotify spotify
# Bauwerkzeug: gehört zum Workspace, wird hier aber nicht gebaut
COPY xtask xtask
# Programm samt Migrationen (src/db/migrations/)
COPY src src
# Schrift und Icons; build.rs bettet static/ ins Programm ein (/static/…)
COPY static static
# Oberfläche aus der Stufe oben; build.rs bettet sie ins Programm ein (Text und WebAssembly
# gzip) – der Server braucht dafür keine Dateien mehr.
COPY --from=ui /out/public ui/dist/public
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,id=nrzmusic-target-$TARGETARCH \
    env $(cat /cross-env) cargo build --release --locked --target "$(cat /rust-target)" \
 && cp "target/$(cat /rust-target)/release/nrzmusic" /nrzmusic

# ---------------------------------------------------------------- Programm allein
# Für die Release-Assets: nrzmusic-<version>-linux-<arch>
FROM scratch AS dist
COPY --from=build /nrzmusic /nrzmusic
COPY Rocket.toml /Rocket.toml

# ---------------------------------------------------------------- Werkzeuge für Downloads
# yt-dlp als eigenständiges Programm (bringt Python und die YouTube-Skripte yt-dlp-ejs schon mit)
# und Deno, die JavaScript-Laufzeit, die yt-dlp für YouTube braucht (yt-dlp sucht `deno` im PATH).
# Geladen und geprüft auf dem Build-Rechner; die Zielarchitektur bestimmt nur, welche Datei.
# Versionen und Prüfsummen stehen in xtask/src/docker.rs.
# nrzMusic kopiert yt-dlp nach /data/tools/ und hält die Kopie dort selbst aktuell (`yt-dlp -U`);
# die Datei hier ist nur der Ausgangsstand.
FROM --platform=$BUILDPLATFORM ${DEBIAN} AS tools
ARG TARGETARCH
ARG YTDLP_VERSION={{YTDLP_VERSION}}
ARG YTDLP_SHA256_AMD64={{YTDLP_SHA256_AMD64}}
ARG YTDLP_SHA256_ARM64={{YTDLP_SHA256_ARM64}}
ARG DENO_VERSION={{DENO_VERSION}}
ARG DENO_SHA256_AMD64={{DENO_SHA256_AMD64}}
ARG DENO_SHA256_ARM64={{DENO_SHA256_ARM64}}
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl unzip \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) ytdlp=yt-dlp_linux;         ytdlp_sha=$YTDLP_SHA256_AMD64; deno=x86_64;  deno_sha=$DENO_SHA256_AMD64 ;; \
      arm64) ytdlp=yt-dlp_linux_aarch64; ytdlp_sha=$YTDLP_SHA256_ARM64; deno=aarch64; deno_sha=$DENO_SHA256_ARM64 ;; \
      *) echo "Architektur $TARGETARCH nicht unterstützt" >&2; exit 1 ;; \
    esac; \
    mkdir -p /dl /tools; \
    cd /dl; \
    curl -fsSL --retry 3 -o yt-dlp \
      "https://github.com/yt-dlp/yt-dlp/releases/download/$YTDLP_VERSION/$ytdlp"; \
    curl -fsSL --retry 3 -o deno.zip \
      "https://github.com/denoland/deno/releases/download/v$DENO_VERSION/deno-$deno-unknown-linux-gnu.zip"; \
    printf '%s  yt-dlp\n%s  deno.zip\n' "$ytdlp_sha" "$deno_sha" | sha256sum -c -; \
    unzip -q deno.zip deno -d /tools; \
    install -m 0755 yt-dlp /tools/yt-dlp; \
    chmod 0755 /tools/deno; \
    cd /; \
    rm -rf /dl

# ---------------------------------------------------------------- Laufzeit
FROM ${DEBIAN} AS runtime
# ca-certificates für HTTPS zu YouTube Music und Spotify; ffmpeg (mit ffprobe) für die Downloads:
# yt-dlp wandelt damit nach MP3 um bzw. packt M4A und Opus um. Benutzer 1000 wie unter umbrelOS.
# /music ist der Ablageordner: Dateien hineinlegen, nrzMusic nimmt sie automatisch auf.
# Größe: ffmpeg aus Debian zieht rund 200 Pakete nach (entpackt ~400 MB auf arm64, ~460 MB auf
# amd64, u. a. Mesa/LLVM über SDL), Deno ~95 MB, yt-dlp ~40 MB. Das Image wächst damit von
# ~40 MB (1.0.0, gepackt) auf grob 250–300 MB gepackt bzw. um gut 550 MB entpackt.
# PKGSYSTEM_ENABLE_FSYNC=0: update-mime-database (über librsvg/gdk-pixbuf) ist unter QEMU (arm64
# auf dem x86-Runner) sonst minutenlang mit fsync beschäftigt.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive PKGSYSTEM_ENABLE_FSYNC=0 \
    apt-get install -y --no-install-recommends ca-certificates ffmpeg \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --user-group --home-dir /data --shell /usr/sbin/nologin nrzmusic \
 && mkdir -p /data /music \
 && chown 1000:1000 /data /music
COPY --from=tools /tools/yt-dlp /tools/deno /usr/local/bin/
COPY --from=build /nrzmusic /usr/local/bin/nrzmusic
COPY Rocket.toml /app/Rocket.toml

WORKDIR /app
# DENO_DIR: Deno legt bei jedem Aufruf durch yt-dlp einen kleinen Analyse-Cache an – nach /tmp
# statt ins Home (/data). DENO_NO_UPDATE_CHECK: keine Update-Prüfung im Netz; yt-dlp verwirft das
# Ergebnis, sobald Deno irgendetwas auf stderr schreibt.
ENV NRZMUSIC_CONFIG=/app/Rocket.toml \
    NRZMUSIC_DATA_DIR=/data \
    NRZMUSIC_MUSIC_DIR=/music \
    NRZMUSIC_ADDRESS=0.0.0.0 \
    NRZMUSIC_PORT=8350 \
    NRZMUSIC_YTDLP=/usr/local/bin/yt-dlp \
    NRZMUSIC_FFMPEG=/usr/bin/ffmpeg \
    DENO_DIR=/tmp/deno \
    DENO_NO_UPDATE_CHECK=1 \
    NRZMUSIC_LOG=info,rocket::server=warn,rocket::launch::_=warn,rocket::shield=warn,lofty=error
USER 1000:1000
VOLUME ["/data", "/music"]
EXPOSE 8350
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["nrzmusic", "healthcheck"]
STOPSIGNAL SIGTERM
CMD ["nrzmusic"]
"##;

/// Das fertige Dockerfile.
pub fn dockerfile() -> String {
    let tailwind = theme::tailwind_input();
    let mut file = TEMPLATE.to_string();
    for (name, value) in [
        ("DEBIAN", DEBIAN),
        ("RUST", RUST),
        ("DIOXUS_CLI", DIOXUS_CLI),
        ("TAILWIND", tailwind.as_str()),
        ("YTDLP_VERSION", YTDLP_VERSION),
        ("YTDLP_SHA256_AMD64", YTDLP_SHA256_AMD64),
        ("YTDLP_SHA256_ARM64", YTDLP_SHA256_ARM64),
        ("DENO_VERSION", DENO_VERSION),
        ("DENO_SHA256_AMD64", DENO_SHA256_AMD64),
        ("DENO_SHA256_ARM64", DENO_SHA256_ARM64),
    ] {
        file = file.replace(&format!("{{{{{name}}}}}"), value);
    }
    file
}

/// Docker-Befehl mit diesem Dockerfile auf stdin: `docker <args…> -f - <extra…> .`
pub fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new("docker");
    cmd.current_dir(root()).args(args).arg("-f").arg("-");
    cmd
}

/// Befehl aus [`command`] mit dem Dockerfile auf stdin ausführen.
pub fn run(mut cmd: Command) -> Result {
    cmd.arg(".").stdin(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("docker lässt sich nicht starten: {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin ist umgeleitet")
        .write_all(dockerfile().as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("docker buildx build ist fehlgeschlagen ({status})").into());
    }
    Ok(())
}

/// `cargo xtask docker <args…>` → `docker buildx build -f - <args…> .`
pub fn build(args: &[String]) -> Result {
    let mut cmd = command(&["buildx", "build"]);
    cmd.args(args);
    run(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_placeholders_filled() {
        let file = dockerfile();
        assert!(!file.contains("{{"), "offener Platzhalter");
        assert!(file.contains(&format!("ARG YTDLP_VERSION={YTDLP_VERSION}\n")));
        assert!(file.contains(&format!("cargo install dioxus-cli --version {DIOXUS_CLI} ")));
    }

    #[test]
    fn tailwind_input_as_heredoc() {
        let file = dockerfile();
        let start = file
            .find(r#"COPY <<"TAILWIND" ui/tailwind.css"#)
            .expect("Heredoc");
        let body = &file[start..];
        let body = &body[body.find('\n').unwrap() + 1..];
        let end = body.find("\nTAILWIND\n").expect("Ende des Heredocs");
        assert_eq!(&body[..=end], theme::tailwind_input());
        // Keine Zeile darf das Ende vorwegnehmen
        assert!(!theme::tailwind_input().lines().any(|l| l == "TAILWIND"));
    }

    #[test]
    fn workspace_members_copied() {
        let file = dockerfile();
        for member in ["types", "ytmusic", "spotify", "xtask"] {
            assert!(
                file.contains(&format!("COPY {member} {member}\n")),
                "{member}"
            );
        }
    }
}
