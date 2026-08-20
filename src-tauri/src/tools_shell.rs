//! Shell capability. Process execution lives here, in the native layer, never
//! in the webview. Output is streamed to the UI line by line while the process
//! runs, and returned to the model as a structured result.

use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const MAX_CAPTURE: usize = 40_000;

#[derive(Serialize, Clone)]
pub struct ShellOutputEvent {
    pub call_id: String,
    pub stream: String,
    pub line: String,
}

/// Where live output goes. Abstracted so the executor can be tested without a
/// running Tauri app.
pub type OutputSink = Arc<dyn Fn(&'static str, String) + Send + Sync>;

/// Process-group leaders of commands currently running, by tool call id, so the
/// user's Stop can actually reach a long render.
pub type ProcessRegistry = Arc<Mutex<HashMap<String, u32>>>;

pub async fn run(
    app: &AppHandle,
    ws: &Workspace,
    args: &Value,
    call_id: &str,
    timeout_secs: u64,
    registry: ProcessRegistry,
) -> Result<Value, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing required argument 'command'")?
        .to_string();

    let handle = app.clone();
    let id = call_id.to_string();
    let emit: OutputSink = Arc::new(move |stream: &'static str, line: String| {
        let _ = handle.emit(
            "agent://shell-output",
            ShellOutputEvent {
                call_id: id.clone(),
                stream: stream.to_string(),
                line,
            },
        );
    });

    run_core(ws, &command, timeout_secs, emit, registry, call_id).await
}

pub async fn run_core(
    ws: &Workspace,
    command: &str,
    timeout_secs: u64,
    emit: OutputSink,
    registry: ProcessRegistry,
    call_id: &str,
) -> Result<Value, String> {
    let command = command.to_string();
    let started = std::time::Instant::now();
    let mut builder = Command::new("sh");
    builder
        .arg("-c")
        .arg(&command)
        .current_dir(&ws.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Lead a new process group so a timeout can reach everything the command
    // started, not just the shell. Killing only the shell leaves an ffmpeg it
    // spawned running, still holding the output pipes.
    #[cfg(unix)]
    builder.process_group(0);
    let mut child = builder
        .spawn()
        .map_err(|e| format!("failed to start shell: {}", e))?;
    let pid = child.id();
    if let (Some(pid), Ok(mut map)) = (pid, registry.lock()) {
        map.insert(call_id.to_string(), pid);
    }
    let _guard = Unregister(registry.clone(), call_id.to_string());

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let out_buf = Arc::new(Mutex::new(String::new()));
    let err_buf = Arc::new(Mutex::new(String::new()));

    let out_task = pump(stdout, out_buf.clone(), emit.clone(), "stdout");
    let err_task = pump(stderr, err_buf.clone(), emit.clone(), "stderr");

    let wait = child.wait();
    let status = match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), wait).await
    {
        Ok(res) => res.map_err(|e| format!("shell failed: {}", e))?,
        Err(_) => {
            // Ask the whole group to stop, give it a moment to close files
            // cleanly, then insist.
            signal_group(pid, Signal::Term);
            if tokio::time::timeout(std::time::Duration::from_millis(1500), child.wait())
                .await
                .is_err()
            {
                signal_group(pid, Signal::Kill);
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            // Bounded, so a pipe held open by a stray process can never hang
            // the agent.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), out_task).await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), err_task).await;
            let duration_ms = started.elapsed().as_millis() as u64;
            return Ok(serde_json::json!({
                "command": command,
                "exit_code": Value::Null,
                "timed_out": true,
                "duration_ms": duration_ms,
                "stdout": take(&out_buf),
                "stderr": take(&err_buf),
                "error": format!("command exceeded the {}s timeout and was terminated", timeout_secs),
            }));
        }
    };
    let _ = out_task.await;
    let _ = err_task.await;

    Ok(serde_json::json!({
        "command": command,
        "exit_code": status.code(),
        "timed_out": false,
        "duration_ms": started.elapsed().as_millis() as u64,
        "stdout": take(&out_buf),
        "stderr": take(&err_buf),
    }))
}

/// Drops the process out of the registry however the command ends.
struct Unregister(ProcessRegistry, String);

impl Drop for Unregister {
    fn drop(&mut self) {
        if let Ok(mut map) = self.0.lock() {
            map.remove(&self.1);
        }
    }
}

pub enum Signal {
    Term,
    Kill,
}

/// Stop a command the user asked to abort. SIGTERM first so ffmpeg can finalise
/// the file it is writing, then SIGKILL if it ignores that.
pub fn cancel(registry: &ProcessRegistry, call_id: &str) -> bool {
    let pid = registry
        .lock()
        .ok()
        .and_then(|map| map.get(call_id).copied());
    let Some(pid) = pid else { return false };
    signal_group(Some(pid), Signal::Term);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        signal_group(Some(pid), Signal::Kill);
    });
    true
}

