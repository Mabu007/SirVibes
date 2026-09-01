//! Shell capability. Process execution lives here, in the native layer, never
//! in the webview. Output is streamed to the UI while the process runs, and
//! returned to the model as a structured result.
//!
//! The lifecycle is deliberate, because everything the agent does that takes
//! real time happens through here — a HyperFrames render, an ffmpeg encode, a
//! download:
//!
//! ```text
//! start → running → process exited → streams closed → result
//!                 ↘ cancelled / timed out → process tree terminated → result
//! ```
//!
//! **The command's own exit is what completes the call.** It is not end-of-file
//! on the pipes: a command like `npx → node → hyperframes → browser worker` can
//! leave a descendant holding the inherited stdout for minutes after the
//! command itself has finished and printed its last line. Waiting for EOF there
//! means waiting for the stray descendant, which is how a finished render used
//! to leave the agent stuck with a completed artifact it never got told about.

use crate::jobs::Job;
use crate::output::{self, Accepted, Capture, Captured, Progress, Splitter};
use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_CAPTURE: usize = 40_000;
/// One read from a pipe. Big enough that a chatty renderer is not a syscall
/// storm, small enough to stay responsive.
const CHUNK: usize = 16 * 1024;
/// How often a progress reading is passed on. The bar is redrawn per frame;
/// nobody needs to be told per frame.
const PROGRESS_EVERY: Duration = Duration::from_millis(400);
/// How often live output reaches the UI. A render prints a line per frame, and
/// one IPC message plus one React render per frame is a stutter on any machine;
/// batching keeps the log complete and the window responsive.
const FLUSH_EVERY: Duration = Duration::from_millis(120);
/// Flush early when a burst is big, so a batch never grows unbounded.
const MAX_BATCH: usize = 200;
/// What a process gets between "please stop" and "stop". Termination does not
/// depend on this — SIGKILL follows regardless — it is the window in which
/// ffmpeg can finalise the file it is writing.
const GRACE: Duration = Duration::from_millis(1500);

#[derive(Serialize, Clone)]
pub struct ShellOutputEvent {
    pub call_id: String,
    pub stream: String,
    /// A batch of lines, oldest first.
    pub lines: Vec<String>,
}

/// Where a long piece of work has got to. One of these replaces the hundreds of
/// redraws that a renderer or an encoder would otherwise send.
#[derive(Serialize, Clone)]
pub struct ShellProgressEvent {
    pub call_id: String,
    pub stream: String,
    /// Ready to show: "Streaming frame — 55% · 207/375".
    pub summary: String,
    pub label: String,
    pub percent: Option<u8>,
    pub done: Option<u64>,
    pub total: Option<u64>,
    /// How many redraws this reading stands for.
    pub updates: usize,
}

/// What the runner has to say while it works. Lines are things that were said
/// once; progress is a reading that replaces itself.
pub enum Event {
    Lines(Vec<String>),
    Progress {
        progress: Progress,
        updates: usize,
    },
}

/// Where live output goes. Abstracted so the executor can be tested without a
/// running Tauri app.
pub type OutputSink = Arc<dyn Fn(&'static str, Event) + Send + Sync>;

/// Where the full, uncollapsed logs are kept. Set once, at startup; unset in
/// tests, which simply means no file is written.
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_log_dir(dir: PathBuf) {
    let _ = LOG_DIR.set(dir);
}

/// Where the full logs go. The app sets this at startup; `SIRVIBE_LOG_DIR`
/// overrides nothing but fills in when it has not been set, which is how a
/// developer gets the uncollapsed output of a render without running the app.
fn log_dir() -> Option<PathBuf> {
    LOG_DIR
        .get()
        .cloned()
        .or_else(|| std::env::var_os("SIRVIBE_LOG_DIR").map(PathBuf::from))
}

/// Which of the three things that can end a command happened first.
enum Woken {
    Exited(std::io::Result<std::process::ExitStatus>),
    Cancelled,
    TimedOut,
}

/// How a command ended. Every path through the runner produces exactly one of
/// these, and each one is a terminal state.
enum Ending {
    Exited(std::process::ExitStatus),
    Cancelled(Option<std::process::ExitStatus>),
    TimedOut,
}

pub async fn run(
    app: &AppHandle,
    ws: &Workspace,
    args: &Value,
    call_id: &str,
    timeout_secs: u64,
    job: &Job,
) -> Result<Value, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing required argument 'command'")?
        .to_string();

    let handle = app.clone();
    let id = call_id.to_string();
    let emit: OutputSink = Arc::new(move |stream: &'static str, event: Event| match event {
        Event::Lines(lines) => {
            let _ = handle.emit(
                "agent://shell-output",
                ShellOutputEvent {
                    call_id: id.clone(),
                    stream: stream.to_string(),
                    lines,
                },
            );
        }
        Event::Progress { progress, updates } => {
            let _ = handle.emit(
                "agent://shell-progress",
                ShellProgressEvent {
                    call_id: id.clone(),
                    stream: stream.to_string(),
                    summary: progress.summary(),
                    label: progress.label.clone(),
                    percent: progress.percent,
                    done: progress.done,
                    total: progress.total,
                    updates,
                },
            );
        }
    });

    run_core(ws, &command, timeout_secs, emit, job, call_id).await
}

