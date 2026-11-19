//! Ampel (Schliessen, Minimieren, Zoomen) tiefer und weiter innen – so, wie macOS sie in Fenstern
//! mit Toolbar setzt (Musik, Finder, nrzX): eine leere native Toolbar im Stil „unified“ macht die
//! Titelleiste höher, AppKit hält die Knöpfe darin mittig – auch beim Vergrössern, Zoomen und im
//! Vollbild. Klicks in dem Bereich gehen weiter an den WebView.
//!
//! [`spot`] misst die tatsächliche Lage; die Oberfläche richtet die oberste Reihe danach aus
//! (`--nrz-tl-y` Mitte, `--nrz-tl-r` rechter Rand des Zoom-Knopfs, siehe `pages::NATIVE_CSS`):
//! Logo und Name stehen rechts neben der Ampel auf derselben Höhe, die Suche ebenso.

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSTitlebarSeparatorStyle, NSToolbar, NSWindow, NSWindowButton, NSWindowToolbarStyle};
use objc2_foundation::NSString;
use tao::platform::macos::WindowExtMacOS;
use tao::window::Window;

fn ns(window: &Window) -> Option<&NSWindow> {
    // SAFETY: NSWindow des lebenden tao-Fensters, Aufruf im Haupt-Thread.
    unsafe { (window.ns_window() as *const NSWindow).as_ref() }
}

/// Leere Toolbar ans Fenster: die Ampel sitzt danach so tief wie in nativen Apps mit Toolbar.
pub fn install(window: &Window) {
    let (Some(ns), Some(mtm)) = (ns(window), MainThreadMarker::new()) else { return };
    let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("nrzmusic"));
    ns.setToolbar(Some(&toolbar));
    ns.setToolbarStyle(NSWindowToolbarStyle::Unified);
    // Keine Linie unter der Titelleiste – die Oberfläche geht bis oben durch
    ns.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
}

/// Lage der Ampel in Punkten: Mitte unter der Oberkante und rechter Rand des Zoom-Knopfs.
/// `None` im Vollbild (Ampel verborgen).
pub fn spot(window: &Window) -> Option<(f64, f64)> {
    let ns = ns(window)?;
    let close = ns.standardWindowButton(NSWindowButton::CloseButton)?;
    let zoom = ns.standardWindowButton(NSWindowButton::ZoomButton)?;
    if close.isHidden() || window.fullscreen().is_some() {
        return None;
    }
    let h = ns.frame().size.height;
    let c = close.convertRect_toView(close.bounds(), None);
    let z = zoom.convertRect_toView(zoom.bounds(), None);
    let center = h - (c.origin.y + c.size.height / 2.0);
    let right = z.origin.x + z.size.width;
    (center > 0.0 && center < 80.0 && right > 0.0 && right < 200.0).then_some((center, right))
}
