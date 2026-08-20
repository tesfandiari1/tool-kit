//! The conversion service as a child process the app owns.
//!
//! In Sidecar mode the user deploys nothing: `tool-kit-converter` ships inside
//! the bundle beside the app binary, and this module starts it, hands it a
//! token, finds out which port the kernel gave it, restarts it when it dies,
//! and kills it on the way out.
//!
//! Three details carry most of the weight.
//!
//! The port is ephemeral. A fixed port hands the bearer token to whatever
//! process squats on it, and the origin guard cannot tell the difference
//! because both URLs match. So the service binds `127.0.0.1:0` and logs the
//! address it actually got, and the one line it writes on stdout is the whole
//! handshake.
//!
//! The stdin pipe is both the parent-death signal and the shutdown channel. The
//! service quits on stdin EOF, which fires even when the app is killed outright
//! and can signal nothing, so that one pipe covers the orphan case
//! `RunEvent::Exit` cannot reach. The write handle is held for the life of the
//! child and closed only by a deliberate stop, which closes it *before* the
//! signal for the reason `stop` gives.
//!
//! Every signal goes through `/bin/kill`. `unsafe_code = "forbid"` blocks
//! `libc::kill` and an inner `#[allow]` cannot lift it, and neither
//! `tokio::process` nor the shell plugin can send anything but SIGKILL.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

use crate::secrets;
use crate::settings::{self, LocalBackendMode};

/// Sits beside the app binary in `Contents/MacOS` and in `target/debug` alike,
/// because tauri-bundler strips the `-{target}` suffix when it copies.
const CONVERTER_BIN: &str = "tool-kit-converter";

/// The service logs this once, as JSON on stdout, with the bound address.
const LISTENING_MESSAGE: &str = "conversion service listening";

/// Matches the Docker start period. Cold start is milliseconds; this budget is
/// for a machine under load, not for a healthy launch.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS`. The default 30 makes ⌘Q hang for
/// half a minute whenever a job is in flight.
const SHUTDOWN_GRACE_SECS: u64 = 5;

/// How long SIGTERM gets before SIGKILL. The service force-cancels its runner
/// at the end of its own grace window, so waiting longer than that only ever
/// waits on a process that was never going to exit.
const TERM_WAIT: Duration = Duration::from_secs(5);
const KILL_WAIT: Duration = Duration::from_millis(1500);

/// The cap on the `RunEvent::Exit` path. A blocking drain there sits inside
/// `applicationWillTerminate`, so a long one reads to the user as a hang.
const EXIT_BUDGET: Duration = Duration::from_secs(7);

/// The budget everywhere the app is not being torn down under us, so a service
/// finishing a job gets the whole of its own grace window.
pub(crate) const STOP_BUDGET: Duration = Duration::from_secs(10);

const LOG_CAP_BYTES: u64 = 2 * 1024 * 1024;

const MAX_RESTARTS: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(300);

/// How long to wait for the stderr drain to finish after a failed start, so
/// the message the user sees is the last thing the service actually said.
const STDERR_FLUSH: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum BackendState {
    #[default]
    Stopped,
    Starting,
    Running {
        port: u16,
        pid: u32,
        vision: bool,
    },
    Failed {
        message: String,
    },
}

/// The webview's view of the state. Flat on purpose: the status row holds every
/// slot at every state and dims what it does not know yet.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackendStatus {
    state: &'static str,
    port: Option<u16>,
    pid: Option<u32>,
    vision: bool,
    message: Option<String>,
}

impl From<&BackendState> for BackendStatus {
    fn from(state: &BackendState) -> Self {
        match state {
            BackendState::Stopped => Self {
                state: "stopped",
                port: None,
                pid: None,
                vision: false,
                message: None,
            },
            BackendState::Starting => Self {
                state: "starting",
                port: None,
                pid: None,
                vision: false,
                message: None,
            },
            BackendState::Running { port, pid, vision } => Self {
                state: "running",
                port: Some(*port),
                pid: Some(*pid),
                vision: *vision,
                message: None,
            },
            BackendState::Failed { message } => Self {
                state: "failed",
                port: None,
                pid: None,
                vision: false,
                message: Some(message.clone()),
            },
        }
    }
}