pub async fn run_core(
    ws: &Workspace,
    command: &str,
    timeout_secs: u64,
    emit: OutputSink,
    job: &Job,
    call_id: &str,
) -> Result<Value, String> {
    let command = command.to_string();
    let started = Instant::now();

    let mut builder = Command::new("sh");
    builder
        .arg("-c")
        .arg(&command)
        .current_dir(&ws.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Lead a new process group, so stopping the command can reach everything it
    // started rather than only the shell.
    #[cfg(unix)]
    builder.process_group(0);

    let mut child = builder
        .spawn()
        .map_err(|e| format!("failed to start shell: {}", e))?;
    let pid = child.id();
    log(call_id, format!("start pid={} · {}", show(pid), first_line(&command)));

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let out = pump(stdout, emit.clone(), "stdout");
    let err = pump(stderr, emit.clone(), "stderr");

    // Three ways to leave, and only three. Whichever wins, the process is dead
    // and the streams are closed before this function returns.
    let woken = {
        let wait = child.wait();
        tokio::pin!(wait);
        tokio::select! {
            status = &mut wait => Woken::Exited(status),
            _ = job.cancelled() => Woken::Cancelled,
            _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => Woken::TimedOut,
        }
    };
    // A failure to even wait for the child is still an ending: clean up first,
    // report afterwards, so no path leaves a process tree or a reader behind.
    let mut failure: Option<String> = None;
    let ending = match woken {
        Woken::Exited(Ok(status)) => Ending::Exited(status),
        Woken::Exited(Err(e)) => {
            failure = Some(format!("shell failed: {}", e));
            Ending::Cancelled(shutdown(&mut child, pid, call_id).await)
        }
        Woken::Cancelled => {
            log(call_id, "cancel requested");
            Ending::Cancelled(shutdown(&mut child, pid, call_id).await)
        }
        Woken::TimedOut => {
            log(call_id, format!("timeout after {}s", timeout_secs));
            shutdown(&mut child, pid, call_id).await;
            Ending::TimedOut
        }
    };
    // The command is gone. Anything still in its process group is something it
    // left behind: it can no longer be reached by Stop, and while it lives it
    // holds the write end of our pipes open. Clear it before reading the
    // streams out, so closing them is a fact rather than a hope. Work that is
    // meant to outlive a command detaches itself (`setsid`, `nohup`) and is in
    // its own group by then, so this does not touch it.
    reap_group(pid);

    // Deterministic: each pump takes what the pipe still holds — data already
    // written cannot grow once the writers are gone — and then lets go, even if
    // some stray descendant is still holding the write end open.
    let out = out.finish().await;
    let err = err.finish().await;
    if let Some(why) = failure {
        log(call_id, &why);
        return Err(why);
    }

    // What the model reads is the digest: everything that was said, with runs
    // of redraws collapsed to where they got to. The uncollapsed logs go to a
    // file whenever that made a difference, so nothing is lost for debugging.
    let collapsed = out.updates + err.updates > 0;
    let out_text = output::clamp(out.digest.clone(), MAX_CAPTURE);
    let err_text = output::clamp(err.digest.clone(), MAX_CAPTURE);
    let trimmed = out_text.len() < out.digest.len() || err_text.len() < err.digest.len();
    let raw_log = match (collapsed || trimmed, log_dir()) {
        (true, Some(dir)) => write_raw_log(&dir, call_id, &command, &out, &err),
        _ => None,
    };
    let progress = progress_json(if out.updates >= err.updates { &out } else { &err });
    let out_lines = out.lines;
    let err_lines = err.lines;

    let duration_ms = started.elapsed().as_millis() as u64;
    let result = match ending {
        Ending::Exited(status) => {
            log(
                call_id,
                format!(
                    "exited code={} after {:.1}s · streams closed ({} lines)",
                    show(status.code().map(|c| c as u32)),
                    duration_ms as f64 / 1000.0,
                    out_lines + err_lines
                ),
            );
            serde_json::json!({
                "status": "completed",
                "command": command,
                "progress": progress.clone(),
                "raw_log": raw_log.clone(),
                "exit_code": status.code(),
                "timed_out": false,
                "cancelled": false,
                "duration_ms": duration_ms,
                "stdout": out_text,
                "stderr": err_text,
            })
        }
        Ending::Cancelled(status) => {
            log(call_id, format!("cancelled after {:.1}s", duration_ms as f64 / 1000.0));
            serde_json::json!({
                "status": "cancelled",
                "command": command,
                "progress": progress.clone(),
                "raw_log": raw_log.clone(),
                "exit_code": status.and_then(|s| s.code()),
                "timed_out": false,
                "cancelled": true,
                "duration_ms": duration_ms,
                "stdout": out_text,
                "stderr": err_text,
                "error": "The user stopped this command. It was terminated along with everything it \
                          started. Do not simply run it again — say what was done and ask.",
            })
        }
        Ending::TimedOut => {
            log(call_id, format!("timed out after {:.1}s", duration_ms as f64 / 1000.0));
            serde_json::json!({
                "status": "timed_out",
                "command": command,
                "progress": progress.clone(),
                "raw_log": raw_log.clone(),
                "exit_code": Value::Null,
                "timed_out": true,
                "cancelled": false,
                "duration_ms": duration_ms,
                "stdout": out_text,
                "stderr": err_text,
                "error": format!("command exceeded the {}s timeout and was terminated", timeout_secs),
            })
        }
    };
    Ok(result)
}

/// Stop a running command and everything it started, then wait for it to
/// actually be gone. Returns the child's status if it exited before the kill.
async fn shutdown(
    child: &mut tokio::process::Child,
    pid: Option<u32>,
    call_id: &str,
) -> Option<std::process::ExitStatus> {
    // Snapshot the tree *before* signalling: once the shell dies its children
    // are reparented, and a descendant that gave itself a new session cannot be
    // found from our process group at all.
    let tree = descendants(pid);
    log(
        call_id,
        format!("terminating process tree ({} descendant(s))", tree.len()),
    );

    signal_group(pid, Signal::Term);
    signal_each(&tree, Signal::Term);

    if let Ok(status) = tokio::time::timeout(GRACE, child.wait()).await {
        // The shell is gone, but a descendant may have outlived it.
        let leftovers = still_alive(&tree);
        if !leftovers.is_empty() {
            signal_each(&leftovers, Signal::Kill);
        }
        signal_group(pid, Signal::Kill);
        log(call_id, "process tree terminated");
        return status.ok();
    }

    // It ignored the polite request.
    signal_group(pid, Signal::Kill);
    signal_each(&tree, Signal::Kill);
    signal_each(&descendants(pid), Signal::Kill);
    let _ = child.start_kill();
    let status = child.wait().await.ok();
    log(call_id, "process tree killed");
    status
}

/// Clear anything the finished command left running in its own process group.
fn reap_group(pid: Option<u32>) {
    signal_group(pid, Signal::Term);
    signal_group(pid, Signal::Kill);
}

pub enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
pub fn signal_group(pid: Option<u32>, signal: Signal) {
    let Some(pid) = pid else { return };
    // Safe: killpg against a group this process created.
    unsafe {
        libc::killpg(pid as libc::pid_t, code(signal));
    }
}

#[cfg(not(unix))]
pub fn signal_group(_pid: Option<u32>, _signal: Signal) {}

#[cfg(unix)]
fn signal_each(pids: &[u32], signal: Signal) {
    let sig = code(signal);
    for pid in pids {
        // Safe: a plain kill(2) against a pid we enumerated as our descendant.
        unsafe {
            libc::kill(*pid as libc::pid_t, sig);
        }
    }
}

#[cfg(not(unix))]
fn signal_each(_pids: &[u32], _signal: Signal) {}

#[cfg(unix)]
fn code(signal: Signal) -> libc::c_int {
    match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    }
}

