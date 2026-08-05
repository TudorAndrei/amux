use crate::config::Config;
use crate::event::{self, TmuxContext};
use crate::ipc::HookRequest;
use crate::projection;
use crate::state::EventCommit;
use serde_json::{Map, Value};
use std::io::Read;

pub use crate::projection::MAX_INPUT_BYTES;

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
    request: HookRequest,
    tmux: TmuxContext,
) -> Result<EventCommit, String> {
    let projection::ProjectedEvent {
        key,
        record,
        history,
    } = projection::project(request, tmux, event::now())?;
    let timestamp = record.updated_at;
    crate::state::commit_event(config, key, record, &history, timestamp)
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
