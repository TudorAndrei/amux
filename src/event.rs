use crate::lifecycle::{self, PolicyInput};
use crate::model::Record;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const ALLOWLISTED_RAW_STRINGS: &[&str] = &[
    "session_id",
    "cwd",
    "hook_event_name",
    "source",
    "turn_id",
    "tool_name",
    "permission_mode",
    "reason",
    "agent_id",
    "agent_type",
    "parent_agent_id",
    "parent_session_id",
    "is_subagent",
];
const MAX_RAW_BYTES: usize = 4 * 1024;
const MAX_CONTAINER_ENTRIES: usize = 32;
const MAX_RAW_DEPTH: usize = 3;

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Preserve useful hook metadata while putting a hard, serialized-size bound on
/// untrusted hook payloads. The value is intentionally JSON-shaped so existing
/// integrations can continue to inspect the retained fields.
pub fn compact_raw(raw: Value) -> Value {
    let compacted = compact_value(raw, 0, None);
    // `compact_value` greedily checks container candidates, but a scalar string
    // can still be large enough to exceed the envelope by itself.
    if serde_json::to_vec(&compacted).is_ok_and(|serialized| serialized.len() <= MAX_RAW_BYTES) {
        compacted
    } else {
        Value::String(truncate_utf8(
            compacted.as_str().unwrap_or_default(),
            MAX_RAW_BYTES.saturating_sub(2),
        ))
    }
}

fn compact_value(value: Value, depth: usize, key: Option<&str>) -> Value {
    match value {
        Value::String(value) => Value::String(truncate_utf8(
            &value,
            if key.is_some_and(|key| ALLOWLISTED_RAW_STRINGS.contains(&key)) {
                1024
            } else {
                256
            },
        )),
        Value::Array(_) if depth >= MAX_RAW_DEPTH => Value::Null,
        Value::Array(values) => {
            let mut output = Vec::new();
            for value in values.into_iter().take(MAX_CONTAINER_ENTRIES) {
                let candidate = compact_value(value, depth + 1, None);
                let mut proposed = output.clone();
                proposed.push(candidate);
                if serde_json::to_vec(&Value::Array(proposed.clone()))
                    .is_ok_and(|serialized| serialized.len() <= MAX_RAW_BYTES)
                {
                    output = proposed;
                } else {
                    break;
                }
            }
            Value::Array(output)
        }
        Value::Object(_) if depth >= MAX_RAW_DEPTH => Value::Null,
        Value::Object(values) => {
            let mut output = serde_json::Map::new();
            for (key, value) in values.into_iter().take(MAX_CONTAINER_ENTRIES) {
                let candidate = compact_value(value, depth + 1, Some(&key));
                let mut proposed = output.clone();
                proposed.insert(key.clone(), candidate);
                if serde_json::to_vec(&Value::Object(proposed.clone()))
                    .is_ok_and(|serialized| serialized.len() <= MAX_RAW_BYTES)
                {
                    output = proposed;
                } else {
                    break;
                }
            }
            Value::Object(output)
        }
        value => value,
    }
}

fn raw_string(raw: &Value, paths: &[&[&str]]) -> String {
    for path in paths {
        let mut item = raw;
        for key in *path {
            item = match item.get(*key) {
                Some(value) => value,
                None => break,
            };
        }
        if let Some(value) = item.as_str() {
            return value.to_owned();
        }
    }
    String::new()
}

pub(crate) fn tmux_value(format: &str, pane: &str) -> String {
    if env::var_os("TMUX").is_none() {
        return String::new();
    }
    let mut command = Command::new("tmux");
    command.arg("display-message").arg("-p");
    if !pane.is_empty() {
        command.arg("-t").arg(pane);
    }
    command.arg(format);
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TmuxContext {
    pub pane: String,
    pub session: String,
    pub window: String,
}

pub struct NormalizeInput<'a> {
    pub agent: &'a str,
    pub event_override: &'a str,
    pub status_override: &'a str,
    pub attention_override: &'a str,
    pub reason_override: &'a str,
    pub raw: Value,
    pub fallback_cwd: String,
    pub tmux: TmuxContext,
}