#[derive(Default)]
struct Inner {
    state: BackendState,
    /// Bumped by every stop and every restart. The supervisor captures it once
    /// and quits the moment it stops matching, which is what makes a stop that
    /// lands mid-launch still terminate the child it never saw.
    epoch: u64,
    pid: Option<u32>,
    /// The write end of the child's stdin, parked here rather than in the
    /// supervisor so that a stop can close it. See `stop` for why that has to
    /// happen before the signal.
    stdin: Option<ChildStdin>,
    /// One token per app launch, reused across restarts.
    minted: bool,
    supervising: bool,
    /// Set by the first `Running` of the launch. Recovery builds one job row
    /// per ledger entry, so running it again after a restart would show every
    /// still-unfinished conversion twice.
    recovered: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    /// False while a supervisor owns a child. `stop` waits on this rather than
    /// on the process, because the supervisor is the only holder of the `Child`.
    idle: watch::Sender<bool>,
}

#[derive(Clone)]
pub(crate) struct BackendHost {
    shared: Arc<Shared>,
}

impl BackendHost {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner::default()),
                idle: watch::channel(true).0,
            }),
        }
    }

    /// Poisoning is recovered rather than propagated: the critical sections are
    /// short, and one panic must not wedge every later transition.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.shared
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    fn stale(&self, epoch: u64) -> bool {
        self.lock().epoch != epoch
    }

    /// Take custody of the running child, unless a stop landed while it was
    /// starting. Handing the pid and the stdin handle over under one lock is
    /// what closes the handoff: either `stop` gets them and shuts the child
    /// down, or it does not, and the stdin comes back here so the supervisor
    /// can do it itself.
    fn claim(&self, epoch: u64, pid: u32, stdin: ChildStdin) -> Result<(), ChildStdin> {
        let mut inner = self.lock();
        if inner.epoch != epoch {
            return Err(stdin);
        }
        inner.pid = Some(pid);
        inner.stdin = Some(stdin);
        Ok(())
    }

    fn release(&self) {
        let mut inner = self.lock();
        inner.pid = None;
        inner.stdin = None;
    }

    fn request_stop(&self) -> (Option<u32>, Option<ChildStdin>) {
        let mut inner = self.lock();
        inner.epoch += 1;
        (inner.pid.take(), inner.stdin.take())
    }

    fn finish(&self) {
        self.lock().supervising = false;
        let _ = self.shared.idle.send(true);
    }

    /// True once per launch, on the first service that comes up.
    fn claim_recovery(&self) -> bool {
        !std::mem::replace(&mut self.lock().recovered, true)
    }

    fn publish(&self, app: &AppHandle, state: BackendState) {
        let status = {
            let mut inner = self.lock();
            if inner.state == state {
                return;
            }
            inner.state = state;
            BackendStatus::from(&inner.state)
        };
        let _ = app.emit("backend-status", status);
    }

    fn status(&self) -> BackendStatus {
        BackendStatus::from(&self.lock().state)
    }

    /// Mint the token once per launch and put it where both sides read it.
    ///
    /// The keychain write goes first and goes through `secrets::set_key`, which
    /// is the only write that drops the process memo. Straight to the keychain
    /// and every backend job fails with "Backend token is unavailable" for the
    /// rest of the session, because a `None` read earlier is cached forever.
    fn ensure_token(&self, layout: &Layout) -> Result<(), String> {
        if self.lock().minted {
            return Ok(());
        }
        let token = mint_token();
        secrets::set_key("backend", &token)?;
        write_token_file(&layout.token_file, &token)?;
        self.lock().minted = true;
        Ok(())
    }
}

fn host(app: &AppHandle) -> Option<BackendHost> {
    app.try_state::<BackendHost>().map(|state| (*state).clone())
}

/// Start the sidecar when this install owns the process. A no-op in Manual
/// mode, where the user runs the service and the app only points at a URL.
pub(crate) fn start(app: &AppHandle) {
    if settings::load(app).local_backend_mode != LocalBackendMode::Sidecar {
        return;
    }
    let Some(host) = host(app) else {
        return;
    };
    {
        let mut inner = host.lock();
        if inner.supervising {
            return;
        }
        inner.supervising = true;
    }
    let _ = host.shared.idle.send(false);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        supervise(app, host).await;
    });
}

