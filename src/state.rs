use crate::config::Config;
use crate::model::{Record, State};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

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
                let heartbeat = config.lock_dir().join("heartbeat");
                let stale = fs::metadata(&heartbeat)
                    .or_else(|_| fs::metadata(config.lock_dir()))
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > Duration::from_secs(config.lock_timeout_seconds));
                if stale {
                    // Removing may race another contender; the following create retries safely.
                    let _ = fs::remove_dir_all(config.lock_dir());
                }
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("timed out waiting for state lock".to_owned())
}

struct Lock<'a> {
    config: &'a Config,
    stop: Option<mpsc::Sender<()>>,
    heartbeat: Option<thread::JoinHandle<()>>,
}

impl<'a> Lock<'a> {
    fn new(config: &'a Config) -> Self {
        let marker = config.lock_dir().join("heartbeat");
        let token = format!("{}-{:?}", std::process::id(), SystemTime::now());
        // The marker's mtime, not the directory creation time, is the lease.
        // A previous owner verifies this token before it can refresh anything.
        let _ = fs::write(&marker, &token);
        let (stop, receiver) = mpsc::channel();
        let interval = Duration::from_secs((config.lock_timeout_seconds / 2).clamp(1, 15));
        let heartbeat = thread::spawn(move || {
            loop {
                if receiver.recv_timeout(interval).is_ok() {
                    break;
                }
                if fs::read_to_string(&marker).ok().as_deref() != Some(&token) {
                    break;
                }
                let _ = OpenOptions::new()
                    .write(true)
                    .open(&marker)
                    .and_then(|file| {
                        file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()))
                    });
            }
        });
        Self {
            config,
            stop: Some(stop),
            heartbeat: Some(heartbeat),
        }
    }
}

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        self.stop.take().map(|stop| stop.send(()));
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        let _ = fs::remove_dir_all(self.config.lock_dir());
    }
}

