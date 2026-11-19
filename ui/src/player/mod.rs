//! Player: ein globaler Zustand (Warteschlange, Position, Lautstärke, Zufall, Wiederholen …)
//! in Dioxus-Signalen und ein einziges `<audio>`-Element ([`audio`]) für alles: lokale Titel
//! von `/api/v1/tracks/<id>/stream`, YouTube-Titel von `/api/v1/yt/stream/<videoId>` (der
//! Server holt die Tonspur mit yt-dlp – kein IFrame, keine Werbung). Rückrufe des Browsers
//! (Audio-Ereignisse, Media Session, Tastatur) laufen ausserhalb von Dioxus und schicken nur
//! Nachrichten über einen Kanal; [`run`] verarbeitet sie in der App.

pub mod audio;
pub mod keyboard;
pub mod media_session;
mod persist;

use crate::api;
use crate::components::overlay::{toast, toast_error, toast_link};
use crate::i18n::{tr, trf};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use nrzmusic_types::{FeedPage, FeedRequest, ListenReport, PlayableTrack, YtTrack};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};

// ------------------------------------------------------------------ Zustand

/// Ein Eintrag der Warteschlange; `uid` unterscheidet denselben Titel, wenn er mehrfach drin ist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueItem {
    pub uid: u64,
    pub track: PlayableTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// Ob gerade ein Titel geladen ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    None,
    Local,
}

pub static QUEUE: GlobalSignal<Vec<QueueItem>> = Signal::global(Vec::new);
pub static INDEX: GlobalSignal<Option<usize>> = Signal::global(|| None);
pub static PLAYING: GlobalSignal<bool> = Signal::global(|| false);
/// Sekunden
pub static POSITION: GlobalSignal<f64> = Signal::global(|| 0.0);
/// Sekunden (0 = unbekannt)
pub static DURATION: GlobalSignal<f64> = Signal::global(|| 0.0);
/// 0.0 … 1.0
pub static VOLUME: GlobalSignal<f64> = Signal::global(|| 0.8);
pub static MUTED: GlobalSignal<bool> = Signal::global(|| false);
pub static SHUFFLE: GlobalSignal<bool> = Signal::global(|| false);
pub static REPEAT: GlobalSignal<Repeat> = Signal::global(Repeat::default);
pub static BUFFERING: GlobalSignal<bool> = Signal::global(|| false);
pub static ERROR: GlobalSignal<Option<String>> = Signal::global(|| None);
pub static ACTIVE: GlobalSignal<Backend> = Signal::global(Backend::default);
/// Am Ende der Warteschlange weitermachen (YouTube-Radio bzw. Feed) – nie einfach aufhören
pub static AUTOPLAY_RADIO: GlobalSignal<bool> = Signal::global(|| true);
/// Die Warteschlange kommt aus dem Feed ("Für dich") und wird von dort endlos nachgefüllt
pub static FEED_MODE: GlobalSignal<bool> = Signal::global(|| false);

/// Ansicht: "Läuft gerade" (gross) und die Warteschlange (Seitenleiste)
pub static NOW_PLAYING_OPEN: GlobalSignal<bool> = Signal::global(|| false);
pub static QUEUE_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Ursprüngliche Reihenfolge (uids), solange Zufall an ist.
static ORIGINAL: GlobalSignal<Option<Vec<u64>>> = Signal::global(|| None);

thread_local! {
    static TX: RefCell<Option<UnboundedSender<Ev>>> = const { RefCell::new(None) };
    static NEXT_UID: Cell<u64> = const { Cell::new(1) };
    /// Gehörte Sekunden des aktuellen Titels und ob er schon gezählt ist
    static LISTENED: Cell<(f64, bool)> = const { Cell::new((0.0, false)) };
    /// Aufeinanderfolgende Fehler (gegen endloses Überspringen)
    static FAILS: Cell<u32> = const { Cell::new(0) };
    /// Laufende Wiedergabe fürs Hörverhalten (Feed, Statistik)
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// Neue Versuche für den aktuellen Eintrag (Hänger, Netzwerkfehler)
    static RETRIES: Cell<u32> = const { Cell::new(0) };
    /// Feed wird gerade nachgeladen
    static FEED_LOADING: Cell<bool> = const { Cell::new(false) };
    /// Browser hat play() abgelehnt (z. B. im Hintergrund) – beim Zurückkommen weiterspielen
    static RESUME_ON_VISIBLE: Cell<bool> = const { Cell::new(false) };
}

/// Eine Wiedergabe: welcher Eintrag, wie lange wirklich gehört.
struct Session {
    uid: u64,
    track: PlayableTrack,
    heard: f64,
    duration: f64,
    feed: bool,
}

/// Neue Wiedergabe des aktuellen Eintrags beginnen.
fn start_session() {
    let (Some(uid), Some(track)) = (current_uid(), current_peek()) else { return };
    let duration = track.duration_ms().map(|d| d as f64 / 1000.0).unwrap_or(0.0);
    let feed = *FEED_MODE.peek();
    SESSION.with(|s| *s.borrow_mut() = Some(Session { uid, track, heard: 0.0, duration, feed }));
}

/// Bericht über eine beendete Wiedergabe (`None`, wenn kaum gehört).
fn session_report(s: Session, completed: bool) -> Option<ListenReport> {
    if s.heard < 1.0 && !completed {
        return None;
    }
    let dur = if s.duration > 0.0 { s.duration } else { s.track.duration_ms().unwrap_or(0) as f64 / 1000.0 };
    let skipped = !completed && if dur > 0.0 { s.heard < dur * 0.9 } else { s.heard < 30.0 };
    Some(ListenReport {
        track: s.track.to_ref(),
        listened_ms: (s.heard * 1000.0) as u64,
        duration_ms: (dur * 1000.0) as u64,
        skipped,
        completed,
        source: if s.feed { "feed".into() } else { String::new() },
    })
}

