//! Der Feed ("Für dich"): unendliche Empfehlungen aus dem Hörverhalten.
//!
//! **Geschmack.** Jede Wiedergabe (`listens`) zählt für Titel und Künstler – gewichtet nach
//! Alter (Halbwertszeit 30 Tage) und danach, wie sie lief: bis zum Ende gehört +1, zu einem
//! Teil gehört anteilig, früh übersprungen −1. Favoriten zählen stark (+3 Titel, +1,5 Künstler),
//! "Nicht interessiert" stark dagegen; der alte Verlauf (vor 1.6, nur Zähler) hilft beim Start.
//!
//! **Kandidaten.** YouTube-Music-Radio zu Titeln, die man mag (zufällig nach Gewicht gezogen),
//! Top-Songs von Lieblingskünstlern, lange nicht gehörte Favoriten, Titel aus der Bibliothek
//! und – zum Ausprobieren – Radio zu einem Titel aus der zweiten Reihe oder die Charts.
//!
//! **Auswahl.** Punkte aus Quelle, Künstler- und Titelgeschmack, Neuheit, minus kürzlich
//! gehört; dazu etwas Zufall (sonst käme immer dasselbe). Höchstens zwei Titel je Künstler pro
//! Seite, nie zweimal hintereinander derselbe Künstler, ein fester Anteil zum Entdecken.

use crate::db::listens::{self, artist_of};
use crate::db::playable;
use crate::db::{library, now};
use crate::error::AppResult;
use nrzmusic_types::{FeedItem, FeedPage, FeedReason, PlayableTrack, YtItem, YtTrack};
use nrzmusic_ytmusic::{SearchFilter, YtMusic};
use rand::Rng;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const HALF_LIFE_DAYS: f64 = 30.0;
const RADIO_TTL: Duration = Duration::from_secs(6 * 3600);
const LOOKUP_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Clone)]
pub struct Feed {
    inner: Arc<Inner>,
}

struct Inner {
    pool: SqlitePool,
    yt: Arc<YtMusic>,
    radio: Mutex<HashMap<String, (Instant, Vec<YtTrack>)>>,
    /// Lokaler Titel / Künstlername → YouTube-Video bzw. -Kanal (aus der Suche)
    lookup: Mutex<HashMap<String, (Instant, Option<String>)>>,
}

/// Geschmack eines Kontos.
#[derive(Default)]
struct Taste {
    track: HashMap<String, f64>,
    /// Künstler (klein) → (Punkte, Anzeigename, Kanal-ID)
    artist: HashMap<String, (f64, String, Option<String>)>,
    /// In den letzten 3 h bzw. 24 h gehört
    last_3h: HashSet<String>,
    last_24h: HashSet<String>,
    /// Zuletzt gehört (Zeitpunkt)
    last_heard: HashMap<String, i64>,
    /// Mehrfach früh übersprungen und nie zu Ende gehört
    skipped: HashSet<String>,
    disliked: HashSet<String>,
    /// Bekannte Titel (für Seeds): Schlüssel → Titel
    known: HashMap<String, PlayableTrack>,
    signals: usize,
}

impl Taste {
    fn add_artist(&mut self, name: &str, id: Option<&str>, w: f64) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let e = self.artist.entry(name.to_lowercase()).or_insert_with(|| (0.0, name.to_string(), None));
        e.0 += w;
        if e.2.is_none() {
            e.2 = id.map(str::to_string);
        }
    }

    fn artist_score(&self, name: &str) -> f64 {
        self.artist.get(&name.trim().to_lowercase()).map(|a| a.0).unwrap_or(0.0)
    }
}

struct Candidate {
    track: PlayableTrack,
    reason: FeedReason,
    base: f64,
}

fn decay(age_secs: i64, half_life_days: f64) -> f64 {
    let days = age_secs.max(0) as f64 / 86_400.0;
    0.5f64.powf(days / half_life_days)
}