/// Which of these processes are still running. Used to prove a kill actually
/// landed rather than assuming it did.
#[cfg(unix)]
pub fn still_alive(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        // Signal 0 tests for existence without sending anything.
        .filter(|pid| unsafe { libc::kill(*pid as libc::pid_t, 0) } == 0)
        .collect()
}

#[cfg(not(unix))]
pub fn still_alive(_pids: &[u32]) -> Vec<u32> {
    Vec::new()
}

/// Every process descended from this one, deepest last. A single pid is not
/// enough to stop real work: `npx` runs node, node runs the renderer, the
/// renderer runs a browser, and the browser may put itself in a session of its
/// own where a process-group signal will never reach it.
pub fn descendants(pid: Option<u32>) -> Vec<u32> {
    let Some(root) = pid else { return Vec::new() };
    let mut found = Vec::new();
    let mut frontier = vec![root];
    // The tree is walked breadth-first with a hard bound, so a fork bomb or a
    // cycle in a bad /proc read cannot spin here.
    while let Some(parent) = frontier.pop() {
        if found.len() > 512 {
            break;
        }
        for child in children_of(parent) {
            if child != root && !found.contains(&child) {
                found.push(child);
                frontier.push(child);
            }
        }
    }
    found
}

#[cfg(target_os = "linux")]
fn children_of(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let tasks = match std::fs::read_dir(format!("/proc/{}/task", pid)) {
        Ok(tasks) => tasks,
        Err(_) => return out,
    };
    for task in tasks.filter_map(Result::ok) {
        let path = task.path().join("children");
        if let Ok(raw) = std::fs::read_to_string(path) {
            out.extend(raw.split_whitespace().filter_map(|p| p.parse::<u32>().ok()));
        }
    }
    out
}

/// No /proc: ask ps for the parent of every process, once per level.
#[cfg(all(unix, not(target_os = "linux")))]
fn children_of(pid: u32) -> Vec<u32> {
    let Ok(out) = std::process::Command::new("ps").args(["-eo", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let child = parts.next()?.parse::<u32>().ok()?;
            let parent = parts.next()?.parse::<u32>().ok()?;
            (parent == pid).then_some(child)
        })
        .collect()
}

