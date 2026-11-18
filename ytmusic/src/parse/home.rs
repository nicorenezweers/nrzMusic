//! Startseite (`browse` mit `FEmusic_home`) und ihre Fortsetzungen.

use nrzmusic_types::{YtItem, YtShelf};
use serde_json::Value;

use super::items::*;
use crate::json::JsonExt;
use crate::{Error, Result};

const SECTION_LIST: &str =
    "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer";

/// Erste Seite: Regale und Fortsetzungs-Token.
pub(crate) fn parse_home(resp: &Value) -> Result<(Vec<YtShelf>, Option<String>)> {
    let Some(list) = resp.at(SECTION_LIST) else {
        return Err(Error::Parse("Startseite ohne sectionListRenderer".into()));
    };
    Ok((parse_sections(list.arr_at("/contents")), next_token(list)))
}

/// Weitere Regale (`continuationContents.sectionListContinuation`).
pub(crate) fn parse_home_continuation(resp: &Value) -> (Vec<YtShelf>, Option<String>) {
    match resp.at("/continuationContents/sectionListContinuation") {
        Some(list) => (parse_sections(list.arr_at("/contents")), next_token(list)),
        None => (Vec::new(), None),
    }
}

fn next_token(list: &Value) -> Option<String> {
    list.str_at("/continuations/0/nextContinuationData/continuation")
        .map(str::to_string)
}

/// Karussells → Regale; unbekannte Renderer und leere Regale fallen weg.
pub(crate) fn parse_sections(sections: &[Value]) -> Vec<YtShelf> {
    sections
        .iter()
        .filter_map(|s| {
            s.get("musicCarouselShelfRenderer")
                .or_else(|| s.get("musicImmersiveCarouselShelfRenderer"))
        })
        .filter_map(parse_carousel)
        .collect()
}

fn parse_carousel(carousel: &Value) -> Option<YtShelf> {
    let title = [
        "/header/musicCarouselShelfBasicHeaderRenderer/title",
        "/header/musicImmersiveCarouselShelfBasicHeaderRenderer/title",
        "/header/musicCarouselShelfBasicHeaderRenderer/strapline",
    ]
    .iter()
    .find_map(|p| carousel.text_at(p))?;
    let items: Vec<YtItem> = carousel
        .arr_at("/contents")
        .iter()
        .filter_map(|entry| {
            if let Some(tile) = entry.get("musicTwoRowItemRenderer") {
                item_from_two_row(tile, "Album")
            } else if let Some(row) = entry.get("musicResponsiveListItemRenderer") {
                item_from_list_item(row, true)
            } else {
                None
            }
        })
        .collect();
    (!items.is_empty()).then_some(YtShelf { title, items })
}