/// Wert einer Wiedergabe für den Geschmack (−1 … +1).
fn listen_signal(l: &listens::ListenRow) -> f64 {
    let dur = if l.duration_ms > 0 { l.duration_ms as f64 } else { 180_000.0 };
    let ratio = (l.listened_ms as f64 / dur).clamp(0.0, 1.0);
    if l.completed {
        1.0
    } else if l.skipped && (l.listened_ms < 30_000 || ratio < 0.25) {
        -1.0
    } else if l.skipped {
        ratio * 0.9 - 0.3
    } else {
        ratio * 0.8
    }
}

/// Gewichtete Zufallsauswahl von bis zu `n` verschiedenen Einträgen.
fn weighted_pick<T: Clone>(items: &[(T, f64)], n: usize, rng: &mut impl Rng) -> Vec<T> {
    let mut pool: Vec<(T, f64)> = items.iter().filter(|(_, w)| *w > 0.0).cloned().collect();
    let mut out = Vec::new();
    while out.len() < n && !pool.is_empty() {
        let total: f64 = pool.iter().map(|(_, w)| w).sum();
        let mut x = rng.gen_range(0.0..total);
        let mut idx = pool.len() - 1;
        for (i, (_, w)) in pool.iter().enumerate() {
            if x < *w {
                idx = i;
                break;
            }
            x -= w;
        }
        out.push(pool.swap_remove(idx).0);
    }
    out
}

impl Feed {
    pub fn new(pool: SqlitePool, yt: Arc<YtMusic>) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                yt,
                radio: Mutex::new(HashMap::new()),
                lookup: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    async fn taste(&self, uid: i64) -> AppResult<Taste> {
        let pool = self.pool();
        let now = now();
        let mut t = Taste::default();

        // Wiedergaben der letzten 180 Tage
        let rows = listens::recent(pool, uid, now - 180 * 86_400, 5000).await?;
        let mut skips: HashMap<String, (u32, u32)> = HashMap::new();
        for l in &rows {
            let w = decay(now - l.started_at, HALF_LIFE_DAYS) * listen_signal(l);
            *t.track.entry(l.key.clone()).or_default() += w;
            t.add_artist(&l.artist, l.artist_id.as_deref(), w * 0.6);
            let age = now - l.started_at;
            if age < 3 * 3600 {
                t.last_3h.insert(l.key.clone());
            }
            if age < 86_400 {
                t.last_24h.insert(l.key.clone());
            }
            t.last_heard.entry(l.key.clone()).or_insert(l.started_at);
            let s = skips.entry(l.key.clone()).or_default();
            if l.skipped && l.listened_ms < 30_000 {
                s.0 += 1;
            }
            if l.completed {
                s.1 += 1;
            }
        }
        t.signals += rows.len();
        t.skipped = skips.into_iter().filter(|(_, (s, c))| *s >= 2 && *c == 0).map(|(k, _)| k).collect();

        // Favoriten, alter Verlauf – dafür die Titel auflösen (Künstler)
        let favs = listens::favorites(pool, uid).await?;
        let hist = listens::history_counts(pool, uid).await?;
        let mut keys: Vec<String> = favs.iter().map(|f| f.0.clone()).collect();
        keys.extend(hist.iter().map(|h| h.0.clone()));
        keys.extend(rows.iter().take(300).map(|l| l.key.clone()));
        keys.sort();
        keys.dedup();
        for (k, track) in playable::resolve_present(pool, keys).await? {
            t.known.insert(k, track);
        }
        for (key, added) in &favs {
            let w = decay(now - added, 120.0).max(0.4);
            *t.track.entry(key.clone()).or_default() += 3.0 * w;
            if let Some(track) = t.known.get(key) {
                let (a, id, _) = artist_of(track);
                t.add_artist(&a, id.as_deref(), 1.5 * w);
            }
        }
        t.signals += favs.len() * 3;
        // Alter Verlauf zählt nur, solange es kaum neue Wiedergaben gibt
        let hist_weight = if rows.len() > 200 { 0.1 } else { 0.4 };
        for (key, count, at) in &hist {
            let w = hist_weight * (1.0 + *count as f64).ln() * decay(now - at, 90.0);
            *t.track.entry(key.clone()).or_default() += w;
            if let Some(track) = t.known.get(key) {
                let (a, id, _) = artist_of(track);
                t.add_artist(&a, id.as_deref(), w * 0.6);
            }
            t.last_heard.entry(key.clone()).or_insert(*at);
        }
        t.signals += hist.len();

        for (key, artist) in listens::dislikes(pool, uid).await? {
            *t.track.entry(key.clone()).or_default() -= 6.0;
            t.add_artist(&artist, None, -1.5);
            t.disliked.insert(key);
        }
        Ok(t)
    }