/// Wiedergabe abschliessen und melden. `completed`: bis zum Ende gelaufen.
fn finish_session(completed: bool) {
    let Some(s) = SESSION.with(|x| x.borrow_mut().take()) else { return };
    if let Some(report) = session_report(s, completed) {
        spawn_forever(async move {
            if api::post::<serde_json::Value, _>("/listens", &report).await.is_ok() {
                *crate::state::STATS_VERSION.write() += 1;
            }
        });
    }
}

/// Seite wird geschlossen: laufende Wiedergabe noch schnell melden (sendBeacon).
fn beacon_session() {
    let Some(s) = SESSION.with(|x| x.borrow_mut().take()) else { return };
    let (uid, track, duration, feed) = (s.uid, s.track.clone(), s.duration, s.feed);
    if let Some(report) = session_report(s, false)
        && let Ok(body) = serde_json::to_string(&ListenReport { skipped: false, ..report })
        && let Some(w) = web_sys::window()
    {
        let _ = w.navigator().send_beacon_with_opt_str(&format!("{}/listens", api::BASE), Some(&body));
    }
    // Kommt die Seite zurück (bfcache), zählt ab hier neu
    SESSION.with(|x| *x.borrow_mut() = Some(Session { uid, track, heard: 0.0, duration, feed }));
}

fn new_uid() -> u64 {
    NEXT_UID.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    })
}

fn items(tracks: Vec<PlayableTrack>) -> Vec<QueueItem> {
    tracks.into_iter().map(|track| QueueItem { uid: new_uid(), track }).collect()
}

/// Nachrichten an den Player (aus Browser-Rückrufen).
pub enum Ev {
    AudioTime(f64),
    AudioDuration(f64),
    AudioPlaying,
    AudioPause,
    AudioEnded,
    AudioWaiting,
    AudioError(String),
    /// play() wurde vom Browser abgelehnt (Autoplay-Sperre)
    AudioBlocked,
    /// Ein vorladender Eintrag (uid) konnte nicht geladen werden
    PreloadFailed(u64),
    Cmd(Cmd),
}

#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
    SeekBy(f64),
    SeekTo(f64),
    Mute,
    FocusSearch,
    /// Esc: "Läuft gerade" bzw. Warteschlange schliessen
    Close,
}

/// Nachricht an den Player schicken (aus Rückrufen ausserhalb von Dioxus).
pub fn send(ev: Ev) {
    TX.with(|t| {
        if let Some(tx) = t.borrow().as_ref() {
            let _ = tx.unbounded_send(ev);
        }
    });
}

// ------------------------------------------------------------------ Abfragen

/// Aktueller Titel.
pub fn current() -> Option<PlayableTrack> {
    let i = (*INDEX.read())?;
    QUEUE.read().get(i).map(|q| q.track.clone())
}

/// Aktueller Titel ohne Abhängigkeit (für Ereignis-Code).
fn current_peek() -> Option<PlayableTrack> {
    let i = (*INDEX.peek())?;
    QUEUE.peek().get(i).map(|q| q.track.clone())
}

/// Kennung des aktuellen Eintrags der Warteschlange.
pub fn current_uid() -> Option<u64> {
    let i = (*INDEX.peek())?;
    QUEUE.peek().get(i).map(|q| q.uid)
}

/// Eintrag noch in der Warteschlange (z. B. um zu merken, dass inzwischen etwas anderes läuft).
pub fn queue_contains(uid: u64) -> bool {
    QUEUE.peek().iter().any(|q| q.uid == uid)
}

/// Der aktuelle YouTube-Titel liegt heruntergeladen im Musikordner.
pub fn playing_download() -> bool {
    matches!(current(), Some(PlayableTrack::Youtube(t)) if crate::downloads::local_id(&t.video_id).is_some())
}

/// Schlüssel des aktuellen Titels (zum Hervorheben in Listen).
pub fn current_key() -> Option<String> {
    current().map(|t| t.key())
}

// ------------------------------------------------------------------ Warteschlange

/// Liste ersetzen und ab `start` spielen. Bei Zufall bleibt der gewählte Titel vorne.
pub fn play_now(tracks: Vec<PlayableTrack>, start: usize) {
    if tracks.is_empty() {
        return;
    }
    *FEED_MODE.write() = false;
    let start = start.min(tracks.len() - 1);
    let mut q = items(tracks);
    if *SHUFFLE.peek() {
        *ORIGINAL.write() = Some(q.iter().map(|i| i.uid).collect());
        let first = q.remove(start);
        crate::util::shuffle(&mut q);
        q.insert(0, first);
        *QUEUE.write() = q;
        *INDEX.write() = Some(0);
    } else {
        *ORIGINAL.write() = None;
        *QUEUE.write() = q;
        *INDEX.write() = Some(start);
    }
    load_current(true, 0.0);
}

/// Feed ab `start` spielen: der Feed füllt die Warteschlange danach endlos nach.
pub fn play_feed(tracks: Vec<PlayableTrack>, start: usize) {
    if tracks.is_empty() {
        return;
    }
    *SHUFFLE.write() = false;
    play_now(tracks, start);
    *FEED_MODE.write() = true;
    SESSION.with(|s| {
        if let Some(s) = s.borrow_mut().as_mut() {
            s.feed = true;
        }
    });
    refill_feed();
}

