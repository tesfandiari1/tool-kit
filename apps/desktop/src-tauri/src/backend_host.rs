//! The conversion service as a child process the app owns: started, tokened,
//! restarted when it dies, killed on the way out.
//!
//! The port is ephemeral, because a fixed one hands the bearer token to
//! whatever squats on it. The service logs the address it got, and that one
//! stdout line is the whole handshake.
//!
//! The child quits on stdin EOF, which covers the orphan case `RunEvent::Exit`
//! cannot reach. Signals go through `/bin/kill`, because `unsafe_code =
//! "forbid"` blocks `libc::kill`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

/// Sits beside the app binary in `Contents/MacOS` and in `target/debug` alike,
/// because tauri-bundler strips the `-{target}` suffix when it copies.
const CONVERTER_BIN: &str = "tool-kit-converter";

/// The service logs this once, as JSON on stdout, with the bound address.
const LISTENING_MESSAGE: &str = "conversion service listening";

/// Cold start is milliseconds, so this is for a machine under load.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS`. The default 30 makes ⌘Q hang for
/// half a minute whenever a job is in flight.
const SHUTDOWN_GRACE_SECS: u64 = 5;

/// How long SIGTERM gets before SIGKILL. The service force-cancels its runner
/// at the end of its grace window, so a longer wait buys nothing.
const TERM_WAIT: Duration = Duration::from_secs(5);
const KILL_WAIT: Duration = Duration::from_millis(1500);

/// The cap on the `RunEvent::Exit` path, which runs inside
/// `applicationWillTerminate`, where a long wait reads as a hang.
const EXIT_BUDGET: Duration = Duration::from_secs(7);

/// The budget off the exit path, so a service finishing a job gets its grace.
pub(crate) const STOP_BUDGET: Duration = Duration::from_secs(10);

const LOG_CAP_BYTES: u64 = 2 * 1024 * 1024;

const MAX_RESTARTS: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(300);

/// How long the stderr drain gets after a failed start, so the message shown
/// is the last thing the service said.
const STDERR_FLUSH: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum BackendState {
    #[default]
    Stopped,
    Starting,
    Running {
        port: u16,
    },
    Failed {
        message: String,
    },
}