/// Stop the child and wait for it, giving up after `budget`.
///
/// Closing stdin comes first because it is the one path that needs no pid: the
/// service reads EOF and quits in milliseconds. SIGTERM follows and reaches the
/// same drain, so either alone is enough. Both go out because they fail in
/// different places. Stdin does nothing to a build with the knob off, and the
/// signal does nothing to a child whose pid we never learned.
pub(crate) async fn stop(app: &AppHandle, budget: Duration) {
    let Some(host) = host(app) else {
        return;
    };
    let (pid, stdin) = host.request_stop();
    drop(stdin);
    let mut idle = host.shared.idle.subscribe();
    if *idle.borrow_and_update() {
        return;
    }

    let Some(pid) = pid else {
        // The supervisor is mid-launch and has no pid to signal yet. It checks
        // the epoch the moment the child is up and terminates it there.
        let _ = tokio::time::timeout(budget, wait_idle(&mut idle)).await;
        return;
    };

    signal(pid, "-TERM");
    if tokio::time::timeout(budget.min(TERM_WAIT), wait_idle(&mut idle))
        .await
        .is_err()
    {
        signal(pid, "-KILL");
        let _ = tokio::time::timeout(KILL_WAIT, wait_idle(&mut idle)).await;
    }
}

/// The last chance to take the child with us. Called from `RunEvent::Exit`,
/// which is the only exit event this app sees: `ExitRequested` never fires on
/// ⌘Q because the window hides instead of being destroyed.
pub(crate) fn stop_on_exit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::block_on(async move {
        stop(&app, EXIT_BUDGET).await;
    });
}

async fn wait_idle(idle: &mut watch::Receiver<bool>) {
    while idle.changed().await.is_ok() {
        if *idle.borrow_and_update() {
            return;
        }
    }
}

// ----------------------------------------------------------------- the address

/// What a caller hears while the service is coming up. A refused connection
/// says nothing anyone can act on, and the answer to this one is to wait.
const STARTING: &str = "The conversion service is starting. Try again in a moment.";

/// Never a URL: it names the service rather than locating it, and
/// `validate_base_url` rejects it on sight.
pub(crate) const SIDECAR_ALIAS: &str = "sidecar";

/// The origin every backend request goes to, read at the moment of the request.
///
/// Nothing may hold on to this. The kernel picks the sidecar's port at each
/// launch and again at each restart, so an origin kept from one run is refused
/// by the next.
pub(crate) fn backend_origin(app: &AppHandle) -> Result<String, String> {
    let settings = settings::load(app);
    match settings.local_backend_mode {
        LocalBackendMode::Sidecar => sidecar_origin(app),
        LocalBackendMode::Manual => Ok(settings.backend_url),
    }
}

fn sidecar_origin(app: &AppHandle) -> Result<String, String> {
    let Some(host) = host(app) else {
        return Err(STARTING.to_string());
    };
    let state = host.lock().state.clone();
    match state {
        BackendState::Running { port, .. } => Ok(origin_for(port)),
        BackendState::Failed { message } => Err(message),
        BackendState::Stopped | BackendState::Starting => Err(STARTING.to_string()),
    }
}

/// The bearer token, from the one keychain slot both modes share. This app
/// minted it and gave it to nothing but its own child in Sidecar mode; the user
/// pasted it into Settings in Manual mode.
pub(crate) fn backend_token(app: &AppHandle) -> Result<String, String> {
    if let Some(token) = secrets::get_key("backend").filter(|value| !value.trim().is_empty()) {
        return Ok(token);
    }
    if settings::load(app).local_backend_mode == LocalBackendMode::Sidecar {
        // Minted at the top of the launch, so an empty slot means the launch
        // has not got that far rather than that anything is missing.
        return Err(STARTING.to_string());
    }
    Err("Backend token is unavailable. Add it in Settings, then retry.".to_string())
}

/// What the ledger records as the origin a row was submitted against.
///
/// The live URL cannot be it. Recovery compares the recorded origin to the
/// configured one by exact equality, and the port moves every launch, so a live
/// URL fails that comparison for every in-flight row on every relaunch and the
/// row is deleted. The alias holds still and keeps the guarantee the comparison
/// exists for: in Sidecar mode this app mints the token and hands it to its own
/// child alone. Manual mode records the URL and its guard is untouched.
pub(crate) fn ledger_origin(mode: LocalBackendMode, live: &str) -> &str {
    match mode {
        LocalBackendMode::Sidecar => SIDECAR_ALIAS,
        LocalBackendMode::Manual => live,
    }
}

