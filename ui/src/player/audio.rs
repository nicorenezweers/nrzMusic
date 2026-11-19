//! Wiedergabe über einen kleinen Pool von `<audio>`-Elementen (nie Teil des Dioxus-Baums):
//! eines spielt, die nächsten Titel laden schon vor, der vorige bleibt eine Weile geladen,
//! falls jemand zurückspringt. Ein Wechsel ist dann nur noch "anderes Element abspielen" –
//! ohne Pause. Was nicht mehr gebraucht wird, wird entladen (Speicher, Datenvolumen).
//!
//! Nur das aktive Element meldet Ereignisse an den Player; ein vorladendes meldet nur Fehler
//! (`Ev::PreloadFailed`), damit der Player nicht verfügbare Titel früh erkennt.
//!
//! Übergänge: Beim automatischen Wechsel blenden die Titel ineinander über ([`Transition::Cross`],
//! der alte wird leiser und hält dann an, der neue wird lauter) – wie ein Radio, ohne Pause.
//! Beim Springen von Hand kurz ([`Transition::Quick`]), damit es nicht knackt. Lautstärke =
//! Grundlautstärke × Blendfaktor des Elements (iOS kennt keine Lautstärke im Element – dort
//! wechselt es hart).
//!
//! iOS lässt ein Element nur nach einer Berührung spielen: Beim ersten Tippen werden darum
//! alle Elemente mit einem stillen Ton "entsperrt". Scheitert ein Wechsel trotzdem
//! (`NotAllowedError`), übernimmt das zuletzt spielende Element die Quelle.

use super::{Ev, send};
use crate::i18n::tr;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

const SLOTS: usize = 4;

type Listener = Closure<dyn FnMut(web_sys::Event)>;

/// Wie weit vorladen: nur die Kopfdaten (YouTube: Server löst die Tonspur auf) oder ganz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Metadata,
    Auto,
}

impl Level {
    fn attr(self) -> &'static str {
        match self {
            Level::Metadata => "metadata",
            Level::Auto => "auto",
        }
    }
}

struct Slot {
    el: web_sys::HtmlAudioElement,
    uid: Option<u64>,
    url: String,
    level: Level,
    /// Hat schon einmal gespielt bzw. ist entsperrt (iOS)
    unlocked: bool,
    _listeners: Vec<Listener>,
}

struct Pool {
    slots: Vec<Slot>,
    active: usize,
    /// Zuletzt aktives Element vor dem letzten Wechsel
    last: Option<usize>,
    /// Voriger Titel bleibt geladen bis (ms, `Date.now()`)
    keep_prev: Option<(usize, f64)>,
    /// Zuletzt gewünschte Vorlade-Einträge (werden nicht verdrängt)
    wanted: Vec<u64>,
    silence: Option<String>,
}

/// Wie ein Wechsel klingt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    /// Hart (neu laden desselben Titels, Start ohne laufende Wiedergabe)
    Cut,
    /// Kurz ein- und ausblenden (Springen von Hand)
    Quick,
    /// Überblenden über so viele Sekunden (automatischer Wechsel am Ende)
    Cross(f64),
}

/// Laufende Blende eines Elements.
struct Fade {
    slot: usize,
    from: f64,
    to: f64,
    start: f64,
    ms: f64,
    /// Am Ende anhalten (ausblendender alter Titel)
    pause_at_end: bool,
}

