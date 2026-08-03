use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub status: String,
    pub attention: bool,
    pub reason: String,
}

pub struct PolicyInput<'a> {
    pub agent: &'a str,
    pub event: &'a str,
    pub status_override: &'a str,
    pub attention_override: &'a str,
    pub reason_override: &'a str,
    pub metadata: &'a Value,
}

pub fn classify(input: PolicyInput<'_>) -> Classification {
    let inferred = agent_policy(input.agent, input.event, input.metadata);
    let status = if input.status_override.is_empty() {
        inferred.0.to_owned()
    } else {
        input.status_override.to_owned()
    };
    let attention = if input.attention_override.is_empty() {
        inferred.1
    } else {
        input.attention_override == "1" || input.attention_override.eq_ignore_ascii_case("true")
    };
    let reason = if input.reason_override.is_empty() {
        raw_string(
            input.metadata,
            &[&["reason"], &["notification_type"], &["notificationType"]],
        )
        .unwrap_or(input.event)
        .to_owned()
    } else {
        input.reason_override.to_owned()
    };
    Classification {
        status,
        attention,
        reason,
    }
}

fn agent_policy(agent: &str, event: &str, metadata: &Value) -> (&'static str, bool) {
    if agent.eq_ignore_ascii_case("codex") {
        return match event.to_ascii_lowercase().as_str() {
            "sessionstart" | "userpromptsubmit" | "pretooluse" | "posttooluse" | "precompact"
            | "postcompact" => ("running", false),
            "permissionrequest" => ("attention", true),
            "stop" => ("done", false),
            "sessionend" => ("offline", false),
            _ => generic_policy(event),
        };
    }
    if agent.eq_ignore_ascii_case("claude") {
        return match event.to_ascii_lowercase().as_str() {
            "sessionstart" | "userpromptsubmit" => ("running", false),
            "stop" => ("done", false),
            "notification" => {
                match raw_string(metadata, &[&["notification_type"], &["notificationType"]])
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "idle_prompt" | "agent_completed" => ("done", false),
                    "permission_prompt" | "agent_needs_input" => ("attention", true),
                    _ => ("attention", true),
                }
            }
            _ => generic_policy(event),
        };
    }
    if agent.eq_ignore_ascii_case("pi") {
        return match event.to_ascii_lowercase().as_str() {
            "session_start" | "agent_start" => ("running", false),
            "agent_settled" => ("done", false),
            "session_shutdown" => ("offline", false),
            _ => generic_policy(event),
        };
    }
    if agent.eq_ignore_ascii_case("opencode") {
        return match event.to_ascii_lowercase().as_str() {
            "session.created" | "permission.replied" => ("running", false),
            "permission.asked" => ("attention", true),
            "session.status" => {
                match raw_string(metadata, &[&["event", "properties", "status", "type"]])
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "idle" => ("done", false),
                    "busy" | "retry" => ("running", false),
                    _ => ("running", false),
                }
            }
            "session.deleted" => ("offline", false),
            _ => generic_policy(event),
        };
    }
    generic_policy(event)
}

fn generic_policy(event: &str) -> (&'static str, bool) {
    let lower = event.to_ascii_lowercase();
    if lower.contains("sessionend") || lower.contains("session_end") {
        return ("offline", false);
    }
    if [
        "permission",
        "approval",
        "notification",
        "idle",
        "ask",
        "waiting",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        return ("attention", true);
    }
    if ["stop", "end", "done", "complete"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return ("done", false);
    }
    // Unknown activity is deliberately non-terminal and non-attention. A new
    // upstream event cannot manufacture a completed or user-blocked state.
    ("running", false)
}

