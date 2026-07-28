use serde_json::{Map, Value};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    DryRun,
    Write,
}

#[derive(Clone, Debug)]
struct Paths {
    root: PathBuf,
    home: PathBuf,
}

impl Paths {
    fn from_env() -> Result<Self, String> {
        let root = env::var_os("AMUX_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is not set".to_owned())?;
        Ok(Self { root, home })
    }

    fn launcher(&self) -> PathBuf {
        self.root.join("bin/amux")
    }
}

pub fn install(mode: Mode) -> Result<(), String> {
    install_at(&Paths::from_env()?, mode)
}

pub fn uninstall(mode: Mode) -> Result<(), String> {
    uninstall_at(&Paths::from_env()?, mode)
}

/// Describe differences between the installed JSON-merged hooks and the
/// templates shipped by this checkout. Text-template integrations deliberately
/// are not included: they have no safe merge boundary to inspect.
pub fn drift() -> Result<Vec<String>, String> {
    drift_at(&Paths::from_env()?)
}

fn drift_at(paths: &Paths) -> Result<Vec<String>, String> {
    let launcher = paths.launcher();
    let integrations = [
        (
            "Codex",
            paths.home.join(".codex/hooks.json"),
            paths.root.join("hooks/codex/hooks.json"),
        ),
        (
            "Claude",
            paths.home.join(".claude/settings.json"),
            paths.root.join("hooks/claude/settings.fragment.json"),
        ),
    ];
    let mut output = Vec::new();
    for (name, installed_path, template_path) in integrations {
        let template = template_json(&template_path, &launcher)?;
        let installed = read_json(&installed_path)?;
        output.extend(drift_document(name, &template, &installed));
    }
    Ok(output)
}

#[derive(Debug, PartialEq, Eq)]
struct OwnedHook {
    event: String,
    matcher: Option<Value>,
    arguments: Vec<String>,
}

fn drift_document(name: &str, template: &Value, installed: &Value) -> Vec<String> {
    let expected = owned_hooks(template);
    let actual = owned_hooks(installed);
    let mut messages = Vec::new();
    for hook in &expected {
        let matches: Vec<_> = actual
            .iter()
            .filter(|item| item.event == hook.event)
            .collect();
        if matches.is_empty() {
            messages.push(format!("{name}: missing amux hook for {}", hook.event));
            continue;
        }
        if !matches.iter().any(|item| item.matcher == hook.matcher) {
            messages.push(format!("{name}: matcher drift for {}", hook.event));
        }
        if !matches.iter().any(|item| item.arguments == hook.arguments) {
            messages.push(format!("{name}: argument drift for {}", hook.event));
        }
    }
    for hook in &actual {
        if !expected.iter().any(|item| item.event == hook.event) {
            messages.push(format!("{name}: stale amux hook for {}", hook.event));
        }
    }
    messages
}

fn owned_hooks(document: &Value) -> Vec<OwnedHook> {
    let Some(events) = document.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let Some(object) = group.as_object() else {
                continue;
            };
            let Some(hooks) = object.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            let mut arguments: Vec<_> = hooks
                .iter()
                .filter_map(|hook| hook.get("command").and_then(Value::as_str))
                .filter(|command| is_amux_command(command))
                .filter_map(command_arguments)
                .collect();
            if arguments.is_empty() {
                continue;
            }
            arguments.sort();
            output.push(OwnedHook {
                event: event.clone(),
                matcher: object.get("matcher").cloned(),
                arguments,
            });
        }
    }
    output
}

fn is_amux_command(command: &str) -> bool {
    // New commands quote the launcher; retain the legacy spelling for upgrades.
    command.contains("bin/amux event --agent ") || command.contains("bin/amux' event --agent ")
}

/// The launcher location is intentionally ignored: source, TPM, and release
/// installs put the same command at different absolute paths.
fn command_arguments(command: &str) -> Option<String> {
    command
        .find(" event ")
        .map(|index| command[index + " event".len()..].trim().to_owned())
}