/// Liste gemischt spielen (Zufall wird eingeschaltet).
pub fn play_shuffled(tracks: Vec<PlayableTrack>) {
    if tracks.is_empty() {
        return;
    }
    *SHUFFLE.write() = true;
    let start = crate::util::random_index(tracks.len());
    play_now(tracks, start);
}

/// Direkt nach dem aktuellen Titel einreihen.
pub fn play_next(tracks: Vec<PlayableTrack>) {
    if tracks.is_empty() {
        return;
    }
    if QUEUE.peek().is_empty() || INDEX.peek().is_none() {
        play_now(tracks, 0);
        return;
    }
    let n = tracks.len();
    let new = items(tracks);
    let idx = INDEX.peek().unwrap_or(0);
    let cur_uid = QUEUE.peek().get(idx).map(|q| q.uid);
    {
        let mut q = QUEUE.write();
        let at = (idx + 1).min(q.len());
        for (k, it) in new.iter().cloned().enumerate() {
            q.insert(at + k, it);
        }
    }
    if let Some(orig) = ORIGINAL.write().as_mut() {
        let at = cur_uid.and_then(|u| orig.iter().position(|x| *x == u)).map(|p| p + 1).unwrap_or(orig.len());
        for (k, it) in new.iter().enumerate() {
            orig.insert(at + k, it.uid);
        }
    }
    toast(if n == 1 { tr("Wird als Nächstes gespielt.") } else { trf("{0} Titel werden als Nächstes gespielt.", &[&n]) });
    persist::save();
}

/// Hinten an die Warteschlange anhängen.
pub fn add_to_queue(tracks: Vec<PlayableTrack>) {
    if tracks.is_empty() {
        return;
    }
    if QUEUE.peek().is_empty() || INDEX.peek().is_none() {
        play_now(tracks, 0);
        return;
    }
    let n = tracks.len();
    let new = items(tracks);
    if let Some(orig) = ORIGINAL.write().as_mut() {
        orig.extend(new.iter().map(|i| i.uid));
    }
    QUEUE.write().extend(new);
    toast(if n == 1 { tr("Zur Warteschlange hinzugefügt.") } else { trf("{0} Titel zur Warteschlange hinzugefügt.", &[&n]) });
    persist::save();
}

/// Hinten anhängen ohne Meldung (Spotify: weitere Treffer kommen nach und nach).
pub fn extend_queue(tracks: Vec<PlayableTrack>) {
    if tracks.is_empty() {
        return;
    }
    let new = items(tracks);
    if let Some(orig) = ORIGINAL.write().as_mut() {
        orig.extend(new.iter().map(|i| i.uid));
    }
    QUEUE.write().extend(new);
    persist::save();
}

/// Eintrag entfernen; war es der laufende Titel, geht es mit dem nächsten weiter.
pub fn remove(uid: u64) {
    let Some(pos) = QUEUE.peek().iter().position(|q| q.uid == uid) else { return };
    let idx = *INDEX.peek();
    QUEUE.write().remove(pos);
    if let Some(orig) = ORIGINAL.write().as_mut() {
        orig.retain(|u| *u != uid);
    }
    let len = QUEUE.peek().len();
    match idx {
        Some(i) if pos < i => *INDEX.write() = Some(i - 1),
        Some(i) if pos == i => {
            if len == 0 {
                stop_all();
            } else {
                let was_playing = *PLAYING.peek();
                *INDEX.write() = Some(i.min(len - 1));
                load_current(was_playing, 0.0);
            }
        }
        _ => {}
    }
    persist::save();
}

/// Eintrag verschieben (Indizes in der Warteschlange).
pub fn move_item(from: usize, to: usize) {
    let len = QUEUE.peek().len();
    if from >= len || to >= len || from == to {
        return;
    }
    {
        let mut q = QUEUE.write();
        let it = q.remove(from);
        q.insert(to, it);
    }
    // Wert erst kopieren: ein offener Lesezugriff beim Schreiben bringt die App zum Absturz
    let index = *INDEX.peek();
    if let Some(i) = index {
        let ni = if i == from {
            to
        } else if from < i && to >= i {
            i - 1
        } else if from > i && to <= i {
            i + 1
        } else {
            i
        };
        *INDEX.write() = Some(ni);
    }
    // Von Hand umgestellt: das ist jetzt die Reihenfolge
    *ORIGINAL.write() = if *SHUFFLE.peek() { Some(QUEUE.peek().iter().map(|q| q.uid).collect()) } else { None };
    persist::save();
}

/// Alles ausser dem laufenden Titel entfernen.
pub fn clear_queue() {
    let cur = INDEX.peek().and_then(|i| QUEUE.peek().get(i).cloned());
    match cur {
        Some(c) => {
            *ORIGINAL.write() = if *SHUFFLE.peek() { Some(vec![c.uid]) } else { None };
            *QUEUE.write() = vec![c];
            *INDEX.write() = Some(0);
        }
        None => {
            *QUEUE.write() = Vec::new();
            *INDEX.write() = None;
            *ORIGINAL.write() = None;
        }
    }
    persist::save();
}

/// Zu einem Eintrag springen.
pub fn jump(index: usize) {
    if index < QUEUE.peek().len() {
        *INDEX.write() = Some(index);
        load_current(true, 0.0);
    }
}

