use crate::event::TmuxContext;
use crate::ipc::HookRequest;
use crate::lifecycle::{self, PolicyInput};
use crate::model::{HistoryEvent, Record};
use serde::Serialize;
use serde_json::{Map, Value};

pub const MAX_INPUT_BYTES: u64 = 256 * 1024;
const MAX_AGENT_BYTES: usize = 64;
const MAX_EVENT_BYTES: usize = 128;
const MAX_SESSION_BYTES: usize = 256;
const MAX_TMUX_BYTES: usize = 256;
const MAX_CWD_BYTES: usize = 1024;
const MAX_REASON_BYTES: usize = 256;
const MAX_STATUS_BYTES: usize = 32;
const MAX_METADATA_STRING_BYTES: usize = 1024;

const ROOT_STRING_FIELDS: &[&str] = &[
    "session_id",
    "sessionID",
    "sessionId",
    "id",
    "cwd",
    "directory",
    "hook_event_name",
    "type",
    "notification_type",
    "notificationType",
    "source",
    "turn_id",
    "reason",
    "agent_id",
    "agent_type",
    "parent_agent_id",
    "parent_session_id",
];

/// The complete, safe output for one durable event write.
///
/// The input is deliberately singular. One CLI or IPC request carries one
/// event and waits for one durable result, so a batch interface would change
/// the current protocol and its durability timing.
#[derive(Debug)]
pub struct ProjectedEvent {
    pub key: String,
    pub record: Record,
    pub history: Value,
}

/// Transform one owned hook request into the canonical state-v1 record and
/// JSONL history value. This module owns all projection policy; it performs no
/// file, socket, process, or environment input/output.
pub fn project(
    mut request: HookRequest,
    mut tmux: TmuxContext,
    timestamp: i64,
) -> Result<ProjectedEvent, String> {
    if serde_json::to_vec(&request.raw)
        .map_err(|error| error.to_string())?
        .len() as u64
        > MAX_INPUT_BYTES
    {
        return Err(format!(
            "event input exceeds {} KiB",
            MAX_INPUT_BYTES / 1024
        ));
    }

    request.agent = truncate_utf8(&request.agent, MAX_AGENT_BYTES);
    request.event = truncate_utf8(&request.event, MAX_EVENT_BYTES);
    request.status = truncate_utf8(&request.status, MAX_STATUS_BYTES);
    request.attention = truncate_utf8(&request.attention, 16);
    request.reason = truncate_utf8(&request.reason, MAX_REASON_BYTES);
    request.cwd = truncate_utf8(&request.cwd, MAX_CWD_BYTES);
    tmux.session = truncate_utf8(&tmux.session, MAX_TMUX_BYTES);
    tmux.window = truncate_utf8(&tmux.window, MAX_TMUX_BYTES);
    tmux.pane = truncate_utf8(&tmux.pane, MAX_TMUX_BYTES);

    let raw = retained_metadata(&request.raw);
    let event = if request.event.is_empty() {
        raw_string(&raw, &[&["hook_event_name"], &["event", "type"], &["type"]])
            .if_empty("activity".to_owned())
    } else {
        request.event
    };
    let agent_session_id = truncate_utf8(
        &raw_string(
            &raw,
            &[
                &["session_id"],
                &["sessionID"],
                &["sessionId"],
                &["session", "id"],
                &["event", "properties", "sessionID"],
                &["id"],
            ],
        ),
        MAX_SESSION_BYTES,
    );
    let cwd = truncate_utf8(
        &raw_string(&raw, &[&["cwd"], &["directory"], &["project", "directory"]])
            .if_empty(request.cwd),
        MAX_CWD_BYTES,
    );
    let classification = lifecycle::classify(PolicyInput {
        agent: &request.agent,
        event: &event,
        status_override: &request.status,
        attention_override: &request.attention,
        reason_override: &request.reason,
        metadata: &raw,
    });
    let record = Record {
        agent: request.agent,
        agent_session_id,
        tmux_session: tmux.session,
        tmux_window: tmux.window,
        tmux_pane: tmux.pane,
        cwd,
        status: truncate_utf8(&classification.status, MAX_STATUS_BYTES),
        attention: classification.attention,
        reason: truncate_utf8(&classification.reason, MAX_REASON_BYTES),
        last_event: truncate_utf8(&event, MAX_EVENT_BYTES),
        updated_at: timestamp,
        updated_at_iso: iso_at(timestamp),
        raw,
    };
    let key = record_key(&record);
    let history = history_value(&key, &record)?;
    Ok(ProjectedEvent {
        key,
        record,
        history,
    })
}

