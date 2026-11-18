//! Aussehen der Oberfläche, in Rust beschrieben – auf Basis von rust-ui
//! (https://github.com/rust-ui/ui, MIT). Nur Grautöne: das dunkle Theme von rust-ui wie bei
//! nrzPaper, Akzente grau statt blau – die Farbe bringen die Cover.
//!
//! Die Komponenten benutzen Tailwind-Klassen (`bg-background`, `text-muted-foreground`, …).
//! `cargo xtask css` schreibt aus [`tailwind_input`] die Eingabe für Tailwind nach
//! `ui/tailwind.css` (erzeugt, nicht im Repo); `dx serve` und `dx bundle` lassen Tailwind
//! darüber laufen, das nur die Klassen ausgibt, die in `src/**/*.rs` vorkommen.
//!
//! Das Modul kommt ohne Abhängigkeiten aus, weil `xtask` es direkt einbindet (`#[path]`). Die
//! App braucht daraus nur einzelne Konstanten, `xtask` den Rest – darum `dead_code` erlaubt.
#![allow(dead_code)]

use std::fmt::Write;

/// Grundradius; `rounded-sm` bis `rounded-xl` leiten sich davon ab.
pub const RADIUS: &str = "0.625rem";

/// Schrift: Inter (variabel, liegt unter `/static/fonts/`, siehe [`FONT_FACE`]).
pub const FONT_SANS: &str = r#""Inter", ui-sans-serif, system-ui, sans-serif"#;

/// Farben. Jede wird zur CSS-Variable `--<name>` und zur Tailwind-Farbe `<name>`
/// (`bg-background`, `text-muted-foreground`, `border-border`, …).
pub const COLORS: &[(&str, &str)] = &[
    ("background", "oklch(0.18 0 0)"),
    ("foreground", "oklch(0.985 0 0)"),
    ("card", "oklch(0.205 0 0)"),
    ("card-foreground", "oklch(0.985 0 0)"),
    ("popover", "oklch(0.235 0 0)"),
    ("popover-foreground", "oklch(0.985 0 0)"),
    ("primary", "oklch(0.922 0 0)"),
    ("primary-foreground", "oklch(0.205 0 0)"),
    ("secondary", "oklch(0.269 0 0)"),
    ("secondary-foreground", "oklch(0.985 0 0)"),
    ("muted", "oklch(0.269 0 0)"),
    ("muted-foreground", "oklch(0.708 0 0)"),
    ("accent", "oklch(0.3 0 0)"),
    ("accent-foreground", "oklch(0.985 0 0)"),
    ("destructive", "oklch(0.704 0.191 22.216)"),
    ("border", "oklch(1 0 0 / 10%)"),
    ("input", "oklch(1 0 0 / 15%)"),
    ("ring", "oklch(0.556 0 0)"),
    ("sidenav", "oklch(0.2 0 0)"),
    ("sidenav-foreground", "oklch(0.985 0 0)"),
    ("sidenav-accent", "oklch(0.269 0 0)"),
    ("sidenav-accent-foreground", "oklch(0.985 0 0)"),
    ("sidenav-border", "oklch(1 0 0 / 8%)"),
    ("success", "oklch(0.65 0.16 145)"),
    ("success-foreground", "oklch(0.205 0 0)"),
    ("warning", "oklch(0.65 0.16 55)"),
    ("warning-foreground", "oklch(0.205 0 0)"),
    ("surface", "oklch(0.22 0 0)"),
    // Musik: Spielerleiste und Herz (gefüllt)
    ("player", "oklch(0.2 0 0)"),
    ("heart", "oklch(0.7 0.17 15)"),
];

/// Dauer der Ausblende-Animation der Hinweise (`animate-toast-out`). Die App entfernt einen
/// Hinweis erst danach (`components::overlay`).
pub const TOAST_OUT_MS: u32 = 220;