/// Zufall an/aus. An: aktueller Titel vorne, der Rest gemischt. Aus: alte Reihenfolge zurück.
pub fn toggle_shuffle() {
    let on = !*SHUFFLE.peek();
    *SHUFFLE.write() = on;
    let cur_uid = INDEX.peek().and_then(|i| QUEUE.peek().get(i).map(|q| q.uid));
    if on {
        let mut q = QUEUE.peek().clone();
        *ORIGINAL.write() = Some(q.iter().map(|i| i.uid).collect());
        let first = cur_uid.and_then(|u| q.iter().position(|x| x.uid == u)).map(|p| q.remove(p));
        crate::util::shuffle(&mut q);
        if let Some(f) = first {
            q.insert(0, f);
            *INDEX.write() = Some(0);
        }
        *QUEUE.write() = q;
    } else if let Some(orig) = ORIGINAL.write().take() {
        let mut q = QUEUE.peek().clone();
        q.sort_by_key(|it| orig.iter().position(|u| *u == it.uid).unwrap_or(usize::MAX));
        let idx = cur_uid.and_then(|u| q.iter().position(|x| x.uid == u));
        *QUEUE.write() = q;
        if idx.is_some() {
            *INDEX.write() = idx;
        }
    }
    persist::save();
}

pub fn cycle_repeat() {
    let next = match *REPEAT.peek() {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    };
    *REPEAT.write() = next;
    persist::save();
}

/// Aktuelle Warteschlange als neue Playlist speichern.
pub fn save_queue_as_playlist(name: String) {
    let refs: Vec<_> = QUEUE.peek().iter().map(|q| q.track.to_ref()).collect();
    if refs.is_empty() {
        return;
    }
    spawn_forever(async move {
        let _ = crate::state::create_playlist(name, String::new(), refs).await;
    });
}

// ------------------------------------------------------------------ Steuerung

/// Den aktuellen Titel im passenden Wiedergabeweg laden.
fn load_current(autoplay: bool, start: f64) {
    let Some(track) = current_peek() else {
        stop_all();
        return;
    };
    // Derselbe Eintrag wird nur neu geladen (Fehler, Hänger): die Wiedergabe läuft weiter
    let same = SESSION.with(|s| s.borrow().as_ref().map(|s| s.uid)) == current_uid();
    // Wie wahrscheinlich geht es zurück? Früh übersprungen: kaum (versehentlich), mitten im
    // Titel: eher, zu Ende gelaufen (keine Sitzung mehr): selten.
    let back_ms = SESSION
        .with(|s| s.borrow().as_ref().map(|s| if s.heard < 10.0 { 8_000.0 } else { 30_000.0 }))
        .unwrap_or(15_000.0);
    if !same {
        finish_session(false);
        start_session();
        RETRIES.with(|r| r.set(0));
    }
    *POSITION.write() = start;
    *DURATION.write() = track.duration_ms().map(|d| d as f64 / 1000.0).unwrap_or(0.0);
    *ERROR.write() = None;
    *BUFFERING.write() = autoplay;
    *PLAYING.write() = autoplay;
    LISTENED.with(|l| l.set((0.0, false)));
    *ACTIVE.write() = Backend::Local;
    let uid = current_uid().unwrap_or(0);
    audio::load(uid, &stream_url(&track), start, autoplay, same);
    if !same {
        audio::keep_previous(back_ms);
    }
    refresh_preload();
    refill_feed();
    media_session::set_metadata(Some(&track));
    media_session::set_playing(autoplay);
    persist::save();
}

/// Gemerkte Warteschlange vergessen (Abmelden: das nächste Konto soll sie nicht sehen).
pub fn persist_clear() {
    stop_all();
    persist::clear();
}

/// Adresse der Tonspur: lokale Datei, heruntergeladener YouTube-Titel oder die Tonspur, die
/// der Server mit yt-dlp holt.
pub fn stream_url(track: &PlayableTrack) -> String {
    match track {
        PlayableTrack::Local(t) => format!("/api/v1/tracks/{}/stream", t.id),
        PlayableTrack::Youtube(t) => match crate::downloads::local_id(&t.video_id) {
            Some(id) => format!("/api/v1/tracks/{id}/stream"),
            None => format!("/api/v1/yt/stream/{}", t.video_id),
        },
    }
}

/// Wenig Datenvolumen gewünscht (Datensparmodus des Geräts)?
fn save_data() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w.navigator(), &"connection".into()).ok())
        .filter(|c| c.is_object())
        .and_then(|c| js_sys::Reflect::get(&c, &"saveData".into()).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Die nächsten Einträge (ohne den aktuellen), wie sie gespielt würden.
fn upcoming(n: usize) -> Vec<QueueItem> {
    let Some(i) = *INDEX.peek() else { return Vec::new() };
    if *REPEAT.peek() == Repeat::One {
        return Vec::new();
    }
    let q = QUEUE.peek();
    let len = q.len();
    let wrap = *REPEAT.peek() == Repeat::All;
    (1..=n)
        .filter_map(|k| {
            let j = i + k;
            if j < len { Some(j) } else if wrap && len > 1 { Some(j % len) } else { None }
        })
        .filter(|j| *j != i)
        .filter_map(|j| q.get(j).cloned())
        .collect()
}

/// Vorladen: der nächste Titel ganz (sobald der aktuelle ein paar Sekunden läuft – sonst
/// teilen sie sich die Leitung), der übernächste nur die Kopfdaten; im Datensparmodus nur
/// der nächste. Die nächsten drei YouTube-Titel holt der Server vorab ganz (`prepare`) –
/// nicht verfügbare ersetzt der Player durch eine andere Fassung oder nimmt sie heraus.
/// Läuft bei jedem Wechsel und alle 2 s; ohne Änderung kostet es fast nichts.
fn refresh_preload() {
    if *ACTIVE.peek() != Backend::Local {
        return;
    }
    let started = *POSITION.peek() >= 4.0 || !*PLAYING.peek();
    let n = if save_data() { 1 } else { 2 };
    let next = upcoming(n);
    let wanted: Vec<(u64, String, audio::Level)> = next
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let level = if k == 0 && started && !save_data() { audio::Level::Auto } else { audio::Level::Metadata };
            (it.uid, stream_url(&it.track), level)
        })
        .collect();
    audio::preload(&wanted);
    // Der Server lädt die nächsten drei YouTube-Titel ganz in seinen Zwischenspeicher – dann
    // beginnen sie ohne Wartezeit (auch beim schnellen Weiterspringen)
    for it in upcoming(3) {
        if let PlayableTrack::Youtube(t) = &it.track
            && crate::downloads::local_id(&t.video_id).is_none()
        {
            check_available(it.uid, t.clone());
        }
    }
}

