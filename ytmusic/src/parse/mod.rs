//! Auswertung der InnerTube-Antworten. Grundsatz: nie in Panik geraten – fehlt der Kern
//! einer Seite, gibt es `Error::Parse` (bzw. `Error::NotFound`, wenn YouTube gar keinen
//! Inhalt liefert); einzelne kaputte Einträge werden still übersprungen.

pub(crate) mod album;
pub(crate) mod artist;
pub(crate) mod common;
pub(crate) mod home;
pub(crate) mod items;
pub(crate) mod playlist;
pub(crate) mod search;
pub(crate) mod watch;

#[cfg(test)]
mod tests;