#[cfg(unix)]
pub fn signal_group(pid: Option<u32>, signal: Signal) {
    let Some(pid) = pid else { return };
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // Safe: killpg with a pid we spawned as a group leader.
    unsafe {
        libc::killpg(pid as libc::pid_t, sig);
    }
}

#[cfg(not(unix))]
pub fn signal_group(_pid: Option<u32>, _signal: Signal) {}

fn take(buf: &Arc<Mutex<String>>) -> String {
    let s = buf.lock().map(|b| b.clone()).unwrap_or_default();
    if s.len() > MAX_CAPTURE {
        let tail = &s[s.len() - MAX_CAPTURE / 2..];
        let head = &s[..MAX_CAPTURE / 2];
        format!(
            "{}\n… [{} characters omitted] …\n{}",
            head,
            s.len() - MAX_CAPTURE,
            tail
        )
    } else {
        s
    }
}

fn pump<R>(
    reader: R,
    buf: Arc<Mutex<String>>,
    emit: OutputSink,
    stream: &'static str,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(mut b) = buf.lock() {
                b.push_str(&line);
                b.push('\n');
            }
            emit(stream, line);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("eplug-shell-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Workspace::open(dir.to_str().unwrap()).unwrap()
    }

    fn sink() -> OutputSink {
        Arc::new(|_, _| {})
    }

    fn registry() -> ProcessRegistry {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[tokio::test]
    async fn captures_stdout_stderr_and_exit_code() {
        let ws = workspace("basic");
        let r = run_core(&ws, "echo out; echo err >&2; exit 3", 30, sink(), registry(), "test")
            .await
            .unwrap();
        assert_eq!(r["exit_code"], 3);
        assert_eq!(r["stdout"].as_str().unwrap().trim(), "out");
        assert_eq!(r["stderr"].as_str().unwrap().trim(), "err");
        assert_eq!(r["timed_out"], false);
        assert!(r["duration_ms"].as_u64().is_some());
    }

    #[tokio::test]
    async fn runs_in_the_workspace_directory() {
        let ws = workspace("cwd");
        std::fs::write(ws.root.join("marker.txt"), "x").unwrap();
        let r = run_core(&ws, "ls", 30, sink(), registry(), "test").await.unwrap();
        assert!(r["stdout"].as_str().unwrap().contains("marker.txt"));
        let pwd = run_core(&ws, "pwd", 30, sink(), registry(), "test").await.unwrap();
        assert_eq!(
            pwd["stdout"].as_str().unwrap().trim(),
            ws.root.to_str().unwrap()
        );
    }

    #[tokio::test]
    async fn a_hung_command_is_terminated_and_reported() {
        let ws = workspace("timeout");
        // `sh -c` may fork rather than exec, so the timeout has to reach the
        // grandchild too.
        let r = run_core(&ws, "sleep 30 | cat", 1, sink(), registry(), "test").await.unwrap();
        assert_eq!(r["timed_out"], true);
        assert!(r["error"].as_str().unwrap().contains("timeout"));
        assert!(r["duration_ms"].as_u64().unwrap() < 5000);
    }

    #[tokio::test]
    async fn output_is_streamed_line_by_line_while_running() {
        let ws = workspace("stream");
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = seen.clone();
        let emit: OutputSink = Arc::new(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        run_core(&ws, "for i in 1 2 3 4 5; do echo line$i; done", 30, emit, registry(), "test")
            .await
            .unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn stop_terminates_a_running_command_and_its_children() {
        let ws = workspace("cancel");
        let reg = registry();
        let reg2 = reg.clone();
        let started = std::time::Instant::now();
        let task = tokio::spawn(async move {
            run_core(&ws, "sleep 60 | cat", 120, sink(), reg2, "call-1")
                .await
                .unwrap()
        });
        // Wait for the command to register itself, then stop it.
        for _ in 0..100 {
            if reg.lock().unwrap().contains_key("call-1") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(cancel(&reg, "call-1"), "command should be registered");
        let r = task.await.unwrap();
        assert!(started.elapsed().as_secs() < 10, "should not wait out the sleep");
        assert_ne!(r["exit_code"], 0);
        assert!(
            reg.lock().unwrap().is_empty(),
            "registry should not leak entries"
        );
    }

    #[tokio::test]
    async fn cancelling_an_unknown_call_is_harmless() {
        assert!(!cancel(&registry(), "nope"));
    }

    #[tokio::test]
    async fn a_failing_program_returns_a_result_not_an_error() {
        let ws = workspace("failure");
        // The model must see the failure text so it can diagnose and retry.
        let r = run_core(&ws, "ls /definitely/not/here", 30, sink(), registry(), "test")
            .await
            .unwrap();
        assert_ne!(r["exit_code"], 0);
        assert!(!r["stderr"].as_str().unwrap().is_empty());
    }
}