pub fn write_event(
    config: &Config,
    key: String,
    record: Record,
    event_log: &Value,
    now: i64,
) -> Result<WriteEventResult, String> {
    acquire(config)?;
    let _lock = Lock::new(config);
    restrict_file(&config.state_file())?;
    let mut state = load(config)?;
    let changed = state.records.get(&key).is_none_or(|previous| {
        previous.status != record.status
            || previous.attention != record.attention
            || previous.reason != record.reason
    });
    if changed {
        append_event(config, event_log)?;
    }
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
    fs::rename(&temp, config.state_file()).map_err(|error| error.to_string())?;
    Ok(WriteEventResult {
        logged: changed,
        over_compact_threshold: changed
            && config.events_per_session != 0
            && config
                .events_file()
                .metadata()
                .is_ok_and(|meta| meta.len() >= config.events_compact_bytes),
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteEventResult {
    pub logged: bool,
    pub over_compact_threshold: bool,
}

fn append_event(config: &Config, event_log: &Value) -> Result<(), String> {
    restrict_file(&config.events_file())?;
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
    events.sync_data().map_err(|error| error.to_string())
}

/// Recover the old log after a crash between moving it aside and installing a
/// compacted replacement. The original log predates the live log, so append it
/// first and atomically replace the live file under the normal state lock.
pub fn adopt_orphaned_log(config: &Config) -> Result<(), String> {
    let orphan = config.compacting_events_file();
    if !orphan.exists() {
        return Ok(());
    }
    acquire(config)?;
    let _lock = Lock::new(config);
    if !orphan.exists() {
        return Ok(());
    }
    let temp = config
        .state_dir
        .join(format!("events.jsonl.adopt.{}", std::process::id()));
    let mut output = private_writer(&temp)?;
    copy_file(&orphan, &mut output)?;
    copy_file(&config.events_file(), &mut output)?;
    output.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temp, config.events_file()).map_err(|error| error.to_string())?;
    sync_dir(&config.state_dir)?;
    fs::remove_file(orphan).map_err(|error| error.to_string())?;
    Ok(())
}

/// Retain the most recent configured number of valid events for every live
/// record key. The potentially long streaming pass intentionally happens with
/// no state lock held so hooks can continue appending to a fresh log.
pub fn compact_events(config: &Config, retain_keys: &BTreeSet<String>) -> Result<(), String> {
    if config.events_per_session == 0 || !config.events_file().exists() {
        return Ok(());
    }
    let compacting = config.compacting_events_file();
    acquire(config)?;
    {
        let _lock = Lock::new(config);
        if compacting.exists() {
            return Err(format!(
                "orphaned event log exists: {}",
                compacting.display()
            ));
        }
        if !config.events_file().exists() {
            return Ok(());
        }
        fs::rename(config.events_file(), &compacting).map_err(|error| error.to_string())?;
    }

    let retained = retained_events(config, retain_keys, &compacting)?;
    let retained_path = config.retained_events_file();
    write_lines(&retained_path, &retained)?;

    acquire(config)?;
    let _lock = Lock::new(config);
    let temp = config
        .state_dir
        .join(format!("events.jsonl.compact.{}", std::process::id()));
    let mut output = private_writer(&temp)?;
    copy_file(&retained_path, &mut output)?;
    copy_file(&config.events_file(), &mut output)?;
    output.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temp, config.events_file()).map_err(|error| error.to_string())?;
    sync_dir(&config.state_dir)?;
    let _ = fs::remove_file(&retained_path);
    fs::remove_file(&compacting).map_err(|error| error.to_string())?;
    Ok(())
}

fn retained_events(
    config: &Config,
    retain_keys: &BTreeSet<String>,
    source: &std::path::Path,
) -> Result<Vec<String>, String> {
    let input = File::open(source).map_err(|error| error.to_string())?;
    let mut input = BufReader::new(input);
    let mut per_key: BTreeMap<String, VecDeque<(usize, String)>> = BTreeMap::new();
    let mut ordinal = 0;
    loop {
        let mut bytes = Vec::new();
        let count = input
            .read_until(b'\n', &mut bytes)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let current_ordinal = ordinal;
        ordinal += 1;
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Ok(line) = String::from_utf8(bytes) else {
            continue;
        };
        let Some(key) = value.get("key").and_then(Value::as_str) else {
            continue;
        };
        if !retain_keys.contains(key) {
            continue;
        }
        let entries = per_key.entry(key.to_owned()).or_default();
        entries.push_back((current_ordinal, line));
        if entries.len() > config.events_per_session {
            entries.pop_front();
        }
    }
    let mut output: Vec<_> = per_key
        .into_values()
        .flat_map(VecDeque::into_iter)
        .collect();
    output.sort_by_key(|(ordinal, _)| *ordinal);
    Ok(output.into_iter().map(|(_, line)| line).collect())
}

fn private_writer(path: &std::path::Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    Ok(file)
}

fn copy_file(path: &std::path::Path, output: &mut File) -> Result<(), String> {
    if path.exists() {
        let mut input = File::open(path).map_err(|error| error.to_string())?;
        io::copy(&mut input, output).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn write_lines(path: &std::path::Path, lines: &[String]) -> Result<(), String> {
    let mut output = private_writer(path)?;
    for line in lines {
        output
            .write_all(line.as_bytes())
            .map_err(|error| error.to_string())?;
        output.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    output.sync_all().map_err(|error| error.to_string())
}

fn sync_dir(path: &std::path::Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

pub fn clear(config: &Config) -> Result<(), String> {
    acquire(config)?;
    let _lock = Lock::new(config);
    for path in [config.state_file(), config.events_file()] {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn config(events_per_session: usize) -> Config {
        let state_dir = std::env::temp_dir().join(format!(
            "amux-state-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Config {
            state_dir,
            stale_seconds: 86_400,
            events_per_session,
            events_compact_bytes: 1,
            lock_timeout_seconds: 30,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        }
    }

    fn line(key: &str, ordinal: usize) -> String {
        serde_json::json!({"key": key, "ordinal": ordinal}).to_string()
    }

    #[test]
    fn compaction_caps_each_live_key_and_preserves_global_chronology() {
        let config = config(2);
        ensure_private_dir(&config).unwrap();
        let a = "codex:one:%1";
        let b = "codex:one:%2";
        let dead = "codex:old:%9";
        let mut lines = vec![
            line(a, 0),
            line(b, 1),
            "not json".to_owned(),
            serde_json::json!({"ordinal": 3}).to_string(),
            line(dead, 4),
        ];
        for ordinal in 5..505 {
            lines.push(line(a, ordinal));
            if ordinal == 250 {
                lines.push(line(b, ordinal));
            }
        }
        let contents = lines.join("\n");
        fs::write(config.events_file(), format!("{contents}\n")).unwrap();
        OpenOptions::new()
            .append(true)
            .open(config.events_file())
            .unwrap()
            .write_all(&[0xff, b'\n'])
            .unwrap();
        let keys = BTreeSet::from([a.to_owned(), b.to_owned()]);
        compact_events(&config, &keys).unwrap();
        let events: Vec<Value> = fs::read_to_string(config.events_file())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            events
                .iter()
                .map(|event| event["ordinal"].as_u64())
                .collect::<Vec<_>>(),
            vec![Some(1), Some(250), Some(503), Some(504)]
        );
        assert_eq!(events.iter().filter(|event| event["key"] == a).count(), 2);
        assert_eq!(
            fs::metadata(config.events_file())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn orphaned_log_is_adopted_without_losing_the_live_log() {
        let config = config(2);
        ensure_private_dir(&config).unwrap();
        fs::write(
            config.compacting_events_file(),
            format!("{}\n", line("old", 1)),
        )
        .unwrap();
        fs::write(config.events_file(), format!("{}\n", line("live", 2))).unwrap();
        adopt_orphaned_log(&config).unwrap();
        assert_eq!(
            fs::read_to_string(config.events_file()).unwrap(),
            format!("{}\n{}\n", line("old", 1), line("live", 2))
        );
        assert!(!config.compacting_events_file().exists());
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn zero_retention_disables_compaction() {
        let config = config(0);
        ensure_private_dir(&config).unwrap();
        let contents = format!("{}\n{}\n", line("a", 1), line("a", 2));
        fs::write(config.events_file(), &contents).unwrap();
        let written = write_event(
            &config,
            "a".to_owned(),
            Record {
                status: "running".to_owned(),
                reason: "working".to_owned(),
                ..Record::default()
            },
            &serde_json::json!({"key": "a", "ordinal": 3}),
            10,
        )
        .unwrap();
        assert!(written.logged);
        assert!(!written.over_compact_threshold);
        compact_events(&config, &BTreeSet::from(["a".to_owned()])).unwrap();
        assert_eq!(
            fs::read_to_string(config.events_file())
                .unwrap()
                .lines()
                .count(),
            3
        );
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn fresh_lock_is_respected_until_the_acquisition_budget_expires() {
        let config = config(200);
        ensure_private_dir(&config).unwrap();
        fs::create_dir(config.lock_dir()).unwrap();
        fs::write(config.lock_dir().join("heartbeat"), "live").unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            acquire(&config).unwrap_err(),
            "timed out waiting for state lock"
        );
        assert!(started.elapsed() >= Duration::from_secs(5));
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn lock_heartbeat_refreshes_a_long_hold() {
        let mut config = config(200);
        config.lock_timeout_seconds = 1;
        acquire(&config).unwrap();
        let _lock = Lock::new(&config);
        let first = fs::metadata(config.lock_dir().join("heartbeat"))
            .unwrap()
            .modified()
            .unwrap();
        thread::sleep(Duration::from_millis(1_100));
        let refreshed = fs::metadata(config.lock_dir().join("heartbeat"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(refreshed > first);
        drop(_lock);
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn stale_lock_is_taken_over_before_writing() {
        let mut config = config(200);
        config.lock_timeout_seconds = 0;
        ensure_private_dir(&config).unwrap();
        fs::create_dir(config.lock_dir()).unwrap();
        fs::write(config.lock_dir().join("heartbeat"), "abandoned").unwrap();
        thread::sleep(Duration::from_millis(2));
        write_event(
            &config,
            "codex:one".to_owned(),
            Record {
                status: "running".to_owned(),
                ..Record::default()
            },
            &serde_json::json!({"key":"codex:one"}),
            10,
        )
        .unwrap();
        assert!(load(&config).unwrap().records.contains_key("codex:one"));
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn repeated_state_does_not_append_another_event() {
        let config = config(200);
        let record = Record {
            status: "running".to_owned(),
            reason: "working".to_owned(),
            ..Record::default()
        };
        let first = write_event(
            &config,
            "codex:one".to_owned(),
            record.clone(),
            &serde_json::json!({"key": "codex:one"}),
            10,
        )
        .unwrap();
        let second = write_event(
            &config,
            "codex:one".to_owned(),
            record,
            &serde_json::json!({"key": "codex:one"}),
            11,
        )
        .unwrap();
        assert!(first.logged);
        assert!(!second.logged);
        assert_eq!(
            fs::read_to_string(config.events_file())
                .unwrap()
                .lines()
                .count(),
            1
        );
        fs::remove_dir_all(config.state_dir).unwrap();
    }
}