thread_local! {
    /// Schon geprüfte YouTube-Videos (einmal pro Sitzung reicht)
    static CHECKED: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}

/// Einmal prüfen, ob YouTube die Tonspur herausgibt (der Server merkt sich das Ergebnis und
/// hat die Adresse danach bereit – der Titel beginnt dann ohne Pause).
fn check_available(uid: u64, t: YtTrack) {
    let first = CHECKED.with(|c| c.borrow_mut().insert(t.video_id.clone()));
    if !first {
        return;
    }
    spawn_forever(async move {
        if let Err(e) = api::action(&format!("/yt/stream/{}/prepare", t.video_id)).await
            && e.status == 410
        {
            replace_unavailable(uid, t).await;
        }
    });
}

/// "Fast gleich" für Titel: ohne Zusätze in Klammern, klein.
fn base_title(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for ch in t.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.extend(ch.to_lowercase()),
            _ => {}
        }
    }
    let out = out.split(" - ").next().unwrap_or("").to_string();
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Nicht verfügbarer YouTube-Titel: dieselbe Aufnahme in einer anderen Fassung suchen und
/// still austauschen; gibt es keine, aus der Warteschlange nehmen (läuft er gerade: weiter).
async fn replace_unavailable(uid: u64, t: YtTrack) {
    let artist = t.artists.first().map(|a| a.name.to_lowercase()).unwrap_or_default();
    let base = base_title(&t.title);
    let q = format!("{} {}", t.artist_line(), t.title);
    let found = match api::get::<nrzmusic_types::YtSearchResults>(&format!("/yt/search?q={}&filter=songs", api::enc(&q))).await {
        Ok(r) => r.songs.into_iter().find(|c| {
            c.video_id != t.video_id
                && base_title(&c.title) == base
                && c.artists.iter().any(|a| a.name.to_lowercase() == artist)
                && match (c.duration_ms, t.duration_ms) {
                    (Some(a), Some(b)) => a.abs_diff(b) <= 20_000,
                    _ => true,
                }
        }),
        Err(_) => None,
    };
    let Some(pos) = QUEUE.peek().iter().position(|q| q.uid == uid) else { return };
    let is_current = current_uid() == Some(uid);
    audio::forget(uid);
    match found {
        Some(alt) => {
            QUEUE.write()[pos].track = PlayableTrack::Youtube(alt);
            persist::save();
            if is_current {
                let playing = *PLAYING.peek();
                load_current(playing || *BUFFERING.peek(), 0.0);
            } else {
                refresh_preload();
            }
        }
        None => {
            toast(trf("„{0}“ ist auf YouTube Music nicht verfügbar – übersprungen.", &[&t.title]));
            if is_current {
                SESSION.with(|s| s.borrow_mut().take());
                skip_after_error();
            } else {
                remove(uid);
                refresh_preload();
            }
        }
    }
}

fn stop_all() {
    finish_session(false);
    audio::stop();
    *ACTIVE.write() = Backend::None;
    *PLAYING.write() = false;
    *BUFFERING.write() = false;
    *POSITION.write() = 0.0;
    media_session::set_metadata(None);
    media_session::set_playing(false);
}

pub fn play() {
    if INDEX.peek().is_none() {
        if !QUEUE.peek().is_empty() {
            jump(0);
        }
        return;
    }
    if *ACTIVE.peek() == Backend::None {
        let pos = *POSITION.peek();
        load_current(true, pos);
        return;
    }
    *PLAYING.write() = true;
    if SESSION.with(|s| s.borrow().is_none()) {
        start_session();
    }
    if *ACTIVE.peek() == Backend::Local {
        audio::play();
    }
    media_session::set_playing(true);
}

pub fn pause() {
    *PLAYING.write() = false;
    if *ACTIVE.peek() == Backend::Local {
        audio::pause();
    }
    media_session::set_playing(false);
    persist::save();
}

pub fn toggle() {
    if *PLAYING.peek() { pause() } else { play() }
}

pub fn seek(secs: f64) {
    let dur = *DURATION.peek();
    let secs = if dur > 0.0 { secs.clamp(0.0, dur) } else { secs.max(0.0) };
    *POSITION.write() = secs;
    if *ACTIVE.peek() == Backend::Local {
        audio::seek(secs);
    }
    media_session::set_position(secs, dur);
}

pub fn seek_by(delta: f64) {
    let p = *POSITION.peek();
    seek(p + delta);
}

pub fn set_volume(v: f64) {
    let v = v.clamp(0.0, 1.0);
    *VOLUME.write() = v;
    if v > 0.0 && *MUTED.peek() {
        *MUTED.write() = false;
    }
    apply_volume();
    persist::save();
}

pub fn toggle_mute() {
    let m = !*MUTED.peek();
    *MUTED.write() = m;
    apply_volume();
    persist::save();
}

fn apply_volume() {
    let (v, m) = (*VOLUME.peek(), *MUTED.peek());
    audio::set_volume(v, m);
}

/// Nächster Titel. `auto`: der vorige ist zu Ende gelaufen.
pub fn next() {
    advance(false);
}