// ---------------------------------------------------------------- supervisor

async fn supervise(app: AppHandle, host: BackendHost) {
    let epoch = host.epoch();

    let layout = match Layout::resolve(&app) {
        Ok(layout) => layout,
        Err(message) => {
            host.publish(&app, BackendState::Failed { message });
            host.finish();
            return;
        }
    };
    reap_orphan(&layout.runtime_file).await;
    if let Err(message) = host.ensure_token(&layout) {
        host.publish(&app, BackendState::Failed { message });
        host.finish();
        return;
    }

    // Instants of the failures still inside the window, not a running count, so
    // an app left open for a week does not accumulate its way to Failed.
    let mut failures: Vec<Instant> = Vec::new();
    let mut last_message;

    loop {
        host.publish(&app, BackendState::Starting);
        match launch(&app, &layout).await {
            Ok(running) => {
                let Running {
                    mut child,
                    stdin,
                    pid,
                    port,
                    vision,
                } = running;
                if let Err(stdin) = host.claim(epoch, pid, stdin) {
                    terminate(&mut child, pid, Some(stdin)).await;
                    break;
                }
                write_runtime(&layout.runtime_file, pid, port);
                host.publish(&app, BackendState::Running { port, pid, vision });
                if host.claim_recovery() {
                    // Recovery waits for a service that answers. It runs from
                    // `setup` in Manual mode, but here the port does not exist
                    // yet at that point, and the resume path fails a job on one
                    // refused connection with no retry behind it.
                    crate::jobs::recover_in_flight(app.clone());
                }
                let _ = child.wait().await;
                host.release();
                let _ = std::fs::remove_file(&layout.runtime_file);
                last_message = "The conversion service stopped unexpectedly".to_string();
            }
            Err(message) => last_message = message,
        }

        if host.stale(epoch) {
            break;
        }
        failures.retain(|at| at.elapsed() < RESTART_WINDOW);
        failures.push(Instant::now());
        if failures.len() > MAX_RESTARTS {
            host.publish(
                &app,
                BackendState::Failed {
                    message: format!("{last_message}. It has been restarted five times in five minutes, so Tool-Kit stopped trying."),
                },
            );
            host.finish();
            return;
        }

        host.publish(
            &app,
            BackendState::Failed {
                message: last_message.clone(),
            },
        );
        tokio::time::sleep(restart_delay(failures.len() - 1)).await;
        if host.stale(epoch) {
            break;
        }
    }

    host.publish(&app, BackendState::Stopped);
    host.finish();
}

/// 1, 2, 4, 8, 16, then 30 forever.
fn restart_delay(attempt: usize) -> Duration {
    let seconds = 1u64 << attempt.min(5);
    Duration::from_secs(seconds.min(30))
}

struct Running {
    child: Child,
    /// Held for the life of the child. Dropping it is EOF on the service's
    /// stdin, which is how it learns the app died and how a deliberate stop
    /// gets it to quit.
    stdin: ChildStdin,
    pid: u32,
    port: u16,
    vision: bool,
}

/// Spawn the service, drain both pipes, and wait for it to say where it is.
///
/// The spawn and the drain live in one function on purpose. Split them and a
/// full pipe buffer blocks the service the first time it logs anything, which
/// surfaces as a conversion that hangs for no visible reason.
async fn launch(app: &AppHandle, layout: &Layout) -> Result<Running, String> {
    let binary = converter_binary()?;
    let bcmaps = bcmaps_dir(app)?;

    let mut command = tokio::process::Command::new(&binary);
    command
        .envs(converter_env(layout, &bcmaps))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start the conversion service: {e}"))?;
    let pid = child
        .id()
        .ok_or_else(|| "The conversion service exited before it reported a pid".to_string())?;
    let (stdin, stdout, stderr) =
        match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(stdin), Some(stdout), Some(stderr)) => (stdin, stdout, stderr),
            _ => {
                let _ = child.start_kill();
                return Err("Could not open the conversion service pipes".to_string());
            }
        };

    let sink = Arc::new(LogSink::new(layout.log_file.clone()));
    let last_error = Arc::new(Mutex::new(None::<String>));
    let (port_tx, port_rx) = oneshot::channel::<SocketAddr>();
    tauri::async_runtime::spawn(drain_stdout(stdout, Arc::clone(&sink), port_tx));
    let stderr_drain =
        tauri::async_runtime::spawn(drain_stderr(stderr, sink, Arc::clone(&last_error)));

    // A closed channel means stdout reached EOF first, so the process is
    // already gone and there is nothing to wait 30 seconds for.
    let bound = match tokio::time::timeout(HANDSHAKE_TIMEOUT, port_rx).await {
        Ok(Ok(address)) => address,
        Ok(Err(_)) => {
            let detail = failure_detail(stderr_drain, &last_error).await;
            terminate(&mut child, pid, Some(stdin)).await;
            return Err(join(
                "The conversion service exited before it started listening",
                detail,
            ));
        }
        Err(_) => {
            let detail = failure_detail(stderr_drain, &last_error).await;
            terminate(&mut child, pid, Some(stdin)).await;
            return Err(join(
                "The conversion service did not start listening within 30 seconds",
                detail,
            ));
        }
    };

    let port = bound.port();
    let vision = match confirm_engines(port).await {
        Ok(vision) => vision,
        Err(headline) => {
            let detail = failure_detail(stderr_drain, &last_error).await;
            terminate(&mut child, pid, Some(stdin)).await;
            return Err(join(&headline, detail));
        }
    };

    Ok(Running {
        child,
        stdin,
        pid,
        port,
        vision,
    })
}