/// Apply current privacy, cap, and identity policy to a stored JSONL record.
/// Malformed JSON is rejected by the state reader before this transform.
pub fn canonical_history(mut event: HistoryEvent) -> HistoryEvent {
    event.record.agent = truncate_utf8(&event.record.agent, MAX_AGENT_BYTES);
    event.record.agent_session_id =
        truncate_utf8(&event.record.agent_session_id, MAX_SESSION_BYTES);
    event.record.tmux_session = truncate_utf8(&event.record.tmux_session, MAX_TMUX_BYTES);
    event.record.tmux_window = truncate_utf8(&event.record.tmux_window, MAX_TMUX_BYTES);
    event.record.tmux_pane = truncate_utf8(&event.record.tmux_pane, MAX_TMUX_BYTES);
    event.record.cwd = truncate_utf8(&event.record.cwd, MAX_CWD_BYTES);
    event.record.status = truncate_utf8(&event.record.status, MAX_STATUS_BYTES);
    event.record.reason = truncate_utf8(&event.record.reason, MAX_REASON_BYTES);
    event.record.last_event = truncate_utf8(&event.record.last_event, MAX_EVENT_BYTES);
    event.record.raw = retained_metadata(&event.record.raw);
    event.key = record_key(&event.record);
    event
}

fn record_key(record: &Record) -> String {
    if !record.tmux_session.is_empty() && !record.tmux_pane.is_empty() {
        format!(
            "{}:{}:{}",
            record.agent, record.tmux_session, record.tmux_pane
        )
    } else if !record.agent_session_id.is_empty() {
        format!("{}:{}", record.agent, record.agent_session_id)
    } else {
        format!("{}:{}", record.agent, record.cwd)
    }
}

#[derive(Serialize)]
struct HistoryValue<'a> {
    key: &'a str,
    #[serde(flatten)]
    record: &'a Record,
}

fn history_value(key: &str, record: &Record) -> Result<Value, String> {
    serde_json::to_value(HistoryValue { key, record }).map_err(|error| error.to_string())
}

fn retained_metadata(raw: &Value) -> Value {
    let Some(input) = raw.as_object() else {
        return Value::Object(Map::new());
    };
    let mut output = Map::new();
    for key in ROOT_STRING_FIELDS {
        if let Some(value) = input.get(*key).and_then(Value::as_str) {
            output.insert(
                (*key).to_owned(),
                Value::String(truncate_utf8(value, metadata_limit(key))),
            );
        }
    }
    if let Some(value) = input.get("is_subagent").and_then(Value::as_bool) {
        output.insert("is_subagent".to_owned(), Value::Bool(value));
    }
    retain_nested(input, &mut output, "event", &["type"]);
    if let Some(event) = input.get("event").and_then(Value::as_object)
        && let Some(session) = event.get("session").and_then(Value::as_object)
        && let Some(id) = session.get("id").and_then(Value::as_str)
    {
        output
            .entry("event".to_owned())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("event projection is an object")
            .insert(
                "session".to_owned(),
                serde_json::json!({"id": truncate_utf8(id, MAX_SESSION_BYTES)}),
            );
    }
    if let Some(event) = input.get("event").and_then(Value::as_object)
        && let Some(properties) = event.get("properties").and_then(Value::as_object)
    {
        let mut retained_properties = Map::new();
        if let Some(session_id) = properties.get("sessionID").and_then(Value::as_str) {
            retained_properties.insert(
                "sessionID".to_owned(),
                Value::String(truncate_utf8(session_id, MAX_SESSION_BYTES)),
            );
        }
        if let Some(status_type) = properties
            .get("status")
            .and_then(Value::as_object)
            .and_then(|status| status.get("type"))
            .and_then(Value::as_str)
        {
            retained_properties.insert(
                "status".to_owned(),
                serde_json::json!({
                    "type": truncate_utf8(status_type, MAX_STATUS_BYTES)
                }),
            );
        }
        if !retained_properties.is_empty() {
            output
                .entry("event".to_owned())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .expect("event projection is an object")
                .insert("properties".to_owned(), Value::Object(retained_properties));
        }
    }
    retain_nested(input, &mut output, "session", &["id"]);
    retain_nested(input, &mut output, "project", &["directory"]);
    Value::Object(output)
}