thread_local! {
    /// Grundlautstärke (schon quadratisch) und Blendfaktor je Element
    static BASE: std::cell::Cell<f64> = const { std::cell::Cell::new(0.64) };
    static GAIN: RefCell<[f64; SLOTS]> = const { RefCell::new([1.0; SLOTS]) };
    static FADES: RefCell<Vec<Fade>> = const { RefCell::new(Vec::new()) };
    static FADER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Lässt sich die Lautstärke eines Elements setzen? (iOS ignoriert sie – dort würden beim
/// Überblenden beide Titel voll laut laufen, also hart wechseln.)
pub fn can_fade() -> bool {
    thread_local! { static OK: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
    OK.with(|ok| {
        if let Some(v) = ok.get() {
            return v;
        }
        let v = web_sys::HtmlAudioElement::new()
            .map(|el| {
                el.set_volume(0.5);
                (el.volume() - 0.5).abs() < 0.01
            })
            .unwrap_or(false);
        ok.set(Some(v));
        v
    })
}

fn apply_volume(i: usize, el: &web_sys::HtmlAudioElement) {
    let g = GAIN.with(|g| g.borrow()[i]);
    el.set_volume((BASE.with(|b| b.get()) * g).clamp(0.0, 1.0));
}

fn set_gain(i: usize, g: f64) {
    GAIN.with(|x| x.borrow_mut()[i] = g.clamp(0.0, 1.0));
    if let Some(el) = with(|p| p.slots[i].el.clone()) {
        apply_volume(i, &el);
    }
}

fn cancel_fade(i: usize) {
    FADES.with(|f| f.borrow_mut().retain(|x| x.slot != i));
}

/// Element `i` in `ms` Millisekunden auf Faktor `to` blenden.
fn fade(i: usize, to: f64, ms: f64, pause_at_end: bool) {
    let from = GAIN.with(|g| g.borrow()[i]);
    cancel_fade(i);
    if ms <= 0.0 {
        set_gain(i, to);
        return;
    }
    FADES.with(|f| f.borrow_mut().push(Fade { slot: i, from, to, start: js_sys::Date::now(), ms, pause_at_end }));
    if !FADER.with(|f| f.replace(true)) {
        wasm_bindgen_futures::spawn_local(fader());
    }
}

/// Führt die Blenden aus (alle 40 ms), solange es welche gibt.
async fn fader() {
    loop {
        gloo_timers::future::TimeoutFuture::new(40).await;
        let now = js_sys::Date::now();
        let mut done: Vec<(usize, bool)> = Vec::new();
        let steps: Vec<(usize, f64)> = FADES.with(|f| {
            let f = f.borrow();
            f.iter()
                .map(|x| {
                    let t = ((now - x.start) / x.ms).clamp(0.0, 1.0);
                    // Gleichmässig für das Ohr: sanfter Anfang und sanftes Ende
                    let e = t * t * (3.0 - 2.0 * t);
                    if t >= 1.0 {
                        done.push((x.slot, x.pause_at_end));
                    }
                    (x.slot, x.from + (x.to - x.from) * e)
                })
                .collect()
        });
        for (i, g) in steps {
            set_gain(i, g);
        }
        if !done.is_empty() {
            FADES.with(|f| f.borrow_mut().retain(|x| (now - x.start) / x.ms < 1.0));
            for (i, pause) in done {
                if pause {
                    with(|p| {
                        if p.active != i {
                            let _ = p.slots[i].el.pause();
                        }
                    });
                }
            }
        }
        if FADES.with(|f| f.borrow().is_empty()) {
            FADER.with(|f| f.set(false));
            return;
        }
    }
}

thread_local! {
    static POOL: RefCell<Option<Pool>> = const { RefCell::new(None) };
    static GESTURE: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Pool) -> R) -> Option<R> {
    POOL.with(|p| p.try_borrow_mut().ok().and_then(|mut p| p.as_mut().map(f)))
}

/// Ereignis-Code: ist Element `i` das aktive? (`None`, wenn gerade nicht prüfbar)
fn slot_state(i: usize) -> Option<(bool, Option<u64>)> {
    POOL.with(|p| p.try_borrow().ok().and_then(|p| p.as_ref().map(|p| (p.active == i, p.slots[i].uid))))
}

/// Spielt Element `i` gerade den echten Titel (nicht den stillen Ton zum Entsperren)?
fn is_active(i: usize, el: &web_sys::HtmlAudioElement) -> bool {
    slot_state(i).is_some_and(|(a, _)| a) && has_src(el) && !el.src().starts_with("blob:")
}

fn has_src(el: &web_sys::HtmlAudioElement) -> bool {
    el.get_attribute("src").is_some_and(|s| !s.is_empty())
}

fn error_message(el: &web_sys::HtmlAudioElement) -> String {
    match el.error().map(|err| err.code()) {
        Some(2) => tr("Netzwerkfehler"),
        Some(3) => tr("Datei beschädigt"),
        Some(4) => tr("Format wird vom Browser nicht unterstützt"),
        _ => tr("unbekannter Fehler"),
    }
}

fn new_slot(i: usize) -> Option<Slot> {
    let el = web_sys::HtmlAudioElement::new().ok()?;
    el.set_preload("none");
    let mut listeners = Vec::new();
    let mut on = |name: &str, f: Box<dyn FnMut(web_sys::Event)>| {
        let c = Closure::<dyn FnMut(web_sys::Event)>::new(f);
        let _ = el.add_event_listener_with_callback(name, c.as_ref().unchecked_ref());
        listeners.push(c);
    };
    // Nur das aktive Element spricht mit dem Player – und nicht, solange es entsperrt wird
    let e = el.clone();
    on("timeupdate", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioTime(e.current_time())) }));
    let e = el.clone();
    on("durationchange", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioDuration(e.duration())) }));
    let e = el.clone();
    on("loadedmetadata", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioDuration(e.duration())) }));
    let e = el.clone();
    on(
        "playing",
        Box::new(move |_| {
            with(|p| p.slots[i].unlocked = true);
            if is_active(i, &e) {
                send(Ev::AudioPlaying)
            }
        }),
    );
    let e = el.clone();
    on("pause", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioPause) }));
    let e = el.clone();
    on("ended", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioEnded) }));
    let e = el.clone();
    on("waiting", Box::new(move |_| if is_active(i, &e) { send(Ev::AudioWaiting) }));
    let e = el.clone();
    on(
        "error",
        Box::new(move |_| {
            // Ohne Quelle (entladen, entsperrt) ist ein Fehler normal
            if !has_src(&e) || e.src().starts_with("blob:") {
                return;
            }
            match slot_state(i) {
                Some((true, _)) => send(Ev::AudioError(error_message(&e))),
                Some((false, Some(uid))) => send(Ev::PreloadFailed(uid)),
                _ => {}
            }
        }),
    );
    Some(Slot { el, uid: None, url: String::new(), level: Level::Metadata, unlocked: false, _listeners: listeners })
}

