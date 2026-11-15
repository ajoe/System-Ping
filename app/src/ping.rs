//! Probing engine.
//!
//! A background thread owns a small pool of workers. Each worker runs the OS
//! ping command for one target and reports the result back over a channel, so
//! the UI never blocks. The OS command is used on purpose: raw ICMP sockets
//! need administrator rights, which a normal double-click launch does not have.
//!
//! Nothing in here touches eframe or egui, so the whole collection path can be driven
//! from a test with no window (see `tests/collection.rs`).

use std::borrow::Cow;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A request to probe one target.
#[derive(Clone, Debug)]
pub struct ProbeJob {
    pub id: usize,
    pub host: String,
    /// Timeout for the whole probe, in ms.
    pub timeout_ms: u32,
}

/// What came back from a probe.
#[derive(Clone, Debug)]
pub struct ProbeResult {
    pub id: usize,
    /// Round-trip time in ms, or [`crate::history::TIMEOUT`].
    pub rtt_ms: f32,
    /// Seconds since the engine started, stamped by the worker that ran the probe.
    pub t_secs: f64,
    /// Absolute wall-clock time of the reply as Unix milliseconds, taken by the worker at the
    /// moment the probe finished. This is what reaches the saved history and the export.
    pub at_ms: i64,
    /// Why it failed, when `rtt_ms` is a timeout.
    pub fail: Option<ProbeFail>,
}

/// Why a probe did not answer. The UI shows these next to the timeout so an
/// outage looks different from a blocked or unresolvable host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeFail {
    /// The host name did not resolve.
    Resolve,
    /// No reply within the timeout.
    Timeout,
    /// The OS ping could not be run or said something unexpected.
    Error,
}

impl From<ProbeFail> for crate::history::Fail {
    fn from(f: ProbeFail) -> Self {
        match f {
            ProbeFail::Resolve => crate::history::Fail::Resolve,
            ProbeFail::Timeout => crate::history::Fail::Timeout,
            ProbeFail::Error => crate::history::Fail::Error,
        }
    }
}

/// Commands that ask the UI thread to do something.
pub enum EngineCmd {
    Start(ProbeJob),
    /// Run one job and hand the result back on `ack` instead of the shared result
    /// channel. Only a test harness uses this: it proves the worker really produced
    /// a sample, without waiting for the scheduler's interval.
    ProbeNow {
        job: ProbeJob,
        ack: Sender<ProbeResult>,
    },
    StopTarget(usize),
    StopAll,
    SetConcurrency(usize),
}

/// Handle held by the UI. Cheap to clone.
#[derive(Clone)]
pub struct PingEngine {
    tx: Sender<EngineCmd>,
    rx: Arc<Mutex<Receiver<ProbeResult>>>,
    in_flight: Arc<Mutex<Vec<usize>>>,
}

impl Default for PingEngine {
    fn default() -> Self {
        Self::new(4)
    }
}