fn metadata_limit(key: &str) -> usize {
    match key {
        "session_id" | "sessionID" | "sessionId" | "id" | "turn_id" | "agent_id"
        | "parent_agent_id" | "parent_session_id" => MAX_SESSION_BYTES,
        "cwd" | "directory" => MAX_CWD_BYTES,
        "hook_event_name" | "type" | "notification_type" | "notificationType" => MAX_EVENT_BYTES,
        "reason" => MAX_REASON_BYTES,
        _ => MAX_METADATA_STRING_BYTES,
    }
}

fn retain_nested(
    input: &Map<String, Value>,
    output: &mut Map<String, Value>,
    container: &str,
    fields: &[&str],
) {
    let Some(source) = input.get(container).and_then(Value::as_object) else {
        return;
    };
    let mut retained = output
        .remove(container)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for field in fields {
        if let Some(value) = source.get(*field).and_then(Value::as_str) {
            retained.insert(
                (*field).to_owned(),
                Value::String(truncate_utf8(value, MAX_METADATA_STRING_BYTES)),
            );
        }
    }
    if !retained.is_empty() {
        output.insert(container.to_owned(), Value::Object(retained));
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

fn iso_at(seconds: i64) -> String {
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
    use std::path::PathBuf;

    const FIXTURES: &[(&str, &str, &[u8])] = &[
        (
            "claude",
            "claude-stop.json",
            include_bytes!("../tests/fixtures/claude-stop.json"),
        ),
        (
            "codex",
            "codex-permission.json",
            include_bytes!("../tests/fixtures/codex-permission.json"),
        ),
        (
            "codex",
            "codex-subagent-empty-cwd.json",
            include_bytes!("../tests/fixtures/codex-subagent-empty-cwd.json"),
        ),
        (
            "codex",
            "codex-subagent.json",
            include_bytes!("../tests/fixtures/codex-subagent.json"),
        ),
        (
            "opencode",
            "opencode-idle.json",
            include_bytes!("../tests/fixtures/opencode-idle.json"),
        ),
        (
            "opencode",
            "opencode-permission-asked.json",
            include_bytes!("../tests/fixtures/opencode-permission-asked.json"),
        ),
        (
            "opencode",
            "opencode-permission-replied.json",
            include_bytes!("../tests/fixtures/opencode-permission-replied.json"),
        ),
        (
            "opencode",
            "opencode-question-asked.json",
            include_bytes!("../tests/fixtures/opencode-question-asked.json"),
        ),
        (
            "opencode",
            "opencode-session-busy.json",
            include_bytes!("../tests/fixtures/opencode-session-busy.json"),
        ),
        (
            "opencode",
            "opencode-session-created.json",
            include_bytes!("../tests/fixtures/opencode-session-created.json"),
        ),
        (
            "opencode",
            "opencode-session-deleted.json",
            include_bytes!("../tests/fixtures/opencode-session-deleted.json"),
        ),
        (
            "opencode",
            "opencode-session-idle.json",
            include_bytes!("../tests/fixtures/opencode-session-idle.json"),
        ),
        (
            "pi",
            "pi-agent-settled.json",
            include_bytes!("../tests/fixtures/pi-agent-settled.json"),
        ),
        (
            "pi",
            "pi-agent-start.json",
            include_bytes!("../tests/fixtures/pi-agent-start.json"),
        ),
        (
            "pi",
            "pi-session-shutdown.json",
            include_bytes!("../tests/fixtures/pi-session-shutdown.json"),
        ),
        (
            "pi",
            "pi-session-start.json",
            include_bytes!("../tests/fixtures/pi-session-start.json"),
        ),
        (
            "pi",
            "pi-tool-call.json",
            include_bytes!("../tests/fixtures/pi-tool-call.json"),
        ),
    ];

    fn request(agent: &str, raw: Value) -> HookRequest {
        HookRequest {
            agent: agent.to_owned(),
            event: String::new(),
            status: String::new(),
            attention: String::new(),
            reason: String::new(),
            raw,
            cwd: "/fallback".to_owned(),
            tmux_pane: String::new(),
            tmux_session: String::new(),
            tmux_window: String::new(),
            tmux_server: None,
        }
    }

    #[test]
    fn all_adapter_fixtures_match_the_recorded_v1_projection() {
        let expected = include_str!("../tests/fixtures/projected-v1.jsonl")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), FIXTURES.len());

        for ((agent, name, bytes), expected) in FIXTURES.iter().zip(expected) {
            assert_eq!(expected["fixture"], *name);
            let raw = serde_json::from_slice(bytes).unwrap();
            let projected = project(request(agent, raw), TmuxContext::default(), 0).unwrap();
            assert_eq!(
                serde_json::json!({
                    "fixture": name,
                    "key": &projected.key,
                    "record": &projected.record,
                }),
                expected,
                "{name}"
            );
            let mut expected_history = expected["record"].as_object().unwrap().clone();
            expected_history.insert("key".to_owned(), expected["key"].clone());
            assert_eq!(
                projected.history,
                Value::Object(expected_history),
                "{name} history"
            );
        }
    }

    #[test]
    fn projection_rejects_oversized_raw_data() {
        let raw = serde_json::json!({"unknown": "x".repeat(MAX_INPUT_BYTES as usize)});
        assert!(
            project(request("codex", raw), TmuxContext::default(), 0)
                .unwrap_err()
                .contains("exceeds 256 KiB")
        );
    }

    #[test]
    fn projection_drops_unknown_metadata_and_keeps_subagent_identity() {
        let raw = serde_json::json!({
            "session_id": "session",
            "agent_id": "child",
            "agent_type": "explorer",
            "parent_session_id": "parent",
            "is_subagent": true,
            "cwd": "/workspace",
            "tool_input": {"command": "secret"},
            "message": "secret prompt",
            "command": "rm -rf secret",
            "unknown": "secret"
        });
        let projected = project(request("codex", raw), TmuxContext::default(), 0).unwrap();

        assert_eq!(projected.record.raw["agent_id"], "child");
        assert_eq!(projected.record.raw["is_subagent"], true);
        for sensitive in ["tool_input", "message", "command", "unknown"] {
            assert!(
                projected.record.raw.get(sensitive).is_none(),
                "retained {sensitive}"
            );
            assert!(!projected.history.to_string().contains(sensitive));
        }
    }

    #[test]
    fn identity_order_and_utf8_caps_are_canonical() {
        let repeated = "🦀".repeat(400);
        let mut input = request(
            &"é".repeat(100),
            serde_json::json!({"session_id": repeated, "cwd": "/raw"}),
        );
        input.cwd = "/request".to_owned();
        let with_tmux = project(
            input.clone(),
            TmuxContext {
                session: "tmux".to_owned(),
                pane: "%1".to_owned(),
                window: "@1".to_owned(),
            },
            0,
        )
        .unwrap();
        assert_eq!(
            with_tmux.key,
            format!("{}:tmux:%1", truncate_utf8(&input.agent, MAX_AGENT_BYTES))
        );

        let with_session = project(input, TmuxContext::default(), 0).unwrap();
        assert!(with_session.record.agent.len() <= MAX_AGENT_BYTES);
        assert!(with_session.record.agent_session_id.len() <= MAX_SESSION_BYTES);
        assert_eq!(
            with_session.key,
            format!(
                "{}:{}",
                with_session.record.agent, with_session.record.agent_session_id
            )
        );
        assert!(std::str::from_utf8(with_session.key.as_bytes()).is_ok());

        let by_cwd = project(
            request("pi", serde_json::json!({})),
            TmuxContext::default(),
            0,
        )
        .unwrap();
        assert_eq!(by_cwd.key, "pi:/fallback");
    }

    #[test]
    fn stored_history_uses_the_same_key_caps_and_privacy_policy() {
        let event = HistoryEvent {
            key: "stale".to_owned(),
            record: Record {
                agent: "codex".to_owned(),
                agent_session_id: "session".to_owned(),
                reason: "🦀".repeat(400),
                raw: serde_json::json!({
                    "session_id": "session",
                    "source": "v1",
                    "secret": "drop"
                }),
                ..Record::default()
            },
        };

        let canonical = canonical_history(event);

        assert_eq!(canonical.key, "codex:session");
        assert!(canonical.record.reason.len() <= MAX_REASON_BYTES);
        assert_eq!(canonical.record.raw["source"], "v1");
        assert!(canonical.record.raw.get("secret").is_none());
    }

    #[test]
    fn projected_data_has_no_path_or_server_handles() {
        let mut input = request("codex", serde_json::json!({"session_id": "one"}));
        input.tmux_server = Some(PathBuf::from("/tmp/tmux-server"));
        let projected = project(input, TmuxContext::default(), 0).unwrap();
        assert!(!projected.history.to_string().contains("tmux-server"));
    }
}