fn install_at(paths: &Paths, mode: Mode) -> Result<(), String> {
    let launcher = paths.launcher();
    if !launcher.is_file() {
        return Err(format!(
            "amux launcher does not exist: {}",
            launcher.display()
        ));
    }
    let launcher = launcher
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", launcher.display()))?;
    let codex = paths.home.join(".codex/hooks.json");
    let claude = paths.home.join(".claude/settings.json");
    let opencode = paths.home.join(".config/opencode/plugins/amux.js");
    let pi_extension = paths.home.join(".pi/agent/extensions/amux.ts");
    let pi_settings = paths.home.join(".pi/agent/settings.json");

    merge_hooks(
        &codex,
        &template_json(&paths.root.join("hooks/codex/hooks.json"), &launcher)?,
        "Codex hooks",
        mode,
    )?;
    merge_hooks(
        &claude,
        &template_json(
            &paths.root.join("hooks/claude/settings.fragment.json"),
            &launcher,
        )?,
        "Claude settings hooks",
        mode,
    )?;
    write_template(
        &opencode,
        &paths.root.join("hooks/opencode/amux.js"),
        &launcher,
        "opencode plugin",
        mode,
    )?;
    write_template(
        &pi_extension,
        &paths.root.join("hooks/pi/amux.ts"),
        &launcher,
        "Pi extension",
        mode,
    )?;
    merge_pi_extension(&pi_settings, &pi_extension, mode)
}

fn uninstall_at(paths: &Paths, mode: Mode) -> Result<(), String> {
    let launcher = paths.launcher().to_string_lossy().into_owned();
    remove_hooks(
        &paths.home.join(".codex/hooks.json"),
        &launcher,
        "Codex hooks",
        mode,
    )?;
    remove_hooks(
        &paths.home.join(".claude/settings.json"),
        &launcher,
        "Claude settings hooks",
        mode,
    )?;
    let pi_extension = paths.home.join(".pi/agent/extensions/amux.ts");
    remove_pi_extension(
        &paths.home.join(".pi/agent/settings.json"),
        &pi_extension,
        mode,
    )?;
    remove_file(
        &paths.home.join(".config/opencode/plugins/amux.js"),
        "opencode plugin",
        mode,
    )?;
    remove_file(&pi_extension, "Pi extension", mode)
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn template_json(template: &Path, launcher: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(template)
        .map_err(|error| format!("cannot read {}: {error}", template.display()))?;
    let mut value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("invalid hook template {}: {error}", template.display()))?;
    replace_launcher(&mut value, &shell_quote(launcher));
    Ok(value)
}

fn replace_launcher(value: &mut Value, launcher: &str) {
    match value {
        Value::String(text) => *text = text.replace("__AMUX_BIN__", launcher),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| replace_launcher(value, launcher)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| replace_launcher(value, launcher)),
        _ => {}
    }
}

fn template_text(template: &Path, launcher: &Path) -> Result<String, String> {
    fs::read_to_string(template)
        .map(|text| text.replace("__AMUX_BIN__", &shell_quote(launcher)))
        .map_err(|error| format!("cannot read {}: {error}", template.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| format!("invalid JSON {}: {error}", path.display()))
}

fn merge_hooks(path: &Path, fragment: &Value, name: &str, mode: Mode) -> Result<(), String> {
    let mut document = read_json(path)?;
    let document_object = document
        .as_object_mut()
        .ok_or_else(|| format!("{} must be a JSON object", path.display()))?;
    let source = fragment
        .get("hooks")
        .and_then(Value::as_object)
        .ok_or_else(|| "hook template has no hooks object".to_owned())?;
    let hooks = document_object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("{}.hooks must be a JSON object", path.display()))?;
    // Replace commands from any previous amux checkout. The absolute launcher
    // path is intentionally not part of the match: source, TPM, and release
    // archives all install from different locations.
    for value in hooks.values_mut() {
        remove_matching(value);
    }
    hooks.retain(|_, value| !value.as_array().is_some_and(Vec::is_empty));
    for (event, additions) in source {
        let target = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("{}.hooks.{event} must be an array", path.display()))?;
        for addition in additions
            .as_array()
            .ok_or_else(|| format!("template hooks.{event} must be an array"))?
        {
            if !target.contains(addition) {
                target.push(addition.clone());
            }
        }
    }
    write_json(path, &document, name, mode)
}

fn merge_pi_extension(path: &Path, extension: &Path, mode: Mode) -> Result<(), String> {
    let mut document = read_json(path)?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| format!("{} must be a JSON object", path.display()))?;
    let extensions = object
        .entry("extensions")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| format!("{}.extensions must be an array", path.display()))?;
    let extension = Value::String(extension.to_string_lossy().into_owned());
    if !extensions.contains(&extension) {
        extensions.push(extension);
    }
    write_json(path, &document, "Pi settings", mode)
}