fn advance(auto: bool) {
    let len = QUEUE.peek().len();
    let Some(i) = *INDEX.peek() else { return };
    if auto && *REPEAT.peek() == Repeat::One {
        seek(0.0);
        play();
        start_session();
        return;
    }
    if i + 1 < len {
        *INDEX.write() = Some(i + 1);
        load_current(true, 0.0);
    } else if *REPEAT.peek() == Repeat::All && len > 0 {
        *INDEX.write() = Some(0);
        load_current(true, 0.0);
    } else if *AUTOPLAY_RADIO.peek() && len > 0 {
        continue_endless();
    } else {
        // Ende der Warteschlange: anhalten, zurück an den Anfang des letzten Titels
        pause();
        seek(0.0);
        *BUFFERING.write() = false;
    }
}

pub fn prev() {
    if *POSITION.peek() > 3.0 {
        seek(0.0);
        return;
    }
    let index = *INDEX.peek();
    match index {
        Some(i) if i > 0 => {
            *INDEX.write() = Some(i - 1);
            load_current(true, 0.0);
        }
        Some(_) if *REPEAT.peek() == Repeat::All && QUEUE.peek().len() > 1 => {
            let last = QUEUE.peek().len() - 1;
            *INDEX.write() = Some(last);
            load_current(true, 0.0);
        }
        _ => seek(0.0),
    }
}

fn on_ended() {
    finish_session(true);
    advance(true);
}

// ------------------------------------------------------------------ YouTube-Radio

async fn fetch_radio(video_id: &str) -> Vec<PlayableTrack> {
    match api::get::<Vec<YtTrack>>(&format!("/yt/radio/{}", api::enc(video_id))).await {
        Ok(list) => {
            let known: std::collections::HashSet<String> = QUEUE.peek().iter().map(|q| q.track.key()).collect();
            list.into_iter()
                .filter(|t| t.video_id != video_id)
                .map(PlayableTrack::Youtube)
                .filter(|t| !known.contains(&t.key()))
                .collect()
        }
        Err(e) => {
            toast_error(e);
            Vec::new()
        }
    }
}

/// Nächste Titel aus dem Feed (ohne die schon in der Warteschlange).
async fn fetch_feed(count: u32) -> Vec<PlayableTrack> {
    let queue: Vec<String> = QUEUE.peek().iter().map(|q| q.track.key()).collect();
    let exclude = queue.iter().rev().take(400).cloned().collect();
    match api::post::<FeedPage, _>("/feed", &FeedRequest { exclude, count: Some(count) }).await {
        Ok(p) => {
            let known: std::collections::HashSet<String> = queue.into_iter().collect();
            p.items.into_iter().map(|i| i.track).filter(|t| !known.contains(&t.key())).collect()
        }
        Err(_) => Vec::new(),
    }
}

/// Feed-Modus: rechtzeitig nachladen, damit die Warteschlange nie leerläuft.
fn refill_feed() {
    if !*FEED_MODE.peek() || FEED_LOADING.with(|f| f.get()) {
        return;
    }
    let Some(i) = *INDEX.peek() else { return };
    if QUEUE.peek().len().saturating_sub(i + 1) >= 4 {
        return;
    }
    FEED_LOADING.with(|f| f.set(true));
    spawn_forever(async move {
        let more = fetch_feed(15).await;
        FEED_LOADING.with(|f| f.set(false));
        if *FEED_MODE.peek() && !more.is_empty() {
            extend_queue(more);
            refresh_preload();
        }
    });
}

/// Am Ende der Warteschlange: weiter mit dem Feed (Feed-Modus, lokale Titel) bzw. dem
/// YouTube-Radio zum letzten Titel – und wenn das nichts bringt, mit dem anderen.
fn continue_endless() {
    let cur = current_peek();
    let uid = current_uid();
    *BUFFERING.write() = true;
    spawn_forever(async move {
        let mut more = Vec::new();
        let mut via_feed = false;
        if !*FEED_MODE.peek()
            && let Some(PlayableTrack::Youtube(t)) = &cur
        {
            more = fetch_radio(&t.video_id).await;
        }
        if more.is_empty() {
            more = fetch_feed(15).await;
            via_feed = true;
        }
        if more.is_empty()
            && let Some(PlayableTrack::Youtube(t)) = &cur
        {
            more = fetch_radio(&t.video_id).await;
            via_feed = false;
        }
        // Inzwischen etwas anderes gewählt: nichts anhängen
        if current_uid() != uid {
            return;
        }
        if more.is_empty() {
            pause();
            *BUFFERING.write() = false;
            return;
        }
        let new = items(more);
        if let Some(orig) = ORIGINAL.write().as_mut() {
            orig.extend(new.iter().map(|i| i.uid));
        }
        QUEUE.write().extend(new);
        if via_feed {
            if !*FEED_MODE.peek() {
                toast(tr("Weiter mit Titeln für dich."));
            }
            *FEED_MODE.write() = true;
        } else {
            toast(tr("Weiter mit ähnlichen Titeln (Radio)."));
        }
        let index = *INDEX.peek();
        if let Some(i) = index {
            *INDEX.write() = Some(i + 1);
            load_current(true, 0.0);
        }
    });
}