/// Stiller WAV-Ton (0,1 s) als Blob-Adresse – zum Entsperren auf iOS.
fn silence_url() -> Option<String> {
    let samples = 800u32;
    let mut b: Vec<u8> = Vec::with_capacity(44 + samples as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + samples).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&8000u32.to_le_bytes());
    b.extend_from_slice(&8000u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&samples.to_le_bytes());
    b.resize(44 + samples as usize, 0x80);
    let arr = js_sys::Uint8Array::from(b.as_slice());
    let parts = js_sys::Array::of1(&arr);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("audio/wav");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

pub fn init() {
    if POOL.with(|p| p.borrow().is_some()) {
        return;
    }
    let slots: Vec<Slot> = (0..SLOTS).filter_map(new_slot).collect();
    if slots.len() < SLOTS {
        return;
    }
    let silence = silence_url();
    POOL.with(|p| *p.borrow_mut() = Some(Pool { slots, active: 0, last: None, keep_prev: None, wanted: Vec::new(), silence }));
    // Erste Berührung: alle Elemente entsperren (iOS), danach nicht mehr
    let Some(doc) = crate::util::document() else { return };
    let c = Closure::<dyn FnMut(web_sys::Event)>::new(|_| unlock_all());
    for name in ["pointerdown", "touchend", "keydown"] {
        let _ = doc.add_event_listener_with_callback(name, c.as_ref().unchecked_ref());
    }
    GESTURE.with(|g| *g.borrow_mut() = Some(c));
}

