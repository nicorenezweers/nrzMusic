//! Tastenkürzel (nicht beim Tippen in Eingabefeldern):
//! Leertaste Play/Pause, ←/→ ±10 s, Shift+←/→ vorheriger/nächster Titel, `/` Suche, M stumm.

use super::{Cmd, Ev, send};
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

type Listener = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

thread_local! {
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

/// Tippt der Benutzer gerade in ein Feld?
fn typing(e: &web_sys::KeyboardEvent) -> bool {
    let Some(el) = e.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return false };
    let tag = el.tag_name().to_ascii_lowercase();
    if matches!(tag.as_str(), "textarea" | "select") {
        return true;
    }
    if tag == "input" {
        // Schieberegler und Kästchen zählen nicht als Tippen – ausser für Pfeiltasten
        let kind = el.get_attribute("type").unwrap_or_default().to_ascii_lowercase();
        return !matches!(kind.as_str(), "checkbox" | "radio" | "button" | "submit" | "file")
            && !(kind == "range" && !e.key().starts_with("Arrow"));
    }
    el.closest("[contenteditable=''], [contenteditable='true']").ok().flatten().is_some()
}

/// Ein offener (modaler) Dialog bekommt Tasten selbst.
fn dialog_open() -> bool {
    crate::util::document()
        .and_then(|d| d.query_selector("[aria-modal=true], [role=alertdialog]").ok().flatten())
        .is_some()
}

pub fn init() {
    if LISTENER.with(|l| l.borrow().is_some()) {
        return;
    }
    let Some(w) = web_sys::window() else { return };
    let c = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|e: web_sys::KeyboardEvent| {
        if e.default_prevented() || e.ctrl_key() || e.meta_key() || e.alt_key() || e.is_composing() || typing(&e) || dialog_open() {
            return;
        }
        let cmd = match (e.key().as_str(), e.shift_key()) {
            (" " | "Spacebar", _) => Some(Cmd::Toggle),
            ("ArrowLeft", false) => Some(Cmd::SeekBy(-10.0)),
            ("ArrowRight", false) => Some(Cmd::SeekBy(10.0)),
            ("ArrowLeft", true) => Some(Cmd::Prev),
            ("ArrowRight", true) => Some(Cmd::Next),
            ("/", _) => Some(Cmd::FocusSearch),
            ("m" | "M", _) => Some(Cmd::Mute),
            ("Escape", _) => Some(Cmd::Close),
            _ => None,
        };
        if let Some(c) = cmd {
            e.prevent_default();
            send(Ev::Cmd(c));
        }
    });
    let _ = w.add_event_listener_with_callback("keydown", c.as_ref().unchecked_ref());
    LISTENER.with(|l| *l.borrow_mut() = Some(c));
}