/// The last line the service wrote to stderr, verbatim.
///
/// A configuration error is the common failure and it prints one
/// `Debug`-formatted line and no JSON at all, because tracing initialises after
/// the config is parsed. So the message has to come from that line rather than
/// from a parsed field.
async fn failure_detail(
    stderr_drain: tauri::async_runtime::JoinHandle<()>,
    last_error: &Arc<Mutex<Option<String>>>,
) -> Option<String> {
    let _ = tokio::time::timeout(STDERR_FLUSH, stderr_drain).await;
    last_error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn join(headline: &str, detail: Option<String>) -> String {
    match detail {
        Some(detail) => format!("{headline}: {detail}"),
        None => headline.to_string(),
    }
}

async fn drain_stdout(stdout: ChildStdout, sink: Arc<LogSink>, port: oneshot::Sender<SocketAddr>) {
    let mut port = Some(port);
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        sink.append(&line);
        if let Some(address) = parse_listening(&line) {
            if let Some(port) = port.take() {
                let _ = port.send(address);
            }
        }
    }
}

async fn drain_stderr(
    stderr: ChildStderr,
    sink: Arc<LogSink>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        sink.append(&line);
        if !line.trim().is_empty() {
            *last_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(line);
        }
    }
}

fn parse_listening(line: &str) -> Option<SocketAddr> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let fields = value.get("fields")?;
    if fields.get("message")?.as_str()? != LISTENING_MESSAGE {
        return None;
    }
    fields.get("bind_address")?.as_str()?.parse().ok()
}