fn unlock_all() {
    let done = with(|p| {
        let Some(silence) = p.silence.clone() else { return true };
        for (i, s) in p.slots.iter_mut().enumerate() {
            if s.unlocked || has_src(&s.el) {
                continue;
            }
            s.unlocked = true;
            s.el.set_muted(true);
            s.el.set_src(&silence);
            if let Ok(pr) = s.el.play() {
                let after = Closure::once(move |_: JsValue| {
                    with(|p| {
                        let s = &mut p.slots[i];
                        // Inzwischen fürs Vorladen benutzt? Dann nicht anfassen
                        if s.uid.is_none() {
                            let _ = s.el.pause();
                            s.el.remove_attribute("src").ok();
                            s.el.load();
                        }
                        s.el.set_muted(MUTED.with(|m| *m.borrow()));
                    });
                });
                let _ = pr.then2(&after, &after);
                after.forget();
            }
        }
        p.slots.iter().all(|s| s.unlocked)
    })
    .unwrap_or(false);
    if done
        && let Some(doc) = crate::util::document()
    {
        GESTURE.with(|g| {
            if let Some(c) = g.borrow_mut().take() {
                for name in ["pointerdown", "touchend", "keydown"] {
                    let _ = doc.remove_event_listener_with_callback(name, c.as_ref().unchecked_ref());
                }
            }
        });
    }
}

thread_local! {
    static MUTED: RefCell<bool> = const { RefCell::new(false) };
}

impl Pool {
    fn unload(&mut self, i: usize) {
        FADES.with(|f| f.borrow_mut().retain(|x| x.slot != i));
        GAIN.with(|g| g.borrow_mut()[i] = 1.0);
        let s = &mut self.slots[i];
        let _ = s.el.pause();
        if has_src(&s.el) {
            s.el.remove_attribute("src").ok();
            s.el.load();
        }
        s.el.set_preload("none");
        s.uid = None;
        s.url.clear();
        if self.keep_prev.is_some_and(|(k, _)| k == i) {
            self.keep_prev = None;
        }
    }

    fn assign(&mut self, i: usize, uid: u64, url: &str, level: Level) {
        let s = &mut self.slots[i];
        s.el.set_preload(level.attr());
        s.uid = Some(uid);
        s.url = url.to_string();
        s.level = level;
        s.el.set_src(url);
    }

    /// Freies Element: leer → nicht gewünscht → der vorige → irgendeins (nie das aktive).
    fn pick(&self, avoid: &[usize]) -> Option<usize> {
        let ok = |i: &usize| *i != self.active && !avoid.contains(i);
        let prev = self.keep_prev.map(|(k, _)| k);
        (0..self.slots.len())
            .filter(ok)
            .find(|i| self.slots[*i].uid.is_none())
            .or_else(|| {
                (0..self.slots.len()).filter(ok).find(|i| {
                    Some(*i) != prev && self.slots[*i].uid.is_none_or(|u| !self.wanted.contains(&u))
                })
            })
            .or_else(|| prev.filter(|p| ok(p)))
            .or_else(|| (0..self.slots.len()).rev().find(ok))
    }
}