impl PingEngine {
    pub fn new(concurrency: usize) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCmd>();
        let (res_tx, res_rx) = mpsc::channel::<ProbeResult>();
        let in_flight: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));

        let shared_in_flight = Arc::clone(&in_flight);

        thread::Builder::new()
            .name("system-ping-engine".into())
            .spawn(move || {
                let mut pool: Vec<(SyncSender<WorkerJob>, thread::JoinHandle<()>)> = Vec::new();
                let mut want = concurrency.max(1);
                let mut next_slot = 0usize;

                loop {
                    // Keep the worker pool at the wanted size.
                    while pool.len() < want {
                        let (jtx, jrx) = mpsc::sync_channel::<WorkerJob>(want * 2 + 2);
                        let tx = res_tx.clone();
                        let infl = Arc::clone(&shared_in_flight);
                        let h = thread::Builder::new()
                            .name(format!("system-ping-worker-{}", pool.len()))
                            .spawn(move || worker_loop(jrx, tx, infl))
                            .map(|h| (jtx, h));
                        match h {
                            Ok(pair) => pool.push(pair),
                            Err(_) => break,
                        }
                    }
                    while pool.len() > want && !pool.is_empty() {
                        if pool.last().is_some() {
                            // worker_loop exits when the job channel drops.
                            let last = pool.pop();
                            drop(last);
                        } else {
                            break;
                        }
                    }

                    match cmd_rx.recv_timeout(Duration::from_millis(120)) {
                        Ok(EngineCmd::Start(job)) => {
                            if pool.is_empty() {
                                continue;
                            }
                            infl_push(&shared_in_flight, job.id);
                            let slot = next_slot % pool.len();
                            next_slot = next_slot.wrapping_add(1);
                            // If the queue is full, drop this probe rather than pile up.
                            let _ = pool[slot].0.try_send(WorkerJob { job, ack: None });
                        }
                        Ok(EngineCmd::ProbeNow { job, ack }) => {
                            // One-shot probe that answers only its own caller: used by the
                            // GUI-free tests to prove a sample really comes back. Not tracked
                            // as in flight, because nothing there prunes that list.
                            if pool.is_empty() {
                                continue;
                            }
                            let slot = next_slot % pool.len();
                            next_slot = next_slot.wrapping_add(1);
                            let _ = pool[slot].0.try_send(WorkerJob {
                                job,
                                ack: Some(ack),
                            });
                        }
                        Ok(EngineCmd::StopTarget(id)) => infl_remove(&shared_in_flight, id),
                        Ok(EngineCmd::StopAll) => {
                            if let Ok(mut v) = shared_in_flight.lock() {
                                v.clear();
                            }
                        }
                        Ok(EngineCmd::SetConcurrency(n)) => want = n.clamp(1, 32),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        // The UI handle is gone: shut the pool down.
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                for (jtx, h) in pool {
                    drop(jtx);
                    let _ = h.join();
                }
            })
            .expect("spawn ping engine");

        Self {
            tx: cmd_tx,
            rx: Arc::new(Mutex::new(res_rx)),
            in_flight,
        }
    }

    pub fn start(&self, job: ProbeJob) {
        let _ = self.tx.send(EngineCmd::Start(job));
    }

    pub fn stop_target(&self, id: usize) {
        let _ = self.tx.send(EngineCmd::StopTarget(id));
    }

    pub fn stop_all(&self) {
        let _ = self.tx.send(EngineCmd::StopAll);
    }

    pub fn set_concurrency(&self, n: usize) {
        let _ = self.tx.send(EngineCmd::SetConcurrency(n));
    }

    /// Ask for one probe of `host` right away and wait up to `wait_secs` for the answer.
    /// This is the hook the GUI-free tests use; the app itself never calls it.
    pub fn probe_now(
        &self,
        id: usize,
        host: &str,
        timeout_ms: u32,
        wait_secs: f64,
    ) -> Option<ProbeResult> {
        let (ack_tx, ack_rx) = mpsc::channel();
        let job = ProbeJob {
            id,
            host: host.to_string(),
            timeout_ms,
        };
        if self
            .tx
            .send(EngineCmd::ProbeNow { job, ack: ack_tx })
            .is_err()
        {
            return None;
        }
        ack_rx
            .recv_timeout(Duration::from_secs_f64(wait_secs.max(1.0)))
            .ok()
    }

    /// Take every result that arrived since the last call.
    pub fn drain_results(&self) -> Vec<ProbeResult> {
        let mut out = Vec::with_capacity(16);
        if let Ok(rx) = self.rx.lock() {
            while let Ok(r) = rx.try_recv() {
                out.push(r);
            }
        }
        // Prune anything still marked in flight but finished more than 30s ago.
        if let Ok(mut v) = self.in_flight.lock() {
            if v.len() > 512 {
                v.truncate(512);
            }
        }
        out
    }

    pub fn elapsed_secs(&self) -> f64 {
        // One clock for the whole program (see `runtime`): the engine stamps a result
        // with this, and the chart's "now" is egui's frame time, which eframe counts
        // from the same origin. Two separate `Instant`s here would drift apart.
        crate::runtime::secs_since_start()
    }

    /// No-op in a normal run. The engine keeps its own clock, so a headless
    /// screenshot pass has nothing to advance; kept as a seam for tests.
    pub fn debug_advance(&self, _dt: f64) {}
}

