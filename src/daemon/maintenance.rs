use crate::config::Config;
use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

#[derive(Default)]
struct Status {
    active: bool,
}

#[derive(Clone, Default)]
pub(super) struct Maintenance {
    inner: Arc<(Mutex<Status>, Condvar)>,
}

impl Maintenance {
    pub fn schedule(&self, config: Config, retain_keys: BTreeSet<String>) {
        self.schedule_with(move || {
            crate::state::adopt_orphaned_log(&config)
                .and_then(|_| crate::state::compact_events(&config, &retain_keys))
                .and_then(|_| crate::state::clear_maintenance_diagnostic(&config))
                .inspect_err(|error| {
                    let _ = crate::state::write_maintenance_diagnostic(&config, error);
                })
        });
    }

    pub fn clear(&self, config: &Config) -> Result<(), String> {
        self.wait_idle()?;
        crate::state::clear(config)?;
        crate::state::clear_maintenance_diagnostic(config)
    }

    pub fn wait_idle(&self) -> Result<(), String> {
        let (lock, ready) = &*self.inner;
        let mut status = lock
            .lock()
            .map_err(|_| "maintenance coordinator lock poisoned".to_owned())?;
        while status.active {
            status = ready
                .wait(status)
                .map_err(|_| "maintenance coordinator lock poisoned".to_owned())?;
        }
        Ok(())
    }

    fn schedule_with(&self, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let (lock, _) = &*self.inner;
        let Ok(mut status) = lock.lock() else {
            return;
        };
        if status.active {
            return;
        }
        status.active = true;
        drop(status);

        let inner = Arc::clone(&self.inner);
        thread::spawn(move || {
            if let Err(error) = work() {
                eprintln!("amux daemon: {error}");
            }
            let (lock, ready) = &*inner;
            if let Ok(mut status) = lock.lock() {
                status.active = false;
                ready.notify_all();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CompactionStage, compact_events_with_checkpoint};
    use std::fs;
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;

    fn config() -> Config {
        let state_dir = std::env::temp_dir().join(format!(
            "amux-maintenance-{}-{}",
            std::process::id(),
            crate::event::now()
        ));
        Config {
            state_dir,
            stale_seconds: 86_400,
            events_per_session: 2,
            events_compact_bytes: 1,
            lock_acquire_timeout_ms: 5_000,
            locking_enabled: true,
            rejected_overrides: Vec::new(),
            hide_subagents: true,
            use_color: false,
        }
    }

    #[test]
    fn clear_waits_for_the_renamed_log_then_wins_the_handoff() {
        let config = config();
        crate::state::ensure_private_dir(&config).unwrap();
        fs::write(
            config.events_file(),
            format!("{}\n", serde_json::json!({"key":"a","ordinal":1})),
        )
        .unwrap();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let work_config = config.clone();
        let work_entered = Arc::clone(&entered);
        let work_release = Arc::clone(&release);
        let maintenance = Maintenance::default();
        maintenance.schedule_with(move || {
            compact_events_with_checkpoint(
                &work_config,
                &BTreeSet::from(["a".to_owned()]),
                |stage| {
                    if stage == CompactionStage::Renamed {
                        work_entered.wait();
                        work_release.wait();
                    }
                    Ok(())
                },
            )
        });
        entered.wait();

        let (done, cleared) = mpsc::channel();
        let clear_config = config.clone();
        let clear_maintenance = maintenance.clone();
        thread::spawn(move || {
            let result = clear_maintenance.clear(&clear_config);
            done.send(result).unwrap();
        });
        assert!(cleared.recv_timeout(Duration::from_millis(50)).is_err());
        release.wait();
        cleared
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();

        assert!(!config.events_file().exists());
        assert!(!config.compacting_events_file().exists());
        assert!(!config.retained_events_file().exists());
        fs::remove_dir_all(config.state_dir).unwrap();
    }
}