fn play_slot(i: usize) {
    let Some(el) = with(|p| p.slots[i].el.clone()) else { return };
    if let Ok(pr) = el.play() {
        let on_err = Closure::once(move |e: JsValue| {
            let name = js_sys::Reflect::get(&e, &"name".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
            // AbortError: durch einen neuen Titel abgelöst – kein Fehler
            if name == "NotAllowedError" && !fallback(i) {
                send(Ev::AudioBlocked);
            }
        });
        let _ = pr.catch(&on_err);
        on_err.forget();
    }
}

/// Das neue Element darf nicht spielen (iOS): das zuletzt spielende übernimmt die Quelle.
fn fallback(i: usize) -> bool {
    let target = with(|p| {
        if p.active != i {
            return None;
        }
        let j = p.last.filter(|j| *j != i && p.slots[*j].unlocked)?;
        let (uid, url, t) = (p.slots[i].uid?, p.slots[i].url.clone(), p.slots[i].el.current_time());
        p.unload(i);
        p.assign(j, uid, &url, Level::Auto);
        p.active = j;
        p.last = None;
        if p.keep_prev.is_some_and(|(k, _)| k == j) {
            p.keep_prev = None;
        }
        let el = p.slots[j].el.clone();
        if t > 0.0 {
            el.set_current_time(t);
        }
        Some(el)
    })
    .flatten();
    match target {
        Some(el) => {
            let _ = el.play();
            true
        }
        None => false,
    }
}

/// Eintrag `uid` abspielen. Liegt er schon (vor)geladen bereit, wird nur umgeschaltet.
/// `fresh`: Quelle neu laden (Fehler, Hänger). `transition`: wie der alte Titel aus- und der
/// neue einblendet.
pub fn load(uid: u64, url: &str, start: f64, autoplay: bool, fresh: bool, transition: Transition) {
    let r = with(|p| {
        let prev = p.active;
        let ready = (0..p.slots.len())
            .find(|i| p.slots[*i].uid == Some(uid) && p.slots[*i].url == url && p.slots[*i].el.error().is_none())
            .filter(|_| !fresh);
        let idx = match ready {
            Some(i) => i,
            None => {
                // Neu laden: dasselbe Element, wenn es diesen Eintrag schon spielte, sonst ein freies
                let i = if p.slots[prev].uid == Some(uid) || !has_src(&p.slots[prev].el) {
                    prev
                } else {
                    p.pick(&[]).unwrap_or(prev)
                };
                p.assign(i, uid, url, Level::Auto);
                i
            }
        };
        if idx != prev {
            // Alter Titel: hart anhalten oder ausblenden (hält danach an)
            let playing = !p.slots[prev].el.paused();
            match transition {
                Transition::Cross(secs) if playing && autoplay => fade_later(prev, secs * 1000.0),
                Transition::Quick if playing && autoplay => fade_later(prev, 300.0),
                _ => {
                    let _ = p.slots[prev].el.pause();
                    cancel_fade_later(prev);
                }
            }
            p.last = Some(prev);
            p.active = idx;
            if p.keep_prev.is_some_and(|(k, _)| k == idx) {
                p.keep_prev = None;
            }
        }
        let el = p.slots[idx].el.clone();
        el.set_preload("auto");
        p.slots[idx].level = Level::Auto;
        if ready.is_some() || start > 0.0 {
            // Vor den Metadaten gesetzt gilt das als Startposition
            el.set_current_time(start.max(0.0));
        }
        (idx, el, ready.is_some())
    });
    let Some((idx, el, was_ready)) = r else { return };
    // Ausblenden des alten Titels (ausserhalb des Pool-Zugriffs)
    for (i, ms) in PENDING_OUT.with(|p| std::mem::take(&mut *p.borrow_mut())) {
        match ms {
            Some(ms) => fade(i, 0.0, ms, true),
            None => {
                cancel_fade(i);
                set_gain(i, 1.0);
            }
        }
    }
    // Neuer Titel: einblenden oder gleich voll
    cancel_fade(idx);
    match transition {
        Transition::Cross(secs) if autoplay => {
            set_gain(idx, 0.0);
            fade(idx, 1.0, secs * 1000.0, false);
        }
        Transition::Quick if autoplay => {
            set_gain(idx, 0.0);
            fade(idx, 1.0, 400.0, false);
        }
        _ => set_gain(idx, 1.0),
    }
    if was_ready {
        // Ereignisse dieses Elements kamen schon beim Vorladen
        let d = el.duration();
        if d.is_finite() && d > 0.0 {
            send(Ev::AudioDuration(d));
        }
    }
    if autoplay {
        play_slot(idx);
    } else {
        let _ = el.pause();
    }
}

/// Vorige Quelle `ms` Millisekunden behalten (Zurückspringen ohne Warten), dann entladen.
pub fn keep_previous(ms: f64) {
    with(|p| {
        if let Some(l) = p.last.filter(|l| *l != p.active && p.slots[*l].uid.is_some()) {
            if ms > 0.0 {
                p.keep_prev = Some((l, js_sys::Date::now() + ms));
            } else {
                p.unload(l);
            }
        }
    });
}

/// Diese Einträge vorladen (in Reihenfolge der Wichtigkeit); alles andere ausser dem
/// aktiven und dem behaltenen vorigen wird entladen. Billig, wenn sich nichts ändert.
pub fn preload(wanted: &[(u64, String, Level)]) {
    with(|p| {
        let now = js_sys::Date::now();
        if p.keep_prev.is_some_and(|(k, until)| until < now && k != p.active) {
            let k = p.keep_prev.unwrap().0;
            p.unload(k);
        }
        p.wanted = wanted.iter().map(|w| w.0).collect();
        let prev = p.keep_prev.map(|(k, _)| k);
        for i in 0..p.slots.len() {
            if i != p.active && Some(i) != prev && p.slots[i].uid.is_some_and(|u| !p.wanted.contains(&u)) {
                p.unload(i);
            }
        }
        let mut used: Vec<usize> = Vec::new();
        for (uid, url, level) in wanted {
            let have = (0..p.slots.len()).find(|i| p.slots[*i].uid == Some(*uid));
            match have {
                Some(i) if p.slots[i].url == *url => {
                    // Vorladefehler: nicht ständig neu versuchen – beim Abspielen erneut
                    if *level > p.slots[i].level && p.slots[i].el.error().is_none() {
                        p.slots[i].level = *level;
                        p.slots[i].el.set_preload(level.attr());
                    }
                    used.push(i);
                }
                Some(i) if i != p.active => {
                    p.assign(i, *uid, url, *level);
                    used.push(i);
                }
                Some(_) => {}
                None => {
                    if let Some(i) = p.pick(&used) {
                        if p.slots[i].uid.is_some() {
                            p.unload(i);
                        }
                        p.assign(i, *uid, url, *level);
                        used.push(i);
                    }
                }
            }
        }
    });
}

/// Vorgeladenen Eintrag verwerfen (z. B. nach einem Fehler oder wenn er ersetzt wurde).
pub fn forget(uid: u64) {
    with(|p| {
        if let Some(i) = (0..p.slots.len()).find(|i| p.slots[*i].uid == Some(uid) && *i != p.active) {
            p.unload(i);
        }
    });
}

fn active_el() -> Option<web_sys::HtmlAudioElement> {
    with(|p| p.slots[p.active].el.clone())
}

pub fn play() {
    if let Some(i) = with(|p| p.active) {
        play_slot(i);
    }
}

pub fn pause() {
    if let Some(el) = active_el() {
        let _ = el.pause();
    }
    // Auch ein gerade ausblendender alter Titel hält an
    let fading: Vec<usize> = FADES.with(|f| f.borrow().iter().filter(|x| x.pause_at_end).map(|x| x.slot).collect());
    for i in fading {
        cancel_fade(i);
        with(|p| {
            if p.active != i {
                let _ = p.slots[i].el.pause();
            }
        });
    }
}

thread_local! {
    /// Während `load` vorgemerkte Blenden des alten Titels (`Some(ms)` = ausblenden, `None` = aus)
    static PENDING_OUT: RefCell<Vec<(usize, Option<f64>)>> = const { RefCell::new(Vec::new()) };
}

fn fade_later(i: usize, ms: f64) {
    PENDING_OUT.with(|p| p.borrow_mut().push((i, Some(ms))));
}

fn cancel_fade_later(i: usize) {
    PENDING_OUT.with(|p| p.borrow_mut().push((i, None)));
}

pub fn seek(secs: f64) {
    if let Some(el) = active_el() {
        el.set_current_time(secs);
    }
}

/// Anhalten und alles entladen.
pub fn stop() {
    with(|p| {
        for i in 0..p.slots.len() {
            p.unload(i);
        }
        p.last = None;
        p.keep_prev = None;
        p.wanted.clear();
    });
}

pub fn set_volume(v: f64, muted: bool) {
    MUTED.with(|m| *m.borrow_mut() = muted);
    // Gehörrichtiger: quadratisch
    BASE.with(|b| b.set(v * v));
    with(|p| {
        for (i, s) in p.slots.iter().enumerate() {
            apply_volume(i, &s.el);
            s.el.set_muted(muted);
        }
    });
}