/// Animationen: `animate-<name>` → Wert. Die Keyframes stehen in [`KEYFRAMES`].
fn animations() -> [(&'static str, String); 4] {
    [
        ("in", "enter 150ms ease-out".into()),
        ("up", "up 220ms cubic-bezier(0.16, 1, 0.3, 1)".into()),
        (
            "toast-in",
            "toast-in 260ms cubic-bezier(0.16, 1, 0.3, 1)".into(),
        ),
        (
            "toast-out",
            format!("toast-out {TOAST_OUT_MS}ms ease-in forwards"),
        ),
    ]
}

/// Bewegungen (Hinweise, Einblenden, "Läuft gerade" von unten, Equalizer-Balken) – fast
/// keine, wenn das System "Bewegung reduzieren" verlangt.
const KEYFRAMES: &str = r#"@keyframes toast-in {
  from { opacity: 0; transform: translateX(24px) scale(0.97); }
  to { opacity: 1; transform: none; }
}

@keyframes toast-out {
  from { opacity: 1; transform: none; }
  to { opacity: 0; transform: translateX(24px) scale(0.97); }
}

@keyframes enter {
  from { opacity: 0; transform: translateY(4px) scale(0.98); }
  to { opacity: 1; transform: none; }
}

@keyframes up {
  from { opacity: 0; transform: translateY(24px); }
  to { opacity: 1; transform: none; }
}

/* Kleine "Equalizer"-Balken beim laufenden Titel */
@keyframes eq {
  0%, 100% { transform: scaleY(0.3); }
  50% { transform: scaleY(1); }
}

@media (prefers-reduced-motion: reduce) {
  .animate-toast-in, .animate-toast-out, .animate-up { animation-duration: 1ms; }
  .eq-bar { animation: none !important; }
  .np-sheet.settle, .np-cover { transition: none; }
}
"#;

/// Die Schrift kommt vom eigenen Server (eingebettet, `/static/fonts/`).
const FONT_FACE: &str = r#"@font-face {
  font-family: "Inter";
  font-style: normal;
  font-display: swap;
  font-weight: 100 900;
  src: url("/static/fonts/inter-latin.woff2") format("woff2-variations");
}
"#;

/// Grundregeln für alle Elemente, Touch und Webapp.
const BASE: &str = r#"@layer base {
  * { @apply border-border outline-ring/50; }
  html { color-scheme: dark; -webkit-text-size-adjust: 100%; text-size-adjust: 100%; -webkit-tap-highlight-color: transparent; }
  /* Touch: kein Doppeltipp-Zoom, kein Kontextmenü beim langen Drücken auf Knöpfe/Links */
  a, button, [role="button"] { touch-action: manipulation; -webkit-touch-callout: none; }
  button { -webkit-user-select: none; user-select: none; }
  img { -webkit-user-drag: none; }
  /* iPhone zoomt in Felder mit weniger als 16px – auf dem Handy darum 16px */
  @media (max-width: 767px) {
    input:not([type="range"]):not([type="checkbox"]):not([type="radio"]), select, textarea { font-size: 16px; }
  }
  /* Als Webapp vom Home-Bildschirm: Seite selbst nie verschieben */
  @media (display-mode: standalone) {
    html, body { overscroll-behavior: none; }
  }
  body { @apply bg-background text-foreground antialiased; font-family: var(--font-sans); overscroll-behavior-y: none; }
  ::selection { background: oklch(0.45 0 0); color: oklch(0.985 0 0); }
  ::-webkit-scrollbar { width: 10px; height: 10px; }
  ::-webkit-scrollbar-thumb { background: oklch(0.35 0 0); border-radius: 9999px; border: 2px solid transparent; background-clip: padding-box; }
  ::-webkit-scrollbar-track { background: transparent; }
  input[type="search"]::-webkit-search-cancel-button { filter: grayscale(1) invert(0.7); }
}
"#;