fn raw_string<'a>(raw: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    for path in paths {
        let mut item = raw;
        let mut found = true;
        for key in *path {
            let Some(next) = item.get(*key) else {
                found = false;
                break;
            };
            item = next;
        }
        if found
            && let Some(value) = item.as_str()
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(agent: &str, event: &str, metadata: Value) -> Classification {
        classify(PolicyInput {
            agent,
            event,
            status_override: "",
            attention_override: "",
            reason_override: "",
            metadata: &metadata,
        })
    }

    #[test]
    fn codex_mapping_is_explicit_at_the_policy_seam() {
        for (event, status, attention) in [
            ("SessionStart", "running", false),
            ("UserPromptSubmit", "running", false),
            ("PreToolUse", "running", false),
            ("PostToolUse", "running", false),
            ("PermissionRequest", "attention", true),
            ("PreCompact", "running", false),
            ("PostCompact", "running", false),
            ("Stop", "done", false),
            ("SessionEnd", "offline", false),
        ] {
            let result = policy("codex", event, serde_json::json!({}));
            assert_eq!(result.status, status, "{event}");
            assert_eq!(result.attention, attention, "{event}");
        }
    }

    #[test]
    fn claude_notification_policy_uses_only_lifecycle_metadata() {
        for (notification_type, status, attention) in [
            ("idle_prompt", "done", false),
            ("agent_completed", "done", false),
            ("permission_prompt", "attention", true),
            ("agent_needs_input", "attention", true),
            ("future_notification", "attention", true),
        ] {
            let result = policy(
                "claude",
                "Notification",
                serde_json::json!({"notification_type": notification_type}),
            );
            assert_eq!(result.status, status, "{notification_type}");
            assert_eq!(result.attention, attention, "{notification_type}");
        }
    }

    #[test]
    fn explicit_overrides_keep_their_independent_precedence() {
        let metadata = serde_json::json!({"notification_type": "idle_prompt"});
        let result = classify(PolicyInput {
            agent: "claude",
            event: "Notification",
            status_override: "running",
            attention_override: "1",
            reason_override: "explicit reason",
            metadata: &metadata,
        });
        assert_eq!(
            result,
            Classification {
                status: "running".to_owned(),
                attention: true,
                reason: "explicit reason".to_owned(),
            }
        );
    }

    #[test]
    fn generic_and_unknown_events_are_conservative() {
        assert_eq!(
            policy("pi", "session_end", serde_json::json!({})).status,
            "offline"
        );
        assert_eq!(
            policy("opencode", "session.idle", serde_json::json!({})).status,
            "attention"
        );
        let unknown = policy("future-agent", "new.signal", serde_json::json!({}));
        assert_eq!(unknown.status, "running");
        assert!(!unknown.attention);
    }

    #[test]
    fn pi_policy_uses_only_confirmed_lifecycle_signals() {
        for (event, status, attention) in [
            ("session_start", "running", false),
            ("agent_start", "running", false),
            ("agent_settled", "done", false),
            ("session_shutdown", "offline", false),
            ("tool_call", "running", false),
            ("input", "running", false),
            ("future_signal", "running", false),
        ] {
            let result = policy("pi", event, serde_json::json!({}));
            assert_eq!(result.status, status, "{event}");
            assert_eq!(result.attention, attention, "{event}");
        }
    }

    #[test]
    fn opencode_policy_uses_status_metadata_and_stable_attention_only() {
        for (event, metadata, status, attention) in [
            ("session.created", serde_json::json!({}), "running", false),
            (
                "session.status",
                serde_json::json!({"event":{"properties":{"status":{"type":"busy"}}}}),
                "running",
                false,
            ),
            (
                "session.status",
                serde_json::json!({"event":{"properties":{"status":{"type":"retry"}}}}),
                "running",
                false,
            ),
            (
                "session.status",
                serde_json::json!({"event":{"properties":{"status":{"type":"future"}}}}),
                "running",
                false,
            ),
            (
                "session.status",
                serde_json::json!({"event":{"properties":{"status":{"type":"idle"}}}}),
                "done",
                false,
            ),
            ("permission.asked", serde_json::json!({}), "attention", true),
            (
                "permission.replied",
                serde_json::json!({}),
                "running",
                false,
            ),
            ("session.deleted", serde_json::json!({}), "offline", false),
        ] {
            let result = policy("opencode", event, metadata);
            assert_eq!(result.status, status, "{event}");
            assert_eq!(result.attention, attention, "{event}");
        }
    }
}