#[cfg(not(unix))]
fn children_of(_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// A reader that owns one of the child's pipes.
struct Pump {
    handle: tokio::task::JoinHandle<Captured>,
    stop: Arc<tokio::sync::Notify>,
}

impl Pump {
    /// Tell the reader the command is over and wait for it to hand back what it
    /// captured. It drains whatever the pipe still holds and returns; it never
    /// waits on a writer that may not be there any more.
    async fn finish(self) -> Captured {
        // `notify_one` rather than `notify_waiters`: a very short command can
        // finish before its reader has been scheduled even once, and a signal
        // sent to nobody would be lost. `notify_one` leaves a permit behind, so
        // the reader stops the moment it first looks.
        self.stop.notify_one();
        match self.handle.await {
            Ok(captured) => captured,
            // A reader that fell over must not take the command's result with
            // it: the process still ran, and what it did is still true.
            Err(_) => Capture::default().finish(),
        }
    }
}

fn pump<R>(mut reader: R, emit: OutputSink, stream: &'static str) -> Pump
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let stop = Arc::new(tokio::sync::Notify::new());
    let stopped_on = stop.clone();

    let handle = tokio::spawn(async move {
        let mut splitter = Splitter::default();
        let mut capture = Capture::default();
        let mut chunk = vec![0u8; CHUNK];
        let mut pending: Vec<String> = Vec::new();
        let mut last_flush = Instant::now();
        let mut last_progress: Option<Instant> = None;
        let mut unsent_progress = false;

        let stopped = stopped_on.notified();
        tokio::pin!(stopped);

        loop {
            let flush_at = last_flush + FLUSH_EVERY;
            tokio::select! {
                biased;
                // `read` is cancel-safe: if another branch wins, nothing was
                // taken off the pipe.
                read = reader.read(&mut chunk) => match read {
                    // End of file, or a pipe that broke. Either way there is
                    // nothing more to read and nothing to clean up.
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        take(&chunk[..n], &mut splitter, &mut capture, &mut pending, &mut unsent_progress);
                        if pending.len() >= MAX_BATCH || last_flush.elapsed() >= FLUSH_EVERY {
                            flush(&emit, stream, &mut pending, &mut last_flush);
                        }
                        if unsent_progress && due(last_progress, PROGRESS_EVERY) {
                            send_progress(&emit, stream, &capture, &mut last_progress, &mut unsent_progress);
                        }
                    }
                },
                // Nothing new for a moment: show what is already in hand rather
                // than holding it until the next line arrives.
                _ = tokio::time::sleep_until(flush_at.into()), if !pending.is_empty() || unsent_progress => {
                    flush(&emit, stream, &mut pending, &mut last_flush);
                    if unsent_progress {
                        send_progress(&emit, stream, &capture, &mut last_progress, &mut unsent_progress);
                    }
                }
                _ = &mut stopped => {
                    drain(&mut reader, &mut splitter, &mut capture, &mut pending, &mut unsent_progress).await;
                    break;
                }
            }
        }

        if let Some(rest) = splitter.finish() {
            accept(&rest, &mut capture, &mut pending, &mut unsent_progress);
        }
        flush(&emit, stream, &mut pending, &mut last_flush);
        // Whatever the last reading was, it is worth one final event: a render
        // that finished at 100% should not be left on screen at 97%.
        if unsent_progress {
            send_progress(&emit, stream, &capture, &mut last_progress, &mut unsent_progress);
        }
        capture.finish()
    });

    Pump { handle, stop }
}

/// Take everything the pipe can give right now, and not one poll more. Data
/// already written is delivered; a writer that is still attached but silent is
/// not waited on.
async fn drain<R>(
    reader: &mut R,
    splitter: &mut Splitter,
    capture: &mut Capture,
    pending: &mut Vec<String>,
    unsent_progress: &mut bool,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use std::future::Future;
    use std::task::Poll;

    let mut chunk = vec![0u8; CHUNK];
    loop {
        let mut read = std::pin::pin!(reader.read(&mut chunk));
        let polled = std::future::poll_fn(|cx| Poll::Ready(read.as_mut().poll(cx))).await;
        match polled {
            Poll::Ready(Ok(n)) if n > 0 => {
                take(&chunk[..n], splitter, capture, pending, unsent_progress)
            }
            // Either the pipe is empty for now, or it is finished. Both mean
            // there is nothing further to collect.
            _ => break,
        }
    }
}

/// Bytes → lines → either something to show or a progress reading.
fn take(
    bytes: &[u8],
    splitter: &mut Splitter,
    capture: &mut Capture,
    pending: &mut Vec<String>,
    unsent_progress: &mut bool,
) {
    let mut lines = Vec::new();
    splitter.push(bytes, &mut lines);
    for line in lines {
        accept(&line, capture, pending, unsent_progress);
    }
}

fn accept(line: &str, capture: &mut Capture, pending: &mut Vec<String>, unsent_progress: &mut bool) {
    // Control sequences are for a terminal. This is a web view and a language
    // model; neither has a cursor to move.
    let clean = output::strip_ansi(line);
    match capture.accept(&clean) {
        Accepted::Text => pending.push(clean.into_owned()),
        Accepted::Progress => *unsent_progress = true,
    }
}

fn due(last: Option<Instant>, every: Duration) -> bool {
    last.map(|at| at.elapsed() >= every).unwrap_or(true)
}