/// The in-flight id list is kept as a plain `Vec` and searched linearly. That is deliberate:
/// the window's own cap is 16 targets, so a set or map costs more than it saves here — at this
/// size the search runs while the thread is parked waiting for ping.exe anyway. If the cap ever
/// grows past a few hundred, swap these two helpers for a `HashSet`.
fn infl_push(m: &Arc<Mutex<Vec<usize>>>, id: usize) {
    if let Ok(mut v) = m.lock() {
        if !v.contains(&id) {
            v.push(id);
        }
    }
}

fn infl_remove(m: &Arc<Mutex<Vec<usize>>>, id: usize) {
    if let Ok(mut v) = m.lock() {
        v.retain(|x| *x != id);
    }
}

fn worker_loop(jobs: Receiver<WorkerJob>, out: Sender<ProbeResult>, infl: Arc<Mutex<Vec<usize>>>) {
    while let Ok(wj) = jobs.recv() {
        // An ack-less job (the sender was dropped) is simply skipped.
        let ack = wj.ack;
        let job = wj.job;
        let t0 = Instant::now();
        let rtt = probe_once(&job);
        infl_remove(&infl, job.id);
        let res = ProbeResult {
            id: job.id,
            fail: rtt.as_ref().err().copied(),
            rtt_ms: rtt.unwrap_or(crate::history::TIMEOUT),
            t_secs: t0.elapsed().as_secs_f64(),
            at_ms: chrono::Utc::now().timestamp_millis(),
        };
        // A one-shot probe answers its own caller; a scheduled probe goes to the shared queue.
        match ack {
            Some(a) => {
                let _ = a.send(res);
            }
            None => {
                let _ = out.send(res);
            }
        }
    }
}

/// A job plus an optional private reply channel (see [`EngineCmd::ProbeNow`]).
struct WorkerJob {
    job: ProbeJob,
    ack: Option<Sender<ProbeResult>>,
}

/// Run the OS ping once and read the round-trip time out of its text output.
fn probe_once(job: &ProbeJob) -> Result<f32, ProbeFail> {
    if !host_is_safe(&job.host) {
        return Err(ProbeFail::Error);
    }
    #[cfg(windows)]
    let r = probe_windows(job);
    #[cfg(not(windows))]
    let r = probe_unix(job);
    r
}

/// Only allow characters that a real host name or IP address can contain. This
/// keeps user input from turning into extra command arguments.
fn host_is_safe(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

#[cfg(windows)]
fn probe_windows(job: &ProbeJob) -> Result<f32, ProbeFail> {
    // `ping -n 1` sends one echo request; `-w` is the per-reply timeout in ms.
    let mut cmd = Command::new("ping");
    cmd.args(["-n", "1", "-w"]);
    cmd.arg(job.timeout_ms.to_string());
    cmd.arg(&job.host);
    run_and_parse(cmd, &["time=", "Zeit="], job.timeout_ms)
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
fn probe_unix(job: &ProbeJob) -> Result<f32, ProbeFail> {
    let mut cmd = Command::new("ping");
    cmd.args(["-c", "1", "-W"]);
    // Unix ping takes whole seconds for the wait; round up to at least 1.
    cmd.arg(((job.timeout_ms + 999) / 1000).to_string());
    cmd.arg(&job.host);
    run_and_parse(cmd, &["time="], job.timeout_ms)
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "macos",
    target_os = "android"
)))]
fn probe_unix(_job: &ProbeJob) -> Result<f32, ProbeFail> {
    Err(ProbeFail::Error)
}

