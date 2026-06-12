use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::discovery::discover_cdp_url_with_timeout;

const STARFISH_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const STARFISH_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STARFISH_DISCOVERY_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_LOG_LINES: usize = 40;

/// The headless Starfish shell exits after `onload` if the page has no pending
/// timer. The launch URL embeds a no-op `setInterval` so the process stays
/// alive until the daemon drives it (Starfish CDP ANALYSIS section 0.4).
const STARFISH_KEEPALIVE_URL: &str =
    "data:text/html,<body><script>setInterval(function(){},300)</script></body>";

pub struct StarfishProcess {
    child: Child,
    pub ws_url: String,
    _log_drainers: Vec<std::thread::JoinHandle<()>>,
}

impl StarfishProcess {
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for StarfishProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

#[derive(Default)]
pub struct StarfishLaunchOptions {
    pub executable_path: Option<String>,
    pub port: Option<u16>,
}

#[derive(Clone, Default)]
struct LaunchLogBuffer {
    stdout: Arc<Mutex<VecDeque<String>>>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl LaunchLogBuffer {
    fn push_stdout(&self, line: String) {
        push_bounded(&self.stdout, line);
    }

    fn push_stderr(&self, line: String) {
        push_bounded(&self.stderr, line);
    }

    fn snapshot_stdout(&self) -> Vec<String> {
        self.stdout
            .lock()
            .expect("stdout log buffer poisoned")
            .iter()
            .cloned()
            .collect()
    }

    fn snapshot_stderr(&self) -> Vec<String> {
        self.stderr
            .lock()
            .expect("stderr log buffer poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

fn push_bounded(buffer: &Mutex<VecDeque<String>>, line: String) {
    let mut guard = buffer.lock().expect("log buffer poisoned");
    if guard.len() >= MAX_LOG_LINES {
        guard.pop_front();
    }
    guard.push_back(line);
}

/// Locate the Starfish binary. Resolution order:
/// 1. `STARFISH_BIN` environment variable
/// 2. `Starfish` / `starfish` on `PATH`
/// 3. Common build outputs relative to the current working directory
pub fn find_starfish() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STARFISH_BIN") {
        if !p.is_empty() {
            let path = PathBuf::from(p);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(unix)]
    {
        for name in ["Starfish", "starfish"] {
            if let Ok(output) = Command::new("which").arg(name).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        for name in ["Starfish.exe", "Starfish"] {
            if let Ok(output) = Command::new("where").arg(name).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let candidates = [
            "out/headless/bin/Starfish",
            "out/x11/release/bin/Starfish",
            "out/x11/debug/bin/Starfish",
            "starfish/out/headless/bin/Starfish",
        ];
        for c in &candidates {
            let path = cwd.join(c);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

pub async fn launch_starfish(options: &StarfishLaunchOptions) -> Result<StarfishProcess, String> {
    let binary_path = match &options.executable_path {
        Some(p) => PathBuf::from(p),
        None => find_starfish().ok_or(
            "Starfish not found. Set STARFISH_BIN, pass --executable-path, or run from a directory containing out/headless/bin/Starfish.",
        )?,
    };

    let port = match options.port {
        Some(p) => p,
        None => TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| format!("Failed to find an available port for Starfish: {}", e))?,
    };

    let mut child = Command::new(&binary_path)
        .arg(STARFISH_KEEPALIVE_URL)
        .env("STARFISH_ENABLE_CDP", "1")
        .env("STARFISH_CDP_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch Starfish at {:?}: {}", binary_path, e))?;

    let (log_buffer, log_drainers) = start_log_drainers(&mut child)?;

    let ws_url = match wait_for_starfish_ready(
        &mut child,
        port,
        &log_buffer,
        STARFISH_STARTUP_TIMEOUT,
    )
    .await
    {
        Ok(url) => url,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };

    Ok(StarfishProcess {
        child,
        ws_url,
        _log_drainers: log_drainers,
    })
}

fn start_log_drainers(
    child: &mut Child,
) -> Result<(LaunchLogBuffer, Vec<std::thread::JoinHandle<()>>), String> {
    let stdout = child.stdout.take().ok_or_else(|| {
        let _ = child.kill();
        "Failed to capture Starfish stdout".to_string()
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        let _ = child.kill();
        "Failed to capture Starfish stderr".to_string()
    })?;

    let logs = LaunchLogBuffer::default();
    let stdout_logs = logs.clone();
    let stderr_logs = logs.clone();

    let stdout_handle =
        std::thread::spawn(move || drain_reader(stdout, move |line| stdout_logs.push_stdout(line)));
    let stderr_handle =
        std::thread::spawn(move || drain_reader(stderr, move |line| stderr_logs.push_stderr(line)));

    Ok((logs, vec![stdout_handle, stderr_handle]))
}

fn drain_reader<R, F>(reader: R, mut push: F)
where
    R: std::io::Read,
    F: FnMut(String),
{
    for line in BufReader::new(reader).lines() {
        match line {
            Ok(line) => push(line),
            Err(_) => break,
        }
    }
}

async fn wait_for_starfish_ready(
    child: &mut Child,
    port: u16,
    logs: &LaunchLogBuffer,
    startup_timeout: Duration,
) -> Result<String, String> {
    let deadline = std::time::Instant::now() + startup_timeout;
    let mut last_probe_error = None;

    loop {
        if let Ok(Some(status)) = child.try_wait() {
            // Give the drainer threads a brief window to flush the last log lines
            // before we snapshot them.
            tokio::time::sleep(Duration::from_millis(25)).await;
            return Err(starfish_launch_error(
                &format!(
                    "Starfish exited before CDP became ready (status: {})",
                    status
                ),
                logs,
                last_probe_error.as_deref(),
            ));
        }

        match discover_cdp_url_with_timeout("127.0.0.1", port, None, STARFISH_DISCOVERY_TIMEOUT)
            .await
        {
            Ok(ws_url) => return Ok(ws_url),
            Err(err) => last_probe_error = Some(err),
        }

        if std::time::Instant::now() >= deadline {
            return Err(starfish_launch_error(
                &format!(
                    "Timed out after {}ms waiting for Starfish CDP endpoint on port {}",
                    startup_timeout.as_millis(),
                    port
                ),
                logs,
                last_probe_error.as_deref(),
            ));
        }

        tokio::time::sleep(STARFISH_POLL_INTERVAL).await;
    }
}

fn starfish_launch_error(
    message: &str,
    logs: &LaunchLogBuffer,
    last_probe_error: Option<&str>,
) -> String {
    let stdout_lines = logs.snapshot_stdout();
    let stderr_lines = logs.snapshot_stderr();
    let mut details = Vec::new();

    if let Some(err) = last_probe_error {
        details.push(format!("Last probe error: {}", err));
    }

    if !stderr_lines.is_empty() {
        details.push(format!(
            "Starfish stderr (last {} lines):\n  {}",
            stderr_lines.len(),
            stderr_lines.join("\n  ")
        ));
    }

    if !stdout_lines.is_empty() {
        details.push(format!(
            "Starfish stdout (last {} lines):\n  {}",
            stdout_lines.len(),
            stdout_lines.join("\n  ")
        ));
    }

    if details.is_empty() {
        format!("{} (no stdout/stderr output from Starfish)", message)
    } else {
        format!("{}\n{}", message, details.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = StarfishLaunchOptions::default();
        assert!(opts.executable_path.is_none());
        assert!(opts.port.is_none());
    }

    #[test]
    fn test_find_starfish_returns_none_when_missing() {
        // Should not panic regardless of environment.
        let _ = find_starfish();
    }

    #[test]
    fn test_starfish_launch_error_no_logs() {
        let logs = LaunchLogBuffer::default();
        let msg = starfish_launch_error("Starfish exited", &logs, None);
        assert!(msg.contains("no stdout/stderr output"));
    }

    #[test]
    fn test_starfish_launch_error_with_lines() {
        let logs = LaunchLogBuffer::default();
        logs.push_stdout("stdout line".to_string());
        logs.push_stderr("stderr line".to_string());
        let msg = starfish_launch_error("Starfish exited", &logs, Some("connect failed"));
        assert!(msg.contains("stdout line"));
        assert!(msg.contains("stderr line"));
        assert!(msg.contains("Last probe error: connect failed"));
    }
}
