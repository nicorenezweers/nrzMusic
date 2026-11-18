//! Den Musikordner überwachen: `notify` meldet Änderungen (inotify, FSEvents, …); geht das
//! nicht (z. B. manche Netzlaufwerke), schaut ein PollWatcher alle 10 Sekunden nach.
//!
//! Gemeldete Pfade werden gesammelt und erst gelesen, wenn sie ~2 Sekunden ruhig waren und
//! ihre Grösse sich nicht mehr ändert – so wird keine halb kopierte Datei gelesen.
//! Dateien werden einzeln abgeglichen, Ordner als Ganzes; Umbenennungen behalten ihre IDs.

use super::{is_ignored_rel, Library};
use notify::event::{AccessKind, AccessMode, ModifyKind, RenameMode};
use notify::{Event, EventKind, PollWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// So lange muss ein Pfad ruhig sein.
const SETTLE: Duration = Duration::from_secs(2);
/// So oft wird geprüft, ob gesammelte Pfade bereit sind.
const TICK: Duration = Duration::from_millis(500);
/// Abfrage-Intervall des Ersatz-Watchers.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Ein gemeldeter, noch nicht gelesener Pfad.
struct Pending {
    since: Instant,
    size: Option<u64>,
    /// Tags neu lesen, auch wenn Grösse und Zeit gleich sind (umbenannt/verschoben)
    force: bool,
}

fn size_of(path: &PathBuf) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Watcher starten (echte Ereignisse, sonst Abfrage). `None`, wenn beides scheitert.
fn start(
    root: &std::path::Path,
    tx: mpsc::UnboundedSender<notify::Result<Event>>,
) -> Option<Box<dyn Watcher + Send>> {
    let tx2 = tx.clone();
    let native = notify::recommended_watcher(move |ev| {
        let _ = tx.send(ev);
    })
    .and_then(|mut w| w.watch(root, RecursiveMode::Recursive).map(|_| w));
    match native {
        Ok(w) => return Some(Box::new(w)),
        Err(e) => {
            tracing::warn!("Überwachung des Musikordners nicht möglich ({e}) – frage alle 10 s ab")
        }
    }
    let poll = PollWatcher::new(
        move |ev| {
            let _ = tx2.send(ev);
        },
        notify::Config::default().with_poll_interval(POLL_INTERVAL),
    )
    .and_then(|mut w| w.watch(root, RecursiveMode::Recursive).map(|_| w));
    match poll {
        Ok(w) => Some(Box::new(w)),
        Err(e) => {
            tracing::warn!("Musikordner wird nicht überwacht ({e}) – nur periodischer Scan");
            None
        }
    }
}

/// Überwachung starten; endet mit dem Server.
pub fn spawn(library: Library, mut shutdown: rocket::Shutdown) {
    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Result<Event>>();
    let watcher = start(library.root(), tx);
    library.set_watching(watcher.is_some());
    if watcher.is_some() {
        tracing::info!("Überwache {}", library.root().display());
    }

    tokio::spawn(async move {
        let _watcher = watcher; // lebt so lange wie die Schleife
        let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Some(Ok(ev)) => handle(&library, &mut pending, ev).await,
                    Some(Err(e)) => tracing::warn!("Überwachung: {e}"),
                    None => break,
                },
                _ = tick.tick() => {
                    if !pending.is_empty() {
                        flush(&library, &mut pending).await;
                    }
                }
                _ = &mut shutdown => break,
            }
        }
        library.set_watching(false);
    });
}

/// Ein Ereignis einsortieren.
async fn handle(library: &Library, pending: &mut HashMap<PathBuf, Pending>, ev: Event) {
    if ev.need_rescan() {
        tracing::info!("Überwachung hat Ereignisse verpasst – voller Scan");
        library.start_full_scan();
        return;
    }
    let mut force = false;
    match ev.kind {
        // Lesen (auch unser eigenes) ist keine Änderung; "fertig geschrieben" schon
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => {}
        EventKind::Access(_) => return,
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if ev.paths.len() == 2 => {
            // Umbenannt/verschoben: Einträge mitnehmen, dann am Ziel neu lesen
            match library.moved(&ev.paths[0], &ev.paths[1]).await {
                Ok(n) if n > 0 => tracing::debug!(n, "Titel verschoben"),
                Err(e) => tracing::warn!("Verschieben nachziehen: {e}"),
                _ => {}
            }
            force = true;
        }
        _ => {}
    }
    for path in ev.paths {
        if let Some(rel) = library.rel_path(&path) {
            if is_ignored_rel(&rel) {
                continue;
            }
        } else if path != library.root() {
            continue;
        }
        let size = size_of(&path);
        let entry = pending.entry(path).or_insert(Pending {
            since: Instant::now(),
            size,
            force: false,
        });
        entry.since = Instant::now();
        entry.size = size;
        entry.force |= force;
    }
}

/// Bereite Pfade abgleichen: ~2 s ruhig und Grösse unverändert.
async fn flush(library: &Library, pending: &mut HashMap<PathBuf, Pending>) {
    let mut ready = Vec::new();
    for (path, p) in pending.iter_mut() {
        if p.since.elapsed() < SETTLE {
            continue;
        }
        let size = size_of(path);
        if size != p.size {
            // wird noch geschrieben
            p.size = size;
            p.since = Instant::now();
            continue;
        }
        ready.push((path.clone(), p.force));
    }
    if ready.is_empty() {
        return;
    }
    for (path, _) in &ready {
        pending.remove(path);
    }
    // Was noch wartet, nicht beim Abgleich eines Ordners mitlesen
    let skip: HashSet<String> = pending.keys().filter_map(|p| library.rel_path(p)).collect();
    if let Err(e) = library.scan_paths(ready, skip).await {
        tracing::warn!("Änderungen im Musikordner: {e}");
    }
}
