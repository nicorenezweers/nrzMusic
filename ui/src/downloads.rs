//! Downloads mit yt-dlp: welche YouTube-Titel schon als Datei im Musikordner liegen
//! (`/downloads/map` – der Player spielt sie lokal) und die Aufträge (`/downloads`), live
//! nachgeführt über `download_update`.

use crate::api;
use crate::components::overlay::{Ask, confirm, toast, toast_error};
use crate::i18n::{tr, trf};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{DownloadJob, DownloadRequest, DownloadStatus, DownloadedTrack, PlayableTrack, YtTrack};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// `video_id` → lokale Titel-ID
pub static DOWNLOADED: GlobalSignal<HashMap<String, i64>> = Signal::global(HashMap::new);
/// Aufträge, neueste zuerst
pub static JOBS: GlobalSignal<Vec<DownloadJob>> = Signal::global(Vec::new);
/// Downloads auf dem Server aus oder yt-dlp fehlt (503) – mit der Meldung des Servers
pub static OFF: GlobalSignal<Option<String>> = Signal::global(|| None);
/// Aufträge wenigstens einmal geladen
pub static LOADED: GlobalSignal<bool> = Signal::global(|| false);
/// Laufende Anfragen zum Einreihen (direkt nach dem Serverstart kann das ~20 s dauern)
pub static PENDING: GlobalSignal<usize> = Signal::global(|| 0);

/// Ab so vielen Titeln wird vor dem Herunterladen nachgefragt.
pub const CONFIRM_FROM: usize = 25;

#[derive(Clone, Copy, Default)]
struct Batch {
    done: u32,
    failed: u32,
    /// Klein genug für eine Meldung pro Titel (beim ersten fertigen Titel entschieden)
    each: bool,
    active: bool,
}

thread_local! {
    /// Seit dem letzten Leerlauf fertige/fehlgeschlagene Aufträge – grosse Mengen werden am
    /// Ende zusammengefasst gemeldet statt mit einer Meldung pro Titel.
    static BATCH: Cell<Batch> = const { Cell::new(Batch { done: 0, failed: 0, each: true, active: false }) };
}

pub fn is_open(s: DownloadStatus) -> bool {
    matches!(s, DownloadStatus::Queued | DownloadStatus::Running)
}

/// Lokale Titel-ID eines heruntergeladenen YouTube-Titels (ohne Abhängigkeit – für den Player).
pub fn local_id(video_id: &str) -> Option<i64> {
    DOWNLOADED.peek().get(video_id).copied()
}

/// Die heruntergeladene Datei liess sich nicht abspielen: vergessen (dann spielt YouTube).
pub fn forget(video_id: &str) {
    if DOWNLOADED.peek().contains_key(video_id) {
        DOWNLOADED.write().remove(video_id);
    }
}

/// Downloads sind (soweit bekannt) möglich.
pub fn available() -> bool {
    OFF.read().is_none()
}

/// Zustand eines YouTube-Titels für Plaketten und Menüs (reaktiv).
#[derive(Clone, Debug, PartialEq)]
pub enum DlState {
    /// Lokale Titel-ID
    Downloaded(i64),
    /// Auftrags-ID
    Queued(i64),
    Running(i64, f32),
    Failed(i64, String),
}

pub fn state_of(video_id: &str) -> Option<DlState> {
    if let Some(id) = DOWNLOADED.read().get(video_id) {
        return Some(DlState::Downloaded(*id));
    }
    let jobs = JOBS.read();
    let job = jobs.iter().find(|j| j.track.video_id == video_id)?;
    match job.status {
        DownloadStatus::Queued => Some(DlState::Queued(job.id)),
        DownloadStatus::Running => Some(DlState::Running(job.id, job.progress)),
        DownloadStatus::Failed => Some(DlState::Failed(job.id, job.error.clone().unwrap_or_default())),
        // Massgebend ist die Liste der Dateien: fehlt der Titel dort, ist die Datei weg
        DownloadStatus::Done | DownloadStatus::Canceled => None,
    }
}

/// Abschnitt eines laufenden Downloads: bis 0.9 laden, bei 0.9 umwandeln (kann dauern),
/// ab 0.93 Tags schreiben und ablegen.
pub fn phase(progress: f32) -> String {
    if progress >= 0.93 {
        tr("Ablegen …")
    } else if progress >= 0.9 {
        tr("Umwandeln …")
    } else {
        tr("Lädt …")
    }
}

/// YouTube will eine Anmeldung sehen (Bot-Prüfung) – dann helfen Cookies.
pub fn needs_cookies(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("sign in to confirm") || e.contains("not a bot")
}

