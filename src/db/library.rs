//! Abfragen der Bibliothek: Titel, Alben und Künstler. Alben und Künstler sind keine eigenen
//! Tabellen, sondern Gruppen der Titel (über `album_id` bzw. `artist_id`).

use crate::error::AppResult;
use crate::library::fold;
use nrzmusic_types::{Album, AlbumDetail, Artist, ArtistDetail, Page, Track};
use sqlx::{AssertSqlSafe, SqlitePool};
use std::collections::HashMap;

pub const TRACK_COLUMNS: &str = "id, title, artist, album, album_artist, album_id, artist_id, \
    track_no, disc_no, year, genre, duration_ms, bitrate_kbps, format, size, path, cover, \
    added_at, play_count, last_played";

#[derive(sqlx::FromRow)]
struct TrackRow {
    id: i64,
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    album_id: String,
    artist_id: String,
    track_no: Option<i64>,
    disc_no: Option<i64>,
    year: Option<i64>,
    genre: Option<String>,
    duration_ms: i64,
    bitrate_kbps: Option<i64>,
    format: String,
    size: i64,
    path: String,
    cover: Option<String>,
    added_at: i64,
    play_count: i64,
    last_played: Option<i64>,
}

impl From<TrackRow> for Track {
    fn from(r: TrackRow) -> Self {
        Track {
            id: r.id,
            title: r.title,
            artist: r.artist,
            album: r.album,
            album_artist: r.album_artist,
            album_id: r.album_id,
            artist_id: r.artist_id,
            track_no: r.track_no.map(|n| n as u32),
            disc_no: r.disc_no.map(|n| n as u32),
            year: r.year.map(|n| n as i32),
            genre: r.genre,
            duration_ms: r.duration_ms.max(0) as u64,
            bitrate_kbps: r.bitrate_kbps.map(|n| n as u32),
            format: r.format,
            size: r.size.max(0) as u64,
            path: r.path,
            cover: r.cover,
            added_at: r.added_at,
            play_count: r.play_count.max(0) as u32,
            last_played: r.last_played,
        }
    }
}

/// Alles, was der Scanner über eine Datei weiss.
#[derive(Debug, Clone)]
pub struct TrackData {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub bitrate_kbps: Option<u32>,
    pub format: String,
    pub size: u64,
    pub mtime: i64,
    pub cover: Option<String>,
    pub added_at: i64,
}

/// Stand einer Datei beim letzten Lesen.
#[derive(Debug, Clone, PartialEq)]
pub struct FileState {
    pub id: i64,
    pub size: i64,
    pub mtime: i64,
    pub cover: Option<String>,
}

