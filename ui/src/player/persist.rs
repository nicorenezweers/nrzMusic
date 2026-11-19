//! Warteschlange, Position und Einstellungen im Browser merken (localStorage) und nach
//! dem Neuladen pausiert wiederherstellen.

use super::*;

const KEY: &str = "nrzmusic-player";
/// Sehr lange Warteschlangen nur um den aktuellen Titel herum speichern (Speichergrenze).
const MAX_SAVED: usize = 1500;

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    queue: Vec<QueueItem>,
    index: Option<usize>,
    position: f64,
    volume: Option<f64>,
    muted: bool,
    shuffle: bool,
    repeat: Repeat,
    original: Option<Vec<u64>>,
}

pub fn save() {
    let queue = QUEUE.peek();
    let index = *INDEX.peek();
    let (queue, index): (Vec<QueueItem>, Option<usize>) = if queue.len() > MAX_SAVED {
        let i = index.unwrap_or(0);
        let start = i.saturating_sub(MAX_SAVED / 4);
        let end = (start + MAX_SAVED).min(queue.len());
        (queue[start..end].to_vec(), index.map(|i| i - start))
    } else {
        (queue.clone(), index)
    };
    let saved = Saved {
        queue,
        index,
        position: *POSITION.peek(),
        volume: Some(*VOLUME.peek()),
        muted: *MUTED.peek(),
        shuffle: *SHUFFLE.peek(),
        repeat: *REPEAT.peek(),
        original: ORIGINAL.peek().clone(),
    };
    if let Ok(json) = serde_json::to_string(&saved) {
        crate::util::storage_set(KEY, &json);
    }
}

/// Nur die Position aktualisieren (läuft alle 2 s während der Wiedergabe).
pub fn save_position() {
    save();
}

pub fn restore() {
    let Some(json) = crate::util::storage_get(KEY) else { return };
    let Ok(s) = serde_json::from_str::<Saved>(&json) else { return };
    let max_uid = s.queue.iter().map(|q| q.uid).max().unwrap_or(0);
    NEXT_UID.with(|n| n.set(max_uid + 1));
    let index = s.index.filter(|i| *i < s.queue.len());
    *QUEUE.write() = s.queue;
    *INDEX.write() = index;
    if let Some(v) = s.volume {
        *VOLUME.write() = v.clamp(0.0, 1.0);
    }
    *MUTED.write() = s.muted;
    *SHUFFLE.write() = s.shuffle;
    *REPEAT.write() = s.repeat;
    *ORIGINAL.write() = s.original.filter(|_| s.shuffle);
    if index.is_some() {
        // Pausiert vorbereiten; Abspielen setzt an der gemerkten Stelle fort
        let pos = s.position.max(0.0);
        if let Some(t) = current_peek() {
            *POSITION.write() = pos;
            *DURATION.write() = t.duration_ms().map(|d| d as f64 / 1000.0).unwrap_or(0.0);
            match &t {
                PlayableTrack::Local(lt) => {
                    *ACTIVE.write() = Backend::Local;
                    let uid = crate::player::current_uid().unwrap_or(0);
                    audio::load(uid, &format!("/api/v1/tracks/{}/stream", lt.id), pos, false, false, audio::Transition::Cut);
                }
                // YouTube erst beim Abspielen laden (sonst startet yt-dlp schon beim Öffnen)
                PlayableTrack::Youtube(_) => *ACTIVE.write() = Backend::None,
            }
            media_session::set_metadata(Some(&t));
            media_session::set_playing(false);
        }
    }
}

/// Gemerkten Stand löschen.
pub fn clear() {
    if let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = st.remove_item(KEY);
    }
}
