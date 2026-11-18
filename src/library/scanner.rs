//! Den Musikordner mit der Datenbank abgleichen.
//!
//! * Voller Scan: alle Dateien durchgehen, nur geänderte (Grösse/Änderungszeit) neu lesen,
//!   Einträge gelöschter Dateien entfernen. Die ID eines Titels hängt am Pfad und bleibt.
//! * Teil-Scans für die Überwachung und Uploads: eine Datei oder ein Ordner.
//!
//! Gelöscht wird nur, was wirklich fehlt: Ordner, die sich nicht lesen lassen, und ein
//! fehlender Musikordner (z. B. nicht eingehängtes Netzlaufwerk) löschen nichts.

use super::{audio_format, covers, is_ignored_name, is_ignored_rel, tags, Library};
use crate::db::library::{self as store, TrackData};
use crate::error::{AppError, AppResult};
use futures::StreamExt;
use nrzmusic_types::{ScanProgress, ServerEvent};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// So viele Dateien werden gleichzeitig gelesen.
const PARALLEL_READS: usize = 4;
/// Höchstens so oft ein Fortschritts-Ereignis.
const PROGRESS_EVERY: Duration = Duration::from_millis(300);
/// Ordner tiefer als das werden nicht mehr durchsucht (Schutz vor Symlink-Schleifen).
const MAX_DEPTH: usize = 40;

/// Eine gefundene Audiodatei.
#[derive(Debug, Clone)]
pub struct Found {
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    /// Änderungszeit in Millisekunden
    pub mtime: i64,
    /// Erstellt (sonst geändert) in Unix-Sekunden – wird zu `added_at`
    pub created: i64,
}

/// Was sich geändert hat.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changes {
    pub added: u64,
    pub updated: u64,
    pub removed: u64,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }

    pub fn add(&mut self, other: Changes) {
        self.added += other.added;
        self.updated += other.updated;
        self.removed += other.removed;
    }
}

/// Ergebnis für eine einzelne Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    Added(i64),
    Updated(i64),
    Unchanged(i64),
    Removed,
    /// Keine Musikdatei, versteckt oder ausserhalb des Musikordners
    Ignored,
}

impl FileOutcome {
    pub fn id(self) -> Option<i64> {
        match self {
            FileOutcome::Added(id) | FileOutcome::Updated(id) | FileOutcome::Unchanged(id) => {
                Some(id)
            }
            _ => None,
        }
    }

    fn changes(self) -> Changes {
        match self {
            FileOutcome::Added(_) => Changes {
                added: 1,
                ..Changes::default()
            },
            FileOutcome::Updated(_) => Changes {
                updated: 1,
                ..Changes::default()
            },
            FileOutcome::Removed => Changes {
                removed: 1,
                ..Changes::default()
            },
            _ => Changes::default(),
        }
    }
}

/// Welcher Teil des Musikordners abgeglichen wird.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    /// Ordner relativ zum Musikordner, `None` = der ganze Ordner
    pub dir: Option<String>,
    pub recursive: bool,
    /// Auch unveränderte Dateien neu lesen (z. B. nach neuem `folder.jpg`)
    pub force: bool,
    /// Diese Pfade nicht anfassen (die Überwachung wartet noch, bis sie fertig geschrieben sind)
    pub skip: HashSet<String>,
}

impl Scope {
    pub fn all() -> Self {
        Scope {
            recursive: true,
            ..Scope::default()
        }
    }
}

#[derive(Default)]
struct Walk {
    files: Vec<Found>,
    /// Ordner (relativ, "" = Musikordner), die sich nicht lesen liessen
    failed: Vec<String>,
}

fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Grösse, Änderungszeit (ms) und "hinzugefügt" (s) aus den Metadaten.
pub fn file_times(meta: &std::fs::Metadata) -> (u64, i64, i64) {
    let mtime = meta.modified().map(unix_ms).unwrap_or(0);
    // Erstellungszeit, wo das Dateisystem sie kennt (eine frisch kopierte Datei ist "neu",
    // auch wenn `cp -p` die Änderungszeit erhalten hat) – sonst die Änderungszeit
    let created = meta.created().map(unix_ms).unwrap_or(mtime) / 1000;
    let now = crate::db::now();
    let added = if created <= 0 { now } else { created.min(now) };
    (meta.len(), mtime, added)
}