/// Neu anlegen oder – gleicher Pfad – aktualisieren; die ID bleibt dabei erhalten.
/// Wiedergaben und `added_at` bleiben unangetastet. Liefert die ID.
pub async fn upsert(pool: &SqlitePool, d: &TrackData) -> AppResult<i64> {
    let search = fold(&format!(
        "{} {} {} {}",
        d.title, d.artist, d.album, d.album_artist
    ));
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO tracks (path, title, artist, album, album_artist, album_id, artist_id, \
            track_no, disc_no, year, genre, duration_ms, bitrate_kbps, format, size, mtime, cover, \
            search, added_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (path) DO UPDATE SET \
            title = excluded.title, artist = excluded.artist, album = excluded.album, \
            album_artist = excluded.album_artist, album_id = excluded.album_id, \
            artist_id = excluded.artist_id, track_no = excluded.track_no, \
            disc_no = excluded.disc_no, year = excluded.year, genre = excluded.genre, \
            duration_ms = excluded.duration_ms, bitrate_kbps = excluded.bitrate_kbps, \
            format = excluded.format, size = excluded.size, mtime = excluded.mtime, \
            cover = excluded.cover, search = excluded.search \
         RETURNING id",
    )
    .bind(&d.path)
    .bind(&d.title)
    .bind(&d.artist)
    .bind(&d.album)
    .bind(&d.album_artist)
    .bind(crate::library::album_id(&d.album_artist, &d.album))
    .bind(crate::library::artist_id(&d.album_artist))
    .bind(d.track_no.map(i64::from))
    .bind(d.disc_no.map(i64::from))
    .bind(d.year.map(i64::from))
    .bind(&d.genre)
    .bind(d.duration_ms as i64)
    .bind(d.bitrate_kbps.map(i64::from))
    .bind(&d.format)
    .bind(d.size as i64)
    .bind(d.mtime)
    .bind(&d.cover)
    .bind(search)
    .bind(d.added_at)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Nur das Cover setzen (z. B. wenn `folder.jpg` später dazukommt).
pub async fn set_cover(pool: &SqlitePool, id: i64, cover: &str) -> AppResult<()> {
    sqlx::query("UPDATE tracks SET cover = ? WHERE id = ?")
        .bind(cover)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Stand aller Dateien – ganz (`prefix` = None) oder unterhalb eines Ordners (`"A/B"`).
pub async fn index(
    pool: &SqlitePool,
    prefix: Option<&str>,
) -> AppResult<HashMap<String, FileState>> {
    let rows: Vec<(String, i64, i64, i64, Option<String>)> = match prefix {
        None => {
            sqlx::query_as("SELECT path, id, size, mtime, cover FROM tracks")
                .fetch_all(pool)
                .await?
        }
        Some(p) => {
            let p = format!("{p}/");
            sqlx::query_as(
                "SELECT path, id, size, mtime, cover FROM tracks WHERE substr(path, 1, ?) = ?",
            )
            .bind(p.chars().count() as i64)
            .bind(p)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows
        .into_iter()
        .map(|(path, id, size, mtime, cover)| {
            (
                path,
                FileState {
                    id,
                    size,
                    mtime,
                    cover,
                },
            )
        })
        .collect())
}

/// Umbenannte Datei bzw. verschobenen Ordner nachziehen (IDs bleiben). Pfade, die es am
/// Ziel schon gibt, bleiben unverändert – der nächste Scan räumt sie auf.
pub async fn move_paths(pool: &SqlitePool, from: &str, to: &str) -> AppResult<u64> {
    let mut tx = pool.begin().await?;
    let files = sqlx::query("UPDATE OR IGNORE tracks SET path = ? WHERE path = ?")
        .bind(to)
        .bind(from)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let prefix = format!("{from}/");
    let len = prefix.chars().count() as i64;
    let nested = sqlx::query(
        "UPDATE OR IGNORE tracks SET path = ? || substr(path, ?) WHERE substr(path, 1, ?) = ?",
    )
    .bind(format!("{to}/"))
    .bind(len + 1)
    .bind(len)
    .bind(prefix)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(files + nested)
}

/// Stand einer einzelnen Datei.
pub async fn state(pool: &SqlitePool, path: &str) -> AppResult<Option<FileState>> {
    let row: Option<(i64, i64, i64, Option<String>)> =
        sqlx::query_as("SELECT id, size, mtime, cover FROM tracks WHERE path = ?")
            .bind(path)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id, size, mtime, cover)| FileState {
        id,
        size,
        mtime,
        cover,
    }))
}

/// Titel zu Pfaden löschen; liefert die Anzahl.
pub async fn delete_paths(pool: &SqlitePool, paths: &[String]) -> AppResult<u64> {
    if paths.is_empty() {
        return Ok(0);
    }
    let json = serde_json::to_string(paths).unwrap_or_default();
    let res = sqlx::query("DELETE FROM tracks WHERE path IN (SELECT value FROM json_each(?))")
        .bind(json)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?");
    let row: Option<TrackRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Track::from))
}

/// Mehrere Titel auf einmal (fehlende fehlen einfach in der Map).
pub async fn get_many(pool: &SqlitePool, ids: &[i64]) -> AppResult<HashMap<i64, Track>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql =
        format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id IN (SELECT value FROM json_each(?))");
    let rows: Vec<TrackRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(serde_json::to_string(ids).unwrap_or_default())
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| (r.id, Track::from(r))).collect())
}

pub async fn exists(pool: &SqlitePool, id: i64) -> AppResult<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tracks WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?
            > 0,
    )
}

