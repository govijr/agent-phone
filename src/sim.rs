//! Simulator-wide state and tooling through `xcrun simctl`: running apps, location, appearance,
//! permissions, the status bar, push, media, the pasteboard, app data, logs and recording.

use crate::device::{self, simctl};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
pub struct Running {
    pub pid: i64,
    pub bundle_id: String,
}

/// Apps with a live process, from launchd inside the simulator.
pub fn running(udid: &str) -> Result<Vec<Running>> {
    let out = simctl(&["spawn", udid, "launchctl", "list"])?;
    let mut apps: Vec<Running> = out
        .lines()
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let pid = cols.next()?.trim().parse().ok()?;
            let label = cols.nth(1)?;
            let bundle = label.strip_prefix("UIKitApplication:")?.split('[').next()?;
            Some(Running { pid, bundle_id: bundle.to_string() })
        })
        .collect();
    apps.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
    Ok(apps)
}

pub fn springboard_pid(udid: &str) -> Result<Option<i64>> {
    let out = simctl(&["spawn", udid, "launchctl", "list"])?;
    Ok(out.lines().find(|l| l.ends_with("\tcom.apple.SpringBoard")).and_then(|l| l.split('\t').next()?.trim().parse().ok()))
}

pub fn set_location(udid: &str, lat: f64, lon: f64) -> Result<()> {
    simctl(&["location", udid, "set", &format!("{lat},{lon}")]).map(drop)
}

pub fn clear_location(udid: &str) -> Result<()> {
    simctl(&["location", udid, "clear"]).map(drop)
}

pub fn ui(udid: &str, option: &str, value: &str) -> Result<()> {
    simctl(&["ui", udid, option, value]).map(drop)
}

pub fn privacy(udid: &str, action: &str, service: &str, bundle: Option<&str>) -> Result<()> {
    let mut args = vec!["privacy", udid, action, service];
    args.extend(bundle);
    simctl(&args).map(drop)
}

/// The status bar from Apple's marketing screenshots: 9:41, full signal and battery.
pub fn status_bar_clean(udid: &str) -> Result<()> {
    simctl(&[
        "status_bar", udid, "override", "--time", "9:41", "--dataNetwork", "wifi", "--wifiMode", "active", "--wifiBars", "3",
        "--cellularMode", "active", "--cellularBars", "4", "--batteryState", "charged", "--batteryLevel", "100",
    ])
    .map(drop)
}

pub fn status_bar_clear(udid: &str) -> Result<()> {
    simctl(&["status_bar", udid, "clear"]).map(drop)
}

