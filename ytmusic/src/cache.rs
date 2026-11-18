//! Kleiner Zwischenspeicher im Arbeitsspeicher für Alben, Künstler, Playlists und die
//! Startseite (Standard: 10 Minuten). Der Schlüssel enthält Sprache und Region.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nrzmusic_types::{YtAlbum, YtArtist, YtPlaylist, YtShelf};

#[derive(Clone)]
pub(crate) enum Cached {
    Album(YtAlbum),
    Artist(YtArtist),
    Playlist(YtPlaylist),
    Home(Vec<YtShelf>),
}

pub(crate) struct Cache {
    ttl: Duration,
    max_entries: usize,
    entries: Mutex<HashMap<String, (Instant, Cached)>>,
}

impl Cache {
    pub fn new(ttl: Duration, max_entries: usize) -> Self {
        Cache {
            ttl,
            max_entries,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<Cached> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(key) {
            Some((at, value)) if at.elapsed() < self.ttl => Some(value.clone()),
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn put(&self, key: String, value: Cached) {
        if self.ttl.is_zero() || self.max_entries == 0 {
            return;
        }
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = self.ttl;
        map.retain(|_, (at, _)| at.elapsed() < ttl);
        while map.len() >= self.max_entries {
            let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            map.remove(&oldest);
        }
        map.insert(key, (Instant::now(), value));
    }

    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(title: &str) -> Cached {
        Cached::Home(vec![YtShelf {
            title: title.into(),
            items: vec![],
        }])
    }

    #[test]
    fn expires_and_evicts() {
        let cache = Cache::new(Duration::from_secs(60), 2);
        cache.put("a".into(), home("a"));
        cache.put("b".into(), home("b"));
        cache.put("c".into(), home("c"));
        assert!(cache.get("a").is_none(), "ältester Eintrag verdrängt");
        assert!(matches!(cache.get("c"), Some(Cached::Home(s)) if s[0].title == "c"));

        let cache = Cache::new(Duration::from_millis(1), 10);
        cache.put("a".into(), home("a"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get("a").is_none());

        let off = Cache::new(Duration::ZERO, 10);
        off.put("a".into(), home("a"));
        assert!(off.get("a").is_none());
    }
}