    // -------------------------------------------------------------- YouTube (mit Zwischenspeicher)

    async fn radio(&self, video_id: &str) -> Vec<YtTrack> {
        let cached = self
            .inner
            .radio
            .lock()
            .unwrap()
            .get(video_id)
            .filter(|(at, _)| at.elapsed() < RADIO_TTL)
            .map(|(_, list)| list.clone());
        if let Some(list) = cached {
            return list;
        }
        match self.inner.yt.radio(video_id).await {
            Ok(list) => {
                let mut cache = self.inner.radio.lock().unwrap();
                if cache.len() > 500 {
                    cache.retain(|_, (at, _)| at.elapsed() < RADIO_TTL);
                }
                cache.insert(video_id.to_string(), (Instant::now(), list.clone()));
                list
            }
            Err(e) => {
                tracing::debug!(video = video_id, "Feed: Radio fehlgeschlagen: {e}");
                Vec::new()
            }
        }
    }

    fn cached_lookup(&self, key: &str) -> Option<Option<String>> {
        let cache = self.inner.lookup.lock().unwrap();
        cache.get(key).filter(|(at, _)| at.elapsed() < LOOKUP_TTL).map(|(_, v)| v.clone())
    }

    fn store_lookup(&self, key: String, v: Option<String>) {
        let mut cache = self.inner.lookup.lock().unwrap();
        if cache.len() > 5000 {
            cache.retain(|_, (at, _)| at.elapsed() < LOOKUP_TTL);
        }
        cache.insert(key, (Instant::now(), v));
    }

    /// YouTube-Video zu einem Titel (lokale Titel über die Suche "Künstler Titel").
    async fn video_for(&self, track: &PlayableTrack) -> Option<String> {
        match track {
            PlayableTrack::Youtube(y) => Some(y.video_id.clone()),
            PlayableTrack::Local(l) => {
                let key = format!("v:{}", l.id);
                if let Some(v) = self.cached_lookup(&key) {
                    return v;
                }
                let q = format!("{} {}", l.artist, l.title);
                let v = match self.inner.yt.search(q.trim(), Some(SearchFilter::Songs)).await {
                    Ok(r) => r.songs.first().map(|s| s.video_id.clone()),
                    Err(_) => return None,
                };
                self.store_lookup(key, v.clone());
                v
            }
        }
    }

    /// Top-Songs eines Künstlers (Kanal-ID oder über die Suche nach dem Namen).
    async fn artist_songs(&self, name: &str, id: Option<&str>) -> Vec<YtTrack> {
        let id = match id {
            Some(id) => Some(id.to_string()),
            None => {
                let key = format!("a:{}", name.to_lowercase());
                match self.cached_lookup(&key) {
                    Some(v) => v,
                    None => {
                        let v = match self.inner.yt.search(name, Some(SearchFilter::Artists)).await {
                            Ok(r) => r
                                .artists
                                .iter()
                                .find(|a| a.name.eq_ignore_ascii_case(name))
                                .or(r.artists.first())
                                .map(|a| a.browse_id.clone()),
                            Err(_) => return Vec::new(),
                        };
                        self.store_lookup(key, v.clone());
                        v
                    }
                }
            }
        };
        let Some(id) = id else { return Vec::new() };
        match self.inner.yt.artist(&id).await {
            Ok(a) => a.top_songs,
            Err(_) => Vec::new(),
        }
    }

