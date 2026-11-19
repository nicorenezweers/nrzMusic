# Architecture

nrzMusic is one Rust program. It serves the JSON API, the web interface (a Dioxus/WebAssembly app embedded at build time), the Android app (`/static/nrzMusic.apk`) and does all background work: scanning and watching the music folder, reading tags and covers, and talking to YouTube Music. Accounts, library, playlists, favorites and history live in one SQLite file; the music itself stays as plain files in the music folder.

```
┌──────────────────────── Browser ────────────────────────┐
│  Dioxus app (WASM, rust-ui components)                  │
│  one <audio> element: local files and YouTube audio     │
│  (also inside the Android app's WebView)                │
└──────────────┬──────────────────────────────────────────┘
               │ /api/v1 (JSON, SSE, audio ranges)
┌──────────────▼──────────────────────────────────────────┐
│ Rocket 0.5 · auth Gate (session cookie / Bearer)        │
│  routes/*  library · stream · covers · upload · events  │
│            playlists · favorites · history · settings   │
│            yt (thin wrapper over nrzmusic-ytmusic)      │
│            yt/stream (yt-dlp → audio URL → passthrough) │
│            auth, users                                  │
│  library/  scanner · watcher (notify) · tags (lofty)    │
│            covers (embedded art, folder.jpg, resizing)  │
│  routes/spa (embedded UI)                               │
└───────┬───────────────────────────────┬─────────────────┘
        │ sqlx                          │ reqwest (rustls)
┌───────▼────────────┐   ┌──────────────▼─────────────────┐
│ data/nrzmusic.db   │   │ music.youtube.com/youtubei/v1  │
│ data/covers/       │   │ (InnerTube, WEB_REMIX client)  │
└────────────────────┘   └────────────────────────────────┘
        ▲
        │ watch + scan
   music folder (/music) ← drop MP3/FLAC/… here, or upload in the app
```

## Crates

| Path | Crate | What |
|---|---|---|
| `/` | `nrzmusic` | Server (Rocket), accounts, scanner, watcher, SQLite |
| `types/` | `nrzmusic-types` | Shared JSON types – the API contract for server, UI and YouTube client |
| `ytmusic/` | `nrzmusic-ytmusic` | YouTube Music client (InnerTube): search, suggestions, albums, playlists, artists, radio, home |
| `spotify/` | `nrzmusic-spotify` | Spotify Web API client (login with PKCE, liked songs, playlists), public playlists from the embed page, and matching of Spotify tracks to YouTube Music |
| `ui/` | `nrzmusic-ui` | Web interface, Dioxus 0.7 → WebAssembly, components adapted from rust-ui; own Cargo project (not in the workspace) |
| `xtask/` | `xtask` | Build tool (`cargo xtask …`, Rust): Tailwind input from `ui/src/theme.rs`, UI bundle, Dockerfile, Android app, umbrelOS package – replaces the former shell scripts and the Dockerfile |
| `macos/` | `nrzmusic-macos` | macOS app (tao, wry/WKWebView, muda, window-vibrancy): transparent window with the system's vibrancy behind the interface; the app injects its own styles into every page (`macos/src/pages.rs` `NATIVE_CSS`: glass surfaces, SF Pro, arrow cursor, no selection of controls, top row aligned to the traffic lights which an empty unified toolbar places like in native apps – `macos/src/lights.rs` measures them, `--nrz-tl-y`/`--nrz-tl-r`), so the look does not depend on the server version; bridge `ui/src/mac.rs`, menu bar with Steuerung (media actions via `navigator.mediaSession` handlers), window frame remembered in `app.json`; automatic updates (`macos/src/updater.rs`, from nrzX): checks the nrzGit releases `v<version>` on git.zweers.ch at start and every 6 h, downloads the DMG in the background (checked against `SHA256SUMS-macos.txt`), then a button at the bottom of the sidebar (and the app menu) replaces the bundle and relaunches; either shows a remote nrzMusic instance (checked via `/api/v1/health`) or runs the server in-process on `127.0.0.1` (`auth = false`, data in `~/Library/Application Support/nrzMusic`, music `~/Music/nrzMusic`). Mode in `app.json`; connect page and window script are Rust strings (`macos/src/pages.rs`). Own Cargo project (mac only); `cargo xtask macos` builds the bundle (Info.plist generated in `xtask/src/macos.rs`) and a DMG |
| branch `android` | – | Android app (Kotlin, Gradle): WebView on the server, JS bridge `NrzAndroid`, offline library with media3/ExoPlayer; `cargo xtask android` checks it out as a worktree into `android/` and builds the APK into `static/` |

