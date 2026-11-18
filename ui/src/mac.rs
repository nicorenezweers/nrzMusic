//! Brücke zur macOS-App (`macos/`): Die App zeigt diese Oberfläche in einem durchsichtigen
//! Fenster (WKWebView auf Milchglas) und setzt vor dem ersten Zeichnen
//!
//! - `window.__nrzMac = { mode: "local" | "server", server: "<adresse>" }` – läuft der eingebaute
//!   Server auf diesem Mac, oder ist die App mit einer Server-Instanz verbunden,
//! - die Klassen `nrz-mac` (Platz für die Ampel, Fenster an der Kopfzeile ziehen) und `nrz-glass`
//!   (durchsichtige Flächen, siehe `theme::GLASS`) am `<html>`.
//!
//! Zurück geht es über `window.ipc.postMessage(…)`, z. B. `connect:open` (Server wechseln).
//! Im Browser gibt es `__nrzMac` nicht – dann tut hier alles nichts.

use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};

/// So läuft die Mac-App gerade.
#[derive(Clone, PartialEq)]
pub struct MacInfo {
    /// Eingebauter Server auf diesem Mac (ohne Server-Instanz).
    pub local: bool,
    /// Adresse der verbundenen Server-Instanz (leer im lokalen Modus).
    pub server: String,
}

fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(obj, &JsValue::from_str(key)).ok().filter(|v| !v.is_undefined() && !v.is_null())
}

/// `Some`, wenn die Oberfläche in der Mac-App läuft.
pub fn info() -> Option<MacInfo> {
    let w: JsValue = web_sys::window()?.into();
    let mac = get(&w, "__nrzMac")?;
    let mode = get(&mac, "mode").and_then(|v| v.as_string()).unwrap_or_default();
    let server = get(&mac, "server").and_then(|v| v.as_string()).unwrap_or_default();
    Some(MacInfo { local: mode != "server", server })
}

/// Nachricht an die App (`window.ipc.postMessage`).
pub fn post(message: &str) {
    let Some(w) = web_sys::window() else { return };
    let Some(ipc) = get(&w.into(), "ipc") else { return };
    if let Some(f) = get(&ipc, "postMessage").and_then(|f| f.dyn_into::<js_sys::Function>().ok()) {
        let _ = f.call1(&ipc, &JsValue::from_str(message));
    }
}