fn remove_pi_extension(path: &Path, extension: &Path, mode: Mode) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut document = read_json(path)?;
    if let Some(extensions) = document.get_mut("extensions").and_then(Value::as_array_mut) {
        let extension = extension.to_string_lossy();
        extensions.retain(|value| value.as_str() != Some(extension.as_ref()));
    }
    write_json(path, &document, "Pi settings", mode)
}

fn remove_hooks(path: &Path, _launcher: &str, name: &str, mode: Mode) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut document = read_json(path)?;
    if let Some(hooks) = document.get_mut("hooks") {
        remove_matching(hooks);
        if hooks.as_object().is_some_and(|object| object.is_empty()) {
            document
                .as_object_mut()
                .expect("JSON object")
                .remove("hooks");
        }
    }
    write_json(path, &document, name, mode)
}

fn remove_matching(value: &mut Value) -> bool {
    match value {
        Value::Array(items) => {
            items.retain_mut(|item| !remove_matching(item));
            false
        }
        Value::Object(object) => {
            let is_amux_command = object
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(is_amux_command);
            if is_amux_command {
                return true;
            }
            for child in object.values_mut() {
                remove_matching(child);
            }
            object
                .get("hooks")
                .is_some_and(|hooks| hooks.as_array().is_some_and(Vec::is_empty))
        }
        _ => false,
    }
}

fn write_template(
    destination: &Path,
    template: &Path,
    launcher: &Path,
    name: &str,
    mode: Mode,
) -> Result<(), String> {
    write_text(destination, &template_text(template, launcher)?, name, mode)
}

fn write_json(path: &Path, value: &Value, name: &str, mode: Mode) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    text.push('\n');
    write_text(path, &text, name, mode)
}

