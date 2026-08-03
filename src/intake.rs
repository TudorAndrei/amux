use crate::config::Config;
use crate::event::{self, NormalizeInput, TmuxContext};
use crate::ipc::HookRequest;
use crate::model::Record;
use crate::state::WriteEventResult;
use serde_json::{Map, Value};
use std::io::Read;

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

pub fn read_hook_json(mut reader: impl Read) -> Result<Value, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(format!(
            "event input exceeds {} KiB",
            MAX_INPUT_BYTES / 1024
        ));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_slice(&bytes).map_err(|_| "event input is not valid JSON".to_owned())
}

pub fn persist(
    config: &Config,
    mut request: HookRequest,
    mut tmux: TmuxContext,
) -> Result<WriteEventResult, String> {
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
    let (_, mut record) = event::normalize_at(NormalizeInput {
        agent: &request.agent,
        event_override: &request.event,
        status_override: &request.status,
        attention_override: &request.attention,
        reason_override: &request.reason,
        raw: raw.clone(),
        fallback_cwd: request.cwd,
        tmux,
    });
    minimize_record(&mut record, raw);
    let key = record_key(&record);
    let timestamp = record.updated_at;
    let mut fields = serde_json::to_value(&record)
        .map_err(|error| error.to_string())?
        .as_object()
        .cloned()
        .unwrap_or_default();
    fields.insert("key".to_owned(), Value::String(key.clone()));
    crate::state::write_event(config, key, record, &Value::Object(fields), timestamp)
}

fn minimize_record(record: &mut Record, raw: Value) {
    record.agent = truncate_utf8(&record.agent, MAX_AGENT_BYTES);
    record.agent_session_id = truncate_utf8(&record.agent_session_id, MAX_SESSION_BYTES);
    record.tmux_session = truncate_utf8(&record.tmux_session, MAX_TMUX_BYTES);
    record.tmux_window = truncate_utf8(&record.tmux_window, MAX_TMUX_BYTES);
    record.tmux_pane = truncate_utf8(&record.tmux_pane, MAX_TMUX_BYTES);
    record.cwd = truncate_utf8(&record.cwd, MAX_CWD_BYTES);
    record.status = truncate_utf8(&record.status, MAX_STATUS_BYTES);
    record.reason = truncate_utf8(&record.reason, MAX_REASON_BYTES);
    record.last_event = truncate_utf8(&record.last_event, MAX_EVENT_BYTES);
    record.raw = raw;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn config(label: &str) -> Config {
        Config {
            state_dir: std::env::temp_dir().join(format!(
                "amux-intake-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            stale_seconds: 86_400,
            events_per_session: 200,
            events_compact_bytes: 8 * 1024 * 1024,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        }
    }

    fn request(raw: Value) -> HookRequest {
        HookRequest {
            agent: "codex".to_owned(),
            event: "PostToolUse".to_owned(),
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
    fn input_is_bounded_and_empty_or_malformed_input_is_explicit() {
        assert_eq!(
            read_hook_json("  \n".as_bytes()).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            read_hook_json("{".as_bytes()).unwrap_err(),
            "event input is not valid JSON"
        );
        assert!(
            read_hook_json(vec![b'x'; MAX_INPUT_BYTES as usize + 1].as_slice())
                .unwrap_err()
                .contains("exceeds 256 KiB")
        );

        let config = config("oversized-request");
        assert!(
            persist(
                &config,
                request(serde_json::json!({"unknown": "x".repeat(MAX_INPUT_BYTES as usize)})),
                TmuxContext::default(),
            )
            .unwrap_err()
            .contains("exceeds 256 KiB")
        );
        assert!(!config.state_file().exists());
        fs::remove_dir_all(config.state_dir).unwrap_or(());
    }

    #[test]
    fn persisted_metadata_is_allowlisted_and_keeps_subagent_identity() {
        let config = config("metadata");
        persist(
            &config,
            request(serde_json::json!({
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
            })),
            TmuxContext::default(),
        )
        .unwrap();
        let state = crate::state::load(&config).unwrap();
        let record = state.records.values().next().unwrap();
        assert_eq!(record.raw["agent_id"], "child");
        assert_eq!(record.raw["is_subagent"], true);
        for sensitive in ["tool_input", "message", "command", "unknown"] {
            assert!(record.raw.get(sensitive).is_none(), "retained {sensitive}");
        }
        let log = fs::read_to_string(config.events_file()).unwrap();
        for sensitive in ["secret prompt", "rm -rf secret", "unknown"] {
            assert!(!log.contains(sensitive), "logged {sensitive}");
        }
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn identifiers_and_key_contributions_are_capped_on_utf8_boundaries() {
        let config = config("caps");
        let repeated = "🦀".repeat(400);
        let mut request = request(serde_json::json!({"session_id": repeated}));
        request.agent = "é".repeat(100);
        persist(&config, request, TmuxContext::default()).unwrap();
        let state = crate::state::load(&config).unwrap();
        let (key, record) = state.records.iter().next().unwrap();
        assert!(record.agent.len() <= MAX_AGENT_BYTES);
        assert!(record.agent_session_id.len() <= MAX_SESSION_BYTES);
        assert_eq!(
            key,
            &format!("{}:{}", record.agent, record.agent_session_id)
        );
        assert!(std::str::from_utf8(key.as_bytes()).is_ok());
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn the_same_intake_request_produces_equivalent_durable_paths() {
        let left = config("left");
        let right = config("right");
        let request = request(serde_json::json!({
            "session_id": "equivalent",
            "cwd": "/workspace",
            "notification_type": "permission_prompt"
        }));
        persist(&left, request.clone(), TmuxContext::default()).unwrap();
        persist(&right, request, TmuxContext::default()).unwrap();
        let normalized_state = |config: &Config| {
            let mut state = serde_json::to_value(crate::state::load(config).unwrap()).unwrap();
            for record in state["records"].as_object_mut().unwrap().values_mut() {
                record["updated_at"] = Value::from(0);
                record["updated_at_iso"] = Value::from("");
            }
            state
        };
        assert_eq!(normalized_state(&left), normalized_state(&right));
        let normalized_log = |config: &Config| {
            let mut event: Value =
                serde_json::from_str(fs::read_to_string(config.events_file()).unwrap().trim())
                    .unwrap();
            event["updated_at"] = Value::from(0);
            event["updated_at_iso"] = Value::from("");
            event
        };
        assert_eq!(normalized_log(&left), normalized_log(&right));
        fs::remove_dir_all(left.state_dir).unwrap();
        fs::remove_dir_all(right.state_dir).unwrap();
    }
}
