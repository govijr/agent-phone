//! A real iPhone through macOS iPhone Mirroring, via `native/mirror.m`.
//!
//! The mirroring window is a video stream, so the screen is read with text recognition and
//! input is sent as Mac mouse and keyboard events while the window is frontmost. All
//! coordinates here are points relative to the window, which shows the phone screen edge to edge.

use crate::keys;
use crate::snapshot::Snapshot;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::ffi::{c_char, c_double, c_int, CStr, CString};
use std::process::Command;
use std::time::{Duration, Instant};

extern "C" {
    fn ap_mirror_status(err: *mut *mut c_char) -> *mut c_char;
    fn ap_mirror_launch(timeout_s: c_double, err: *mut *mut c_char) -> *mut c_char;
    fn ap_mirror_activate(timeout_s: c_double, err: *mut *mut c_char) -> c_int;
    fn ap_mirror_press_button(title: *const c_char, err: *mut *mut c_char) -> *mut c_char;
    fn ap_mirror_ocr(path: *const c_char, w: c_double, h: c_double, err: *mut *mut c_char) -> *mut c_char;
    fn ap_mirror_touch(
        xs: *const c_double,
        ys: *const c_double,
        n: c_int,
        hold_ms: c_int,
        step_ms: c_int,
        err: *mut *mut c_char,
    ) -> c_int;
    fn ap_mirror_scroll(x: c_double, y: c_double, dx: c_double, dy: c_double, err: *mut *mut c_char) -> c_int;
    fn ap_mirror_keys(
        codes: *const u16,
        downs: *const u8,
        flags: *const u64,
        n: c_int,
        err: *mut *mut c_char,
    ) -> c_int;
    fn ap_free(s: *mut c_char);
}

/// Whether `--device` names the mirrored iPhone rather than a simulator.
pub fn selected(device: Option<&str>) -> bool {
    matches!(
        device.map(|d| d.to_lowercase()).as_deref(),
        Some("iphone" | "mirror" | "mirroring" | "iphone-mirroring")
    )
}