/// Eigene Klassen, die Tailwind allein nicht abdeckt (Schieberegler, Handy-Layout, Player).
const COMPONENTS: &str = r#"@layer components {
  /* Schieberegler (Position, Lautstärke): dünne Spur, gefüllter Teil über --value */
  .range {
    -webkit-appearance: none; appearance: none; background: transparent; cursor: pointer; height: 16px; width: 100%;
    --value: 0%;
  }
  .range:focus-visible { outline: none; }
  .range::-webkit-slider-runnable-track {
    height: 4px; border-radius: 9999px;
    background: linear-gradient(to right, oklch(0.92 0 0) var(--value), oklch(1 0 0 / 18%) var(--value));
  }
  .range::-moz-range-track { height: 4px; border-radius: 9999px; background: oklch(1 0 0 / 18%); }
  .range::-moz-range-progress { height: 4px; border-radius: 9999px; background: oklch(0.92 0 0); }
  .range::-webkit-slider-thumb {
    -webkit-appearance: none; appearance: none; width: 12px; height: 12px; margin-top: -4px; border-radius: 9999px;
    background: oklch(0.985 0 0); opacity: 0; transition: opacity 120ms;
  }
  .range::-moz-range-thumb { width: 12px; height: 12px; border: 0; border-radius: 9999px; background: oklch(0.985 0 0); opacity: 0; transition: opacity 120ms; }
  .range:hover::-webkit-slider-thumb, .range:focus-visible::-webkit-slider-thumb, .range.always::-webkit-slider-thumb { opacity: 1; }
  .range:hover::-moz-range-thumb, .range:focus-visible::-moz-range-thumb, .range.always::-moz-range-thumb { opacity: 1; }

  .eq-bar { transform-origin: bottom; animation: eq 900ms ease-in-out infinite; }
  .eq-bar:nth-child(2) { animation-delay: -300ms; }
  .eq-bar:nth-child(3) { animation-delay: -600ms; }

  /* Waagrechte Reihen (Startseite, YouTube-Regale) ohne sichtbaren Balken */
  .shelf { scrollbar-width: none; scroll-snap-type: x proximity; scroll-padding-inline: 1rem; }
  @media (min-width: 768px) { .shelf { scroll-padding-inline: 1.5rem; } }
  .shelf::-webkit-scrollbar { display: none; }
  .shelf > * { scroll-snap-align: start; }

  .pb-safe { padding-bottom: env(safe-area-inset-bottom); }
  .pt-safe { padding-top: env(safe-area-inset-top); }
  .px-safe { padding-left: env(safe-area-inset-left); padding-right: env(safe-area-inset-right); }

  /* ---------- Handy (iPhone-optimiert): Kopf und Dock liegen schwebend über dem Inhalt ----------
     --dock: Höhe von Mini-Player + Navigation (ohne Home-Indikator), setzt das Layout. */
  .glass {
    background: color-mix(in oklab, var(--background) 72%, transparent);
    -webkit-backdrop-filter: blur(24px) saturate(1.6); backdrop-filter: blur(24px) saturate(1.6);
  }
  @media (max-width: 767px) {
    .m-header { position: absolute; inset-inline: 0; top: 0; z-index: 30; padding-top: env(safe-area-inset-top); }
    .m-content {
      padding-top: calc(3.5rem + env(safe-area-inset-top));
      padding-bottom: calc(var(--dock, 3.5rem) + env(safe-area-inset-bottom) + 0.5rem);
      scroll-padding-top: calc(3.5rem + env(safe-area-inset-top));
    }
    .above-dock { bottom: calc(var(--dock, 3.5rem) + env(safe-area-inset-bottom)) !important; }
    .toaster-pos { bottom: calc(var(--dock, 3.5rem) + env(safe-area-inset-bottom) + 0.75rem) !important; left: 1rem; right: 1rem; margin-inline: auto; }
  }
  .m-dock { position: absolute; inset-inline: 0; bottom: 0; z-index: 30; pointer-events: none; }
  .m-dock > * { pointer-events: auto; }
  /* Unterer Rand: weicher Verlauf, damit der schwebende Mini-Player nicht "hängt" */
  .m-dock::before {
    content: ""; position: absolute; inset: -1.5rem 0 0 0; z-index: -1; pointer-events: none;
    background: linear-gradient(to bottom, transparent, color-mix(in oklab, var(--background) 85%, transparent) 45%);
  }
  .tab-active-dot { position: absolute; top: 0; left: 50%; width: 1.75rem; height: 2px; translate: -50% 0; border-radius: 9999px; background: var(--foreground); }

  /* Gedrückt-Rückmeldung für Touch (statt Hover) */
  @media (hover: none) {
    .press { transition: transform 120ms ease, opacity 120ms ease; }
    .press:active { transform: scale(0.96); opacity: 0.85; }
  }

  /* Läuft gerade (Handy): Griff oben, Ziehen nach unten schliesst */
  .grabber { width: 2.25rem; height: 5px; border-radius: 9999px; background: oklch(1 0 0 / 30%); }
  .np-sheet { padding-top: env(safe-area-inset-top); padding-bottom: env(safe-area-inset-bottom); will-change: transform; }
  .np-sheet.settle { transition: transform 280ms cubic-bezier(0.16, 1, 0.3, 1); }
  .np-cover { transition: transform 420ms cubic-bezier(0.34, 1.56, 0.64, 1), box-shadow 300ms; }
  .np-cover.paused { transform: scale(0.86); box-shadow: 0 10px 30px rgb(0 0 0 / 0.35); }

  /* Mini-Player: Cover-Farbe als Hintergrund */
  .mini-tint { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; transform: scale(2); filter: blur(28px) saturate(1.8); opacity: 0.55; pointer-events: none; }

  /* YouTube-Player: liegt ausserhalb der Dioxus-Wurzel (wird nie neu gezeichnet) und
     wird je nach Ansicht über die Seite gelegt – immer sichtbar, solange ein YouTube-Titel läuft. */
  #yt-host {
    position: fixed; z-index: 75; display: none; overflow: hidden; background: #000;
    border-radius: 0.875rem; border: 1px solid oklch(1 0 0 / 12%); box-shadow: 0 20px 50px rgb(0 0 0 / 0.55);
  }
  #yt-host.on { display: block; }
  #yt-host iframe { width: 100%; height: 100%; display: block; border: 0; }
  /* Schwebende Karte über der Spielerleiste (Desktop) */
  #yt-host.floating { right: 1rem; bottom: calc(5rem + 0.75rem); width: 356px; height: 200px; }
  /* Handy: ganze Breite über der kleinen Spielerleiste und der Navigation */
  @media (max-width: 767px) {
    #yt-host.floating {
      left: 0; right: 0; width: 100%; height: max(200px, calc(100vw * 9 / 16)); border-radius: 0; border-left: 0; border-right: 0;
      bottom: calc(var(--dock, 7.5rem) + env(safe-area-inset-bottom));
    }
  }
  /* Gross in "Läuft gerade": Position/Grösse setzt der Player über einem Platzhalter */
  #yt-host.large { border-radius: 1rem; }
}
"#;

