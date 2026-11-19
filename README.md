# nrzMusic – Umbrel-Paket

Dieser Zweig enthält nur fertige Dateien für die Umbrel-App `nrz-music` (nrzStore), damit sie
ohne Container-Registry und ohne Login installiert werden kann. Der Quellcode liegt auf `main`.

| Datei | Inhalt |
|---|---|
| `nrzmusic-<arch>.gz` | Server mit eingebetteter Oberfläche und Android-App (glibc), amd64/arm64 |
| `ffmpeg-<arch>.gz`, `ffprobe-<arch>.gz` | statisches ffmpeg 7.1 aus `mwader/static-ffmpeg:7.1@sha256:a8090df5f5608daef387e1b2e93b98aaacb4d92153ad904e7d715c725724fca4` (GPL) |
| `yt-dlp-<arch>.gz` | yt-dlp 2026.10.26 (eigenständig, Unlicense) |
| `deno-<arch>.gz` | Deno 2.9.8 (MIT), JavaScript für yt-dlp |
| `Rocket.toml` | Standard-Konfiguration |
| `nrzMusic.apk` | Android-App (auch im Server unter /static/nrzMusic.apk) |
| `SHA256SUMS` | Prüfsummen |

Stand: nrzMusic 1.3.0, gebaut aus `6ef06d8` mit `cargo xtask umbrel-release 1.3.0`.