pub async fn count(pool: &SqlitePool) -> AppResult<u64> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tracks")
        .fetch_one(pool)
        .await?;
    Ok(n.max(0) as u64)
}

/// Eine Wiedergabe zählen.
pub async fn record_play(pool: &SqlitePool, id: i64, at: i64) -> AppResult<bool> {
    let res =
        sqlx::query("UPDATE tracks SET play_count = play_count + 1, last_played = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

#[derive(Debug, Default)]
pub struct Stats {
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub duration_ms: u64,
    pub size: u64,
}

pub async fn stats(pool: &SqlitePool) -> AppResult<Stats> {
    let (tracks, duration, size, albums, artists): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), COALESCE(SUM(duration_ms), 0), COALESCE(SUM(size), 0), \
            COUNT(DISTINCT CASE WHEN album <> '' THEN album_id END), COUNT(DISTINCT artist_id) \
         FROM tracks",
    )
    .fetch_one(pool)
    .await?;
    Ok(Stats {
        tracks: tracks as u64,
        albums: albums as u64,
        artists: artists as u64,
        duration_ms: duration as u64,
        size: size as u64,
    })
}

// ---------------------------------------------------------------- Titel-Listen

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSort {
    Title,
    Artist,
    Album,
    Added,
    /// Meistgespielt (Anzahl, dann zuletzt gespielt)
    Played,
    Duration,
}

impl TrackSort {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "added" => Self::Added,
            "played" => Self::Played,
            "duration" => Self::Duration,
            _ => return None,
        })
    }

    /// Ohne `order`: neueste bzw. meistgespielte zuerst, sonst aufsteigend.
    pub fn default_desc(self) -> bool {
        matches!(self, Self::Added | Self::Played)
    }

    fn order_by(self, desc: bool) -> String {
        let d = if desc { "DESC" } else { "ASC" };
        let in_album = "COALESCE(disc_no, 1), COALESCE(track_no, 0), title COLLATE NOCASE, id";
        match self {
            Self::Title => format!("title COLLATE NOCASE {d}, artist COLLATE NOCASE, id"),
            Self::Artist => format!(
                "artist = '', artist COLLATE NOCASE {d}, album COLLATE NOCASE {d}, {in_album}"
            ),
            Self::Album => format!(
                "album = '', album COLLATE NOCASE {d}, album_artist COLLATE NOCASE, {in_album}"
            ),
            Self::Added => format!("added_at {d}, album_id, {in_album}"),
            Self::Played => {
                format!("play_count {d}, COALESCE(last_played, 0) {d}, title COLLATE NOCASE, id")
            }
            Self::Duration => format!("duration_ms {d}, title COLLATE NOCASE, id"),
        }
    }
}

/// `WHERE`-Bedingung für Suchbegriffe (jeder Begriff muss vorkommen) samt Werten.
fn search_clause(q: Option<&str>) -> (String, Vec<String>) {
    let terms: Vec<String> = q
        .map(fold)
        .unwrap_or_default()
        .split_whitespace()
        .map(|t| format!("%{}%", like_escape(t)))
        .collect();
    if terms.is_empty() {
        return (String::new(), terms);
    }
    let clause = vec!["search LIKE ? ESCAPE '\\'"; terms.len()].join(" AND ");
    (format!("WHERE {clause}"), terms)
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Titel seitenweise, optional gefiltert (`q` passt auf Titel, Künstler oder Album).
pub async fn list(
    pool: &SqlitePool,
    q: Option<&str>,
    sort: TrackSort,
    desc: bool,
    offset: u64,
    limit: u64,
) -> AppResult<Page<Track>> {
    let (filter, terms) = search_clause(q);
    let count_sql = format!("SELECT count(*) FROM tracks {filter}");
    let mut count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql));
    for t in &terms {
        count = count.bind(t.clone());
    }
    let total = count.fetch_one(pool).await?.max(0) as u64;

    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks {filter} ORDER BY {} LIMIT ? OFFSET ?",
        sort.order_by(desc)
    );
    let mut query = sqlx::query_as::<_, TrackRow>(AssertSqlSafe(sql));
    for t in &terms {
        query = query.bind(t.clone());
    }
    let rows = query
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;
    Ok(Page {
        items: rows.into_iter().map(Track::from).collect(),
        total,
        offset,
        limit,
    })
}