#[derive(Debug, Deserialize, Clone)]
pub struct Window {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct Status {
    pub running: bool,
    pub state: String,
    pub window: Option<Window>,
    pub blocked_text: Vec<String>,
    pub buttons: Vec<String>,
    pub accessibility: bool,
    pub screen_recording: bool,
}

#[derive(Deserialize)]
struct OcrItem {
    text: String,
    confidence: f64,
    frame: [f64; 4],
}

pub fn status() -> Result<Status> {
    let json = string_call(|err| unsafe { ap_mirror_status(err) })?;
    Ok(serde_json::from_str(&json)?)
}

/// Opens iPhone Mirroring without taking focus, presses Connect if it is offered, and waits for
/// a live session.
pub fn connect(timeout: f64) -> Result<Status> {
    let started = Instant::now();
    let _ = string_call(|err| unsafe { ap_mirror_launch(timeout.min(12.0), err) })?;
    let mut pressed_connect = false;
    loop {
        let s = status()?;
        if !s.accessibility {
            bail!("{}", permission_hint(&s));
        }
        if s.state == "ready" {
            return Ok(s);
        }
        if s.state == "blocked" && !pressed_connect {
            if let Some(button) = s.buttons.iter().find(|b| {
                let b = b.to_lowercase();
                b == "connect" || b == "try again" || b == "continue"
            }) {
                let title = CString::new(button.as_str())?;
                let _ = string_call(|err| unsafe { ap_mirror_press_button(title.as_ptr(), err) });
                pressed_connect = true;
            }
        }
        if started.elapsed().as_secs_f64() > timeout {
            return Err(not_ready(&s));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn permission_hint(s: &Status) -> String {
    let app = host_app().unwrap_or_else(|| "the app you run agent-phone from".into());
    let mut missing = Vec::new();
    if !s.accessibility {
        missing.push("Accessibility (to tap and type)");
    }
    if !s.screen_recording {
        missing.push("Screen Recording (to see the phone; restart the app after granting)");
    }
    format!(
        "{app} needs {} in System Settings > Privacy & Security. Run `agent-phone --device iphone doctor` for details",
        missing.join(" and ")
    )
}

fn not_ready(s: &Status) -> anyhow::Error {
    match s.state.as_str() {
        "not-running" => anyhow!("iPhone Mirroring isn't running. Run `agent-phone --device iphone connect`"),
        "no-window" => anyhow!("iPhone Mirroring has no phone window yet. Open it and finish pairing your iPhone"),
        "unknown" => anyhow!("{}", permission_hint(s)),
        "blocked" => {
            let said = s.blocked_text.join(" / ");
            let buttons = if s.buttons.is_empty() { String::new() } else { format!(" (buttons: {})", s.buttons.join(", ")) };
            anyhow!(
                "iPhone Mirroring isn't showing the phone: \"{said}\"{buttons}. Lock the iPhone, keep it nearby, then run `agent-phone --device iphone connect`"
            )
        }
        other => anyhow!("iPhone Mirroring is {other}"),
    }
}

/// The macOS app that permission prompts are attributed to: the outermost .app above us.
pub fn host_app() -> Option<String> {
    let mut pid = std::process::id().to_string();
    let mut outermost = None;
    for _ in 0..16 {
        let out = Command::new("ps").args(["-o", "ppid=,comm=", "-p", &pid]).output().ok()?;
        let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let (ppid, comm) = line.split_once(char::is_whitespace)?;
        if let Some(end) = comm.find(".app/") {
            let app = &comm[..end];
            outermost = Some(app.rsplit('/').next().unwrap_or(app).to_string());
        }
        if ppid.trim() == "1" || ppid.trim() == "0" {
            break;
        }
        pid = ppid.trim().to_string();
    }
    outermost
}

pub struct Mirror {
    window: Window,
}

impl Mirror {
    /// The live mirroring session, or an error saying what stands in the way.
    pub fn open() -> Result<Self> {
        let s = status()?;
        if !s.accessibility || !s.screen_recording {
            bail!("{}", permission_hint(&s));
        }
        if s.state != "ready" {
            return Err(not_ready(&s));
        }
        let window = s.window.context("iPhone Mirroring has no phone window")?;
        Ok(Self { window })
    }

    pub fn size(&self) -> (f64, f64) {
        (self.window.w, self.window.h)
    }

    /// Saves a PNG of just the phone window.
    pub fn screenshot(&mut self, path: &str) -> Result<()> {
        let captured = |path: &str| std::fs::metadata(path).map(|m| m.len() > 1000).unwrap_or(false);
        let _ = std::fs::remove_file(path);
        let id = self.window.id.to_string();
        let _ = Command::new("screencapture").args(["-x", "-o", "-l", &id, path]).status();
        if captured(path) {
            return Ok(());
        }
        // A window that is not composited can't be captured by id; bring it in front and
        // capture its area instead.
        activate()?;
        if let Some(window) = status()?.window {
            self.window = window;
        }
        let w = &self.window;
        let region = format!("{},{},{},{}", w.x as i64, w.y as i64, w.w as i64, w.h as i64);
        let _ = Command::new("screencapture").args(["-x", "-R", &region, path]).status();
        if captured(path) {
            return Ok(());
        }
        bail!("could not capture the iPhone Mirroring window. Check Screen Recording with `agent-phone --device iphone doctor`")
    }

    /// The screen as recognized text. Each line of text becomes an element with a ref.
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let path = std::env::temp_dir().join("agent-phone-mirror.png");
        let path = path.to_string_lossy().to_string();
        self.screenshot(&path)?;
        let c_path = CString::new(path.as_str())?;
        let (w, h) = self.size();
        let json = string_call(|err| unsafe { ap_mirror_ocr(c_path.as_ptr(), w, h, err) })?;
        let items: Vec<OcrItem> = serde_json::from_str(&json)?;
        let items = items.into_iter().filter(|i| i.confidence >= 0.3 && !i.text.trim().is_empty());
        Ok(Snapshot::from_text(items.map(|i| (i.text, i.frame)).collect()))
    }

    pub fn touch(&self, points: &[(f64, f64)], hold_ms: u32, step_ms: u32) -> Result<()> {
        let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
        status_call(|err| unsafe {
            ap_mirror_touch(xs.as_ptr(), ys.as_ptr(), points.len() as c_int, hold_ms as c_int, step_ms as c_int, err)
        })
    }

    /// dy > 0 reveals content further down; dx > 0 further right.
    pub fn scroll(&self, at: (f64, f64), dx: f64, dy: f64) -> Result<()> {
        status_call(|err| unsafe { ap_mirror_scroll(at.0, at.1, dx, dy, err) })
    }

    pub fn keys(&self, events: &[keys::MacKey]) -> Result<()> {
        let codes: Vec<u16> = events.iter().map(|e| e.code).collect();
        let downs: Vec<u8> = events.iter().map(|e| e.down as u8).collect();
        let flags: Vec<u64> = events.iter().map(|e| e.flags).collect();
        status_call(|err| unsafe {
            ap_mirror_keys(codes.as_ptr(), downs.as_ptr(), flags.as_ptr(), events.len() as c_int, err)
        })
    }

    /// Types by pasting through the Mac clipboard, which avoids iOS autocorrect and works for any
    /// language. The text stays on the Mac clipboard afterwards.
    pub fn paste(&self, text: &str) -> Result<()> {
        use std::io::Write;
        let mut child = Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().context("could not run pbcopy")?;
        child.stdin.take().context("no stdin for pbcopy")?.write_all(text.as_bytes())?;
        child.wait()?;
        let check = Command::new("pbpaste").output()?;
        if check.stdout != text.as_bytes() {
            bail!("the Mac clipboard did not take the text; nothing was pasted");
        }
        std::thread::sleep(Duration::from_millis(150));
        self.keys(&keys::mac_chord(&["cmd"], 'v').context("no key for v")?)
    }

    /// Types with real key presses. iOS autocorrect may change words.
    pub fn type_keys(&self, text: &str) -> Result<()> {
        let events = keys::mac_text(text).context("that text has characters a US keyboard can't type; leave out --keystrokes to paste it")?;
        self.keys(&events)
    }

    /// Opens an app by name through Spotlight (Cmd+3), since bundle IDs mean nothing here.
    pub fn open_app(&self, name: &str) -> Result<()> {
        self.keys(&keys::mac_chord(&["cmd"], '3').context("no key for 3")?)?;
        std::thread::sleep(Duration::from_millis(900));
        self.type_keys(name)?;
        std::thread::sleep(Duration::from_millis(1200));
        self.keys(&keys::mac_named("enter").context("no enter key")?)
    }
}

pub fn activate() -> Result<()> {
    status_call(|err| unsafe { ap_mirror_activate(2.5, err) })
}

fn string_call(call: impl FnOnce(*mut *mut c_char) -> *mut c_char) -> Result<String> {
    let mut err: *mut c_char = std::ptr::null_mut();
    let out = call(&mut err);
    if out.is_null() {
        return Err(take_error(err));
    }
    let text = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
    unsafe { ap_free(out) };
    Ok(text)
}

fn status_call(call: impl FnOnce(*mut *mut c_char) -> c_int) -> Result<()> {
    let mut err: *mut c_char = std::ptr::null_mut();
    if call(&mut err) == 0 {
        Ok(())
    } else {
        Err(take_error(err))
    }
}

fn take_error(err: *mut c_char) -> anyhow::Error {
    if err.is_null() {
        return anyhow!("iPhone Mirroring call failed");
    }
    let message = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
    unsafe { ap_free(err) };
    anyhow!(message)
}
