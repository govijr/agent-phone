//! A background process per device that keeps the simulator connection (and the private
//! frameworks) loaded between commands. The CLI starts one on first use and sends it each command
//! over a Unix socket; it exits after sitting idle, or when the binary it came from changes.

use crate::{execute, forget_phones, Cli, Output};
use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Default)]
struct Request {
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    stamp: String,
    /// `ping` or `stop` instead of a command.
    #[serde(default)]
    control: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Reply {
    #[serde(default)]
    output: Option<Output>,
    /// The daemon came from a different build and is exiting; start a new one.
    #[serde(default)]
    stale: bool,
    #[serde(default)]
    info: Option<Info>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Info {
    pub key: String,
    pub pid: u32,
    pub served: u64,
    pub idle_secs: u64,
}

fn dir() -> Result<PathBuf> {
    let dir = crate::snapshot::home()?.join("daemon");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// One daemon per `--device` value; the default device has its own.
fn key(device: Option<&str>) -> String {
    let raw = device.unwrap_or("default");
    let clean: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(60).collect();
    if clean.is_empty() {
        "default".into()
    } else {
        clean
    }
}

/// Sockets live in a short folder under /tmp, since socket paths are limited to about 100
/// bytes. The name includes a hash of the state folder so separate AGENT_PHONE_HOMEs don't mix.
fn socket(key: &str) -> Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let home = crate::snapshot::home()?;
    std::fs::create_dir_all(&home)?;
    let uid = std::fs::metadata(&home)?.uid();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    home.hash(&mut hasher);
    let dir = PathBuf::from(format!("/tmp/agent-phone-{uid}"));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir.join(format!("{:08x}-{key}.sock", hasher.finish() as u32)))
}

fn socket_prefix() -> Option<String> {
    let path = socket("x").ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    Some(name.trim_end_matches("x.sock").to_string())
}

/// Identifies this build, so a rebuilt binary doesn't keep talking to an old daemon.
fn stamp() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let modified = std::fs::metadata(&exe)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}:{modified}:{}", exe.display(), env!("CARGO_PKG_VERSION"))
}

fn idle_limit() -> Duration {
    let secs = std::env::var("AGENT_PHONE_DAEMON_IDLE").ok().and_then(|s| s.parse().ok()).unwrap_or(1800);
    Duration::from_secs(secs)
}

fn ask(path: &PathBuf, request: &Request) -> Result<Reply> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(serde_json::to_string(request)?.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).context("the daemon sent a reply agent-phone can't read")
}

fn spawn(key: &str) -> Result<std::process::Child> {
    let exe = std::env::current_exe()?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir()?.join(format!("{key}.log")))?;
    use std::os::unix::process::CommandExt;
    let child = std::process::Command::new(exe)
        .args(["__daemon", key])
        .env_remove("AGENT_PHONE_DEVICE")
        .env_remove("AGENT_PHONE_DIALOGS")
        .env_remove("AGENT_PHONE_NO_DAEMON")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()
        .context("could not start the agent-phone daemon")?;
    Ok(child)
}

/// Sends a command to the device's daemon, starting one if needed. An error means the caller
/// should run the command itself.
pub fn forward(args: &[String], device: Option<&str>) -> Result<Output> {
    let key = key(device);
    let path = socket(&key)?;
    let request = Request {
        args: args.to_vec(),
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        device: device.map(String::from),
        stamp: stamp(),
        control: None,
    };
    for attempt in 0..2 {
        let reply = match ask(&path, &request) {
            Ok(reply) => reply,
            Err(_) => {
                let mut child = spawn(&key)?;
                let started = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(20));
                    if let Ok(reply) = ask(&path, &request) {
                        break reply;
                    }
                    if child.try_wait()?.is_some() && ask(&path, &request).is_err() {
                        bail!("the daemon exited; see {}", dir()?.join(format!("{key}.log")).display());
                    }
                    if started.elapsed() > Duration::from_secs(5) {
                        bail!("the daemon did not start");
                    }
                }
            }
        };
        if reply.stale && attempt == 0 {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        return reply.output.context("the daemon sent no output");
    }
    bail!("the daemon kept reporting a different build")
}

fn sockets() -> Vec<PathBuf> {
    let (Ok(path), Some(prefix)) = (socket("x"), socket_prefix()) else { return Vec::new() };
    let Some(Ok(entries)) = path.parent().map(std::fs::read_dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)) && p.extension().is_some_and(|x| x == "sock"))
        .collect()
}

pub fn status() -> Vec<Info> {
    let ping = Request { control: Some("ping".into()), ..Default::default() };
    sockets().iter().filter_map(|p| ask(p, &ping).ok()?.info).collect()
}

/// Stops every daemon and returns how many there were.
pub fn stop_all() -> usize {
    let stop = Request { control: Some("stop".into()), ..Default::default() };
    let mut stopped = 0;
    for path in sockets() {
        match ask(&path, &stop) {
            Ok(_) => stopped += 1,
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    stopped
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The daemon itself: answers one connection at a time on the main thread, since the simulator
/// connection isn't shared across threads.
pub fn serve(key: &str) -> Result<()> {
    let path = socket(key)?;
    if UnixStream::connect(&path).is_ok() {
        return Ok(()); // another daemon won the race
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).context("could not open the daemon socket")?;
    let my_stamp = stamp();
    let last = Arc::new(AtomicU64::new(now_secs()));
    {
        let (last, path, limit) = (last.clone(), path.clone(), idle_limit().as_secs());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(5));
            if now_secs().saturating_sub(last.load(Ordering::Relaxed)) > limit {
                let _ = std::fs::remove_file(&path);
                std::process::exit(0);
            }
        });
    }
    let mut served = 0u64;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Request>(&line) else { continue };
        let idle_secs = now_secs().saturating_sub(last.load(Ordering::Relaxed));
        last.store(now_secs(), Ordering::Relaxed);
        let mut stop = false;
        let reply = match request.control.as_deref() {
            Some("ping") => Reply { output: None, stale: false, info: Some(Info { key: key.into(), pid: std::process::id(), served, idle_secs }) },
            Some(_) => {
                stop = true;
                Reply { output: None, stale: false, info: None }
            }
            None if request.stamp != my_stamp => {
                stop = true;
                Reply { output: None, stale: true, info: None }
            }
            None => {
                served += 1;
                Reply { output: Some(run_request(&request)), stale: false, info: None }
            }
        };
        if stop {
            // Free the socket before answering, so the caller can start a fresh daemon at once.
            let _ = std::fs::remove_file(&path);
        }
        let mut writer = &stream;
        let _ = writer.write_all(format!("{}\n", serde_json::to_string(&reply).unwrap_or_default()).as_bytes());
        if stop {
            std::process::exit(0);
        }
    }
    Ok(())
}

fn run_request(request: &Request) -> Output {
    if std::env::set_current_dir(&request.cwd).is_err() {
        return Output { stdout: String::new(), stderr: format!("error: the folder {} is gone\n", request.cwd), code: 1 };
    }
    match Cli::try_parse_from(&request.args) {
        Ok(mut cli) => {
            if cli.device.is_none() {
                cli.device = request.device.clone();
            }
            let out = execute(cli);
            if out.code != 0 {
                forget_phones();
            }
            out
        }
        Err(e) => Output { stdout: String::new(), stderr: e.render().to_string(), code: e.exit_code() },
    }
}