/// Wait for readiness, then record whether the Vision engine came up.
///
/// Both endpoints are unauthenticated. A missing Vision engine is a release
/// defect rather than a runtime state, so it lands in the status row and
/// `verify-release.sh` stays the real gate.
async fn confirm_engines(port: u16) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| format!("Could not reach the conversion service: {e}"))?;
    let origin = origin_for(port);

    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        let ready = client
            .get(format!("{origin}/health/ready"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success());
        if ready {
            break;
        }
        if Instant::now() >= deadline {
            return Err("The conversion service started but never became ready".to_string());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let capabilities = client
        .get(format!("{origin}/api/v1/capabilities"))
        .send()
        .await
        .map_err(|e| format!("The conversion service did not report its capabilities: {e}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("The conversion service reported unreadable capabilities: {e}"))?;

    let vision = capabilities
        .pointer("/data/conversion/engines")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|engines| {
            engines
                .iter()
                .filter_map(|engine| engine.get("name").and_then(serde_json::Value::as_str))
                .any(|name| name == "apple-vision")
        });
    Ok(vision)
}

fn origin_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

// --------------------------------------------------------------- the process

/// Close stdin and signal, then SIGKILL a service that still will not go.
async fn terminate(child: &mut Child, pid: u32, stdin: Option<ChildStdin>) {
    // EOF first, signal second, for the reason `stop` gives.
    drop(stdin);
    signal(pid, "-TERM");
    if tokio::time::timeout(TERM_WAIT, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

fn signal(pid: u32, sig: &str) {
    let _ = std::process::Command::new("/bin/kill")
        .args([sig, &pid.to_string()])
        .status();
}

/// A converter left behind by a crash or a SIGKILL of the app.
///
/// Exactly one service may own a data root: startup recovery treats every
/// in-flight row as a crash leftover with no liveness check, and the crate
/// takes no process lock.
async fn reap_orphan(runtime_file: &Path) {
    let recorded = std::fs::read(runtime_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RuntimeRecord>(&bytes).ok());
    let _ = std::fs::remove_file(runtime_file);
    let Some(record) = recorded else {
        return;
    };
    // The name check, not just liveness: a pid is reused within minutes on a
    // busy Mac and killing whatever inherited it would be worse than the orphan.
    if !is_converter(record.pid) {
        return;
    }

    signal(record.pid, "-TERM");
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !is_converter(record.pid) {
            return;
        }
    }
    signal(record.pid, "-KILL");
}

fn is_converter(pid: u32) -> bool {
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let command = String::from_utf8_lossy(&output.stdout);
    Path::new(command.trim())
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == CONVERTER_BIN)
}

#[derive(Serialize, Deserialize)]
struct RuntimeRecord {
    pid: u32,
    port: u16,
    origin: String,
}

fn write_runtime(path: &Path, pid: u32, port: u16) {
    let record = RuntimeRecord {
        pid,
        port,
        origin: origin_for(port),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&record) {
        let _ = std::fs::write(path, bytes);
    }
}

// ----------------------------------------------------------------- the layout

struct Layout {
    data_dir: PathBuf,
    token_file: PathBuf,
    runtime_file: PathBuf,
    log_file: PathBuf,
}

impl Layout {
    fn resolve(app: &AppHandle) -> Result<Self, String> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("Could not locate the Tool-Kit data folder: {e}"))?;
        std::fs::create_dir_all(&root)
            .map_err(|e| format!("Could not create {}: {e}", root.display()))?;

        // Left alone otherwise: the service's artifact store creates it, and a
        // symlink here would point every stored artifact somewhere else.
        let data_dir = root.join("converter");
        if data_dir
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(format!(
                "{} is a symbolic link. Remove it and reopen Tool-Kit.",
                data_dir.display()
            ));
        }

        Ok(Self {
            data_dir,
            token_file: root.join("converter.token"),
            runtime_file: root.join("converter-runtime.json"),
            log_file: root.join("converter.log"),
        })
    }
}

fn converter_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not locate the Tool-Kit executable: {e}"))?;
    let path = exe.with_file_name(CONVERTER_BIN);
    if !path.is_file() {
        return Err(format!(
            "The conversion service is missing from this build ({}). Run `pnpm sidecars`.",
            path.display()
        ));
    }
    Ok(path)
}

/// The CMap tables the PDF engine needs for CJK text.
///
/// A missing directory fails the start rather than starting without the
/// variable. Unset, the engine falls back to a path inside the build machine's
/// Cargo registry, so CJK PDFs would lose their ToUnicode mapping on every
/// user's machine and on no developer's, with no error anywhere.
fn bcmaps_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .resolve("pdf-inspector/bcmaps", BaseDirectory::Resource)
        .map_err(|e| format!("Could not locate the bundled CMap tables: {e}"))?;
    if !path.is_dir() {
        return Err(format!(
            "The bundled CMap tables are missing ({}). Run `pnpm sidecars`.",
            path.display()
        ));
    }
    Ok(path)
}