pub fn current_tmux_context() -> TmuxContext {
    if env::var_os("TMUX").is_none() {
        return TmuxContext::default();
    }
    let pane = env::var("TMUX_PANE").unwrap_or_else(|_| tmux_value("#{pane_id}", ""));
    TmuxContext {
        session: tmux_value("#{session_name}", &pane),
        window: tmux_value("#{window_id}", &pane),
        pane,
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn iso_now(seconds: i64) -> String {
    // The legacy format is UTC RFC3339. This compact conversion deliberately has no locale dependency.
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (y + if month <= 2 { 1 } else { 0 }, month, day)
}

pub fn normalize(
    agent: &str,
    event_override: &str,
    status_override: &str,
    attention_override: &str,
    reason_override: &str,
    raw: Value,
) -> (String, Record) {
    let cwd = env::current_dir()
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_default();
    normalize_at(NormalizeInput {
        agent,
        event_override,
        status_override,
        attention_override,
        reason_override,
        raw,
        fallback_cwd: cwd,
        tmux: current_tmux_context(),
    })
}

pub fn normalize_at(input: NormalizeInput<'_>) -> (String, Record) {
    let NormalizeInput {
        agent,
        event_override,
        status_override,
        attention_override,
        reason_override,
        raw,
        fallback_cwd,
        tmux,
    } = input;
    let event = if !event_override.is_empty() {
        event_override.to_owned()
    } else {
        raw_string(&raw, &[&["hook_event_name"], &["event", "type"], &["type"]])
            .if_empty("activity".to_owned())
    };
    let agent_session_id = raw_string(
        &raw,
        &[
            &["session_id"],
            &["sessionID"],
            &["sessionId"],
            &["session", "id"],
            &["id"],
        ],
    );
    let cwd = raw_string(&raw, &[&["cwd"], &["directory"], &["project", "directory"]])
        .if_empty(fallback_cwd);
    let tmux_pane = tmux.pane;
    let tmux_session = tmux.session;
    let tmux_window = tmux.window;
    let classification = lifecycle::classify(PolicyInput {
        agent,
        event: &event,
        status_override,
        attention_override,
        reason_override,
        metadata: &raw,
    });
    let attention = classification.attention;
    let status = classification.status;
    let reason = truncate_utf8(&classification.reason, 256);
    let cwd = truncate_utf8(&cwd, 1024);
    let timestamp = now();
    let key = if !tmux_session.is_empty() && !tmux_pane.is_empty() {
        format!("{agent}:{tmux_session}:{tmux_pane}")
    } else if !agent_session_id.is_empty() {
        format!("{agent}:{agent_session_id}")
    } else {
        format!("{agent}:{cwd}")
    };
    (
        key,
        Record {
            agent: agent.to_owned(),
            agent_session_id,
            tmux_session,
            tmux_window,
            tmux_pane,
            cwd,
            status,
            attention,
            reason,
            last_event: event,
            updated_at: timestamp,
            updated_at_iso: iso_now(timestamp),
            raw: compact_raw(raw),
        },
    )
}

trait IfEmpty {
    fn if_empty(self, fallback: String) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, fallback: String) -> String {
        if self.is_empty() { fallback } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_raw_enforces_all_payload_budgets_without_losing_subagent_metadata() {
        let mut wide = serde_json::Map::new();
        for index in 0..100 {
            wide.insert(format!("field-{index:03}"), Value::String("x".repeat(20)));
        }
        let raw = serde_json::json!({
            "session_id": "session-1",
            "agent_id": "child-1",
            "agent_type": "worker",
            "cwd": "/workspace",
            "last_assistant_message": "x".repeat(10_000),
            "wide": wide,
            "nested": {"one": {"two": {"three": {"four": "discard"}}}}
        });
        let compacted = compact_raw(raw);
        assert_eq!(compacted["session_id"], "session-1");
        assert_eq!(compacted["agent_id"], "child-1");
        assert_eq!(compacted["agent_type"], "worker");
        assert_eq!(
            compacted["last_assistant_message"].as_str().unwrap().len(),
            256
        );
        assert!(compacted["wide"].as_object().unwrap().len() <= MAX_CONTAINER_ENTRIES);
        assert!(serde_json::to_vec(&compacted).unwrap().len() <= MAX_RAW_BYTES);
    }
}