fn simctl_stdin(args: &[&str], input: &[u8]) -> Result<String> {
    use std::io::Write;
    let mut child = Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run xcrun simctl")?;
    child.stdin.take().context("no stdin for simctl")?.write_all(input)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("simctl {}: {}", args[0], err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn push(udid: &str, bundle: &str, payload: &serde_json::Value) -> Result<()> {
    simctl_stdin(&["push", udid, bundle, "-"], payload.to_string().as_bytes()).map(drop)
}

pub fn add_media(udid: &str, paths: &[String]) -> Result<()> {
    let mut args = vec!["addmedia", udid];
    args.extend(paths.iter().map(String::as_str));
    simctl(&args).map(drop)
}

pub fn keychain_reset(udid: &str) -> Result<()> {
    simctl(&["keychain", udid, "reset"]).map(drop)
}

pub fn data_container(udid: &str, bundle: &str) -> Result<PathBuf> {
    let out = simctl(&["get_app_container", udid, bundle, "data"])?;
    let path = PathBuf::from(out.trim());
    if !path.is_dir() {
        bail!("{bundle} has no data container");
    }
    Ok(path)
}

#[derive(Serialize, Deserialize)]
pub struct StateMeta {
    pub bundle_id: String,
    pub saved_at: u64,
}

fn state_dir(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        bail!("state names are plain words, like `logged-in`");
    }
    Ok(crate::snapshot::home()?.join("state").join(name))
}

fn ditto(from: &Path, to: &Path) -> Result<()> {
    let status = Command::new("ditto").arg(from).arg(to).status().context("could not run ditto")?;
    if !status.success() {
        bail!("copying {} failed", from.display());
    }
    Ok(())
}

fn empty_dir(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Preferences are cached by cfprefsd; restart it so replaced plists are read again.
fn restart_prefs(udid: &str) {
    let _ = simctl(&["spawn", udid, "launchctl", "kill", "SIGKILL", "user/501/com.apple.cfprefsd.xpc.daemon"]);
    let _ = simctl(&["spawn", udid, "launchctl", "kill", "SIGKILL", "system/com.apple.cfprefsd.xpc.daemon"]);
}

/// Copies an app's data container (documents, databases, preferences, caches) under `name`.
/// The app is stopped first so its databases are consistent.
pub fn save_state(udid: &str, bundle: &str, name: &str) -> Result<PathBuf> {
    let container = data_container(udid, bundle)?;
    let dir = state_dir(name)?;
    let _ = device::terminate(udid, bundle);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    ditto(&container, &dir.join("data"))?;
    let meta = StateMeta { bundle_id: bundle.into(), saved_at: now() };
    std::fs::write(dir.join("meta.json"), serde_json::to_vec(&meta)?)?;
    Ok(dir)
}

/// Puts a saved data container back. The app is stopped; open it again to use the state.
pub fn load_state(udid: &str, name: &str) -> Result<String> {
    let dir = state_dir(name)?;
    let Ok(bytes) = std::fs::read(dir.join("meta.json")) else { bail!("no saved state named \"{name}\". See `agent-phone state list`") };
    let meta: StateMeta = serde_json::from_slice(&bytes)?;
    let container = data_container(udid, &meta.bundle_id)?;
    let _ = device::terminate(udid, &meta.bundle_id);
    empty_dir(&container)?;
    ditto(&dir.join("data"), &container)?;
    restart_prefs(udid);
    Ok(meta.bundle_id)
}

/// Empties an app's data container, as if freshly installed (the keychain is separate).
pub fn reset_state(udid: &str, bundle: &str) -> Result<()> {
    let container = data_container(udid, bundle)?;
    let _ = device::terminate(udid, bundle);
    empty_dir(&container)?;
    for sub in ["Documents", "Library", "Library/Caches", "Library/Preferences", "tmp"] {
        std::fs::create_dir_all(container.join(sub))?;
    }
    restart_prefs(udid);
    Ok(())
}

pub fn list_states() -> Result<Vec<(String, StateMeta)>> {
    let root = crate::snapshot::home()?.join("state");
    let Ok(entries) = std::fs::read_dir(&root) else { return Ok(Vec::new()) };
    let mut out: Vec<(String, StateMeta)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = serde_json::from_slice(&std::fs::read(e.path().join("meta.json")).ok()?).ok()?;
            Some((e.file_name().to_string_lossy().into_owned(), meta))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn delete_state(name: &str) -> Result<()> {
    let dir = state_dir(name)?;
    if !dir.exists() {
        bail!("no saved state named \"{name}\"");
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The executable name of an installed app, which is the process name in logs.
pub fn executable(udid: &str, bundle: &str) -> Result<String> {
    let apps = device::apps(udid)?;
    let app = apps.iter().find(|a| a.bundle_id == bundle).with_context(|| format!("{bundle} is not installed"))?;
    Ok(app.executable.clone().unwrap_or_else(|| app.name.clone()))
}

pub struct LogQuery {
    pub process: Option<String>,
    pub last: String,
    pub errors: bool,
    pub grep: Option<String>,
}

impl LogQuery {
    fn predicate(&self) -> Option<String> {
        let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut parts = Vec::new();
        if let Some(p) = &self.process {
            parts.push(format!("process == \"{}\"", quote(p)));
        }
        if self.errors {
            parts.push("(messageType == error OR messageType == fault)".into());
        }
        if let Some(g) = &self.grep {
            parts.push(format!("eventMessage CONTAINS[c] \"{}\"", quote(g)));
        }
        (!parts.is_empty()).then(|| parts.join(" AND "))
    }

    pub fn show(&self, udid: &str) -> Result<String> {
        let mut args = vec!["spawn".to_string(), udid.into(), "log".into(), "show".into(), "--style".into(), "compact".into(), "--last".into(), self.last.clone()];
        if self.errors {
            args.extend(["--info".into(), "--debug".into()]);
        }
        if let Some(p) = self.predicate() {
            args.extend(["--predicate".into(), p]);
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        simctl(&refs)
    }

    /// Streams new log lines to stdout until interrupted.
    pub fn follow(&self, udid: &str) -> Result<()> {
        let mut cmd = Command::new("xcrun");
        cmd.args(["simctl", "spawn", udid, "log", "stream", "--style", "compact"]);
        if let Some(p) = self.predicate() {
            cmd.args(["--predicate", &p]);
        }
        cmd.status().context("could not run log stream")?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct Crash {
    pub path: String,
    pub app: String,
    pub time: String,
    pub exception: String,
}

/// Recent crash reports from simulator apps, newest first.
pub fn crashes(app: Option<&str>, limit: usize) -> Result<Vec<Crash>> {
    let dir = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join("Library/Logs/DiagnosticReports");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ips"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut out = Vec::new();
    for (_, path) in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Some((head, body)) = text.split_once('\n') else { continue };
        let head: serde_json::Value = serde_json::from_str(head).unwrap_or_default();
        let body: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let proc_path = body.get("procPath").and_then(|v| v.as_str()).unwrap_or_default();
        if !proc_path.contains("CoreSimulator") {
            continue; // a Mac app, not a simulator app
        }
        let name = head.get("app_name").or_else(|| body.get("procName")).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let bundle = head.get("bundleID").and_then(|v| v.as_str()).unwrap_or_default();
        if let Some(app) = app {
            if !name.eq_ignore_ascii_case(app) && !bundle.eq_ignore_ascii_case(app) {
                continue;
            }
        }
        let exception = body
            .get("exception")
            .map(|e| {
                let kind = e.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                let signal = e.get("signal").and_then(|v| v.as_str()).unwrap_or("");
                format!("{kind} {signal}").trim().to_string()
            })
            .unwrap_or_default();
        out.push(Crash {
            path: path.to_string_lossy().into_owned(),
            app: name,
            time: head.get("timestamp").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            exception,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

/// A tool left running in the background (screen recording, profiling), stopped later with
/// Ctrl-C (SIGINT) so it finishes writing its file.
#[derive(Serialize, Deserialize)]
struct Job {
    pid: u32,
    path: String,
}

fn job_file(kind: &str, udid: &str) -> Result<PathBuf> {
    let dir = crate::snapshot::home()?.join("jobs");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{kind}-{udid}.json")))
}

fn alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Starts `program args` detached and waits until its output contains `ready`.
fn job_start(kind: &str, udid: &str, program: &str, args: &[&str], path: &str, ready: &str) -> Result<()> {
    let file = job_file(kind, udid)?;
    if let Ok(job) = serde_json::from_slice::<Job>(&std::fs::read(&file).unwrap_or_default()) {
        if alive(job.pid) {
            bail!("already running ({}); run `agent-phone {kind} stop` first", job.path);
        }
    }
    use std::os::unix::process::CommandExt;
    // The tool outlives this process, so its output goes to a file rather than a pipe.
    let log = file.with_extension("log");
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(log.with_extension("err"))?)
        .process_group(0)
        .spawn()
        .with_context(|| format!("could not start {program}"))?;
    let output = || format!("{}{}", std::fs::read_to_string(&log).unwrap_or_default(), std::fs::read_to_string(log.with_extension("err")).unwrap_or_default());
    let started = Instant::now();
    loop {
        if output().contains(ready) {
            break;
        }
        if let Some(status) = child.try_wait()? {
            let text = output();
            bail!("{kind} exited ({status}): {}", text.lines().rfind(|l| !l.trim().is_empty()).unwrap_or_default());
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            bail!("{kind} did not start");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::write(file, serde_json::to_vec(&Job { pid: child.id(), path: path.into() })?)?;
    Ok(())
}

/// Interrupts a background tool and waits for it to finish writing. Returns its output path.
fn job_stop(kind: &str, udid: &str, wait: Duration) -> Result<String> {
    let file = job_file(kind, udid)?;
    let Ok(bytes) = std::fs::read(&file) else { bail!("nothing is running; start with `agent-phone {kind} start`") };
    let job: Job = serde_json::from_slice(&bytes)?;
    let _ = std::fs::remove_file(&file);
    if !alive(job.pid) {
        bail!("{kind} to {} had already stopped", job.path);
    }
    Command::new("kill").args(["-INT", &job.pid.to_string()]).status()?;
    let started = Instant::now();
    while alive(job.pid) {
        if started.elapsed() > wait {
            bail!("{kind} did not finish writing {}", job.path);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(job.path)
}

pub fn record_start(udid: &str, path: &str) -> Result<()> {
    // simctl prints "Recording started" once frames are flowing.
    job_start("record", udid, "xcrun", &["simctl", "io", udid, "recordVideo", "--codec", "h264", "--force", path], path, "Recording started")
}

pub fn record_stop(udid: &str) -> Result<String> {
    job_stop("record", udid, Duration::from_secs(15))
}

/// Instruments templates by short name.
pub fn template(name: &str) -> &str {
    match name.to_lowercase().as_str() {
        "time" | "cpu" | "time-profiler" => "Time Profiler",
        "allocations" | "memory" => "Allocations",
        "leaks" => "Leaks",
        "hitches" | "animation" => "Animation Hitches",
        "swiftui" => "SwiftUI",
        "network" => "Network",
        "energy" | "power" => "Power Profiler",
        _ => name,
    }
}

pub fn profile_start(udid: &str, process: &str, template_name: &str, path: &str) -> Result<()> {
    let _ = std::fs::remove_dir_all(path);
    job_start(
        "profile",
        udid,
        "xcrun",
        &["xctrace", "record", "--template", template_name, "--device", udid, "--attach", process, "--output", path],
        path,
        "Ctrl-C to stop",
    )
}

pub fn profile_stop(udid: &str) -> Result<String> {
    job_stop("profile", udid, Duration::from_secs(120))
}

/// CPU time (seconds) and resident memory (bytes) of a process. Simulator apps are Mac processes.
pub fn usage(pid: i64) -> Result<(f64, u64)> {
    let out = Command::new("ps").args(["-p", &pid.to_string(), "-o", "time=,rss="]).output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut cols = text.split_whitespace();
    let (Some(time), Some(rss)) = (cols.next(), cols.next()) else { bail!("process {pid} is gone") };
    // [[hh:]mm:]ss.ss
    let secs = time.split(':').fold(0.0, |acc, part| acc * 60.0 + part.parse::<f64>().unwrap_or(0.0));
    Ok((secs, rss.parse::<u64>().unwrap_or(0) * 1024))
}