fn send_progress(
    emit: &OutputSink,
    stream: &'static str,
    capture: &Capture,
    last: &mut Option<Instant>,
    unsent: &mut bool,
) {
    let Some(progress) = capture.latest().cloned() else {
        *unsent = false;
        return;
    };
    emit(
        stream,
        Event::Progress {
            progress,
            updates: capture.updates(),
        },
    );
    *last = Some(Instant::now());
    *unsent = false;
}

fn flush(emit: &OutputSink, stream: &'static str, pending: &mut Vec<String>, last: &mut Instant) {
    if pending.is_empty() {
        return;
    }
    emit(stream, Event::Lines(std::mem::take(pending)));
    *last = Instant::now();
}

/// The full logs, for when the digest is not enough to debug with. Written only
/// when the model's copy actually lost something.
fn write_raw_log(
    dir: &std::path::Path,
    call_id: &str,
    command: &str,
    out: &Captured,
    err: &Captured,
) -> Option<String> {
    std::fs::create_dir_all(dir).ok()?;
    prune_logs(dir);
    let path = dir.join(format!("{}.log", safe_name(call_id)));
    let body = format!(
        "$ {}\n\n--- stdout ({} lines, {} progress updates{}) ---\n{}\n--- stderr ({} lines, {} progress updates{}) ---\n{}",
        command,
        out.lines,
        out.updates,
        dropped(out.raw_dropped),
        out.raw,
        err.lines,
        err.updates,
        dropped(err.raw_dropped),
        err.raw
    );
    std::fs::write(&path, body).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// Keep the last few runs' logs and no more. These exist for looking at a
/// render that has just gone wrong, not for keeping a history, and nothing else
/// ever deletes them.
const KEEP_LOGS: usize = 40;

fn prune_logs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map(|x| x == "log").unwrap_or(false))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if logs.len() <= KEEP_LOGS {
        return;
    }
    logs.sort_by_key(|(modified, _)| *modified);
    for (_, path) in logs.iter().take(logs.len() - KEEP_LOGS) {
        let _ = std::fs::remove_file(path);
    }
}

/// Even the raw log has a ceiling; say so rather than pretending it is whole.
fn dropped(bytes: usize) -> String {
    if bytes == 0 {
        String::new()
    } else {
        format!(", {} bytes dropped from the middle", bytes)
    }
}

fn safe_name(call_id: &str) -> String {
    call_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(64)
        .collect()
}

/// What the model is told about where a long command got to.
fn progress_json(captured: &Captured) -> Value {
    match &captured.last {
        Some(progress) if captured.updates > 0 => json!({
            "summary": progress.summary(),
            "label": progress.label,
            "percent": progress.percent,
            "done": progress.done,
            "total": progress.total,
            "updates": captured.updates,
        }),
        _ => Value::Null,
    }
}

/// Lifecycle, one line per transition. Not per frame of a render: progress
/// belongs in the UI, which gets it batched.
fn log(call_id: &str, message: impl AsRef<str>) {
    eprintln!("[shell {}] {}", call_id, message.as_ref());
}

fn show(value: Option<u32>) -> String {
    value.map(|v| v.to_string()).unwrap_or_else(|| "?".into())
}