/// Ordner durchgehen (blockierend). Folgt Symlinks, besucht aber keinen Ordner zweimal.
fn walk(root: &Path, dir: Option<&str>, recursive: bool) -> std::io::Result<Walk> {
    let start = match dir {
        Some(d) => root.join(d),
        None => root.to_path_buf(),
    };
    if !std::fs::metadata(&start)?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "kein Ordner",
        ));
    }
    let mut out = Walk::default();
    let mut visited = HashSet::new();
    let mut stack = vec![(start, dir.unwrap_or("").to_string(), 0usize)];
    while let Some((abs_dir, rel_dir, depth)) = stack.pop() {
        if let Ok(canonical) = std::fs::canonicalize(&abs_dir) {
            if !visited.insert(canonical) {
                continue;
            }
        }
        let entries = match std::fs::read_dir(&abs_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = %abs_dir.display(), "Ordner nicht lesbar: {e}");
                out.failed.push(rel_dir);
                continue;
            }
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                tracing::debug!(path = %entry.path().display(), "Dateiname ist kein UTF-8 – übersprungen");
                continue;
            };
            if is_ignored_name(&name) {
                continue;
            }
            let abs = entry.path();
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            // metadata folgt Symlinks
            let Ok(meta) = std::fs::metadata(&abs) else {
                continue;
            };
            if meta.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    stack.push((abs, rel, depth + 1));
                }
            } else if meta.is_file() && audio_format(&abs).is_some() {
                let (size, mtime, created) = file_times(&meta);
                out.files.push(Found {
                    rel,
                    abs,
                    size,
                    mtime,
                    created,
                });
            }
        }
    }
    Ok(out)
}

/// Liegt `path` in `dir` ("" = Musikordner)?
fn is_under(path: &str, dir: &str) -> bool {
    dir.is_empty()
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Liegt `path` direkt in `dir` (nicht in einem Unterordner)?
fn is_direct_child(path: &str, dir: Option<&str>) -> bool {
    match dir {
        None => !path.contains('/'),
        Some(d) => path
            .strip_prefix(d)
            .and_then(|r| r.strip_prefix('/'))
            .is_some_and(|r| !r.contains('/')),
    }
}

/// Gemeinsames für das Lesen in `spawn_blocking`: Cover-Ordner und bereits gesuchte Ordner-Cover.
struct ReadCtx {
    covers: PathBuf,
    folder_covers: Mutex<HashMap<PathBuf, Option<String>>>,
}

impl ReadCtx {
    fn new(covers: &Path) -> Self {
        ReadCtx {
            covers: covers.to_path_buf(),
            folder_covers: Mutex::new(HashMap::new()),
        }
    }

    /// Cover-Datei im Ordner (einmal je Ordner gesucht und gespeichert).
    fn folder_cover(&self, dir: &Path) -> Option<String> {
        let mut cache = self.folder_covers.lock().unwrap();
        if let Some(c) = cache.get(dir) {
            return c.clone();
        }
        let id = covers::folder_cover(dir)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|data| covers::store(&self.covers, &data));
        cache.insert(dir.to_path_buf(), id.clone());
        id
    }

    /// Eine Datei lesen (blockierend): Tags, Fallbacks, Cover.
    fn read(&self, f: &Found) -> TrackData {
        let (info, picture) = tags::read(&f.abs, &f.rel);
        let cover = picture
            .and_then(|p| covers::store(&self.covers, &p))
            .or_else(|| f.abs.parent().and_then(|d| self.folder_cover(d)));
        TrackData {
            path: f.rel.clone(),
            title: info.title,
            artist: info.artist,
            album: info.album,
            album_artist: info.album_artist,
            track_no: info.track_no,
            disc_no: info.disc_no,
            year: info.year,
            genre: info.genre,
            duration_ms: info.duration_ms,
            bitrate_kbps: info.bitrate_kbps,
            format: audio_format(&f.abs).unwrap_or_default(),
            size: f.size,
            mtime: f.mtime,
            cover,
            added_at: f.created,
        }
    }
}

fn join_error(e: tokio::task::JoinError) -> AppError {
    AppError::Io(std::io::Error::other(e))
}

impl Library {
    /// Voller Scan im Hintergrund (läuft schon einer, passiert nichts).
    pub fn start_full_scan(&self) {
        let lib = self.clone();
        tokio::spawn(async move {
            if let Err(e) = lib.full_scan().await {
                tracing::warn!("Scan des Musikordners: {e}");
            }
        });
    }

