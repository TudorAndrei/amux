use serde_json::{Map, Value};
use std::env;
use std::fs;
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

fn template_json(template: &Path, launcher: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(template)
        .map_err(|error| format!("cannot read {}: {error}", template.display()))?;
    serde_json::from_str(&text.replace("__AMUX_BIN__", &launcher.to_string_lossy()))
        .map_err(|error| format!("invalid hook template {}: {error}", template.display()))
}

fn template_text(template: &Path, launcher: &Path) -> Result<String, String> {
    fs::read_to_string(template)
        .map(|text| text.replace("__AMUX_BIN__", &launcher.to_string_lossy()))
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
                .is_some_and(|command| command.contains("bin/amux event --agent "));
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
    fs::write(path, text).map_err(|error| error.to_string())?;
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
        assert!(stop.contains("/bin/amux event --agent claude --event Stop"));
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
}