/// "Radio starten": Titel plus ähnliche Titel von YouTube Music.
pub fn start_radio(track: PlayableTrack) {
    let PlayableTrack::Youtube(t) = track.clone() else { return };
    spawn_forever(async move {
        let more = fetch_radio(&t.video_id).await;
        let same = current_peek().is_some_and(|c| c.key() == track.key());
        if same {
            // Läuft schon: alles danach ersetzen
            let idx = INDEX.peek().unwrap_or(0);
            let new = items(more);
            {
                let mut q = QUEUE.write();
                q.truncate(idx + 1);
                q.extend(new);
            }
            *ORIGINAL.write() = if *SHUFFLE.peek() { Some(QUEUE.peek().iter().map(|q| q.uid).collect()) } else { None };
            persist::save();
        } else {
            let mut all = vec![track];
            all.extend(more);
            // Radio immer in seiner Reihenfolge
            *SHUFFLE.write() = false;
            play_now(all, 0);
        }
        toast(tr("Radio gestartet."));
    });
}

// ------------------------------------------------------------------ Ereignisse

/// Einmal in der App starten: Wiedergabewege einrichten, Zustand wiederherstellen,
/// Nachrichten verarbeiten.
pub async fn run() {
    let (tx, mut rx) = unbounded::<Ev>();
    TX.with(|t| *t.borrow_mut() = Some(tx));
    if let Some(v) = crate::util::storage_get("nrzmusic-autoplay-radio") {
        *AUTOPLAY_RADIO.write() = v != "off";
    }
    audio::init();
    media_session::init();
    keyboard::init();
    persist::restore();
    apply_volume();
    page_listeners();
    spawn_forever(tick());
    while let Some(ev) = rx.next().await {
        handle(ev);
    }
}

/// Seite schliessen → Wiedergabe melden; Seite wieder sichtbar → abgelehntes play() nachholen.
fn page_listeners() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::Closure;
    let Some(w) = web_sys::window() else { return };
    let hide = Closure::<dyn FnMut()>::new(beacon_session);
    let _ = w.add_event_listener_with_callback("pagehide", hide.as_ref().unchecked_ref());
    hide.forget();
    let vis = Closure::<dyn FnMut()>::new(|| {
        let visible = crate::util::document().is_some_and(|d| d.visibility_state() == web_sys::VisibilityState::Visible);
        if visible && RESUME_ON_VISIBLE.with(|r| r.replace(false)) {
            play();
        }
    });
    if let Some(d) = crate::util::document() {
        let _ = d.add_event_listener_with_callback("visibilitychange", vis.as_ref().unchecked_ref());
    }
    vis.forget();
}

fn handle(ev: Ev) {
    let active = *ACTIVE.peek();
    match ev {
        Ev::AudioTime(t) if active == Backend::Local => {
            *POSITION.write() = t;
        }
        Ev::AudioDuration(d) if active == Backend::Local => {
            if d.is_finite() && d > 0.0 {
                *DURATION.write() = d;
            }
        }
        Ev::AudioPlaying if active == Backend::Local => {
            *PLAYING.write() = true;
            *BUFFERING.write() = false;
            FAILS.with(|f| f.set(0));
            media_session::set_playing(true);
        }
        Ev::AudioPause if active == Backend::Local => {
            *PLAYING.write() = false;
            *BUFFERING.write() = false;
            media_session::set_playing(false);
        }
        Ev::AudioWaiting if active == Backend::Local => {
            *BUFFERING.write() = true;
        }
        Ev::AudioEnded if active == Backend::Local => {
            // Mitten im Titel "zu Ende" (Verbindung abgebrochen, WebKit hält das Ende der Daten
            // für das Ende des Titels): an derselben Stelle weiter statt zum nächsten zu springen
            let (pos, dur) = (*POSITION.peek(), *DURATION.peek());
            let tries = RETRIES.with(|r| r.get());
            if dur > 30.0 && pos > 1.0 && pos < dur - 5.0 && tries < 3 {
                RETRIES.with(|r| r.set(tries + 1));
                load_current(true, pos);
            } else {
                on_ended();
            }
        }
        Ev::AudioBlocked if active == Backend::Local => {
            *PLAYING.write() = false;
            *BUFFERING.write() = false;
            RESUME_ON_VISIBLE.with(|r| r.set(true));
        }
        Ev::AudioError(msg) if active == Backend::Local => {
            // Heruntergeladene Datei eines YouTube-Titels kaputt oder weg: über YouTube weiter
            if let Some(PlayableTrack::Youtube(t)) = current_peek()
                && crate::downloads::local_id(&t.video_id).is_some()
            {
                crate::downloads::forget(&t.video_id);
                let (pos, playing) = (*POSITION.peek(), *PLAYING.peek());
                load_current(playing, pos);
                return;
            }
            // YouTube: frische Adressen antworten manchmal kurz mit Fehlern – einmal neu laden
            if matches!(current_peek(), Some(PlayableTrack::Youtube(_))) && RETRIES.with(|r| r.get()) < 1 {
                RETRIES.with(|r| r.set(r.get() + 1));
                let pos = *POSITION.peek();
                spawn_forever(async move {
                    gloo_timers::future::TimeoutFuture::new(1500).await;
                    load_current(true, pos);
                });
                return;
            }
            *ERROR.write() = Some(msg.clone());
            *BUFFERING.write() = false;
            match current_peek() {
                // YouTube: der Server weiss, warum (z. B. Video gesperrt, yt-dlp fehlt)
                Some(PlayableTrack::Youtube(t)) => {
                    let uid = current_uid();
                    spawn_forever(async move {
                        let reason = match api::action(&format!("/yt/stream/{}/prepare", t.video_id)).await {
                            // Dauerhaft weg: andere Fassung suchen bzw. still weiter
                            Err(e) if e.status == 410 => {
                                if let Some(u) = uid {
                                    replace_unavailable(u, t).await;
                                }
                                return;
                            }
                            Err(e) => e.message,
                            Ok(()) => msg,
                        };
                        toast_link(
                            trf("„{0}“ lässt sich nicht abspielen: {1}", &[&t.title, &reason]),
                            tr("Auf YouTube Music öffnen"),
                            crate::util::yt_music_url(&t.video_id),
                            true,
                        );
                        if current_uid() == uid {
                            skip_after_error();
                        }
                    });
                }
                t => {
                    let title = t.map(|t| t.title().to_string()).unwrap_or_default();
                    toast_error(trf("„{0}“ lässt sich nicht abspielen: {1}", &[&title, &msg]));
                    skip_after_error();
                }
            }
        }
        Ev::PreloadFailed(uid) => {
            audio::forget(uid);
            let track = QUEUE.peek().iter().find(|q| q.uid == uid).map(|q| q.track.clone());
            if let Some(PlayableTrack::Youtube(t)) = track {
                // Neu prüfen lassen (vielleicht nur ein Netzaussetzer)
                CHECKED.with(|c| c.borrow_mut().remove(&t.video_id));
                check_available(uid, t);
            }
        }
        Ev::Cmd(c) => command(c),
        _ => {}
    }
}