#[derive(Default)]
struct Inner {
    state: BackendState,
    /// Bumped by every stop and restart. The supervisor captures it once and
    /// quits once it stops matching, so a stop mid-launch still kills the
    /// child it never saw.
    epoch: u64,
    pid: Option<u32>,
    /// The write end of the child's stdin, parked here so a stop can close it.
    stdin: Option<ChildStdin>,
    /// One token per app launch, reused across restarts.
    token: Option<String>,
    supervising: bool,
    /// Set by the first `Running` of the launch. Recovery builds one row per
    /// ledger entry, so running it again after a restart doubles them.
    recovered: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    /// False while a supervisor owns a child. `stop` waits on this, because the
    /// supervisor is the only holder of the `Child`.
    idle: watch::Sender<bool>,
    /// Mirrors `Inner::epoch` so the supervisor can wait on a bump. The restart
    /// backoff races it, so a quit during the delay does not spend
    /// `EXIT_BUDGET` on a child that no longer exists.
    epoch_bumped: watch::Sender<u64>,
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
                epoch_bumped: watch::channel(0).0,
            }),
        }
    }

    /// Poisoning is recovered, not propagated: one panic must not wedge every
    /// later transition.
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
    /// starting. Pid and stdin move under one lock: either `stop` gets them,
    /// or the stdin comes back here.
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
        let _ = self.shared.epoch_bumped.send(inner.epoch);
        (inner.pid.take(), inner.stdin.take())
    }

    /// Wait out a restart backoff, or return the moment a stop bumps the epoch.
    /// A plain sleep reads the epoch only after it returns, so a stop during the
    /// delay waits its whole budget inside `applicationWillTerminate`.
    async fn backoff(&self, delay: Duration, epoch: u64) {
        let mut bumped = self.shared.epoch_bumped.subscribe();
        let _ = tokio::time::timeout(delay, async {
            // The current value first: a bump before the subscribe delivers
            // no change at all.
            while *bumped.borrow_and_update() == epoch {
                if bumped.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
    }

    fn finish(&self) {
        self.lock().supervising = false;
        let _ = self.shared.idle.send(true);
    }

    /// True once per launch, on the first service that comes up.
    fn claim_recovery(&self) -> bool {
        !std::mem::replace(&mut self.lock().recovered, true)
    }

    /// Move the state machine. `backend_origin` is its only reader.
    fn publish(&self, state: BackendState) {
        self.lock().state = state;
    }

    /// Mint the token once per launch: the child reads the file, the host
    /// keeps it in memory.
    fn ensure_token(&self, layout: &Layout) -> Result<(), String> {
        if self.lock().token.is_some() {
            return Ok(());
        }
        let token = mint_token();
        write_token_file(&layout.token_file, &token)?;
        self.lock().token = Some(token);
        Ok(())
    }
}

fn host(app: &AppHandle) -> Option<BackendHost> {
    app.try_state::<BackendHost>().map(|state| (*state).clone())
}

/// Start the sidecar and supervise it.
pub(crate) fn start(app: &AppHandle) {
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

/// Stop the child and wait for it, giving up after `budget`. Stdin closes
/// first, because it needs no pid. SIGTERM follows. Both go out because they
/// fail in different places: stdin does nothing to a build with the knob off,
/// the signal nothing to a child whose pid we never learned.
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
        // Mid-launch or mid-backoff, with no pid to signal. The supervisor
        // checks the epoch once the child is up, and `backoff` wakes on it.
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

/// The last chance to take the child with us. `RunEvent::Exit` is the only exit
/// event this app sees, because ⌘Q hides the window rather than destroying it.
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

/// What a caller hears while the service is coming up, where a refused
/// connection says nothing anyone can act on.
const STARTING: &str = "The conversion service is starting. Try again in a moment.";

/// Never a URL: it names the service rather than locating it, and
/// `validate_base_url` rejects it on sight.
pub(crate) const SIDECAR_ALIAS: &str = "sidecar";

/// The origin every backend request goes to, read at the moment of the request.
/// Nothing may hold on to it: the port moves on every launch and restart.
pub(crate) fn backend_origin(app: &AppHandle) -> Result<String, String> {
    let Some(host) = host(app) else {
        return Err(STARTING.to_string());
    };
    let state = host.lock().state.clone();
    match state {
        BackendState::Running { port } => Ok(origin_for(port)),
        BackendState::Failed { message } => Err(message),
        BackendState::Stopped | BackendState::Starting => Err(STARTING.to_string()),
    }
}

/// The bearer token `ensure_token` minted for this launch.
pub(crate) fn backend_token(app: &AppHandle) -> Result<String, String> {
    // Minted at the top of the launch, so none yet means "not yet".
    host(app)
        .and_then(|host| host.lock().token.clone())
        .ok_or_else(|| STARTING.to_string())
}

// ---------------------------------------------------------------- supervisor

async fn supervise(app: AppHandle, host: BackendHost) {
    let epoch = host.epoch();

    let layout = match Layout::resolve(&app) {
        Ok(layout) => layout,
        Err(message) => {
            host.publish(BackendState::Failed { message });
            host.finish();
            return;
        }
    };
    reap_orphan(&layout.runtime_file).await;
    if let Err(message) = host.ensure_token(&layout) {
        host.publish(BackendState::Failed { message });
        host.finish();
        return;
    }

    // Instants inside the window, not a running count, so a week of uptime does
    // not accumulate its way to Failed.
    let mut failures: Vec<Instant> = Vec::new();
    let mut last_message;

    loop {
        host.publish(BackendState::Starting);
        match launch(&app, &layout).await {
            Ok(running) => {
                let Running {
                    mut child,
                    stdin,
                    pid,
                    port,
                } = running;
                if let Err(stdin) = host.claim(epoch, pid, stdin) {
                    terminate(&mut child, pid, Some(stdin)).await;
                    break;
                }
                write_runtime(&layout.runtime_file, pid);
                host.publish(BackendState::Running { port });
                if host.claim_recovery() {
                    // Recovery needs a service that answers: at `setup` there
                    // is no port, and one refused connection fails a job.
                    crate::jobs::recover_in_flight(app.clone());
                }
                let status = child.wait().await;
                crate::host_log::line(&format!("converter pid {pid} exited: {status:?}"));
                host.release();
                let _ = std::fs::remove_file(&layout.runtime_file);
                last_message = "The conversion service stopped unexpectedly".to_string();
            }
            Err(message) => {
                crate::host_log::line(&format!("converter did not start: {message}"));
                last_message = message;
            }
        }

        if host.stale(epoch) {
            break;
        }
        failures.retain(|at| at.elapsed() < RESTART_WINDOW);
        failures.push(Instant::now());
        if failures.len() > MAX_RESTARTS {
            host.publish(
                BackendState::Failed {
                    message: format!("{last_message}. It has been restarted five times in five minutes, so Tool-Kit stopped trying."),
                },
            );
            host.finish();
            return;
        }

        host.publish(BackendState::Failed {
            message: last_message.clone(),
        });
        host.backoff(restart_delay(failures.len() - 1), epoch).await;
        if host.stale(epoch) {
            break;
        }
    }

    host.publish(BackendState::Stopped);
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
    /// stdin, which is how it learns the app is gone.
    stdin: ChildStdin,
    pid: u32,
    port: u16,
}

/// Spawn the service, drain both pipes, and wait for it to say where it is.
/// One function: split it and a full pipe buffer blocks the service.
async fn launch(app: &AppHandle, layout: &Layout) -> Result<Running, String> {
    let binary = converter_binary()?;
    let bcmaps = bcmaps_dir(app)?;
    let diarizer = diarizer_dir(app)?;

    let mut command = tokio::process::Command::new(&binary);
    command
        .envs(converter_env(layout, &bcmaps, &diarizer))
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

    // A closed channel means stdout hit EOF, so the process is already gone.
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
    if let Err(headline) = await_ready(port).await {
        let detail = failure_detail(stderr_drain, &last_error).await;
        terminate(&mut child, pid, Some(stdin)).await;
        return Err(join(&headline, detail));
    }

    Ok(Running {
        child,
        stdin,
        pid,
        port,
    })
}

/// The last line the service wrote to stderr, verbatim. A configuration error
/// prints one `Debug` line and no JSON, because tracing starts after the parse.
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

/// Wait for the service to answer `/health/ready`, which is unauthenticated.
/// Capabilities are the webview's own probe, not a second round trip here.
async fn await_ready(port: u16) -> Result<(), String> {
    let client = crate::conversion_service::http_client()?;
    let origin = origin_for(port);

    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        let ready = client
            .get(format!("{origin}/health/ready"))
            .timeout(PROBE_TIMEOUT)
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
    Ok(())
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

/// A converter left behind by a crash. Exactly one service may own a data root,
/// and the crate takes no process lock.
async fn reap_orphan(runtime_file: &Path) {
    let recorded = std::fs::read(runtime_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RuntimeRecord>(&bytes).ok());
    let _ = std::fs::remove_file(runtime_file);
    let Some(record) = recorded else {
        return;
    };
    // The name check, not just liveness: a pid is reused within minutes, and
    // killing its heir is worse than leaving the orphan.
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
}

fn write_runtime(path: &Path, pid: u32) {
    if let Ok(bytes) = serde_json::to_vec_pretty(&RuntimeRecord { pid }) {
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

        // The artifact store creates it. A symlink here would redirect every
        // stored artifact.
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

/// The CMap tables the PDF engine needs for CJK text. A missing directory fails
/// the start: unset, the engine falls back into the build machine's Cargo
/// registry and CJK PDFs silently lose their ToUnicode mapping.
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

/// The CoreML speaker models the audio worker loads. A missing directory fails
/// the start the same way: the worker runs offline, so an unset directory is
/// not a slow path, it is no diarization at all.
fn diarizer_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .resolve(
            "fluidaudio/speaker-diarization-coreml",
            BaseDirectory::Resource,
        )
        .map_err(|e| format!("Could not locate the bundled speaker models: {e}"))?;
    if !path.is_dir() {
        return Err(format!(
            "The bundled speaker models are missing ({}). Run `pnpm sidecars`.",
            path.display()
        ));
    }
    Ok(path)
}

/// Every variable the child gets, named here rather than inherited. The
/// environment is not cleared, because the service needs `TMPDIR`. `RUST_LOG`
/// is set, because a quieter value suppresses the listening line.
fn converter_env(layout: &Layout, bcmaps: &Path, diarizer: &Path) -> BTreeMap<String, String> {
    let text = |path: &Path| path.to_string_lossy().into_owned();
    BTreeMap::from([
        // Port 0 asks the kernel: no race, and no squatter to hand a token.
        (
            "TOOLKIT_CONVERTER_BIND_ADDR".to_string(),
            "127.0.0.1:0".to_string(),
        ),
        // The service defaults are /data and /run/secrets, which a sandboxed
        // app cannot create.
        (
            "TOOLKIT_CONVERTER_DATA_DIR".to_string(),
            text(&layout.data_dir),
        ),
        (
            "TOOLKIT_CONVERTER_TOKEN_FILE".to_string(),
            text(&layout.token_file),
        ),
        ("TOOLKIT_CONVERTER_PDF_BCMAPS_DIR".to_string(), text(bcmaps)),
        // The parent that holds `speaker-diarization/`. Unset, a present audio
        // worker refuses to boot rather than reaching Hugging Face.
        (
            "TOOLKIT_CONVERTER_AUDIO_DIARIZER_DIR".to_string(),
            text(diarizer),
        ),
        // The queue is durable and the runner serial, so depth is free.
        ("TOOLKIT_CONVERTER_MAX_JOBS".to_string(), "512".to_string()),
        // JobManager runs four at once and the service uses try_acquire, so a
        // tight ceiling turns straight into refusals.
        (
            "TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS".to_string(),
            "8".to_string(),
        ),
        // One knob covers four deadlines, Vision OCR included.
        (
            "TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS".to_string(),
            "300".to_string(),
        ),
        // Audio needs its own ceilings: an hour of WAV clears the PDF ones by
        // an order of magnitude.
        (
            "TOOLKIT_CONVERTER_AUDIO_TIMEOUT_SECS".to_string(),
            "1800".to_string(),
        ),
        // The converter's own bound. A 132-minute 1.3 GB WAV transcribed in
        // 132 s at a 1.3 GB peak, and 1 GiB refused every recording past
        // about 110 minutes of WAV.
        (
            "TOOLKIT_CONVERTER_MAX_AUDIO_UPLOAD_BYTES".to_string(),
            "4294967296".to_string(),
        ),
        // 1800s, the host's own stream timeout, moves that 4 GiB at 2.3 MB/s
        // and 1 GiB at 0.6 MB/s, a slow network share.
        (
            "TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS".to_string(),
            "1800".to_string(),
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

/// 64 hex characters from two v4 UUIDs, inside the service's 32-to-512 window.
fn mint_token() -> String {
    let mut token = Uuid::new_v4().simple().to_string();
    token.push_str(&Uuid::new_v4().simple().to_string());
    token
}

fn write_token_file(path: &Path, token: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    // The mode applies at creation, so an existing file keeps its own bits.
    // `create_new` makes the unlink safe: a path that is back by the open is a
    // planted symlink, and following it writes the token into its target.
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    file.write_all(token.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Could not write {}: {e}", path.display()))
}

// -------------------------------------------------------------------- the log

/// Both pipes append here. Reopened once the cap is hit, so a service logging
/// a line per request cannot fill a disk.
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
            Path::new(
                "/Applications/Tool-Kit.app/Contents/Resources/fluidaudio/speaker-diarization-coreml",
            ),
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
        assert_eq!(
            env["TOOLKIT_CONVERTER_AUDIO_DIARIZER_DIR"],
            "/Applications/Tool-Kit.app/Contents/Resources/fluidaudio/speaker-diarization-coreml"
        );
        assert_eq!(env["TOOLKIT_CONVERTER_MAX_JOBS"], "512");
        assert_eq!(env["TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS"], "8");
        assert_eq!(env["TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS"], "300");
        assert_eq!(env["TOOLKIT_CONVERTER_AUDIO_TIMEOUT_SECS"], "1800");
        assert_eq!(
            env["TOOLKIT_CONVERTER_MAX_AUDIO_UPLOAD_BYTES"],
            "4294967296"
        );
        assert_eq!(env["TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS"], "1800");
        assert_eq!(env["TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS"], "5");
        assert_eq!(env["TOOLKIT_CONVERTER_SHUTDOWN_ON_STDIN_EOF"], "1");
        assert_eq!(env.len(), 14);
    }

    /// A quieter inherited filter drops the listening line, and the handshake
    /// then waits 30 seconds for nothing.
    #[test]
    fn the_log_filter_is_set_rather_than_inherited() {
        let env = converter_env(
            &layout(),
            Path::new("/tmp/bcmaps"),
            Path::new("/tmp/diarizer"),
        );

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

        // A config error prints this and no JSON, because tracing starts late.
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

    /// The host sends the token the child reads, and a restart keeps it.
    #[test]
    fn the_host_keeps_the_token_it_wrote_for_the_child() {
        let dir = tempfile::tempdir().expect("temp dir");
        let layout = Layout {
            token_file: dir.path().join("converter.token"),
            ..layout()
        };
        let host = BackendHost::new();

        host.ensure_token(&layout).expect("first mint");
        let token = host.lock().token.clone().expect("token kept in memory");
        host.ensure_token(&layout).expect("restart");

        assert_eq!(host.lock().token.as_deref(), Some(token.as_str()));
        assert_eq!(
            std::fs::read_to_string(&layout.token_file).expect("token file"),
            token
        );
    }

    /// The unlink can lose the race, and a symlink back before the open must not
    /// be followed. The read-only parent loses that race on purpose.
    #[test]
    fn a_token_path_that_is_back_before_the_open_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let victim = dir.path().join("notes.md");
        std::fs::write(&victim, "keep me").expect("victim");

        let sealed = dir.path().join("sealed");
        std::fs::create_dir(&sealed).expect("sealed dir");
        let path = sealed.join("converter.token");
        std::os::unix::fs::symlink(&victim, &path).expect("planted link");
        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o500))
            .expect("seal the directory");

        let refused = write_token_file(&path, &mint_token());

        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o700))
            .expect("unseal the directory");
        assert!(refused.is_err(), "the planted link should fail the write");
        assert_eq!(std::fs::read_to_string(&victim).expect("victim"), "keep me");
    }

    /// A stop during the restart backoff has no pid, so only the supervisor
    /// waking ends its wait. A plain sleep hangs `applicationWillTerminate`.
    #[tokio::test]
    async fn a_stop_during_the_backoff_wakes_the_supervisor_at_once() {
        let host = BackendHost::new();
        let epoch = host.epoch();

        let stopper = host.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = stopper.request_stop();
        });

        let started = Instant::now();
        host.backoff(Duration::from_secs(30), epoch).await;

        assert!(host.stale(epoch));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the backoff should end with the stop, not with the delay"
        );
    }
}