/// Every variable the child gets, named here rather than inherited.
///
/// The environment is not cleared: the service needs `TMPDIR`, and its own
/// workers clear theirs already. `RUST_LOG` is set rather than inherited
/// because a quieter value would suppress the listening line and the handshake
/// would hang with nothing to show for it.
fn converter_env(layout: &Layout, bcmaps: &Path) -> BTreeMap<String, String> {
    let text = |path: &Path| path.to_string_lossy().into_owned();
    BTreeMap::from([
        // Port 0 asks the kernel for a free port. Nothing to race, and no
        // squatter to hand the bearer token to.
        (
            "TOOLKIT_CONVERTER_BIND_ADDR".to_string(),
            "127.0.0.1:0".to_string(),
        ),
        // The Docker defaults are /data and /run/secrets/bootstrap_token,
        // neither of which a sandboxed app can create.
        (
            "TOOLKIT_CONVERTER_DATA_DIR".to_string(),
            text(&layout.data_dir),
        ),
        (
            "TOOLKIT_CONVERTER_TOKEN_FILE".to_string(),
            text(&layout.token_file),
        ),
        ("TOOLKIT_CONVERTER_PDF_BCMAPS_DIR".to_string(), text(bcmaps)),
        // The queue is durable and the runner is serial, so depth is free and a
        // 429 at the ceiling is not.
        ("TOOLKIT_CONVERTER_MAX_JOBS".to_string(), "512".to_string()),
        // JobManager runs four jobs at once and the service uses try_acquire,
        // not acquire, so a tight ceiling turns straight into refusals.
        (
            "TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS".to_string(),
            "8".to_string(),
        ),
        // One knob covers four deadlines, Vision OCR included.
        (
            "TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS".to_string(),
            "300".to_string(),
        ),
        (
            "TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS".to_string(),
            SHUTDOWN_GRACE_SECS.to_string(),
        ),
        (
            "TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF".to_string(),
            "1".to_string(),
        ),
        (
            "RUST_LOG".to_string(),
            "tool_kit_converter=info".to_string(),
        ),
    ])
}

/// 64 hex characters from two v4 UUIDs. The service takes 32 to 512 visible
/// ASCII bytes, so this sits well inside the window and adds no dependency.
fn mint_token() -> String {
    let mut token = Uuid::new_v4().simple().to_string();
    token.push_str(&Uuid::new_v4().simple().to_string());
    token
}

fn write_token_file(path: &Path, token: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    // The mode is applied at creation, so an existing file would keep whatever
    // bits it already had.
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    file.write_all(token.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

// -------------------------------------------------------------------- the log

/// Both pipes append here. Opened lazily and reopened once the cap is hit, so a
/// service that logs a line per request cannot fill a disk.
struct LogSink {
    path: PathBuf,
    state: Mutex<Option<std::fs::File>>,
}

impl LogSink {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
        }
    }

    fn append(&self, line: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_none() {
            *state = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
        }
        let Some(file) = state.as_mut() else {
            return;
        };
        if file.metadata().map(|meta| meta.len()).unwrap_or(0) > LOG_CAP_BYTES {
            match std::fs::File::create(&self.path) {
                Ok(fresh) => *file = fresh,
                Err(_) => return,
            }
        }
        // A log write must never take a conversion down with it.
        let _ = writeln!(file, "{line}");
    }
}

// --------------------------------------------------------------- the commands

#[tauri::command]
pub(crate) fn backend_status(app: AppHandle) -> BackendStatus {
    match host(&app) {
        Some(host) => host.status(),
        None => BackendStatus::from(&BackendState::Stopped),
    }
}

