//! Kleine Helfer zum Navigieren in den InnerTube-Antworten. Alle Zugriffe laufen über
//! JSON-Pointer (`/contents/0/…`) und liefern `None` bzw. eine leere Liste, wenn etwas
//! fehlt – nie eine Panik.

use serde_json::Value;

pub(crate) trait JsonExt {
    /// Wert unter einem JSON-Pointer (`""` = der Wert selbst).
    fn at(&self, ptr: &str) -> Option<&Value>;
    /// Zeichenkette unter einem JSON-Pointer.
    fn str_at(&self, ptr: &str) -> Option<&str>;
    /// Liste unter einem JSON-Pointer, leer wenn sie fehlt.
    fn arr_at(&self, ptr: &str) -> &[Value];
    /// Text eines `{"runs": […]}`- oder `{"simpleText": …}`-Objekts unter dem Pointer.
    fn text_at(&self, ptr: &str) -> Option<String>;
}

impl JsonExt for Value {
    fn at(&self, ptr: &str) -> Option<&Value> {
        self.pointer(ptr)
    }

    fn str_at(&self, ptr: &str) -> Option<&str> {
        self.pointer(ptr).and_then(Value::as_str)
    }

    fn arr_at(&self, ptr: &str) -> &[Value] {
        self.pointer(ptr)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn text_at(&self, ptr: &str) -> Option<String> {
        self.pointer(ptr).and_then(text_of)
    }
}

/// Text eines Text-Objekts: alle `runs` aneinandergehängt, sonst `simpleText` oder `content`.
/// Leere Texte ergeben `None`.
pub(crate) fn text_of(v: &Value) -> Option<String> {
    let text = if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        runs.iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect::<String>()
    } else if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        s.to_string()
    } else if let Some(s) = v.get("content").and_then(Value::as_str) {
        s.to_string()
    } else {
        v.as_str()?.to_string()
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Ein einzelner Text-Run (`{"text": …, "navigationEndpoint": …}`).
#[derive(Clone, Copy)]
pub(crate) struct Run<'a>(pub &'a Value);

impl<'a> Run<'a> {
    pub fn text(&self) -> &'a str {
        self.0.str_at("/text").unwrap_or("")
    }

    pub fn has_endpoint(&self) -> bool {
        self.0.get("navigationEndpoint").is_some()
    }

    pub fn browse_id(&self) -> Option<&'a str> {
        self.0.str_at("/navigationEndpoint/browseEndpoint/browseId")
    }

    pub fn page_type(&self) -> Option<&'a str> {
        self.0.str_at(
            "/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
        )
    }

    pub fn video_id(&self) -> Option<&'a str> {
        self.0.str_at("/navigationEndpoint/watchEndpoint/videoId")
    }

    pub fn video_type(&self) -> Option<&'a str> {
        self.0.str_at(
            "/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
        )
    }

    /// Reiner Trenner wie " • ", ", " oder " & " (keine Buchstaben, keine Ziffern).
    pub fn is_separator(&self) -> bool {
        !self.has_endpoint() && !self.text().chars().any(char::is_alphanumeric)
    }

    /// Genau der Punkt-Trenner " • ".
    pub fn is_dot(&self) -> bool {
        !self.has_endpoint() && self.text().trim() == "•"
    }
}

/// Runs eines Text-Objekts (`{"runs": […]}`) unter einem Pointer.
pub(crate) fn runs_at<'a>(v: &'a Value, ptr: &str) -> Vec<Run<'a>> {
    v.arr_at(ptr).iter().map(Run).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn navigation_never_panics() {
        let v = json!({"a": [{"b": "x"}], "t": {"runs": [{"text": "A"}, {"text": " • "}, {"text": "B"}]}});
        assert_eq!(v.str_at("/a/0/b"), Some("x"));
        assert_eq!(v.str_at("/a/5/b"), None);
        assert_eq!(v.str_at("/a/0/b/c"), None);
        assert!(v.arr_at("/nope").is_empty());
        assert_eq!(v.text_at("/t").as_deref(), Some("A • B"));
        assert_eq!(v.text_at("/a"), None);
        assert_eq!(
            text_of(&json!({"simpleText": " 3:45 "})).as_deref(),
            Some("3:45")
        );
        assert_eq!(text_of(&json!({"runs": []})), None);
    }

    #[test]
    fn separators() {
        assert!(Run(&json!({"text": " • "})).is_dot());
        assert!(Run(&json!({"text": ", "})).is_separator());
        assert!(Run(&json!({"text": " & "})).is_separator());
        assert!(!Run(&json!({"text": " und "})).is_separator());
        assert!(!Run(&json!({"text": "•", "navigationEndpoint": {}})).is_dot());
    }
}
