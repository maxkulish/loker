use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::interval;

use crate::run_state::atomic_write;

// ---------------------------------------------------------------------------
// Clock trait (testability)
// ---------------------------------------------------------------------------

/// Abstract clock source so time-dependent logic can be tested deterministically.
pub(crate) trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock backed by `chrono::Utc::now()`.
pub(crate) struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Fake clock that returns a controllable time.  Only available in test builds.
#[cfg(test)]
pub(crate) struct FakeClock {
    current: std::sync::Mutex<DateTime<Utc>>,
}

#[cfg(test)]
impl FakeClock {
    pub fn new(initial: DateTime<Utc>) -> Self {
        Self {
            current: std::sync::Mutex::new(initial),
        }
    }

    /// Advance the clock by `delta`.
    pub fn advance(&self, delta: chrono::Duration) {
        let mut time = self.current.lock().unwrap();
        *time += delta;
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.current.lock().unwrap()
    }
}

// ---------------------------------------------------------------------------
// Heartbeat types
// ---------------------------------------------------------------------------

/// The body of a heartbeat file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeartbeatBody {
    pub writer_pid: u32,
    pub writer_host: String,
    pub tick_at: DateTime<Utc>,
}

/// Configuration for a heartbeat task.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Time-to-live: how old a heartbeat can be before the writer is
    /// considered dead.  Default: 300 seconds.
    pub ttl_seconds: u64,

    /// How often to tick.  Default: `ttl_seconds / 3`.
    pub interval_seconds: u64,

    /// Run directory (`runs/<id>/`). Heartbeat is written to
    /// `run_dir / heartbeat.json` (per the D3 protocol layout).
    pub run_dir: PathBuf,

    /// Markers directory (`runs/<id>/markers/`). Derived from `run_dir`
    /// but overridable for test isolation.
    pub markers_dir: PathBuf,

    /// Process ID of the writer.
    pub writer_pid: u32,

    /// Hostname of the writer machine.
    pub writer_host: String,
}

impl HeartbeatConfig {
    /// Create a config with defaults.
    ///
    /// `run_dir` is the run root (`runs/<id>/`). `markers_dir` defaults
    /// to `run_dir / "markers"`. Heartbeat is written to
    /// `run_dir / "heartbeat.json"` per the D3 protocol.
    ///
    /// `ttl_seconds` defaults to 300; `interval_seconds` defaults to
    /// `ttl_seconds / 3` (every 100s for a 300s TTL).
    pub fn new(run_dir: PathBuf) -> Self {
        Self {
            ttl_seconds: 300,
            interval_seconds: 100,
            markers_dir: run_dir.join("markers"),
            run_dir,
            writer_pid: std::process::id(),
            writer_host: hostname(),
        }
    }
}

// ---------------------------------------------------------------------------
// HeartbeatWriter
// ---------------------------------------------------------------------------

/// Spawnable heartbeat writer that periodically writes a heartbeat file
/// at `config.run_dir / "heartbeat.json"`.
///
/// The task runs until the returned `JoinHandle` is aborted or the markers
/// directory is deleted (exit silently). Note that dropping the
/// `JoinHandle` only detaches the task — callers must call `.abort()` on
/// the handle to stop the heartbeat loop.
pub struct HeartbeatWriter;

impl HeartbeatWriter {
    /// Spawn a Tokio task that writes a heartbeat file every
    /// `config.interval_seconds`.
    ///
    /// Each tick:
    /// 1. Builds a `HeartbeatBody` with the current time.
    /// 2. Atomically writes it to `run_dir / heartbeat.json`.
    /// 3. Logs a warning on write failure and continues (a missed
    ///    heartbeat is not fatal — 2 more ticks must be missed before
    ///    staleness).
    /// 4. Exits silently if the markers directory is deleted.
    ///
    /// To stop the task, call `.abort()` on the returned `JoinHandle`.
    pub fn spawn(config: HeartbeatConfig) -> tokio::task::JoinHandle<()> {
        let interval_secs = config.interval_seconds;
        let markers_dir = config.markers_dir.clone();
        let heartbeat_path = config.run_dir.join("heartbeat.json");
        let writer_pid = config.writer_pid;
        let writer_host = config.writer_host;

        tokio::spawn(async move {
            // Ensure the markers directory exists.
            if let Err(e) = tokio::fs::create_dir_all(&markers_dir).await {
                eprintln!("HeartbeatWriter: failed to create markers dir: {e}");
                return;
            }

            let mut ticker = interval(Duration::from_secs(interval_secs));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                // Check if the markers directory still exists.
                if !markers_dir.exists() {
                    // Run was cleaned up — exit silently.
                    return;
                }

                let body = HeartbeatBody {
                    writer_pid,
                    writer_host: writer_host.clone(),
                    tick_at: Utc::now(),
                };

                let json = match serde_json::to_string_pretty(&body) {
                    Ok(j) => j,
                    Err(e) => {
                        eprintln!("HeartbeatWriter: JSON serialization error: {e}");
                        continue;
                    }
                };

                if let Err(e) = atomic_write(&heartbeat_path, json.as_bytes()) {
                    eprintln!(
                        "HeartbeatWriter: failed to write heartbeat (pid={}): {e}",
                        writer_pid
                    );
                    // Continue — a missed heartbeat is not fatal.
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// is_stale helper
// ---------------------------------------------------------------------------

/// Check whether a heartbeat indicates that its writer is stale (dead).
///
/// Returns `true` if the heartbeat's `tick_at` is older than `now - ttl_seconds`.
///
/// # Boundary behaviour
///
/// - At exactly `ttl_seconds` of age, returns `false` (not stale yet).
/// - At `ttl_seconds + epsilon`, returns `true`.
pub fn is_stale(heartbeat: &HeartbeatBody, now: &DateTime<Utc>, ttl_seconds: u64) -> bool {
    let ttl = chrono::Duration::seconds(ttl_seconds as i64);
    let cutoff = *now - ttl;
    heartbeat.tick_at < cutoff
}

// ---------------------------------------------------------------------------
// Helper: hostname
// ---------------------------------------------------------------------------

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}