    async fn trending(&self) -> Vec<YtTrack> {
        match self.inner.yt.home().await {
            Ok(shelves) => shelves
                .into_iter()
                .flat_map(|s| s.items)
                .filter_map(|i| match i {
                    YtItem::Track(t) => Some(t),
                    _ => None,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    // -------------------------------------------------------------- Seite

    /// Nächste Seite: `exclude` = schon gezeigte Schlüssel.
    pub async fn page(&self, uid: i64, exclude: &[String], count: usize, youtube: bool) -> AppResult<FeedPage> {
        let count = count.clamp(5, 50);
        let taste = self.taste(uid).await?;
        let cold = taste.signals < 8;
        let exclude: HashSet<&str> = exclude.iter().map(String::as_str).collect();

        // Seeds wählen (ohne Zufallsgenerator über await-Punkte zu halten)
        let (seeds, artists, explore_seed, rediscover) = {
            let mut rng = rand::thread_rng();
            let mut liked: Vec<(String, f64)> = taste
                .track
                .iter()
                .filter(|(k, w)| **w > 0.15 && taste.known.contains_key(*k) && !taste.disliked.contains(*k))
                .map(|(k, w)| (k.clone(), w.powf(1.3)))
                .collect();
            liked.sort_by(|a, b| b.1.total_cmp(&a.1));
            let seeds = weighted_pick(&liked[..liked.len().min(60)], 3, &mut rng);
            // Zweite Reihe zum Ausprobieren: mittelmässig gemochte Titel
            let explore_seed = if liked.len() > 12 {
                let tail = &liked[liked.len() / 3..];
                weighted_pick(&tail.iter().map(|(k, _)| (k.clone(), 1.0)).collect::<Vec<_>>(), 1, &mut rng).pop()
            } else {
                None
            };
            let mut arts: Vec<((String, Option<String>), f64)> = taste
                .artist
                .values()
                .filter(|(w, _, _)| *w > 0.2)
                .map(|(w, n, id)| ((n.clone(), id.clone()), w.powf(1.2)))
                .collect();
            arts.sort_by(|a, b| b.1.total_cmp(&a.1));
            let artists = weighted_pick(&arts[..arts.len().min(30)], 2, &mut rng);
            // Wiederentdecken: gemocht, seit 14 Tagen nicht gehört
            let old: Vec<(String, f64)> = liked
                .iter()
                .filter(|(k, _)| taste.last_heard.get(k).is_none_or(|t| now() - t > 14 * 86_400))
                .cloned()
                .collect();
            let rediscover = weighted_pick(&old, 3, &mut rng);
            (seeds, artists, explore_seed, rediscover)
        };

        let mut cands: Vec<Candidate> = Vec::new();

        if youtube {
            // Radio zu den Seeds, Künstler-Top-Songs, Entdecken – alles gleichzeitig
            let seed_tracks: Vec<PlayableTrack> = seeds.iter().filter_map(|k| taste.known.get(k).cloned()).collect();
            let explore_track = explore_seed.as_ref().and_then(|k| taste.known.get(k).cloned());
            let radios = futures::future::join_all(seed_tracks.iter().map(|t| async move {
                match self.video_for(t).await {
                    Some(v) => self.radio(&v).await,
                    None => Vec::new(),
                }
            }));
            let arts = futures::future::join_all(
                artists.iter().map(|(n, id)| async move { (n.clone(), self.artist_songs(n, id.as_deref()).await) }),
            );
            let explore = async {
                match &explore_track {
                    Some(t) => match self.video_for(t).await {
                        Some(v) => self.radio(&v).await,
                        None => Vec::new(),
                    },
                    None => Vec::new(),
                }
            };
            let trend = async { if cold || rand::random::<f64>() < 0.35 { self.trending().await } else { Vec::new() } };
            let (radios, arts, explore, trend) = futures::future::join4(radios, arts, explore, trend).await;

            for (seed, list) in seed_tracks.iter().zip(radios) {
                for (i, y) in list.into_iter().enumerate().take(25) {
                    cands.push(Candidate {
                        track: PlayableTrack::Youtube(y),
                        reason: FeedReason::Similar { to: seed.title().to_string() },
                        base: 1.0 - i as f64 * 0.015,
                    });
                }
            }
            for (name, list) in arts {
                for (i, y) in list.into_iter().enumerate().take(10) {
                    cands.push(Candidate {
                        track: PlayableTrack::Youtube(y),
                        reason: FeedReason::Artist { name: name.clone() },
                        base: 0.9 - i as f64 * 0.02,
                    });
                }
            }
            // Beim Entdecken die hinteren Radio-Titel – weiter weg vom Bekannten
            for y in explore.into_iter().skip(8).take(15) {
                cands.push(Candidate { track: PlayableTrack::Youtube(y), reason: FeedReason::Explore, base: 0.75 });
            }
            for y in trend.into_iter().take(30) {
                cands.push(Candidate { track: PlayableTrack::Youtube(y), reason: FeedReason::Trending, base: 0.55 });
            }
        }

        for k in &rediscover {
            if let Some(t) = taste.known.get(k) {
                cands.push(Candidate { track: t.clone(), reason: FeedReason::Rediscover, base: 0.8 });
            }
        }

        // Bibliothek: Titel der Lieblingskünstler und selten gespielte
        let top_artists: Vec<String> = {
            let mut a: Vec<&(f64, String, Option<String>)> = taste.artist.values().filter(|a| a.0 > 0.3).collect();
            a.sort_by(|x, y| y.0.total_cmp(&x.0));
            a.into_iter().take(15).map(|a| a.1.clone()).collect()
        };
        let mut ids = listens::local_by_artists(self.pool(), &top_artists, 12).await?;
        let random_n = if youtube { 6 } else { 25 };
        ids.extend(listens::local_random(self.pool(), random_n).await?);
        let locals = library::get_many(self.pool(), &ids).await?;
        for id in ids {
            if let Some(t) = locals.get(&id) {
                cands.push(Candidate { track: PlayableTrack::Local(t.clone()), reason: FeedReason::Library, base: 0.65 });
            }
        }

        Ok(FeedPage { items: select(cands, &taste, &exclude, count), cold_start: cold })
    }
}

/// Titel ohne Zusätze in Klammern, klein: "One More Time (Radio Edit)" → "one more time".
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

/// Punkte vergeben und auswählen.
fn select(cands: Vec<Candidate>, taste: &Taste, exclude: &HashSet<&str>, count: usize) -> Vec<FeedItem> {
    let max_artist = taste.artist.values().map(|a| a.0).fold(1.0, f64::max);
    let max_track = taste.track.values().copied().fold(1.0, f64::max);
    let mut rng = rand::thread_rng();
    let mut seen: HashSet<String> = HashSet::new();
    let mut scored: Vec<(f64, Candidate, String)> = Vec::new();
    for c in cands {
        let key = c.track.key();
        if exclude.contains(key.as_str())
            || taste.disliked.contains(&key)
            || taste.skipped.contains(&key)
            || !seen.insert(key.clone())
        {
            continue;
        }
        let artist = artist_of(&c.track).0;
        // Derselbe Song in anderer Fassung ("… (Radio Edit)", "… [Remastered]") nur einmal
        if !seen.insert(format!("t:{}|{}", artist.to_lowercase(), base_title(c.track.title()))) {
            continue;
        }
        let a = taste.artist_score(&artist);
        // Künstler, die man deutlich ablehnt, gar nicht erst
        if a < -1.5 {
            continue;
        }
        let tr = taste.track.get(&key).copied().unwrap_or(0.0);
        let mut s = c.base + 0.8 * (a / max_artist).clamp(-1.0, 1.0) + 0.5 * (tr / max_track).clamp(-1.0, 1.0);
        if !taste.last_heard.contains_key(&key) {
            s += 0.25;
        }
        if taste.last_3h.contains(&key) {
            s -= 2.5;
        } else if taste.last_24h.contains(&key) {
            s -= 0.8;
        }
        if matches!(&c.track, PlayableTrack::Youtube(y) if y.is_video) {
            s -= 0.15;
        }
        s += rng.gen_range(0.0..0.6);
        scored.push((s, c, artist.to_lowercase()));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Anteil zum Entdecken freihalten
    let explore_slots = (count as f64 * 0.2).ceil() as usize;
    let is_explore = |r: &FeedReason| matches!(r, FeedReason::Explore | FeedReason::Trending | FeedReason::Library);
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    let mut picked: Vec<(Candidate, String)> = Vec::new();
    let mut rest: Vec<(f64, Candidate, String)> = Vec::new();
    let mut explore_taken = 0;
    for (s, c, a) in scored {
        if picked.len() >= count {
            break;
        }
        let n = per_artist.get(&a).copied().unwrap_or(0);
        if n >= 2 && !a.is_empty() {
            continue;
        }
        let exp = is_explore(&c.reason);
        if !exp && picked.len() - explore_taken >= count - explore_slots.min(count) {
            rest.push((s, c, a));
            continue;
        }
        if exp {
            explore_taken += 1;
        }
        *per_artist.entry(a.clone()).or_default() += 1;
        picked.push((c, a));
    }
    // Zu wenig zum Entdecken da: mit dem Rest auffüllen
    for (_, c, a) in rest {
        if picked.len() >= count {
            break;
        }
        if per_artist.get(&a).copied().unwrap_or(0) >= 2 && !a.is_empty() {
            continue;
        }
        *per_artist.entry(a.clone()).or_default() += 1;
        picked.push((c, a));
    }

    // Nicht zweimal hintereinander derselbe Künstler
    for i in 1..picked.len() {
        if picked[i].1.is_empty() || picked[i].1 != picked[i - 1].1 {
            continue;
        }
        if let Some(j) = (i + 1..picked.len()).find(|&j| picked[j].1 != picked[i].1) {
            picked.swap(i, j);
        }
    }
    picked.into_iter().map(|(c, _)| FeedItem { track: c.track, reason: c.reason }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(listened: i64, duration: i64, skipped: bool, completed: bool) -> listens::ListenRow {
        listens::ListenRow {
            key: "yt:x".into(),
            artist: "A".into(),
            artist_id: None,
            started_at: 0,
            listened_ms: listened,
            duration_ms: duration,
            skipped,
            completed,
        }
    }

    #[test]
    fn signals() {
        assert_eq!(listen_signal(&row(200_000, 200_000, false, true)), 1.0);
        assert_eq!(listen_signal(&row(5_000, 200_000, true, false)), -1.0);
        assert!(listen_signal(&row(150_000, 200_000, true, false)) > 0.0);
    }

    #[test]
    fn base_titles() {
        assert_eq!(base_title("One More Time (Radio Edit)"), "one more time");
        assert_eq!(base_title("Around the World"), "around the world");
        assert_eq!(base_title("Song - Remastered 2011"), "song");
        assert_eq!(base_title("X [Live] (feat. Y)"), "x");
    }

    #[test]
    fn decay_halves() {
        assert!((decay(30 * 86_400, 30.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn pick_distinct() {
        let items = vec![("a", 1.0), ("b", 2.0), ("c", 0.0)];
        let mut rng = rand::thread_rng();
        let p = weighted_pick(&items, 5, &mut rng);
        assert_eq!(p.len(), 2);
        assert!(!p.contains(&"c"));
    }
}
