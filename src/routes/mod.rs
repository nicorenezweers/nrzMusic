//! Routen: `api` ist die JSON-API unter `/api/v1` (die Oberfläche nutzt nur sie),
//! `spa` liefert die Oberfläche (Dioxus/WebAssembly), `assets` Schrift und Icons,
//! `catchers` beantwortet Fehler als JSON oder HTML.

pub mod api;
pub mod assets;
pub mod catchers;
pub mod spa;
pub mod widget;