/// Nach einem Fehler zum nächsten Titel – aber nicht endlos.
fn skip_after_error() {
    let fails = FAILS.with(|f| {
        let n = f.get() + 1;
        f.set(n);
        n
    });
    let len = QUEUE.peek().len();
    let at_end = INDEX.peek().is_none_or(|i| i + 1 >= len);
    let endless = *AUTOPLAY_RADIO.peek() || *REPEAT.peek() == Repeat::All;
    if fails >= 5 || (at_end && !endless) {
        *PLAYING.write() = false;
        media_session::set_playing(false);
        return;
    }
    // Technischer Fehler – nicht als "übersprungen" lernen
    SESSION.with(|s| s.borrow_mut().take());
    advance(false);
}

pub fn command(c: Cmd) {
    match c {
        Cmd::Toggle => toggle(),
        Cmd::Play => play(),
        Cmd::Pause => pause(),
        Cmd::Next => next(),
        Cmd::Prev => prev(),
        Cmd::SeekBy(d) => seek_by(d),
        Cmd::SeekTo(t) => seek(t),
        Cmd::Mute => toggle_mute(),
        Cmd::FocusSearch => crate::util::focus("global-search"),
        Cmd::Close => {
            if crate::components::tracks::close_menu() {
                // nur das Kontextmenü
            } else if *NOW_PLAYING_OPEN.peek() {
                *NOW_PLAYING_OPEN.write() = false;
            } else if *QUEUE_OPEN.peek() {
                *QUEUE_OPEN.write() = false;
            }
        }
    }
}

/// Wiedergabe hängt: kurz vor dem Ende → als zu Ende behandeln (verpasstes `ended`), sonst
/// an derselben Stelle neu laden; hängt sie wieder, zum nächsten Titel.
fn recover_stall() {
    let (pos, dur) = (*POSITION.peek(), *DURATION.peek());
    if dur > 0.0 && pos >= dur - 3.0 {
        on_ended();
        return;
    }
    let tries = RETRIES.with(|r| {
        let n = r.get() + 1;
        r.set(n);
        n
    });
    if tries <= 2 {
        load_current(true, pos);
    } else {
        let title = current_peek().map(|t| t.title().to_string()).unwrap_or_default();
        toast(trf("„{0}“ hängt – weiter mit dem nächsten Titel.", &[&title]));
        SESSION.with(|s| s.borrow_mut().take());
        advance(false);
    }
}

/// Vier Mal pro Sekunde: gehörte Zeit zählen (Verlauf),
/// Position merken und an die Media Session melden.
async fn tick() {
    let mut last = js_sys::Date::now();
    let mut n: u64 = 0;
    let mut last_pos = -1.0;
    let mut stuck = 0.0;
    loop {
        gloo_timers::future::TimeoutFuture::new(250).await;
        n += 1;
        let now = js_sys::Date::now();
        // Im Hintergrund laufen Zeitgeber seltener – bis zu 2 s zählen
        let dt = ((now - last) / 1000.0).clamp(0.0, 2.0);
        last = now;
        if !*PLAYING.peek() || *ACTIVE.peek() != Backend::Local {
            stuck = 0.0;
            continue;
        }
        // Wächter: soll spielen, aber die Position bewegt sich nicht
        let pos = *POSITION.peek();
        if (pos - last_pos).abs() > 0.01 {
            last_pos = pos;
            stuck = 0.0;
        } else {
            stuck += dt;
            if stuck >= 8.0 {
                stuck = 0.0;
                recover_stall();
                continue;
            }
        }
        if *BUFFERING.peek() {
            continue;
        }
        SESSION.with(|s| {
            if let Some(s) = s.borrow_mut().as_mut() {
                s.heard += dt;
                let d = *DURATION.peek();
                if d > 0.0 {
                    s.duration = d;
                }
            }
        });
        // Verlauf: einmal pro Wiedergabe nach 30 s oder der halben Dauer
        let (mut heard, counted) = LISTENED.with(|l| l.get());
        heard += dt;
        let dur = *DURATION.peek();
        let needed = if dur > 0.0 { (dur / 2.0).min(30.0) } else { 30.0 };
        if !counted && heard >= needed
            && let Some(t) = current_peek() {
                LISTENED.with(|l| l.set((heard, true)));
                spawn_forever(async move {
                    if api::post::<serde_json::Value, _>("/history", &t.to_ref()).await.is_ok() {
                        *crate::state::HISTORY_VERSION.write() += 1;
                    }
                });
                continue;
            }
        LISTENED.with(|l| l.set((heard, counted)));
        if n.is_multiple_of(8) {
            refresh_preload();
            persist::save_position();
            media_session::set_position(*POSITION.peek(), dur);
        }
    }
}