## Decisions

| Area | Choice | Why |
|---|---|---|
| Server | Rocket 0.5.1 | Same stack as the other nrz apps; graceful shutdown, typed routes |
| Storage | One SQLite file (sqlx, bundled) + cover cache in `data/covers/` | Music files are never copied into the database; the folder stays the source of truth |
| Library | `notify` watches the music folder recursively (poll watcher as fallback) plus a periodic full scan; `lofty` reads tags of MP3, FLAC, M4A/AAC, OGG, Opus, WAV, AIFF | Dropping a file into the folder is all it takes |
| Streaming | Own range responder (`206 Partial Content`) | Seeking, and Safari only plays audio with real range support |
| Covers | Embedded front cover, else `cover.*`/`folder.*`/`front.*` in the folder; resized on demand (`?size=`) and cached | Small lists stay fast with large embedded art |
| YouTube Music | Metadata through InnerTube on the server (no account, no key); playback: yt-dlp resolves the best audio-only format (M4A/AAC first, plays everywhere) over IPv4, the URL is cached until shortly before it expires, and the server passes the bytes through with `Range` | No embedded player, no ads; googlevideo URLs are bound to the server's IP, so the browser cannot fetch them itself. Fresh URLs answer 403 for a few seconds – the server retries, and the UI prepares the next queue item |
| Downloads | Optional: **yt-dlp** as an external program (with ffmpeg, and Deno for YouTube's JavaScript challenges), a queue with workers; nrzMusic writes the tags and cover itself (lofty) from the YouTube Music metadata, then the file is scanned like any other | yt-dlp is the tool that keeps up with YouTube; it updates itself into `data/tools/` |
| Spotify | Web API with Authorization Code + **PKCE** (no client secret), tokens in SQLite and refreshed automatically; each Spotify track is matched to YouTube Music (artist + title, duration, ISRC when it helps) and played or downloaded from there | Spotify audio can only be played through its own SDK (Premium, HTTPS); liked songs and playlists are what matters |
| Interface | **Dioxus 0.7** (Rust → WebAssembly) with components adapted from **rust-ui**; built with `dx bundle` (`cargo xtask ui`) and embedded by `build.rs` (gzip). The page head for the web app (viewport, manifest, icons, dark background before load) is added by the server (`routes::spa`) to dx's default `index.html` | One binary, no CDN, same look as nrzPaper |
| Styles | `ui/src/theme.rs`: colors, radius, font and animations as Rust data, other rules embedded; `cargo xtask css` generates the Tailwind input `ui/tailwind.css` (not in git), dx runs Tailwind over `ui/src/**/*.rs`. The toast fade-out duration is one constant shared by CSS and code | One source for every color; the stylesheet is a build artifact |
| Tooling | `main` is Rust only: build steps in `cargo xtask` (the Dockerfile is generated and piped to `docker buildx build -f -`), migrations as Rust modules in `src/db/migrations/` (text byte-identical to the former `.sql` files, SHA-384 checksums pinned by a test) | One language for app and tooling; existing databases keep their migration history |
| Live updates | Server-Sent Events (`/api/v1/events`); events carry an optional recipient (playlists, favorites, Spotify go only to their account) | New files show up without reloading |
| Accounts | `users` + `sessions` in SQLite; Argon2id, 32-byte random tokens (only SHA-256 stored), 180 days rolling; cookie `nrzmusic_session` (HttpOnly, SameSite=Lax, Secure with an https `public_url`) or `Authorization: Bearer`. A request fairing (`auth::Gate`) rewrites unauthenticated `/api/v1/*` (except `health`, `auth/*`) to a 401 route; `CurrentUser`/`Admin` guards in the handlers. Playlists, favorites, history and the Spotify login carry `user_id`; library and downloads are shared. Data from before 1.2 (`user_id = 0`) goes to the first account | Defense in depth: nothing under the API is reachable without a session even if a handler forgets its guard |
| Android | WebView loads the server; `NrzAndroid` bridge only answers on pages of the configured server; downloads with the WebView's cookie only to that server; offline playback natively (ExoPlayer in a `MediaSessionService`), web playback kept alive by a foreground service with a platform `MediaSession` | The UI stays one codebase; offline needs native storage and playback |
| Feed & stats | Every finished playback is reported (`POST /listens`: listened time, skipped, completed; `sendBeacon` on page close) into `listens`; `src/feed.rs` builds a taste profile (time-decayed listens, favorites, dislikes, old history) and mixes candidates from YouTube Music radio, artists' top songs, rediscovered favorites, the library and charts, scored with noise and an exploration share; the player refills its queue from `/feed` in feed mode and at the end of any queue; `/stats` aggregates `listens` in SQL | Learns from behaviour without an account at YouTube; recommendations stay on the server, only YouTube Music and (for genres) the iTunes search are asked |
| Sharing | `shares` (random 16-byte token per account/kind/target, unique, live content); the Gate lets `GET /api/v1/public/*` and `/api/v1/covers/*` through; `/public/share/<token>/stream/<key>` only streams tracks that belong to that link; `/s/<token>` serves index.html with Open Graph tags; the UI shows a standalone page with a small player when nobody is logged in | Share without accounts on the other side, without exposing the rest of the library |
| Preloading | UI: a pool of four `<audio>` elements (active, next fully once the current has played 4 s, next-but-one metadata only, previous kept 8–30 s by back-probability), unlocked with a silent blob on the first touch (iOS), falling back to the last playing element on `NotAllowedError`; server: permanent yt-dlp errors (deleted, private, blocked, age gate) answer 410 and are remembered for an hour, the UI replaces such tracks with another version (same base title and artist) or drops them | Gapless-feeling skips without decoding work or downloading whole queues |
| i18n | German source strings as keys, catalogs in `ui/locales/<lang>.json` (de, en) | Readable code |

## Configuration

Every key in `Rocket.toml` can be overridden with `NRZMUSIC_<NAME>`.

| Variable | Default | Meaning |
|---|---|---|
| `NRZMUSIC_PORT` | `8350` | HTTP port |
| `NRZMUSIC_DATA_DIR` | `./data` (`/data` in Docker) | Database and cover cache |
| `NRZMUSIC_MUSIC_DIR` | `./music` (`/music` in Docker) | The watched music folder; uploads land here too |
| `NRZMUSIC_SCAN_INTERVAL` | `30` | Minutes between full rescans (safety net next to the watcher), `0` = off |
| `NRZMUSIC_YOUTUBE` | `true` | `false` switches YouTube Music off completely |
| `NRZMUSIC_LOG` | `info` | Log filter (`tracing` env-filter syntax) |
| `NRZMUSIC_DOWNLOADS` | `true` | `false` switches downloads off completely |
| `NRZMUSIC_YTDLP` | `yt-dlp` | yt-dlp program; copied to `data/tools/yt-dlp` so it can update itself |
| `NRZMUSIC_FFMPEG` | `ffmpeg` | ffmpeg program (MP3 conversion, remuxing) |
| `NRZMUSIC_DOWNLOAD_WORKERS` | `2` | Parallel downloads |
| `NRZMUSIC_SPOTIFY_CLIENT_ID` | – | Preset for the Spotify client ID (the value saved in the settings wins) |
| `NRZMUSIC_AUTH` | `true` | Accounts; `false` = one local admin user without login |
| `NRZMUSIC_PUBLIC_URL` | – | Public address (`https://…`): secure cookies, default Spotify redirect URI |
| `NRZMUSIC_GENRE_LOOKUP` | `true` | Look up genres of YouTube artists (iTunes search) for the stats |

## API (`/api/v1`)

JSON in and out; types are in `types/src/lib.rs`. Errors: `{"error": "…"}` (`ApiError`) with a fitting status.

### Accounts

Without a session every other route answers `401 {"error": "Bitte anmelden."}`.

| Method & path | Body → Response |
|---|---|
| `GET /auth/status` | → `AuthStatus` (`enabled`, `user`, `registration_open`, `setup`) |
| `POST /auth/register` | `RegisterRequest` → `201 {user, token}` + cookie; the first account becomes admin; `403` while sign-ups are closed |
| `POST /auth/login` | `LoginRequest` → `{user, token}` + cookie; `401` wrong credentials, `429` after 10 failures in 15 minutes |
| `POST /auth/logout` | → `204` |
| `PUT /auth/password` | `PasswordChange` → `204` (other sessions end) |
| `PATCH /auth/account` | `AccountUpdate` → `Account` |
| `DELETE /auth/account` | `{password}` → `204` (deletes the account's playlists, favorites, history, Spotify login) |
| `GET /users` | admin → `Vec<Account>` |
| `PATCH /users/<id>` | admin, `UserUpdate` → `Account` |
| `DELETE /users/<id>` | admin → `204` |

Admin only: `PUT /settings`, `POST /upload`, `POST /library/rescan`, `POST /downloads/tool/update`.

### Library

| Method & path | Body → Response |
|---|---|
| `GET /health` | → `{"ok": true}` (used by `nrzmusic healthcheck`) |
| `GET /status` | → `LibraryStatus` |
| `POST /library/rescan` | → `202` – full scan in the background |
| `GET /home` | → `HomeData` |
| `GET /tracks?q=&sort=&order=&offset=&limit=` | → `Page<Track>`; `sort`: `title`, `artist`, `album`, `added`, `played`, `duration`; `order`: `asc`/`desc`; `limit` default 200, max 1000 |
| `GET /tracks/<id>` | → `Track` |
| `GET /tracks/<id>/stream` | → audio with `Range` support (`206`, `416`), `Accept-Ranges: bytes`, content type by format |
| `GET /tracks/<id>/download` | → the file as attachment |
| `GET /covers/<cover>?size=` | → JPEG, `size` = edge in px (e.g. 96, 300, 600; `0` = original), `Cache-Control: immutable` |
| `GET /albums?q=&sort=` | → `Vec<Album>`; `sort`: `title`, `artist`, `year`, `added` |
| `GET /albums/<id>` | → `AlbumDetail` |
| `GET /artists?q=` | → `Vec<Artist>` |
| `GET /artists/<id>` | → `ArtistDetail` |
| `GET /search?q=&limit=` | → `LibrarySearch` |
| `POST /upload` | multipart, field `files` (repeated; the file name may carry a relative path like `Album/01 Song.mp3`) → `UploadResult` |
| `GET /events` | → `text/event-stream`, each `data:` is a `ServerEvent` |

### Favorites, history

| Method & path | Body → Response |
|---|---|
| `GET /favorites` | → `Vec<PlayableTrack>` (newest first) |
| `GET /favorites/keys` | → `Vec<String>` (`local:<id>`, `yt:<videoId>`) |
| `PUT /favorites` | `TrackRef` → `204` |
| `DELETE /favorites/<key>` | → `204` |
| `POST /history` | `TrackRef` → `204` (counts a play: `play_count`, `last_played`) |
| `GET /history?limit=` | → `Vec<PlayableTrack>` (newest first, no duplicates) |

### Playlists

| Method & path | Body → Response |
|---|---|
| `GET /playlists` | → `Vec<PlaylistSummary>` |
| `POST /playlists` | `NewPlaylist` → `201 PlaylistSummary` |
| `GET /playlists/<id>` | → `Playlist` |
| `PATCH /playlists/<id>` | `PlaylistUpdate` → `PlaylistSummary` |
| `DELETE /playlists/<id>` | → `204` |
| `POST /playlists/<id>/items` | `AddItems` → `PlaylistSummary` |
| `DELETE /playlists/<id>/items/<position>` | → `PlaylistSummary` |
| `PUT /playlists/<id>/order` | `Reorder` → `PlaylistSummary` |

### YouTube Music

`503` with `ApiError` while YouTube Music is switched off, `502` when YouTube does not answer as expected.

| Method & path | Response |
|---|---|
| `GET /yt/search?q=&filter=` | `YtSearchResults`; `filter`: `songs`, `videos`, `albums`, `artists`, `playlists` |
| `GET /yt/suggest?q=` | `Vec<String>` |
| `GET /yt/home` | `Vec<YtShelf>` |
| `GET /yt/album/<browse_id>` | `YtAlbum` |
| `GET /yt/playlist/<playlist_id>` | `YtPlaylist` |
| `GET /yt/artist/<channel_id>` | `YtArtist` |
| `GET /yt/radio/<video_id>` | `Vec<YtTrack>` |
| `POST /yt/import` | `ImportRequest` → `201 PlaylistSummary` |
| `GET /yt/stream/<video_id>` | audio (`206` with `Range`): the downloaded file if there is one, else the audio track resolved by yt-dlp, passed through |
| `POST /yt/stream/<video_id>/prepare` | → `204` once YouTube serves the track (warms the cache for the next queue item); `503` with yt-dlp's message |

### Downloads (yt-dlp)

`503` with `ApiError` while downloads are switched off (env or settings) or yt-dlp is missing. Downloads land in `<music>/YouTube/<Artist>/<Album or Singles>/<Title>.<ext>`.

| Method & path | Body → Response |
|---|---|
| `GET /downloads` | → `Vec<DownloadJob>` (open jobs and the finished ones of the last 7 days, newest first) |
| `POST /downloads` | `DownloadRequest` → `Vec<DownloadJob>` (no duplicates: an existing job is returned) |
| `POST /downloads/<id>/retry` | → `DownloadJob` |
| `DELETE /downloads/<id>` | → `204` – cancels a queued/running job or removes a finished one from the list (the file stays) |
| `DELETE /downloads?finished=true` | → `204` – clears done/failed/canceled jobs from the list |
| `GET /downloads/map` | → `Vec<DownloadedTrack>` (only where the local file still exists) |
| `GET /downloads/tool` | → `YtDlpInfo` |
| `POST /downloads/tool/update` | → `YtDlpInfo` – runs `yt-dlp -U` on the copy in `data/tools/` |

### Spotify

`503` while no client ID is set, `401` while not connected (or after Spotify revoked the login).

| Method & path | Body → Response |
|---|---|
| `GET /spotify/status` | → `SpotifyStatus` |
| `POST /spotify/login` | → `SpotifyLogin` (open `authorize_url`) |
| `GET /spotify/callback?code=&state=` | Redirect target registered at Spotify: finishes the login and redirects to `/spotify` (`?error=…` on failure) |
| `POST /spotify/callback` | `SpotifyCallback` → `SpotifyStatus` – the address Spotify redirected to, pasted by the user |
| `DELETE /spotify/session` | → `204` – disconnect (tokens deleted) |
| `GET /spotify/liked?offset=&limit=` | → `Page<SpTrack>` (`limit` ≤ 50, `youtube` filled from the match cache) |
| `GET /spotify/playlists` | → `Vec<SpPlaylistSummary>` |
| `GET /spotify/playlists/<id>` | → `SpPlaylist` |
| `POST /spotify/match` | `SpMatchRequest` → `Vec<SpTrack>` (cached per Spotify ID) |
| `GET /spotify/public/<id>` | → `SpPlaylist` from the public embed page (no Spotify login, at most 100 tracks) |
| `POST /spotify/import` | `SpImportRequest` (`source`: `liked`, `all` or a playlist ID/link) → `202`, progress as `SpotifyProgress` events (`match`, `import_all`, `import`/`import_failed`); importing the same source again updates its playlist |

### Settings

| Method & path | Body → Response |
|---|---|
| `GET /settings` | → `Settings` |
| `PUT /settings` | `Settings` → `Settings` |

Everything outside `/api/` and `/static/` returns the embedded web interface (`index.html` with the web-app head for client-side routes).

## Security

- Accounts with Argon2id passwords; session tokens are random 256-bit values, stored only as SHA-256; login failures throttled per client IP (last `X-Forwarded-For` entry, set by the own proxy).
- `NRZMUSIC_AUTH=false` only behind an authenticating proxy or on a trusted network.
- Spotify: PKCE without a client secret, `state` checked and bound to the account that started the login, tokens only in the database and never sent to the browser; only read scopes.
- yt-dlp runs as a child process with an argument list (no shell); only video IDs from YouTube Music reach it.
- Uploads: only audio extensions, path components sanitized (no `..`, no absolute paths, no hidden files), never overwrites an existing file.
- CSP: scripts only from the server, no frames (`frame-src 'none'`), media only from the server, images from the server and YouTube's/Spotify's image hosts, `frame-ancestors 'self'`.
- `yt/stream` only accepts 11-character video IDs and only fetches `https://` URLs that yt-dlp returned.
