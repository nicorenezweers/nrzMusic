#!/bin/sh
# Umbrel-Paket ohne Registry und ohne Login (wie nrzMedia):
#   scripts/umbrel-release.sh 1.2.0
# baut das Programm für linux/amd64 und linux/arm64 (Stufe `dist` des Dockerfiles, mit
# eingebetteter Oberfläche und Android-App aus static/), holt ein statisches ffmpeg/ffprobe
# (gepinntes Image mwader/static-ffmpeg) sowie yt-dlp und Deno (Versionen und Prüfsummen aus
# dem Dockerfile) und legt alles als Commit auf den Zweig `umbrel` mit dem Tag `umbrel-<version>`:
#   nrzmusic-<arch>.gz  ffmpeg-<arch>.gz  ffprobe-<arch>.gz  yt-dlp-<arch>.gz  deno-<arch>.gz  SHA256SUMS
# Das `pre-start`-Skript der App `nrz-music` im nrzStore lädt diese Dateien auf dem Gerät von
# http://localhost:8390/nicorenezweers/nrzMusic/raw/tag/umbrel-<version>/… (nrzGit) und startet
# sie in einem gepinnten debian:trixie-slim.
# Vorher: scripts/ui.sh und scripts/android.sh (die APK liegt dann in static/).
# Gepusht wird auf den Remote $REMOTE (Standard: nrz = nrzGit); Docker muss laufen.
set -eu
cd "$(dirname "$0")/.."

version="${1:-}"
if [ -z "$version" ]; then
  echo "Aufruf: $0 <version>   (z. B. $0 1.2.0)" >&2
  exit 1
fi
remote="${REMOTE:-nrz}"
tag="umbrel-$version"
out="dist/umbrel/$version"
ffmpeg_image="mwader/static-ffmpeg:7.1@sha256:a8090df5f5608daef387e1b2e93b98aaacb4d92153ad904e7d715c725724fca4"
builder="${BUILDER:-nrzmusic-builder}"

arg() { sed -n "s/^ARG $1=//p" Dockerfile | head -1; }
ytdlp_version="$(arg YTDLP_VERSION)"
deno_version="$(arg DENO_VERSION)"

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "Tag $tag gibt es schon – neue Version wählen." >&2
  exit 1
fi
if [ ! -f static/nrzMusic.apk ]; then
  echo "static/nrzMusic.apk fehlt – erst scripts/android.sh ausführen." >&2
  exit 1
fi

rm -rf "$out"
mkdir -p "$out/bin" "$out/dl"

echo "==> Programm (linux/amd64, linux/arm64)"
if ! docker buildx inspect "$builder" >/dev/null 2>&1; then
  docker buildx create --name "$builder" --driver docker-container >/dev/null
fi
docker buildx build --builder "$builder" --platform linux/amd64,linux/arm64 --target dist \
  --output "type=local,dest=$out/bin" .

echo "==> ffmpeg, yt-dlp $ytdlp_version, Deno $deno_version"
for arch in amd64 arm64; do
  case "$arch" in
    amd64) ytdlp=yt-dlp_linux; ytdlp_sha="$(arg YTDLP_SHA256_AMD64)"; deno=x86_64; deno_sha="$(arg DENO_SHA256_AMD64)" ;;
    arm64) ytdlp=yt-dlp_linux_aarch64; ytdlp_sha="$(arg YTDLP_SHA256_ARM64)"; deno=aarch64; deno_sha="$(arg DENO_SHA256_ARM64)" ;;
  esac
  docker pull -q --platform "linux/$arch" "$ffmpeg_image" >/dev/null
  container="$(docker create --platform "linux/$arch" "$ffmpeg_image")"
  docker cp "$container:/ffmpeg" "$out/bin/linux_$arch/ffmpeg"
  docker cp "$container:/ffprobe" "$out/bin/linux_$arch/ffprobe"
  docker rm "$container" >/dev/null

  curl -fsSL --retry 3 -o "$out/dl/yt-dlp-$arch" \
    "https://github.com/yt-dlp/yt-dlp/releases/download/$ytdlp_version/$ytdlp"
  curl -fsSL --retry 3 -o "$out/dl/deno-$arch.zip" \
    "https://github.com/denoland/deno/releases/download/v$deno_version/deno-$deno-unknown-linux-gnu.zip"
  printf '%s  %s\n%s  %s\n' "$ytdlp_sha" "$out/dl/yt-dlp-$arch" "$deno_sha" "$out/dl/deno-$arch.zip" | shasum -a 256 -c -
  unzip -q -o "$out/dl/deno-$arch.zip" deno -d "$out/dl/deno-$arch"

  gzip -9 -c "$out/bin/linux_$arch/nrzmusic" > "$out/nrzmusic-$arch.gz"
  gzip -9 -c "$out/bin/linux_$arch/ffmpeg" > "$out/ffmpeg-$arch.gz"
  gzip -9 -c "$out/bin/linux_$arch/ffprobe" > "$out/ffprobe-$arch.gz"
  gzip -9 -c "$out/dl/yt-dlp-$arch" > "$out/yt-dlp-$arch.gz"
  gzip -9 -c "$out/dl/deno-$arch/deno" > "$out/deno-$arch.gz"