/// Meistgespielte Titel (mindestens einmal gespielt).
pub async fn most_played(pool: &SqlitePool, limit: u64) -> AppResult<Vec<Track>> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE play_count > 0 \
         ORDER BY play_count DESC, last_played DESC, id LIMIT ?"
    );
    let rows: Vec<TrackRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Track::from).collect())
}

/// Titel eines Albums: nach Disc, Titelnummer, Titel.
async fn album_tracks(pool: &SqlitePool, album_id: &str) -> AppResult<Vec<Track>> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE album_id = ? \
         ORDER BY COALESCE(disc_no, 1), COALESCE(track_no, 0), title COLLATE NOCASE, id"
    );
    let rows: Vec<TrackRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(album_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Track::from).collect())
}

// ---------------------------------------------------------------- Alben

#[derive(sqlx::FromRow)]
struct AlbumRow {
    id: String,
    title: String,
    artist: String,
    artist_id: String,
    year: Option<i64>,
    track_count: i64,
    duration_ms: i64,
    cover: Option<String>,
    added_at: i64,
}

impl From<AlbumRow> for Album {
    fn from(r: AlbumRow) -> Self {
        Album {
            id: r.id,
            title: r.title,
            artist: r.artist,
            artist_id: r.artist_id,
            year: r.year.map(|y| y as i32),
            track_count: r.track_count.max(0) as u32,
            duration_ms: r.duration_ms.max(0) as u64,
            cover: r.cover,
            added_at: r.added_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumSort {
    Title,
    Artist,
    /// Neueste zuerst
    Year,
    /// Zuletzt hinzugefügt zuerst
    Added,
    Random,
}

impl AlbumSort {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "year" => Self::Year,
            "added" => Self::Added,
            _ => return None,
        })
    }

    pub fn default_desc(self) -> bool {
        matches!(self, Self::Year | Self::Added)
    }

    fn order_by(self, desc: bool) -> String {
        let d = if desc { "DESC" } else { "ASC" };
        match self {
            Self::Title => format!("title COLLATE NOCASE {d}, artist COLLATE NOCASE"),
            Self::Artist => format!(
                "artist = '', artist COLLATE NOCASE {d}, year IS NULL, year, title COLLATE NOCASE"
            ),
            Self::Year => format!("year IS NULL, year {d}, added_at DESC, title COLLATE NOCASE"),
            Self::Added => format!("added_at {d}, title COLLATE NOCASE"),
            Self::Random => "random()".into(),
        }
    }
}

const ALBUM_SELECT: &str =
    "SELECT album_id AS id, MIN(album) AS title, MIN(album_artist) AS artist, \
    MIN(artist_id) AS artist_id, MAX(year) AS year, count(*) AS track_count, \
    COALESCE(SUM(duration_ms), 0) AS duration_ms, MIN(cover) AS cover, MAX(added_at) AS added_at \
    FROM tracks";

/// Welche Alben?
pub enum AlbumScope<'a> {
    All,
    Artist(&'a str),
}

/// Alben (ohne lose Titel ohne Album), optional gefiltert und begrenzt.
pub async fn albums(
    pool: &SqlitePool,
    scope: AlbumScope<'_>,
    q: Option<&str>,
    sort: AlbumSort,
    desc: bool,
    limit: Option<u64>,
) -> AppResult<Vec<Album>> {
    let filter = match scope {
        AlbumScope::All => "",
        AlbumScope::Artist(_) => "AND artist_id = ?",
    };
    let terms: Vec<String> = q
        .map(fold)
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();
    // Mit Suchbegriffen wird in Rust gefiltert – die Begrenzung kommt danach.
    let sql_limit = match limit {
        Some(n) if terms.is_empty() => format!("LIMIT {n}"),
        _ => String::new(),
    };
    let sql = format!(
        "{ALBUM_SELECT} WHERE album <> '' {filter} GROUP BY album_id ORDER BY {} {sql_limit}",
        sort.order_by(desc)
    );
    let mut query = sqlx::query_as::<_, AlbumRow>(AssertSqlSafe(sql));
    if let AlbumScope::Artist(id) = scope {
        query = query.bind(id.to_string());
    }
    let mut albums: Vec<Album> = query
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Album::from)
        .collect();
    if !terms.is_empty() {
        albums.retain(|a| {
            let hay = fold(&format!("{} {}", a.title, a.artist));
            terms.iter().all(|t| hay.contains(t.as_str()))
        });
        if let Some(n) = limit {
            albums.truncate(n as usize);
        }
    }
    Ok(albums)
}