#[tauri::command]
pub(crate) async fn restart_backend(app: AppHandle) -> Result<(), String> {
    if settings::load(&app).local_backend_mode != LocalBackendMode::Sidecar {
        return Err("Tool-Kit does not own the conversion service in Manual mode".to_string());
    }
    stop(&app, STOP_BUDGET).await;
    start(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn open_backend_log(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let layout = Layout::resolve(&app)?;
    if !layout.log_file.is_file() {
        return Err("The conversion service has not logged anything yet".to_string());
    }
    app.opener()
        .open_path(layout.log_file.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout {
            data_dir: PathBuf::from("/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter"),
            token_file: PathBuf::from("/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter.token"),
            runtime_file: PathBuf::from("/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter-runtime.json"),
            log_file: PathBuf::from("/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter.log"),
        }
    }

    #[test]
    fn the_child_gets_every_variable_the_desktop_depends_on() {
        let env = converter_env(
            &layout(),
            Path::new("/Applications/Tool-Kit.app/Contents/Resources/pdf-inspector/bcmaps"),
        );

        assert_eq!(env["TOOLKIT_CONVERTER_BIND_ADDR"], "127.0.0.1:0");
        assert_eq!(
            env["TOOLKIT_CONVERTER_DATA_DIR"],
            "/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter"
        );
        assert_eq!(
            env["TOOLKIT_CONVERTER_TOKEN_FILE"],
            "/Users/x/Library/Application Support/dev.esfandiari.toolkit/converter.token"
        );
        assert_eq!(
            env["TOOLKIT_CONVERTER_PDF_BCMAPS_DIR"],
            "/Applications/Tool-Kit.app/Contents/Resources/pdf-inspector/bcmaps"
        );
        assert_eq!(env["TOOLKIT_CONVERTER_MAX_JOBS"], "512");
        assert_eq!(env["TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS"], "8");
        assert_eq!(env["TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS"], "300");
        assert_eq!(env["TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS"], "5");
        assert_eq!(env["TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF"], "1");
        assert_eq!(env.len(), 10);
    }

    /// A quieter inherited filter drops the listening line and the handshake
    /// then waits the full 30 seconds for something that will never arrive.
    #[test]
    fn the_log_filter_is_set_rather_than_inherited() {
        let env = converter_env(&layout(), Path::new("/tmp/bcmaps"));

        assert_eq!(env["RUST_LOG"], "tool_kit_converter=info");
    }

    /// Captured from a release build of the service, byte for byte.
    #[test]
    fn the_bound_port_is_read_off_the_listening_line() {
        let line = r#"{"timestamp":"2026-08-20T06:32:03.849202Z","level":"INFO","fields":{"message":"conversion service listening","bind_address":"127.0.0.1:64707","service_version":"0.4.4"},"target":"tool_kit_converter"}"#;

        let address = parse_listening(line).expect("the listening line should carry the port");

        assert_eq!(address.port(), 64707);
        assert_eq!(address.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn every_other_line_is_ignored() {
        let shutdown = r#"{"timestamp":"2026-08-20T06:32:03.850024Z","level":"INFO","fields":{"message":"shutdown signal received","reason":"stdin_eof"},"target":"tool_kit_converter"}"#;
        assert_eq!(parse_listening(shutdown), None);

        // A config error prints this and no JSON at all, because tracing
        // initialises after the config is parsed.
        assert_eq!(
            parse_listening(r#"Error: PdfEngine(InvalidWorker { path: "/x" })"#),
            None
        );
        assert_eq!(parse_listening(""), None);
        // The right message with nothing to read the port from.
        assert_eq!(
            parse_listening(r#"{"fields":{"message":"conversion service listening"}}"#),
            None
        );
    }

    #[test]
    fn restarts_back_off_to_thirty_seconds_and_stay_there() {
        let schedule: Vec<u64> = (0..7).map(|n| restart_delay(n).as_secs()).collect();

        assert_eq!(schedule, [1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn the_token_fits_the_window_the_service_admits() {
        let token = mint_token();

        assert_eq!(token.len(), 64);
        assert!((32..=512).contains(&token.len()));
        assert!(token.bytes().all(|byte| byte.is_ascii_graphic()));
        assert_ne!(token, mint_token());
    }

    #[test]
    fn the_status_holds_every_slot_at_every_state() {
        let starting = BackendStatus::from(&BackendState::Starting);
        assert_eq!(starting.state, "starting");
        assert_eq!(starting.port, None);
        assert!(!starting.vision);

        let running = BackendStatus::from(&BackendState::Running {
            port: 64707,
            pid: 4242,
            vision: true,
        });
        assert_eq!(running.state, "running");
        assert_eq!(running.port, Some(64707));
        assert_eq!(running.pid, Some(4242));
        assert!(running.vision);

        let failed = BackendStatus::from(&BackendState::Failed {
            message: "no CMap tables".to_string(),
        });
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.message.as_deref(), Some("no CMap tables"));
    }

    #[test]
    fn the_status_serializes_flat_for_the_webview() {
        let value = serde_json::to_value(BackendStatus::from(&BackendState::Running {
            port: 8080,
            pid: 11,
            vision: false,
        }))
        .expect("the status should serialize");

        assert_eq!(value["state"], "running");
        assert_eq!(value["port"], 8080);
        assert_eq!(value["pid"], 11);
        assert_eq!(value["vision"], false);
        assert!(value["message"].is_null());
    }

    #[test]
    fn a_failed_start_names_the_last_thing_the_service_said() {
        assert_eq!(
            join(
                "The conversion service exited before it started listening",
                Some("Error: PdfEngine(InvalidWorker)".to_string())
            ),
            "The conversion service exited before it started listening: Error: PdfEngine(InvalidWorker)"
        );
        assert_eq!(join("It never started", None), "It never started");
    }

    #[test]
    fn the_token_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("converter.token");
        let token = mint_token();
        write_token_file(&path, &token).expect("the token file should be written");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).expect("token"), token);
    }
}