done
files="nrzmusic-amd64.gz nrzmusic-arm64.gz ffmpeg-amd64.gz ffmpeg-arm64.gz ffprobe-amd64.gz ffprobe-arm64.gz yt-dlp-amd64.gz yt-dlp-arm64.gz deno-amd64.gz deno-arm64.gz"
cp Rocket.toml "$out/Rocket.toml"
(cd "$out" && shasum -a 256 $files Rocket.toml > SHA256SUMS)

echo "==> Zweig umbrel, Tag $tag"
worktree="$(mktemp -d -t nrzmusic-umbrel.XXXXXX)"
rmdir "$worktree"
trap 'git worktree remove --force "$worktree" >/dev/null 2>&1 || true' EXIT
git fetch -q "$remote" umbrel 2>/dev/null || true
if git rev-parse -q --verify "refs/remotes/$remote/umbrel" >/dev/null; then
  git worktree add -q -B umbrel "$worktree" "$remote/umbrel"
else
  git worktree add -q --orphan -b umbrel "$worktree"
fi
rm -f "$worktree"/*.gz "$worktree"/SHA256SUMS "$worktree"/Rocket.toml
for f in $files SHA256SUMS Rocket.toml; do cp "$out/$f" "$worktree/"; done
cp static/nrzMusic.apk "$worktree/nrzMusic.apk"
cat > "$worktree/README.md" <<EOF
# nrzMusic – Umbrel-Paket

Dieser Zweig enthält nur fertige Dateien für die Umbrel-App \`nrz-music\` (nrzStore), damit sie
ohne Container-Registry und ohne Login installiert werden kann. Der Quellcode liegt auf \`main\`.

| Datei | Inhalt |
|---|---|
| \`nrzmusic-<arch>.gz\` | Server mit eingebetteter Oberfläche und Android-App (glibc), amd64/arm64 |
| \`ffmpeg-<arch>.gz\`, \`ffprobe-<arch>.gz\` | statisches ffmpeg 7.1 aus \`$ffmpeg_image\` (GPL) |
| \`yt-dlp-<arch>.gz\` | yt-dlp $ytdlp_version (eigenständig, Unlicense) |
| \`deno-<arch>.gz\` | Deno $deno_version (MIT), JavaScript für yt-dlp |
| \`Rocket.toml\` | Standard-Konfiguration |
| \`nrzMusic.apk\` | Android-App (auch im Server unter /static/nrzMusic.apk) |
| \`SHA256SUMS\` | Prüfsummen |

Stand: nrzMusic $version, gebaut aus \`$(git rev-parse --short HEAD)\` mit \`scripts/umbrel-release.sh $version\`.
EOF
git -C "$worktree" add -A
git -C "$worktree" -c user.name="$(git config user.name || echo nrz)" -c user.email="$(git config user.email || echo nrz@localhost)" \
  commit -q -m "Umbrel-Paket nrzMusic $version"
git -C "$worktree" tag -a "$tag" -m "Umbrel-Paket nrzMusic $version"
git push -q "$remote" umbrel "refs/tags/$tag"
echo "Fertig: $tag auf $remote (Zweig umbrel)."
cat "$out/SHA256SUMS"
