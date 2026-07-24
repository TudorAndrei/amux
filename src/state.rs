use crate::config::Config;
use crate::model::{Record, State};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::thread;
use std::time::Duration;

pub fn load(config: &Config) -> Result<State, String> {
    let path = config.state_file();
    if !path.exists() {
        return Ok(State::initial());
    }
    restrict_file(&path)?;
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("invalid state file {}: {error}", path.display()))
}

pub fn ensure_private_dir(config: &Config) -> Result<(), String> {
    fs::create_dir_all(&config.state_dir).map_err(|error| error.to_string())?;
    fs::set_permissions(&config.state_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())
}

fn restrict_file(path: &std::path::Path) -> Result<(), String> {
    if path.exists() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn acquire(config: &Config) -> Result<(), String> {
    ensure_private_dir(config)?;
    for _ in 0..500 {
        match fs::create_dir(config.lock_dir()) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("timed out waiting for state lock".to_owned())
}

struct Lock<'a>(&'a Config);
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_dir(self.0.lock_dir());
    }
}

pub fn write_event(
    config: &Config,
    key: String,
    record: Record,
    event_log: &Value,
    now: i64,
) -> Result<(), String> {
    acquire(config)?;
    let _lock = Lock(config);
    restrict_file(&config.events_file())?;
    restrict_file(&config.state_file())?;
    let mut events = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(config.events_file())
        .map_err(|error| error.to_string())?;
    events
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut events, event_log).map_err(|error| error.to_string())?;
    events.write_all(b"\n").map_err(|error| error.to_string())?;
    events.sync_data().map_err(|error| error.to_string())?;
    let mut state = load(config)?;
    state.version = 1;
    state.records.insert(key, record);
    let cutoff = now - config.stale_seconds;
    state
        .records
        .retain(|_, record| record.updated_at >= cutoff);
    let temp = config
        .state_dir
        .join(format!("state.json.{}", std::process::id()));
    let payload = serde_json::to_vec(&state).map_err(|error| error.to_string())?;
    let mut temporary = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|error| error.to_string())?;
    temporary
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    temporary
        .write_all(&payload)
        .map_err(|error| error.to_string())?;
    temporary.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temp, config.state_file()).map_err(|error| error.to_string())
}

pub fn clear(config: &Config) -> Result<(), String> {
    acquire(config)?;
    let _lock = Lock(config);
    for path in [config.state_file(), config.events_file()] {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