fn first_line(command: &str) -> String {
    let line = command.lines().next().unwrap_or_default();
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Jobs;

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("eplug-shell-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Workspace::open(dir.to_str().unwrap()).unwrap()
    }

    fn sink() -> OutputSink {
        Arc::new(|_, _| {})
    }

    /// What the UI would have received.
    #[derive(Default)]
    struct Seen {
        batches: usize,
        lines: Vec<String>,
        progress: Vec<String>,
        last_progress: Option<Progress>,
    }

    fn recorder() -> (OutputSink, Arc<std::sync::Mutex<Seen>>) {
        let seen = Arc::new(std::sync::Mutex::new(Seen::default()));
        let recorded = seen.clone();
        let sink: OutputSink = Arc::new(move |_, event| {
            let mut seen = recorded.lock().unwrap();
            match event {
                Event::Lines(lines) => {
                    seen.batches += 1;
                    seen.lines.extend(lines);
                }
                Event::Progress { progress, .. } => {
                    seen.progress.push(progress.summary());
                    seen.last_progress = Some(progress);
                }
            }
        });
        (sink, seen)
    }

    /// A job nobody is going to cancel, for the ordinary cases.
    fn quiet() -> (Jobs, Job) {
        let jobs = Jobs::new();
        let (job, guard) = jobs.start("test");
        std::mem::forget(guard);
        (jobs, job)
    }

    #[tokio::test]
    async fn captures_stdout_stderr_and_exit_code() {
        let ws = workspace("basic");
        let (_jobs, job) = quiet();
        let r = run_core(&ws, "echo out; echo err >&2; exit 3", 30, sink(), &job, "test")
            .await
            .unwrap();
        assert_eq!(r["exit_code"], 3);
        assert_eq!(r["status"], "completed");
        assert_eq!(r["stdout"].as_str().unwrap().trim(), "out");
        assert_eq!(r["stderr"].as_str().unwrap().trim(), "err");
        assert_eq!(r["timed_out"], false);
        assert_eq!(r["cancelled"], false);
        assert!(r["duration_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn runs_in_the_workspace_directory() {
        let ws = workspace("cwd");
        std::fs::write(ws.root.join("marker.txt"), "x").unwrap();
        let (_jobs, job) = quiet();
        let r = run_core(&ws, "ls", 30, sink(), &job, "test").await.unwrap();
        assert!(r["stdout"].as_str().unwrap().contains("marker.txt"));
        let pwd = run_core(&ws, "pwd", 30, sink(), &job, "test").await.unwrap();
        assert_eq!(
            pwd["stdout"].as_str().unwrap().trim(),
            ws.root.to_str().unwrap()
        );
    }

    #[tokio::test]
    async fn a_hung_command_is_terminated_and_reported() {
        let ws = workspace("timeout");
        let (_jobs, job) = quiet();
        // `sh -c` may fork rather than exec, so the timeout has to reach the
        // grandchild too.
        let r = run_core(&ws, "sleep 30 | cat", 1, sink(), &job, "test").await.unwrap();
        assert_eq!(r["timed_out"], true);
        assert_eq!(r["status"], "timed_out");
        assert!(r["error"].as_str().unwrap().contains("timeout"));
        assert!(r["duration_ms"].as_u64().unwrap() < 5000);
    }

    #[tokio::test]
    async fn output_is_streamed_while_the_command_runs() {
        let ws = workspace("stream");
        let (emit, seen) = recorder();
        let (_jobs, job) = quiet();
        run_core(
            &ws,
            "for i in 1 2 3 4 5; do echo line$i; sleep 0.2; done",
            30,
            emit,
            &job,
            "test",
        )
        .await
        .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.lines.len(), 5, "no line may be dropped");
        assert!(seen.batches > 1, "output must arrive while it runs");
    }

    #[tokio::test]
    async fn a_flood_of_output_reaches_the_ui_in_batches_not_one_message_per_line() {
        let ws = workspace("flood");
        let (emit, seen) = recorder();
        let (_jobs, job) = quiet();
        let r = run_core(&ws, "seq 1 4000", 30, emit, &job, "test").await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.lines.len(), 4000, "the log must stay complete");
        assert!(
            seen.batches < 400,
            "4000 lines should not be 4000 UI messages, got {}",
            seen.batches
        );
        assert!(r["stdout"].as_str().unwrap().contains("4000"));
    }

    /// The crash, reproduced through the real runner.
    ///
    /// Before the fix the capture was cut with `&text[..20_000]`, which panics
    /// the instant that byte lands inside a multi-byte character — and a wall
    /// of `█` makes that a certainty rather than a possibility. The output
    /// below is laid out so byte 20 000 is the second byte of a `█`; the test
    /// asserts that about the command's own output before it asserts anything
    /// about what the runner did with it.
    #[tokio::test]
    async fn a_unicode_bar_across_the_truncation_boundary_does_not_panic() {
        // 200 lines of alternating three-byte and two-byte characters: 80 KB of
        // output, with never three bars in a row, so none of it is taken for a
        // progress bar and collapsed away.
        let line = "█é".repeat(80);
        let printed: String = std::iter::repeat(format!("{}\n", line)).take(200).collect();
        assert!(printed.len() > MAX_CAPTURE, "long enough to be cut at all");
        assert!(
            !printed.is_char_boundary(MAX_CAPTURE / 2),
            "this test only means something if the old cut point falls mid-character"
        );

        let ws = workspace("boundary");
        let (_jobs, job) = quiet();
        let r = run_core(
            &ws,
            "python3 -c \"print(('█é'*80 + chr(10))*200, end='')\"",
            60,
            sink(),
            &job,
            "test",
        )
        .await
        .unwrap();

        assert_eq!(r["status"], "completed");
        let captured = r["stdout"].as_str().unwrap();
        // Byte for byte what the safe clamp makes of exactly that output.
        assert_eq!(captured, output::clamp(printed, MAX_CAPTURE));
        assert!(captured.contains("bytes omitted"), "it should say what it dropped");
        assert!(!captured.contains('\u{fffd}'), "no character may be cut in half");
        assert!(captured.starts_with('\u{2588}'), "the head must survive intact");
        assert!(captured.trim_end().ends_with('\u{e9}'), "and so must the tail");
    }

    #[tokio::test]
    async fn every_kind_of_character_a_program_might_print_comes_back_whole() {
        let ws = workspace("unicode");
        let (emit, seen) = recorder();
        let (_jobs, job) = quiet();
        let r = run_core(
            &ws,
            "printf 'caf\u{e9} \u{fc}n\u{ef}c\u{f6}d\u{e9}\\n\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}\\n\u{1f3ac} emoji\\n\u{2588}\u{2591}\u{2593} bars\\n'",
            30,
            emit,
            &job,
            "test",
        )
        .await
        .unwrap();
        let captured = r["stdout"].as_str().unwrap();
        for expected in ["caf\u{e9} \u{fc}n\u{ef}c\u{f6}d\u{e9}", "\u{65e5}\u{672c}\u{8a9e}", "\u{1f3ac} emoji"] {
            assert!(captured.contains(expected), "{:?} was lost from {:?}", expected, captured);
        }
        assert!(!captured.contains('\u{fffd}'));
        let seen = seen.lock().unwrap();
        assert!(seen.lines.iter().any(|l| l.contains('\u{65e5}')));
    }

    #[tokio::test]
    async fn terminal_colour_is_stripped_rather_than_shown_as_gibberish() {
        let ws = workspace("ansi");
        let (_jobs, job) = quiet();
        let r = run_core(
            &ws,
            "printf '\\033[32mgreen\\033[0m\\n\\033[2K\\033[1Grewritten\\n'",
            30,
            sink(),
            &job,
            "test",
        )
        .await
        .unwrap();
        let captured = r["stdout"].as_str().unwrap();
        assert!(captured.contains("green"), "{:?}", captured);
        assert!(captured.contains("rewritten"), "{:?}", captured);
        assert!(!captured.contains('\u{1b}'), "escape codes should not reach the model");
    }

    #[tokio::test]
    async fn output_that_is_not_text_at_all_is_survivable() {
        let ws = workspace("binary");
        let (_jobs, job) = quiet();
        let r = run_core(
            &ws,
            "head -c 4096 /dev/urandom; printf '\\nstill here\\n'",
            30,
            sink(),
            &job,
            "test",
        )
        .await
        .unwrap();
        assert_eq!(r["status"], "completed");
        assert_eq!(r["exit_code"], 0);
        assert!(
            r["stdout"].as_str().unwrap().contains("still here"),
            "reading must continue past the binary"
        );
    }

    /// What a HyperFrames render actually sends: a bar redrawn per frame.
    #[tokio::test]
    async fn a_render_reports_progress_instead_of_four_hundred_redraws() {
        let ws = workspace("progress");
        let (emit, seen) = recorder();
        let (_jobs, job) = quiet();
        let script = concat!(
            "python3 -c \"\n",
            "print('[INFO] render started')\n",
            "for f in range(1, 376):\n",
            "    filled = f * 25 // 375\n",
            "    print('  ' + '█'*filled + '░'*(25-filled) + '  %d%%  Streaming frame %d/375' % (f*100//375, f))\n",
            "print('Render complete')\n",
            "\""
        );
        let r = run_core(&ws, script, 60, emit, &job, "test").await.unwrap();

        let seen = seen.lock().unwrap();
        assert!(
            seen.lines.len() <= 4,
            "the agent should see the lines that were said, not the redraws: {:?}",
            seen.lines
        );
        assert!(seen.lines.iter().any(|l| l.contains("render started")));
        assert!(
            !seen.progress.is_empty() && seen.progress.len() < 40,
            "progress should arrive periodically, got {} updates",
            seen.progress.len()
        );

        // And the model's copy is a log it can actually read.
        let captured = r["stdout"].as_str().unwrap();
        assert_eq!(
            captured.lines().count(),
            3,
            "start, one progress statement, end: {:?}",
            captured
        );
        assert!(captured.contains("[progress]"), "{}", captured);
        assert!(captured.contains("375/375"), "{}", captured);
        assert!(captured.contains("Render complete"), "{}", captured);

        // With the structured reading beside it.
        assert_eq!(r["progress"]["done"], 375);
        assert_eq!(r["progress"]["total"], 375);
        assert_eq!(r["progress"]["percent"], 100);
        assert_eq!(r["progress"]["updates"], 375);
    }

    #[tokio::test]
    async fn a_spinner_that_only_uses_carriage_returns_still_reports_progress() {
        let ws = workspace("spinner");
        let (emit, seen) = recorder();
        let (_jobs, job) = quiet();
        // Not a newline anywhere: a line reader would hold all of this in one
        // buffer until the process exited.
        let script = concat!(
            "python3 -c \"\n",
            "import sys\n",
            "for f in range(1, 51):\n",
            "    sys.stdout.write('  ' + '█'*(f//2) + '  %d%%  frame %d/50' % (f*2, f) + chr(13))\n",
            "    sys.stdout.flush()\n",
            "\""
        );
        run_core(&ws, script, 60, emit, &job, "test").await.unwrap();
        let seen = seen.lock().unwrap();
        assert!(!seen.progress.is_empty(), "a carriage-return spinner is still progress");
        assert_eq!(seen.last_progress.as_ref().unwrap().total, Some(50));
    }

    /// The failure this runner is built around: `npx` → node → hyperframes →
    /// a browser worker that puts itself in its own session. The command exits,
    /// but the descendant still holds the stdout pipe, so waiting for
    /// end-of-file waits for the descendant instead of the command.
    #[tokio::test]
    async fn a_command_whose_descendant_outlives_it_still_returns_when_it_exits() {
        let ws = workspace("detached");
        let (_jobs, job) = quiet();
        let started = std::time::Instant::now();
        let r = run_core(&ws, "setsid sleep 30 & echo done", 120, sink(), &job, "test")
            .await
            .unwrap();
        assert_eq!(r["exit_code"], 0);
        assert_eq!(r["status"], "completed");
        assert!(
            r["stdout"].as_str().unwrap().contains("done"),
            "output written before the exit must still be captured"
        );
        assert!(
            started.elapsed().as_secs() < 10,
            "the command finished; the runner waited {}s for a stray descendant",
            started.elapsed().as_secs()
        );
    }

    #[tokio::test]
    async fn work_left_running_in_the_group_does_not_survive_the_command() {
        let ws = workspace("leftover");
        let (_jobs, job) = quiet();
        // A background child in the command's own group, still running at exit.
        let r = run_core(&ws, "sleep 30 & echo $!", 60, sink(), &job, "test")
            .await
            .unwrap();
        let stray: u32 = r["stdout"].as_str().unwrap().trim().parse().unwrap();
        // Give the kernel a moment to reap it, then confirm it is gone.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            still_alive(&[stray]).is_empty(),
            "pid {} outlived the command that started it",
            stray
        );
    }

    #[tokio::test]
    async fn stop_terminates_the_whole_process_tree_and_returns_at_once() {
        let ws = workspace("cancel");
        let jobs = Arc::new(Jobs::new());
        let running = jobs.clone();
        let ws2 = ws.clone();
        let started = std::time::Instant::now();

        let task = tokio::spawn(async move {
            let (job, _guard) = running.start("call-1");
            // A tree: the shell, a pipeline, and a descendant in its own
            // session that a process-group signal can never reach.
            run_core(
                &ws2,
                "setsid sleep 60 & sleep 60 | cat",
                120,
                sink(),
                &job,
                "call-1",
            )
            .await
            .unwrap()
        });

        for _ in 0..100 {
            if jobs.is_running("call-1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(jobs.cancel("call-1"), "the call should be registered");

        let r = tokio::time::timeout(Duration::from_secs(15), task)
            .await
            .expect("Stop must return control to the UI")
            .unwrap();
        assert_eq!(r["status"], "cancelled");
        assert_eq!(r["cancelled"], true);
        assert!(started.elapsed().as_secs() < 15, "should not wait out the sleeps");
        assert!(!jobs.is_running("call-1"), "the job must not be left registered");
    }

    #[tokio::test]
    async fn a_cancelled_call_frees_the_runner_for_the_next_one() {
        let ws = workspace("reuse");
        let jobs = Arc::new(Jobs::new());
        let running = jobs.clone();
        let ws2 = ws.clone();
        let first = tokio::spawn(async move {
            let (job, _guard) = running.start("call-1");
            run_core(&ws2, "sleep 60", 120, sink(), &job, "call-1").await.unwrap()
        });
        for _ in 0..100 {
            if jobs.is_running("call-1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        jobs.cancel("call-1");
        assert_eq!(first.await.unwrap()["status"], "cancelled");

        // The next command must behave as if nothing happened.
        let (job, _guard) = jobs.start("call-2");
        let r = run_core(&ws, "echo after", 30, sink(), &job, "call-2").await.unwrap();
        assert_eq!(r["status"], "completed");
        assert_eq!(r["stdout"].as_str().unwrap().trim(), "after");
    }

    #[tokio::test]
    async fn a_failing_program_returns_a_result_not_an_error() {
        let ws = workspace("failure");
        let (_jobs, job) = quiet();
        // The model must see the failure text so it can diagnose and retry.
        let r = run_core(&ws, "ls /definitely/not/here", 30, sink(), &job, "test")
            .await
            .unwrap();
        assert_ne!(r["exit_code"], 0);
        assert_eq!(r["status"], "completed");
        assert!(!r["stderr"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_uncollapsed_log_is_kept_when_the_model_only_gets_the_digest() {
        let ws = workspace("rawlog");
        let logs = ws.root.join("logs");
        let (_jobs, job) = quiet();
        let script = concat!(
            "python3 -c \"\n",
            "for f in range(1, 201):\n",
            "    print('  ' + '█'*(f//8) + '  %d%%  frame %d/200' % (f//2, f))\n",
            "print('done')\n",
            "\""
        );
        let (emit, _seen) = recorder();
        let r = run_core(&ws, script, 60, emit, &job, "call-42").await.unwrap();
        // No log directory is configured in tests, so nothing is written…
        assert!(r["raw_log"].is_null());

        // …but the writer itself keeps every line it was given.
        let mut out = Capture::default();
        for f in 1..=200 {
            out.accept(&format!("  {}  {}%  frame {}/200", "█".repeat(f / 8), f / 2, f));
        }
        out.accept("done");
        let out = out.finish();
        let err = Capture::default().finish();
        let path = write_raw_log(&logs, "call-42", "python3 …", &out, &err).expect("a log file");
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written.lines().filter(|l| l.contains("frame")).count(),
            200,
            "the raw log must keep every redraw"
        );
        assert!(written.contains("200 progress updates"));
        assert_eq!(
            out.digest.lines().count(),
            2,
            "while the model's copy stays short: {:?}",
            out.digest
        );

        // And the folder does not grow without limit: these are for looking at
        // the render that just went wrong, not for keeping a history.
        for n in 0..KEEP_LOGS + 10 {
            let _ = write_raw_log(&logs, &format!("filler-{}", n), "x", &out, &err);
        }
        let kept = std::fs::read_dir(&logs).unwrap().count();
        assert!(kept <= KEEP_LOGS + 1, "logs are not being pruned: {} files", kept);
    }

    #[test]
    fn the_process_tree_is_walked_not_just_the_child() {
        // This test process has no children; the walk must be honest about it
        // rather than inventing pids.
        assert!(descendants(None).is_empty());
        assert!(descendants(Some(std::process::id())).len() < 512);
    }
}