fn write_text(path: &Path, text: &str, name: &str, mode: Mode) -> Result<(), String> {
    if mode == Mode::DryRun {
        println!("would update {name}: {}", path.display());
        print!("{text}");
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    backup(path)?;
    let temporary = path.with_extension(format!("amux.tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    println!("updated {name}: {}", path.display());
    Ok(())
}

fn remove_file(path: &Path, name: &str, mode: Mode) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if mode == Mode::DryRun {
        println!("would remove {name}: {}", path.display());
        return Ok(());
    }
    backup(path)?;
    fs::remove_file(path).map_err(|error| error.to_string())?;
    println!("removed {name}: {}", path.display());
    Ok(())
}

fn backup(path: &Path) -> Result<(), String> {
    if path.exists() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup = PathBuf::from(format!("{}.amux.bak.{timestamp}", path.display()));
        fs::copy(path, backup).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn paths() -> Paths {
        let home = std::env::temp_dir().join(format!(
            "amux-hooks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&home).unwrap();
        Paths {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            home,
        }
    }

    #[test]
    fn install_is_idempotent_and_uninstall_preserves_other_hooks() {
        let paths = paths();
        let codex = paths.home.join(".codex/hooks.json");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::write(
            &codex,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other command"}]}]}}"#,
        )
        .unwrap();
        install_at(&paths, Mode::Write).unwrap();
        install_at(&paths, Mode::Write).unwrap();
        let installed: Value = serde_json::from_str(&fs::read_to_string(&codex).unwrap()).unwrap();
        assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
        uninstall_at(&paths, Mode::Write).unwrap();
        let removed: Value = serde_json::from_str(&fs::read_to_string(&codex).unwrap()).unwrap();
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            removed["hooks"]["Stop"][0]["hooks"][0]["command"],
            "other command"
        );
        fs::remove_dir_all(paths.home).unwrap();
    }

    #[test]
    fn install_replaces_stale_launchers_and_tool_use_hooks() {
        let paths = paths();
        let claude = paths.home.join(".claude/settings.json");
        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        fs::write(
            &claude,
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"/old/amux/bin/amux event --agent claude --event PostToolUse"}]}],"Stop":[{"hooks":[{"type":"command","command":"/old/amux/bin/amux event --agent claude --event Stop"}]}]}}"#,
        )
        .unwrap();
        install_at(&paths, Mode::Write).unwrap();
        let installed: Value = serde_json::from_str(&fs::read_to_string(&claude).unwrap()).unwrap();
        assert!(installed["hooks"].get("PostToolUse").is_none());
        let stop = installed["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(stop.contains("/bin/amux' event --agent claude --event Stop"));
        assert!(!stop.starts_with("/old/"));
        fs::remove_dir_all(paths.home).unwrap();
    }

    #[test]
    fn dry_run_is_read_only() {
        let paths = paths();
        install_at(&paths, Mode::DryRun).unwrap();
        assert!(!paths.home.join(".codex/hooks.json").exists());
        fs::remove_dir_all(paths.home).unwrap();
    }

    #[test]
    fn templates_quote_paths_with_spaces_quotes_and_backslashes() {
        let launcher = PathBuf::from("/tmp/amux path/it'\\\"quoted\\\\bin/amux");
        let rendered = template_json(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hooks/codex/hooks.json"),
            &launcher,
        )
        .unwrap();
        let command = rendered["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.starts_with("'/tmp/amux path/"));
        assert!(is_amux_command(command));
        assert!(serde_json::to_string(&rendered).is_ok());
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                &format!("set -- {}; printf '%s' \"$1\"", shell_quote(&launcher)),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            launcher.to_string_lossy()
        );
    }

    #[test]
    fn codex_template_has_the_complete_explicit_nine_event_contract() {
        let template: Value = serde_json::from_str(
            &fs::read_to_string(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hooks/codex/hooks.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let hooks = template["hooks"].as_object().unwrap();
        let expected = [
            ("SessionStart", Some("startup|resume|clear"), "running", "0"),
            ("UserPromptSubmit", None, "running", "0"),
            ("PreToolUse", None, "running", "0"),
            ("PostToolUse", None, "running", "0"),
            ("PermissionRequest", Some("*"), "attention", "1"),
            ("PreCompact", None, "running", "0"),
            ("PostCompact", None, "running", "0"),
            ("Stop", None, "done", "0"),
            ("SessionEnd", None, "offline", "0"),
        ];
        assert_eq!(hooks.len(), expected.len());
        for (event, matcher, status, attention) in expected {
            let group = &hooks[event][0];
            assert_eq!(group.get("matcher").and_then(Value::as_str), matcher);
            let command = group["hooks"][0]["command"].as_str().unwrap();
            assert!(command.contains(&format!("--event {event}")));
            assert!(command.contains(&format!("--status {status}")));
            assert!(command.contains(&format!("--attention {attention}")));
        }
    }

    #[test]
    fn drift_ignores_launcher_paths_and_foreign_hooks_but_reports_real_changes() {
        let paths = paths();
        install_at(&paths, Mode::Write).unwrap();
        assert!(drift_at(&paths).unwrap().is_empty());
        let codex = paths.home.join(".codex/hooks.json");
        let mut installed = read_json(&codex).unwrap();
        let hooks = installed["hooks"].as_object_mut().unwrap();
        hooks.remove("PreToolUse");
        hooks["Stop"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "hooks": [{"type": "command", "command": "other command"}]
            }));
        write_json(&codex, &installed, "test", Mode::Write).unwrap();
        let drift = drift_at(&paths).unwrap();
        assert_eq!(
            drift
                .iter()
                .filter(|line| line.contains("PreToolUse"))
                .count(),
            1
        );
        assert!(!drift.iter().any(|line| line.contains("Stop")));

        install_at(&paths, Mode::Write).unwrap();
        let mut installed = read_json(&codex).unwrap();
        let stop_group = installed["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .position(|group| {
                group["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(is_amux_command)
            })
            .unwrap();
        let command = installed["hooks"]["Stop"][stop_group]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .replace("/bin/amux event", "/another/bin/amux event");
        installed["hooks"]["Stop"][stop_group]["hooks"][0]["command"] = Value::String(command);
        write_json(&codex, &installed, "test", Mode::Write).unwrap();
        assert!(drift_at(&paths).unwrap().is_empty());

        let mut installed = read_json(&codex).unwrap();
        let stop_group = installed["hooks"]["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .position(|group| {
                group["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(is_amux_command)
            })
            .unwrap();
        installed["hooks"]["Stop"][stop_group]["hooks"][0]["command"] =
            Value::String("/old/bin/amux event --agent codex --event Stop".to_owned());
        installed["hooks"]["SessionStart"][0]["matcher"] = Value::String("startup".to_owned());
        write_json(&codex, &installed, "test", Mode::Write).unwrap();
        let drift = drift_at(&paths).unwrap();
        assert!(
            drift
                .iter()
                .any(|line| line.contains("argument drift for Stop"))
        );
        assert!(
            drift
                .iter()
                .any(|line| line.contains("matcher drift for SessionStart"))
        );
        fs::remove_dir_all(paths.home).unwrap();
    }
}