/// macOS-App (`macos/`): Milchglas des Systems hinter dem Fenster, wie bei nrzX. Die App setzt
/// `nrz-glass` (Flächen durchsichtig: Grund, Seitenleiste und Spielerleiste nur leicht getönt,
/// der Schreibtisch scheint weichgezeichnet durch) und `nrz-mac` (Platz für die Ampel oben links)
/// am `<html>`, bevor die Oberfläche zeichnet. Dialoge und Menüs bleiben fast deckend.
const GLASS: &str = r#"html.nrz-glass {
  --sidenav: oklch(0.2 0 0 / 22%);
  --sidenav-border: oklch(1 0 0 / 9%);
  --player: oklch(0.14 0 0 / 38%);
  --card: oklch(1 0 0 / 5%);
  --surface: oklch(1 0 0 / 6%);
  --secondary: oklch(1 0 0 / 8%);
  --muted: oklch(1 0 0 / 7%);
  --accent: oklch(1 0 0 / 10%);
  --popover: oklch(0.21 0 0 / 88%);
  --background: oklch(0.17 0 0 / 92%);
  --border: oklch(1 0 0 / 12%);
}
html.nrz-glass, html.nrz-glass body { background: transparent; }
/* Grund des Fensters: kaum getönt */
html.nrz-glass body { background: oklch(0.14 0 0 / 28%); }
html.nrz-glass .bg-popover, html.nrz-glass [role="dialog"], html.nrz-glass [role="alertdialog"] {
  -webkit-backdrop-filter: blur(30px) saturate(180%); backdrop-filter: blur(30px) saturate(180%);
}
html.nrz-glass .bg-player { -webkit-backdrop-filter: blur(24px) saturate(160%); backdrop-filter: blur(24px) saturate(160%); }
html.nrz-glass ::-webkit-scrollbar-thumb { background: oklch(1 0 0 / 18%); background-clip: padding-box; }
/* Ampel oben links: die Seitenleiste beginnt darunter, die Kopfzeile zieht das Fenster */
html.nrz-mac aside { padding-top: 1.75rem; }
html.nrz-mac, html.nrz-mac body { overscroll-behavior: none; }
html.nrz-mac a, html.nrz-mac [role="link"] { cursor: default; }
"#;