fn run_and_parse(mut cmd: Command, markers: &[&str], timeout_ms: u32) -> Result<f32, ProbeFail> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        // Keep the ping window from flashing on every probe.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Err(ProbeFail::Error),
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64 + 1500);
    let mut buf = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let mut sink = Vec::new();
        // Read with a hard deadline so a stuck ping can never hang the worker.
        loop {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            let mut chunk = [0u8; 512];
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => sink.extend_from_slice(&chunk[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // Windows ping prints in the OEM codepage; try UTF-8 then Latin-1. `from_utf8`
        // consumes the buffer, so the output is never copied twice.
        match String::from_utf8(sink) {
            Ok(s) => buf = s,
            Err(e) => buf = e.into_bytes().iter().map(|&b| b as char).collect(),
        }
    }

    let _status = match child.wait() {
        Ok(s) => s,
        Err(_) => return Err(ProbeFail::Error),
    };
    // The exit code is deliberately dropped. `ping.exe` exits 2 when a host gives no reply and
    // also when the name cannot be resolved (see `report.rs`, where the real run shows both), so
    // the number cannot tell those two apart; the text can, and that is what decides here.
    // ping exits non-zero when there was no reply, so the text decides:
    // a name that did not resolve is different from a host that stayed silent.
    match parse_rtt(&buf, markers) {
        Some(v) => Ok(v),
        None => Err(classify_failure(&buf)),
    }
}

/// Sort out why there was no reply time in the output.
fn classify_failure(text: &str) -> ProbeFail {
    let t = text.to_ascii_lowercase();
    const RESOLVE: &[&str] = &[
        "could not find",
        "name or service",
        "unknown host",
        "unbekannter",
        "nicht gefunden",
        "failed to resolve",
        "no such host",
    ];
    const TIMEOUT: &[&str] = &[
        "timed out",
        "request timeout",
        "zeitüberschreitung",
        "zeitsberschreitung",
        "100% loss",
        "100 % verlust",
        "keine antwort",
    ];
    if RESOLVE.iter().any(|m| t.contains(m)) {
        ProbeFail::Resolve
    } else if TIMEOUT.iter().any(|m| t.contains(m)) {
        ProbeFail::Timeout
    } else {
        ProbeFail::Error
    }
}

/// Pull `time=42ms` / `time=1.2ms` (and the German `Zeit=`) out of ping output.
fn parse_rtt(text: &str, markers: &[&str]) -> Option<f32> {
    for line in text.lines() {
        for marker in markers {
            if let Some(pos) = line.find(marker) {
                // Read the number straight out of the line instead of collecting it into a
                // String first; only a decimal comma needs replacing before parsing.
                let tail = &line[pos + marker.len()..];
                let end = tail
                    .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
                    .unwrap_or(tail.len());
                let raw = tail[..end].trim_matches('.');
                if raw.is_empty() {
                    continue;
                }
                let cleaned = if raw.contains(',') {
                    Cow::Owned(raw.replace(',', "."))
                } else {
                    Cow::Borrowed(raw)
                };
                if let Ok(v) = cleaned.parse::<f32>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// `ProbeFail` as the short label the UI and the CSV use.
pub fn fail_label(f: ProbeFail) -> &'static str {
    match f {
        ProbeFail::Resolve => "DNS",
        ProbeFail::Timeout => "timeout",
        ProbeFail::Error => "Fehler",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_and_german() {
        assert_eq!(
            parse_rtt("Reply from 1.1.1.1: time=42ms", &["time="]),
            Some(42.0)
        );
        assert_eq!(
            parse_rtt("Antwort von 8.8.8.8: Zeit=1,53ms", &["Zeit="]),
            Some(1.53)
        );
        assert_eq!(parse_rtt("Request timed out.", &["time="]), None);
    }

    #[test]
    fn classifies_failures() {
        assert_eq!(
            classify_failure("Ping request could not find host foo.bar."),
            ProbeFail::Resolve
        );
        assert_eq!(
            classify_failure("Antwort von 10.0.0.1: Zeitüberschreitung des Anfor..."),
            ProbeFail::Timeout
        );
        assert_eq!(classify_failure("Request timed out."), ProbeFail::Timeout);
        assert_eq!(classify_failure("100% loss"), ProbeFail::Timeout);
        assert_eq!(classify_failure(""), ProbeFail::Error);
    }

    #[test]
    fn rejects_bad_hosts() {
        assert!(!host_is_safe(""));
        assert!(!host_is_safe("a;rm -x"));
        assert!(!host_is_safe("host \"x"));
        assert!(host_is_safe("example.com"));
        assert!(host_is_safe("192.168.0.1"));
        assert!(host_is_safe("fe80::1"));
    }
}
