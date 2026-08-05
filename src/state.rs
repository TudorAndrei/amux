use crate::config::Config;
use crate::fsutil::sync_dir;
use crate::model::{HistoryEvent, Record, State};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
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
    // state.lock was a directory in the old mkdir lease scheme. Remove only
    // that migration artifact; advisory locking needs this stable file path.
    let lock_path = config.lock_file();
    if lock_path.is_dir() {
        match fs::remove_dir_all(&lock_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
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

/// The guard owns the fresh file description whose advisory lock we acquired.
/// Never share, clone, or cache this File: re-locking one handle succeeds
/// silently and does not exclude other work in this process. Dropping this
/// file deliberately releases the advisory lock; there is no lock-file cleanup.
#[derive(Debug)]
struct Lock {
    _file: Option<File>,
}

fn unsupported_lock(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    // ENOTSUP/EOPNOTSUPP are Uncategorized on macOS (45), unlike ENOSYS.
    // Keep raw errno checks because ErrorKind alone misses that filesystem case.
    #[cfg(target_os = "macos")]
    const UNSUPPORTED_ERRNOS: &[i32] = &[45, 78];
    #[cfg(target_os = "linux")]
    const UNSUPPORTED_ERRNOS: &[i32] = &[38, 95];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    const UNSUPPORTED_ERRNOS: &[i32] = &[];
    error
        .raw_os_error()
        .is_some_and(|code| UNSUPPORTED_ERRNOS.contains(&code))
}

fn acquire_with<F>(config: &Config, try_lock: F) -> Result<Lock, String>
where
    F: Fn(&File) -> io::Result<()>,
{
    ensure_private_dir(config)?;
    if !config.locking_enabled {
        return Ok(Lock { _file: None });
    }
    // flock locks belong to an open file description, so every acquisition
    // opens a fresh handle and this guard owns it until the critical section ends.
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(config.lock_file())
        .map_err(|error| error.to_string())?;
    let attempts = (config.lock_acquire_timeout_ms / 10).max(1);
    for attempt in 0..attempts {
        match try_lock(&file) {
            Ok(()) => return Ok(Lock { _file: Some(file) }),
            Err(error) if unsupported_lock(&error) => return Ok(Lock { _file: None }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && attempt + 1 < attempts => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err("timed out waiting for state lock".to_owned());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("timed out waiting for state lock".to_owned())
}

fn acquire(config: &Config) -> Result<Lock, String> {
    acquire_with(config, |file| file.try_lock().map_err(io::Error::from))
}

pub fn write_event(
    config: &Config,
    key: String,
    record: Record,
    event_log: &Value,
    now: i64,
) -> Result<WriteEventResult, String> {
    let _lock = acquire(config)?;
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
    sync_dir(&config.state_dir)?;
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

#[derive(Clone, Copy, Debug, Default)]
pub struct EventHistoryFilter<'a> {
    pub agent: Option<&'a str>,
    pub session: Option<&'a str>,
    pub pane: Option<&'a str>,
}

pub const MAX_EVENT_HISTORY_LIMIT: usize = 1_000;

/// Read the newest matching retained events while returning them in their
/// original chronological order. Malformed lines are skipped just as they are
/// during compaction, and every record is passed through canonical projection
/// policy before it can be returned.
pub fn event_history(
    config: &Config,
    filter: EventHistoryFilter<'_>,
    limit: usize,
) -> Result<Vec<HistoryEvent>, String> {
    let limit = limit.min(MAX_EVENT_HISTORY_LIMIT);
    if limit == 0 {
        return Ok(Vec::new());
    }
    let input = match File::open(config.events_file()) {
        Ok(input) => input,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut input = BufReader::new(input);
    let mut selected = VecDeque::with_capacity(limit);
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
        let Ok(event) = serde_json::from_slice::<HistoryEvent>(&bytes) else {
            continue;
        };
        let event = crate::projection::canonical_history(event);
        if filter
            .agent
            .is_some_and(|agent| event.record.agent != agent)
            || filter.session.is_some_and(|session| {
                event.record.tmux_session != session && event.record.agent_session_id != session
            })
            || filter
                .pane
                .is_some_and(|pane| event.record.tmux_pane != pane)
        {
            continue;
        }
        selected.push_back(event);
        if selected.len() > limit {
            selected.pop_front();
        }
    }
    Ok(selected.into_iter().collect())
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
    let _lock = acquire(config)?;
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
    sync_dir(&config.state_dir)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompactionStage {
    Renamed,
    Retained,
    RetainedWritten,
    Composed,
    Installed,
    Synced,
}

/// Retain the most recent configured number of valid events for every live
/// record key. The potentially long streaming pass intentionally happens with
/// no state lock held so hooks can continue appending to a fresh log.
pub fn compact_events(config: &Config, retain_keys: &BTreeSet<String>) -> Result<(), String> {
    compact_events_with(config, retain_keys, |_| Ok(()))
}

fn compact_events_with(
    config: &Config,
    retain_keys: &BTreeSet<String>,
    mut checkpoint: impl FnMut(CompactionStage) -> Result<(), String>,
) -> Result<(), String> {
    if config.events_per_session == 0 || !config.events_file().exists() {
        return Ok(());
    }
    let compacting = config.compacting_events_file();
    {
        let _lock = acquire(config)?;
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
    let retained_path = config.retained_events_file();
    let temp = config
        .state_dir
        .join(format!("events.jsonl.compact.{}", std::process::id()));
    let mut installed = false;
    let result = (|| {
        sync_dir(&config.state_dir)?;
        checkpoint(CompactionStage::Renamed)?;
        let retained = retained_events(config, retain_keys, &compacting)?;
        checkpoint(CompactionStage::Retained)?;
        write_lines(&retained_path, &retained)?;
        checkpoint(CompactionStage::RetainedWritten)?;

        let _lock = acquire(config)?;
        let mut output = private_writer(&temp)?;
        copy_file(&retained_path, &mut output)?;
        copy_file(&config.events_file(), &mut output)?;
        output.sync_all().map_err(|error| error.to_string())?;
        checkpoint(CompactionStage::Composed)?;
        fs::rename(&temp, config.events_file()).map_err(|error| error.to_string())?;
        installed = true;
        checkpoint(CompactionStage::Installed)?;
        sync_dir(&config.state_dir)?;
        checkpoint(CompactionStage::Synced)?;
        remove_if_exists(&retained_path)?;
        remove_if_exists(&compacting)?;
        sync_dir(&config.state_dir)
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            let recovery = recover_compaction(config, installed, &retained_path, &temp);
            match recovery {
                Ok(()) => Err(format!(
                    "event-log compaction failed and was recovered: {error}"
                )),
                Err(recovery) => Err(format!(
                    "event-log compaction failed: {error}; recovery failed: {recovery}"
                )),
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn compact_events_with_checkpoint(
    config: &Config,
    retain_keys: &BTreeSet<String>,
    checkpoint: impl FnMut(CompactionStage) -> Result<(), String>,
) -> Result<(), String> {
    compact_events_with(config, retain_keys, checkpoint)
}

fn recover_compaction(
    config: &Config,
    installed: bool,
    retained_path: &std::path::Path,
    temp: &std::path::Path,
) -> Result<(), String> {
    remove_if_exists(temp)?;
    remove_if_exists(retained_path)?;
    if installed {
        remove_if_exists(&config.compacting_events_file())?;
        sync_dir(&config.state_dir)
    } else {
        adopt_orphaned_log(config)
    }
}

fn remove_if_exists(path: &std::path::Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
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

pub(crate) fn write_maintenance_diagnostic(config: &Config, message: &str) -> Result<(), String> {
    ensure_private_dir(config)?;
    let mut output = private_writer(&config.maintenance_diagnostic_file())?;
    output
        .write_all(message.as_bytes())
        .map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())?;
    sync_dir(&config.state_dir)
}

pub(crate) fn clear_maintenance_diagnostic(config: &Config) -> Result<(), String> {
    let path = config.maintenance_diagnostic_file();
    if !path.exists() {
        return Ok(());
    }
    remove_if_exists(&path)?;
    sync_dir(&config.state_dir)
}

pub fn clear(config: &Config) -> Result<(), String> {
    let _lock = acquire(config)?;
    let mut removed = false;
    for path in [config.state_file(), config.events_file()] {
        if path.exists() {
            fs::remove_file(path).map_err(|error| error.to_string())?;
            removed = true;
        }
    }
    if removed {
        sync_dir(&config.state_dir)?;
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
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
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
    fn every_post_rename_failure_recovers_and_a_later_compaction_succeeds() {
        for stage_to_fail in [
            CompactionStage::Renamed,
            CompactionStage::Retained,
            CompactionStage::RetainedWritten,
            CompactionStage::Composed,
            CompactionStage::Installed,
            CompactionStage::Synced,
        ] {
            let config = config(2);
            ensure_private_dir(&config).unwrap();
            fs::write(
                config.events_file(),
                format!("{}\n{}\n", line("a", 1), line("a", 2)),
            )
            .unwrap();
            let mut appended_live = false;
            let error = compact_events_with_checkpoint(
                &config,
                &BTreeSet::from(["a".to_owned()]),
                |stage| {
                    if !appended_live && stage == CompactionStage::Renamed {
                        fs::write(config.events_file(), format!("{}\n", line("a", 3))).unwrap();
                        appended_live = true;
                    }
                    if stage == stage_to_fail {
                        Err(format!("injected failure at {stage:?}"))
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            assert!(
                error.contains("was recovered"),
                "{stage_to_fail:?}: {error}"
            );
            assert!(!config.compacting_events_file().exists());
            assert!(!config.retained_events_file().exists());

            compact_events(&config, &BTreeSet::from(["a".to_owned()])).unwrap();
            let ordinals = fs::read_to_string(config.events_file())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap()["ordinal"].as_u64())
                .collect::<Vec<_>>();
            assert_eq!(ordinals, [Some(2), Some(3)], "{stage_to_fail:?}");
            fs::remove_dir_all(config.state_dir).unwrap();
        }
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
        let mut config = config(200);
        config.lock_acquire_timeout_ms = 200;
        let _held = acquire(&config).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            acquire(&config).unwrap_err(),
            "timed out waiting for state lock"
        );
        assert!(started.elapsed() >= Duration::from_millis(190));
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn migration_removes_legacy_lock_directory() {
        let config = config(200);
        fs::create_dir_all(config.lock_file()).unwrap();
        let lock = acquire(&config).unwrap();
        assert!(config.lock_file().is_file());
        drop(lock);
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn unsupported_locking_degrades_to_unlocked() {
        let config = config(200);
        let lock = acquire_with(&config, |_| {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        })
        .unwrap();
        assert!(lock._file.is_none());
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn locking_can_be_disabled() {
        let mut config = config(200);
        config.locking_enabled = false;
        write_event(
            &config,
            "one".to_owned(),
            Record::default(),
            &serde_json::json!({"key":"one"}),
            1,
        )
        .unwrap();
        assert!(load(&config).unwrap().records.contains_key("one"));
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn child_holds_lock_for_cross_process_release_test() {
        let Some(state_dir) = std::env::var_os("AMUX_TEST_LOCK_HOLDER_DIR") else {
            return;
        };
        let mut config = config(200);
        config.state_dir = state_dir.into();
        let _lock = acquire(&config).unwrap();
        fs::write(config.state_dir.join("lock-held"), "ready").unwrap();
        thread::sleep(Duration::from_millis(750));
    }

    #[test]
    fn cross_process_lock_is_released_when_the_holder_is_killed() {
        let mut config = config(200);
        config.lock_acquire_timeout_ms = 50;
        ensure_private_dir(&config).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "state::tests::child_holds_lock_for_cross_process_release_test",
            ])
            .env("AMUX_TEST_LOCK_HOLDER_DIR", &config.state_dir)
            .spawn()
            .unwrap();
        let ready = config.state_dir.join("lock-held");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !ready.exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ready.exists(), "child did not acquire the lock");
        assert_eq!(
            acquire(&config).unwrap_err(),
            "timed out waiting for state lock"
        );
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        acquire(&config).expect("kernel must release the lock when the child dies");
        fs::remove_dir_all(config.state_dir).unwrap();
    }

    #[test]
    fn independently_opened_thread_locks_do_not_overlap() {
        use std::sync::{Arc, Barrier};
        let config = Arc::new(config(200));
        let barrier = Arc::new(Barrier::new(3));
        let inside = Arc::new(AtomicUsize::new(0));
        let violations = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let (config, barrier, inside, violations) = (
                config.clone(),
                barrier.clone(),
                inside.clone(),
                violations.clone(),
            );
            workers.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..20 {
                    let _lock = acquire(&config).unwrap();
                    if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                        violations.fetch_add(1, Ordering::SeqCst);
                    }
                    thread::sleep(Duration::from_millis(1));
                    inside.fetch_sub(1, Ordering::SeqCst);
                }
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(violations.load(Ordering::SeqCst), 0);
        fs::remove_dir_all(&config.state_dir).unwrap();
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

    #[test]
    fn durable_state_replacement_and_clear_cover_both_directory_mutations() {
        let config = config(200);
        write_event(
            &config,
            "codex:durable".to_owned(),
            Record {
                status: "running".to_owned(),
                ..Record::default()
            },
            &serde_json::json!({"key": "codex:durable"}),
            10,
        )
        .unwrap();
        assert!(config.state_file().is_file());
        assert!(config.events_file().is_file());

        clear(&config).unwrap();
        assert!(!config.state_file().exists());
        assert!(!config.events_file().exists());
        // A second clear proves the no-mutation path does not require files to
        // exist while still acquiring the normal durable-state lock.
        clear(&config).unwrap();
        fs::remove_dir_all(config.state_dir).unwrap();
    }
}