/// Die Eingabe für Tailwind – Inhalt von `ui/tailwind.css` (schreibt `cargo xtask css`).
pub fn tailwind_input() -> String {
    let mut css = String::from(
        "/* Erzeugt von `cargo xtask css` aus ui/src/theme.rs – nicht von Hand ändern. */\n\
         @import \"tailwindcss\";\n\n\
         @source \"./src/**/*.rs\";\n\n\
         @custom-variant dark (&:is(.dark *));\n\n",
    );

    // Werte als CSS-Variablen …
    css.push_str(":root {\n");
    let _ = writeln!(css, "  --radius: {RADIUS};");
    let _ = writeln!(css, "  --font-sans: {FONT_SANS};");
    for (name, value) in COLORS {
        let _ = writeln!(css, "  --{name}: {value};");
    }
    css.push_str("}\n\n");

    // … und als Tailwind-Theme (inline: die Klassen verweisen direkt auf die Variablen)
    css.push_str(
        "@theme inline {\n  \
           --font-sans: var(--font-sans);\n  \
           --radius-sm: calc(var(--radius) - 4px);\n  \
           --radius-md: calc(var(--radius) - 2px);\n  \
           --radius-lg: var(--radius);\n  \
           --radius-xl: calc(var(--radius) + 4px);\n",
    );
    for (name, _) in COLORS {
        let _ = writeln!(css, "  --color-{name}: var(--{name});");
    }
    for (name, value) in animations() {
        let _ = writeln!(css, "  --animate-{name}: {value};");
    }
    css.push_str("}\n\n");

    for part in [KEYFRAMES, FONT_FACE, BASE, COMPONENTS, GLASS] {
        css.push_str(part);
        css.push('\n');
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_color_is_variable_and_theme_color() {
        let css = tailwind_input();
        for (name, value) in COLORS {
            assert!(css.contains(&format!("  --{name}: {value};\n")), "{name}");
            assert!(
                css.contains(&format!("  --color-{name}: var(--{name});\n")),
                "{name}"
            );
        }
    }

    #[test]
    fn braces_balanced() {
        let css = tailwind_input();
        let mut depth = 0i32;
        for c in css.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
    }

    #[test]
    fn toast_duration_shared() {
        assert!(
            tailwind_input().contains(&format!("--animate-toast-out: toast-out {TOAST_OUT_MS}ms "))
        );
    }
}
