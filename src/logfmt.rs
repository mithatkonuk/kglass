//! Readable rendering of structured (JSON) log lines, e.g. Elastic Common Schema.
//!
//! Output uses the same column layout as plain logs —
//! `YYYY-MM-DD HH:MM:SS.mmm LEVEL logger message` — so level colouring applies.

use serde_json::{Map, Value};

use crate::text::sanitize;

const TS_WIDTH: usize = 23;
const LOGGER_WIDTH: usize = 16;

const TIMESTAMP_KEYS: &[&str] = &["@timestamp", "timestamp", "time", "ts"];
const LEVEL_KEYS: &[&str] = &["log.level", "level", "severity", "lvl"];
const LOGGER_KEYS: &[&str] = &["log.logger", "logger", "logger_name", "caller"];
const MESSAGE_KEYS: &[&str] = &["message", "msg"];

/// A value by dotted path: a literal `"log.level"` key, or nested `log` → `level`.
fn lookup<'a>(obj: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    if let Some(v) = obj.get(path) {
        return Some(v);
    }
    let (head, rest) = path.split_once('.')?;
    lookup(obj.get(head)?.as_object()?, rest)
}

fn first_str<'a>(obj: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| lookup(obj, k)?.as_str())
}

/// `2026-11-06T08:47:21.082960321Z` → `2026-11-06 08:47:21.082`, padded to 23 columns.
fn timestamp(raw: Option<&str>) -> String {
    let formatted = match raw {
        Some(ts) if ts.as_bytes().get(10) == Some(&b'T') => {
            let date = ts.get(..10).unwrap_or_default();
            let time = ts.get(11..19).unwrap_or_default();
            let millis: String = ts
                .get(19..)
                .and_then(|r| r.strip_prefix('.'))
                .map(|r| r.chars().take_while(char::is_ascii_digit).take(3).collect())
                .unwrap_or_default();
            let millis = if millis.is_empty() {
                "000".to_string()
            } else {
                millis
            };
            format!("{date} {time}.{millis}")
        }
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let cut: String = formatted.chars().take(TS_WIDTH).collect();
    format!("{cut:<TS_WIDTH$}")
}

/// Upper-case level, with common spellings mapped to the 5-letter forms that get colour.
fn normalize_level(level: &str) -> String {
    match level.to_uppercase().as_str() {
        "WARNING" => "WARN".to_string(),
        "ERR" => "ERROR".to_string(),
        "INFORMATION" => "INFO".to_string(),
        other => other.to_string(),
    }
}

/// `org.example.HttpExporter` → `HttpExporter`; `pkg/handler.go:42` → `handler.go:42`.
fn short_logger(logger: &str) -> &str {
    if logger.contains('/') {
        logger.rsplit('/').next().unwrap_or(logger)
    } else {
        logger.rsplit('.').next().unwrap_or(logger)
    }
}

/// Format a JSON log line, or `None` when it is not a JSON object with a message.
pub fn format_line(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let obj = value.as_object()?;
    let message = first_str(obj, MESSAGE_KEYS)?;
    let level = normalize_level(first_str(obj, LEVEL_KEYS).unwrap_or_default());
    let logger = first_str(obj, LOGGER_KEYS)
        .map(short_logger)
        .unwrap_or_default();
    let error = match (
        first_str(obj, &["error.type"]),
        first_str(obj, &["error.message"]),
    ) {
        (Some(kind), Some(msg)) => format!(" ✕ {kind}: {msg}"),
        (Some(only), None) | (None, Some(only)) => format!(" ✕ {only}"),
        // logrus / zap put the error text in a plain `error` (or `err`) string.
        (None, None) => first_str(obj, &["error", "err"])
            .map(|e| format!(" ✕ {e}"))
            .unwrap_or_default(),
    };
    let ts = timestamp(first_str(obj, TIMESTAMP_KEYS));
    // JSON may encode control characters (\u001b, \n); sanitize after decoding.
    Some(sanitize(&format!(
        "{ts} {level:<5} {logger:<LOGGER_WIDTH$} {message}{error}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecs_line_formatted() {
        let raw = r#"{"@timestamp":"2026-11-06T08:47:21.082960321Z","log":{"level":"ERROR","logger":"io.opentelemetry.exporter.internal.http.HttpExporter"},"message":"Failed to export spans.","error":{"type":"java.net.UnknownHostException","message":"collector.monitoring","stack_trace":"java.net.Unknown..."},"ecs":{"version":"8.11"}}"#;
        assert_eq!(
            format_line(raw).unwrap(),
            "2026-11-06 08:47:21.082 ERROR HttpExporter     Failed to export spans. ✕ java.net.UnknownHostException: collector.monitoring"
        );
    }

    #[test]
    fn nested_and_dotted_keys() {
        let dotted = r#"{"@timestamp":"2026-11-06T08:46:02Z","log.level":"info","log.logger":"a.b.Starter","message":"hi"}"#;
        assert_eq!(
            format_line(dotted).unwrap(),
            "2026-11-06 08:46:02.000 INFO  Starter          hi"
        );
        let plain_keys =
            r#"{"time":"2026-11-06T08:46:02.5+01:00","level":"warn","logger":"db","msg":"slow"}"#;
        assert_eq!(
            format_line(plain_keys).unwrap(),
            "2026-11-06 08:46:02.5   WARN  db               slow"
        );
    }

    #[test]
    fn error_fields_appended_type_only() {
        let raw = r#"{"message":"boom","error":{"type":"IOError"}}"#;
        assert!(format_line(raw).unwrap().ends_with("boom ✕ IOError"));
    }

    #[test]
    fn non_json_or_without_message_is_none() {
        for raw in [
            "plain text line",
            "[1,2,3]",
            "42",
            r#"{"a":1}"#,
            "{not json",
            "",
        ] {
            assert_eq!(format_line(raw), None, "{raw}");
        }
    }

    #[test]
    fn numeric_timestamp_and_missing_fields_keep_alignment() {
        let out = format_line(r#"{"ts":1730880000.5,"severity":"debug","msg":"x"}"#).unwrap();
        assert_eq!(out, format!("{} DEBUG {:<16} x", " ".repeat(23), ""));
    }

    #[test]
    fn zap_and_logrus_shapes() {
        let zap = r#"{"ts":"2026-11-06T08:00:00Z","level":"warning","caller":"pkg/handler.go:42","msg":"slow","error":"connection refused"}"#;
        assert_eq!(
            format_line(zap).unwrap(),
            "2026-11-06 08:00:00.000 WARN  handler.go:42    slow ✕ connection refused"
        );
    }

    #[test]
    fn decoded_escape_sequences_are_stripped() {
        let out = format_line(r#"{"message":"red \u001b[31malert\u001b[0m\nnext"}"#).unwrap();
        assert!(!out.contains('\u{1b}') && !out.contains('\n'), "{out:?}");
        assert!(out.ends_with("red alert next"), "{out:?}");
    }
}