/// Laufende und wartende Aufträge (Plakette in der Navigation).
pub fn open_count() -> usize {
    JOBS.read().iter().filter(|j| is_open(j.status)).count()
}

/// YouTube-Titel aus einer gemischten Liste.
pub fn youtube_of(tracks: &[PlayableTrack]) -> Vec<YtTrack> {
    tracks
        .iter()
        .filter_map(|t| match t {
            PlayableTrack::Youtube(y) => Some(y.clone()),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ Laden

pub async fn load() {
    futures_util::future::join(load_map(), load_jobs()).await;
}

pub async fn load_map() {
    if let Ok(list) = api::get::<Vec<DownloadedTrack>>("/downloads/map").await {
        *DOWNLOADED.write() = list.into_iter().map(|d| (d.video_id, d.track_id)).collect();
    }
}

/// Aufträge laden; merkt sich, ob Downloads auf dem Server aus sind.
pub async fn load_jobs() {
    match api::get::<Vec<DownloadJob>>("/downloads").await {
        Ok(jobs) => {
            *JOBS.write() = jobs;
            if OFF.peek().is_some() {
                *OFF.write() = None;
            }
        }
        Err(e) if e.unavailable() => {
            *OFF.write() = Some(e.message);
            JOBS.write().clear();
        }
        Err(_) => {}
    }
    *LOADED.write() = true;
}

// ------------------------------------------------------------------ Live

/// `download_update`: Auftrag ersetzen oder vorne einfügen; fertige in die Liste der Dateien.
pub fn on_update(job: DownloadJob) {
    let prev = JOBS.peek().iter().find(|j| j.id == job.id).map(|j| j.status);
    upsert(job.clone());
    if OFF.peek().is_some() {
        *OFF.write() = None;
    }
    if prev == Some(job.status) {
        return;
    }
    match job.status {
        DownloadStatus::Done => {
            if let Some(id) = job.track_id {
                DOWNLOADED.write().insert(job.track.video_id.clone(), id);
            }
            finished(&job, true);
        }
        DownloadStatus::Failed => finished(&job, false),
        _ => {}
    }
}

fn upsert(job: DownloadJob) {
    let mut jobs = JOBS.write();
    if let Some(j) = jobs.iter_mut().find(|j| j.id == job.id) {
        *j = job;
    } else {
        jobs.insert(0, job);
    }
}

/// Meldungen: wenige Titel einzeln, viele am Ende zusammengefasst.
fn finished(job: &DownloadJob, ok: bool) {
    let open = JOBS.peek().iter().filter(|j| is_open(j.status)).count() as u32;
    let mut b = BATCH.with(|b| b.get());
    if !b.active {
        b = Batch { done: 0, failed: 0, each: open < 3, active: true };
    }
    if ok {
        b.done += 1;
    } else {
        b.failed += 1;
    }
    if b.each {
        if ok {
            toast(trf("„{0}“ heruntergeladen.", &[&job.track.title]));
        } else {
            toast_error(trf("„{0}“ konnte nicht heruntergeladen werden: {1}", &[&job.track.title, &job.error.as_deref().map(tr).unwrap_or_default()]));
        }
    }
    if open == 0 {
        if !b.each {
            if b.done > 0 {
                toast(if b.done == 1 { tr("1 Titel heruntergeladen.") } else { trf("{0} Titel heruntergeladen.", &[&b.done]) });
            }
            if b.failed > 0 {
                toast_error(if b.failed == 1 {
                    tr("1 Download fehlgeschlagen – Details unter „Downloads“.")
                } else {
                    trf("{0} Downloads fehlgeschlagen – Details unter „Downloads“.", &[&b.failed])
                });
            }
        }
        b = Batch::default();
    }
    BATCH.with(|c| c.set(b));
}

// ------------------------------------------------------------------ Aktionen

/// Titel herunterladen: schon geladene und laufende auslassen, fehlgeschlagene neu versuchen.
pub async fn enqueue(tracks: Vec<YtTrack>) {
    let mut seen = HashSet::new();
    let mut fresh = Vec::new();
    let mut retry_ids = Vec::new();
    for t in tracks {
        if !seen.insert(t.video_id.clone()) || DOWNLOADED.peek().contains_key(&t.video_id) {
            continue;
        }
        let existing = JOBS.peek().iter().find(|j| j.track.video_id == t.video_id).map(|j| (j.id, j.status));
        match existing {
            Some((_, s)) if is_open(s) => {}
            Some((id, DownloadStatus::Failed)) => retry_ids.push(id),
            _ => fresh.push(t),
        }
    }
    if fresh.is_empty() && retry_ids.is_empty() {
        toast(tr("Schon auf dem Server oder in der Warteschlange."));
        return;
    }
    let mut n = 0usize;
    for id in retry_ids {
        match api::action_json::<DownloadJob>(&format!("/downloads/{id}/retry")).await {
            Ok(j) => {
                upsert(j);
                n += 1;
            }
            Err(e) => {
                report(e);
                return;
            }
        }
    }
    if !fresh.is_empty() {
        // Dauert es (yt-dlp wird nach dem Serverstart noch eingerichtet), kurz Bescheid geben
        let done = std::rc::Rc::new(Cell::new(false));
        let flag = done.clone();
        spawn_forever(async move {
            gloo_timers::future::TimeoutFuture::new(1500).await;
            if !flag.get() {
                toast(tr("Downloads werden vorbereitet – das kann beim ersten Mal etwas dauern …"));
            }
        });
        *PENDING.write() += 1;
        let res = api::post::<Vec<DownloadJob>, _>("/downloads", &DownloadRequest { tracks: fresh }).await;
        done.set(true);
        *PENDING.write() -= 1;
        match res {
            Ok(jobs) => {
                n += jobs.iter().filter(|j| is_open(j.status)).count();
                for j in jobs {
                    if j.status == DownloadStatus::Done
                        && let Some(id) = j.track_id
                    {
                        DOWNLOADED.write().insert(j.track.video_id.clone(), id);
                    }
                    upsert(j);
                }
            }
            Err(e) => {
                report(e);
                return;
            }
        }
    }
    if n == 0 {
        toast(tr("Schon auf dem Server oder in der Warteschlange."));
    } else {
        toast(if n == 1 { tr("1 Titel zum Herunterladen eingereiht.") } else { trf("{0} Titel zum Herunterladen eingereiht.", &[&n]) });
    }
}

/// Wie [`enqueue`], bei vielen Titeln mit Rückfrage. Nur Titel, die noch fehlen, zählen.
pub async fn enqueue_many(tracks: Vec<YtTrack>) {
    let missing = tracks.iter().filter(|t| !DOWNLOADED.peek().contains_key(&t.video_id)).count();
    if missing >= CONFIRM_FROM {
        let ask = Ask::new(
            trf("{0} Titel herunterladen?", &[&missing]),
            tr("Sie werden nacheinander mit yt-dlp in den Musikordner geladen (unter „YouTube/…“) und erscheinen danach in der Bibliothek."),
        )
        .confirm(tr("Herunterladen"));
        if !confirm(ask).await {
            return;
        }
    }
    enqueue(tracks).await;
}

/// Kurzform für Knöpfe und Menüs.
pub fn download(tracks: Vec<YtTrack>) {
    spawn_forever(enqueue_many(tracks));
}

/// Fehler einer Aktion melden. `OFF` kommt nur von der Liste (`GET /downloads`): fehlt bloss
/// yt-dlp, bleibt die Liste sichtbar und die Seite zeigt, was fehlt.
fn report(e: api::ApiError) {
    toast_error(e);
}

pub fn retry(id: i64) {
    spawn_forever(async move {
        match api::action_json::<DownloadJob>(&format!("/downloads/{id}/retry")).await {
            Ok(j) => {
                upsert(j);
                toast(tr("Wird erneut versucht."));
            }
            Err(e) => report(e),
        }
    });
}

/// Laufenden Auftrag abbrechen bzw. erledigten aus der Liste nehmen (die Datei bleibt).
pub fn remove(id: i64) {
    spawn_forever(async move {
        match api::delete(&format!("/downloads/{id}")).await {
            Ok(()) => {
                let open = JOBS.peek().iter().find(|j| j.id == id).is_some_and(|j| is_open(j.status));
                if open {
                    if let Some(j) = JOBS.write().iter_mut().find(|j| j.id == id) {
                        j.status = DownloadStatus::Canceled;
                    }
                    toast(tr("Download abgebrochen."));
                } else {
                    JOBS.write().retain(|j| j.id != id);
                }
            }
            Err(e) => report(e),
        }
    });
}

/// Erledigte, fehlgeschlagene und abgebrochene Aufträge aus der Liste nehmen.
pub fn clear_finished() {
    spawn_forever(async move {
        match api::delete("/downloads?finished=true").await {
            Ok(()) => JOBS.write().retain(|j| is_open(j.status)),
            Err(e) => report(e),
        }
    });
}