pub async fn album(pool: &SqlitePool, id: &str) -> AppResult<Option<AlbumDetail>> {
    let sql = format!("{ALBUM_SELECT} WHERE album_id = ? GROUP BY album_id");
    let row: Option<AlbumRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(AlbumDetail {
        album: row.into(),
        tracks: album_tracks(pool, id).await?,
    }))
}

// ---------------------------------------------------------------- Künstler

#[derive(sqlx::FromRow)]
struct ArtistRow {
    id: String,
    name: String,
    album_count: i64,
    track_count: i64,
    cover: Option<String>,
}

impl From<ArtistRow> for Artist {
    fn from(r: ArtistRow) -> Self {
        Artist {
            id: r.id,
            name: r.name,
            album_count: r.album_count.max(0) as u32,
            track_count: r.track_count.max(0) as u32,
            cover: r.cover,
        }
    }
}

const ARTIST_SELECT: &str = "SELECT artist_id AS id, MIN(album_artist) AS name, \
    COUNT(DISTINCT CASE WHEN album <> '' THEN album_id END) AS album_count, \
    count(*) AS track_count, \
    COALESCE(MIN(CASE WHEN album <> '' THEN cover END), MIN(cover)) AS cover \
    FROM tracks";

/// Künstler (nach Album-Künstler gruppiert), alphabetisch; unbekannte zuletzt.
pub async fn artists(
    pool: &SqlitePool,
    q: Option<&str>,
    limit: Option<u64>,
) -> AppResult<Vec<Artist>> {
    let sql = format!("{ARTIST_SELECT} GROUP BY artist_id ORDER BY name = '', name COLLATE NOCASE");
    let rows: Vec<ArtistRow> = sqlx::query_as(AssertSqlSafe(sql)).fetch_all(pool).await?;
    let terms: Vec<String> = q
        .map(fold)
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();
    let mut artists: Vec<Artist> = rows
        .into_iter()
        .map(Artist::from)
        .filter(|a| {
            terms.is_empty() || {
                let hay = fold(&a.name);
                terms.iter().all(|t| hay.contains(t.as_str()))
            }
        })
        .collect();
    if let Some(n) = limit {
        artists.truncate(n as usize);
    }
    Ok(artists)
}

pub async fn artist(pool: &SqlitePool, id: &str) -> AppResult<Option<ArtistDetail>> {
    let sql = format!("{ARTIST_SELECT} WHERE artist_id = ? GROUP BY artist_id");
    let row: Option<ArtistRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let artist = Artist::from(row);
    let albums = albums(
        pool,
        AlbumScope::Artist(id),
        None,
        AlbumSort::Year,
        true,
        None,
    )
    .await?;
    // Seine Titel – auch solche auf Samplern, bei denen er nur Titel-Künstler ist
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks \
         WHERE artist_id = ? OR (? <> '' AND artist = ? COLLATE NOCASE) \
         ORDER BY play_count DESC, COALESCE(last_played, 0) DESC, album COLLATE NOCASE, \
            COALESCE(disc_no, 1), COALESCE(track_no, 0), id"
    );
    let rows: Vec<TrackRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .bind(&artist.name)
        .bind(&artist.name)
        .fetch_all(pool)
        .await?;
    Ok(Some(ArtistDetail {
        artist,
        albums,
        tracks: rows.into_iter().map(Track::from).collect(),
    }))
}