    /// Voller Scan. `None`, wenn gerade schon einer läuft.
    pub async fn full_scan(&self) -> AppResult<Option<Changes>> {
        if self.inner.scanning.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }
        struct Reset<'a>(&'a Library);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.inner.scanning.store(false, Ordering::SeqCst);
                self.0.set_progress(None);
            }
        }
        let _reset = Reset(self);

        let started = Instant::now();
        self.set_progress(Some(ScanProgress::default()));
        self.events().send(ServerEvent::ScanStarted);
        let result = self.sync(Scope::all(), true).await;
        self.inner
            .last_scan
            .store(crate::db::now(), Ordering::SeqCst);
        let changes = result.as_ref().copied().unwrap_or_default();
        self.events().send(ServerEvent::ScanFinished {
            added: changes.added,
            updated: changes.updated,
            removed: changes.removed,
        });
        let changes = result?;
        if !changes.is_empty() {
            self.notify_changed().await;
        }
        tracing::info!(
            added = changes.added,
            updated = changes.updated,
            removed = changes.removed,
            ms = started.elapsed().as_millis() as u64,
            "Musikordner gescannt"
        );
        Ok(Some(changes))
    }

    /// `LibraryChanged` mit der aktuellen Anzahl Titel senden.
    pub async fn notify_changed(&self) {
        match store::count(self.pool()).await {
            Ok(tracks) => self.events().send(ServerEvent::LibraryChanged { tracks }),
            Err(e) => tracing::warn!("Titel zählen: {e}"),
        }
    }

    /// Einen Teil (oder alles) mit der Datenbank abgleichen.
    pub async fn sync(&self, scope: Scope, report: bool) -> AppResult<Changes> {
        let root = self.root().to_path_buf();
        let (dir, recursive) = (scope.dir.clone(), scope.recursive);
        let walked = tokio::task::spawn_blocking(move || walk(&root, dir.as_deref(), recursive))
            .await
            .map_err(join_error)?;
        let walked = match walked {
            Ok(w) => w,
            Err(e) => match scope.dir.as_deref() {
                // Ein verschwundener Unterordner: alles darin ist weg
                Some(d) if !self.abs_path(d).exists() => Walk::default(),
                d => {
                    return Err(AppError::Config(format!(
                        "Musikordner {} nicht lesbar ({e}) – nichts geändert",
                        self.root().join(d.unwrap_or("")).display()
                    )))
                }
            },
        };

        let mut index = store::index(self.pool(), scope.dir.as_deref()).await?;
        if !scope.recursive {
            index.retain(|p, _| is_direct_child(p, scope.dir.as_deref()));
        }

        let files: Vec<Found> = walked
            .files
            .into_iter()
            .filter(|f| !scope.skip.contains(&f.rel))
            .collect();
        let total = files.len() as u64;
        let mut changes = Changes::default();
        let ctx = Arc::new(ReadCtx::new(self.covers_dir()));
        let present: HashSet<String> = files.iter().map(|f| f.rel.clone()).collect();

        // Unveränderte Dateien überspringen; fehlt ihnen ein Cover, nach folder.jpg schauen.
        let mut to_read = Vec::new();
        let mut done = 0u64;
        for f in files {
            match index.get(&f.rel) {
                Some(s) if !scope.force && s.size == f.size as i64 && s.mtime == f.mtime => {
                    done += 1;
                    if s.cover.is_none() {
                        let (ctx2, dir) = (ctx.clone(), f.abs.parent().map(Path::to_path_buf));
                        let cover = tokio::task::spawn_blocking(move || {
                            dir.and_then(|d| ctx2.folder_cover(&d))
                        })
                        .await
                        .map_err(join_error)?;
                        if let Some(c) = cover {
                            store::set_cover(self.pool(), s.id, &c).await?;
                            changes.updated += 1;
                        }
                    }
                }
                _ => to_read.push(f),
            }
        }

        let mut last_report = Instant::now();
        let mut report_progress = |done: u64, force: bool| {
            if !report {
                return;
            }
            let p = ScanProgress { done, total };
            self.set_progress(Some(p));
            if force || last_report.elapsed() >= PROGRESS_EVERY {
                last_report = Instant::now();
                self.events().send(ServerEvent::ScanProgress(p));
            }
        };
        report_progress(done, true);

        let mut reads = futures::stream::iter(to_read.into_iter().map(|f| {
            let ctx = ctx.clone();
            async move { tokio::task::spawn_blocking(move || ctx.read(&f)).await }
        }))
        .buffer_unordered(PARALLEL_READS);
        while let Some(data) = reads.next().await {
            let data = data.map_err(join_error)?;
            store::upsert(self.pool(), &data).await?;
            if index.contains_key(&data.path) {
                changes.updated += 1;
            } else {
                changes.added += 1;
            }
            done += 1;
            report_progress(done, false);
        }
        report_progress(done, true);

        // Entfernen, was nicht mehr da ist (und wirklich fehlt)
        let gone: Vec<String> = index
            .keys()
            .filter(|p| {
                !present.contains(*p)
                    && !scope.skip.contains(*p)
                    && !walked.failed.iter().any(|d| is_under(p, d))
                    && !self.abs_path(p).is_file()
            })
            .cloned()
            .collect();
        changes.removed += store::delete_paths(self.pool(), &gone).await?;
        Ok(changes)
    }

    /// Eine einzelne Datei abgleichen (neu, geändert, gelöscht).
    pub async fn scan_file(&self, abs: &Path, force: bool) -> AppResult<FileOutcome> {
        let Some(rel) = self.rel_path(abs) else {
            return Ok(FileOutcome::Ignored);
        };
        if is_ignored_rel(&rel) || audio_format(abs).is_none() {
            return Ok(FileOutcome::Ignored);
        }
        let meta = match tokio::fs::metadata(abs).await {
            Ok(m) if m.is_file() => m,
            Ok(_) => return Ok(FileOutcome::Ignored),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let n = store::delete_paths(self.pool(), &[rel]).await?;
                return Ok(if n > 0 {
                    FileOutcome::Removed
                } else {
                    FileOutcome::Ignored
                });
            }
            Err(e) => return Err(e.into()),
        };
        let (size, mtime, created) = file_times(&meta);
        let existing = store::state(self.pool(), &rel).await?;
        if let Some(s) = &existing {
            if !force && s.size == size as i64 && s.mtime == mtime {
                return Ok(FileOutcome::Unchanged(s.id));
            }
        }
        let found = Found {
            rel,
            abs: abs.to_path_buf(),
            size,
            mtime,
            created,
        };
        let ctx = ReadCtx::new(self.covers_dir());
        let data = tokio::task::spawn_blocking(move || ctx.read(&found))
            .await
            .map_err(join_error)?;
        let id = store::upsert(self.pool(), &data).await?;
        Ok(if existing.is_some() {
            FileOutcome::Updated(id)
        } else {
            FileOutcome::Added(id)
        })
    }

    /// Gemeldete Pfade abgleichen (Überwachung). `force` gilt für Pfade, deren Tags neu
    /// gelesen werden sollen (umbenannt/verschoben). Sendet `LibraryChanged`, wenn sich
    /// etwas geändert hat.
    pub async fn scan_paths(
        &self,
        paths: Vec<(PathBuf, bool)>,
        skip: HashSet<String>,
    ) -> AppResult<Changes> {
        let mut changes = Changes::default();
        for (abs, force) in paths {
            let Some(rel) = self.rel_path(&abs) else {
                if abs == self.root() {
                    self.start_full_scan();
                }
                continue;
            };
            if is_ignored_rel(&rel) {
                continue;
            }
            let name = rel.rsplit('/').next().unwrap_or(&rel);
            let parent = rel.rsplit_once('/').map(|(d, _)| d.to_string());
            let c = if covers::is_cover_file_name(name) {
                // Neues/geändertes/gelöschtes Ordner-Cover: Titel daneben neu lesen
                self.sync(
                    Scope {
                        dir: parent,
                        recursive: false,
                        force: true,
                        skip: skip.clone(),
                    },
                    false,
                )
                .await?
            } else if abs.is_dir() {
                self.sync(
                    Scope {
                        dir: Some(rel),
                        recursive: true,
                        force,
                        skip: skip.clone(),
                    },
                    false,
                )
                .await?
            } else if abs.exists() {
                self.scan_file(&abs, force).await?.changes()
            } else {
                // Weg: eine Datei oder ein ganzer Ordner
                let mut c = self.scan_file(&abs, false).await?.changes();
                c.add(
                    self.sync(
                        Scope {
                            dir: Some(rel),
                            recursive: true,
                            force: false,
                            skip: skip.clone(),
                        },
                        false,
                    )
                    .await?,
                );
                c
            };
            changes.add(c);
        }
        if !changes.is_empty() {
            tracing::info!(
                added = changes.added,
                updated = changes.updated,
                removed = changes.removed,
                "Musikordner geändert"
            );
            self.notify_changed().await;
        }
        Ok(changes)
    }

    /// Umbenennen/Verschieben (Datei oder Ordner) in der Datenbank nachziehen, damit die IDs
    /// bleiben. Liefert die Anzahl verschobener Titel.
    pub async fn moved(&self, from: &Path, to: &Path) -> AppResult<u64> {
        let (Some(from), Some(to)) = (self.rel_path(from), self.rel_path(to)) else {
            return Ok(0);
        };
        if is_ignored_rel(&from) || is_ignored_rel(&to) {
            return Ok(0);
        }
        store::move_paths(self.pool(), &from, &to).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_relations() {
        assert!(is_under("A/B/c.mp3", "A"));
        assert!(is_under("A/B/c.mp3", ""));
        assert!(!is_under("AB/c.mp3", "A"));
        assert!(is_direct_child("A/c.mp3", Some("A")));
        assert!(!is_direct_child("A/B/c.mp3", Some("A")));
        assert!(is_direct_child("c.mp3", None));
        assert!(!is_direct_child("A/c.mp3", None));
    }
}
