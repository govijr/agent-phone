//! agent-phone: iPhone automation CLI for AI agents.

mod annotate;
mod daemon;
mod device;
mod keys;
mod mcp;
mod mirror;
mod native;
mod sim;
mod snapshot;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;
use snapshot::{By, Element, Query, RenderOptions, Session, Snapshot};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const SKILL: &str = include_str!("../skills/agent-phone/SKILL.md");

#[derive(Parser)]
#[command(name = "agent-phone", version, about = "iPhone automation CLI for AI agents")]
#[command(after_help = "Devices: a simulator name or UDID, or `iphone` for your real iPhone through iPhone Mirroring.\n\
Targets: @e3 (a ref from `snapshot`), \"Sign in\" (text on screen), id=login-button (accessibility id), or 120,480 (points).")]
pub struct Cli {
    /// Simulator name or UDID, or `iphone` for iPhone Mirroring. Defaults to the only booted simulator.
    #[arg(long, short = 'd', global = true, env = "AGENT_PHONE_DEVICE")]
    device: Option<String>,

    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Run in this process instead of the background daemon.
    #[arg(long, global = true, env = "AGENT_PHONE_NO_DAEMON")]
    no_daemon: bool,

    /// Answer system alerts (permission prompts) automatically before each command.
    #[arg(long, global = true, env = "AGENT_PHONE_DIALOGS")]
    dialogs: Option<DialogAnswer>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List simulators and the mirrored iPhone.
    Devices,
    /// Check that everything agent-phone needs is in place.
    Doctor,
    /// iPhone Mirroring: open it in the background and wait for the phone.
    Connect {
        #[arg(long, default_value_t = 30.0)]
        timeout: f64,
    },
    /// Boot a simulator (headless) and wait until it is ready.
    Boot {
        /// Name or UDID; defaults to --device, then the newest iPhone.
        name: Option<String>,
    },
    /// Shut the simulator down.
    Shutdown,
    /// List installed apps and their bundle IDs (simulator).
    Apps {
        /// Only apps that are running, with their process IDs.
        #[arg(long)]
        running: bool,
    },
    /// Launch an app (bundle ID on a simulator, app name on an iPhone), or open a URL.
    Open {
        target: String,
        /// Restart the app if it is already running (simulator).
        #[arg(long)]
        fresh: bool,
    },
    /// Terminate an app (simulator).
    Close { bundle_id: String },
    /// Install a .app bundle (simulator).
    Install { path: String },
    /// Print what is on screen with refs.
    Snapshot {
        /// Only interactive elements (buttons, fields, switches…).
        #[arg(short, long)]
        interactive: bool,
        /// Only print this many levels deep.
        #[arg(long)]
        depth: Option<usize>,
        /// Only print this element and what is inside it.
        #[arg(short, long)]
        scope: Option<String>,
        /// Only print what changed since the last snapshot (+ new, - gone).
        #[arg(long)]
        diff: bool,
        /// Print the raw accessibility tree as JSON (for debugging agent-phone).
        #[arg(long, hide = true)]
        raw: bool,
    },
    /// Tap an element or point.
    #[command(visible_alias = "tap")]
    Click {
        target: String,
        /// Hold for this many milliseconds (a long press).
        #[arg(long, default_value_t = 0)]
        hold: u32,
        /// Tap even if the element is disabled.
        #[arg(long)]
        force: bool,
    },
    /// Tap a text field, clear it, and type into it.
    Fill {
        target: String,
        text: String,
        /// iPhone: type with key presses instead of pasting (autocorrect may change words).
        #[arg(long)]
        keystrokes: bool,
        /// Fill even if the field is disabled.
        #[arg(long)]
        force: bool,
    },
    /// Type text into the focused field.
    Type {
        text: String,
        /// iPhone: type with key presses instead of pasting (autocorrect may change words).
        #[arg(long)]
        keystrokes: bool,
    },
    /// Press a key or button: home, lock, switcher, spotlight, enter, tab, backspace, escape, arrows.
    Press { key: String },
    /// Swipe a finger in a direction.
    Swipe {
        direction: Direction,
        /// Where to start: a target, or the middle of the screen.
        #[arg(long)]
        from: Option<String>,
        /// How far to move, in points.
        #[arg(long)]
        distance: Option<f64>,
    },
    /// Scroll content in a direction (the finger moves the opposite way).
    Scroll {
        direction: Direction,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        distance: Option<f64>,
    },
    /// Go back: the navigation bar's back button, or a swipe from the left edge.
    Back,
    /// Relaunch the frontmost app (simulator).
    Reload,
    /// Double-tap an element or point.
    Dblclick { target: String },
    /// Drag from one target to another. Add --hold 600 to pick an item up first (reordering).
    Drag {
        from: String,
        to: String,
        /// Hold at the start for this many milliseconds.
        #[arg(long, default_value_t = 0)]
        hold: u32,
        /// How long the move takes, in milliseconds.
        #[arg(long, default_value_t = 400)]
        duration: u32,
    },
    /// Turn a switch or checkbox on (does nothing if it already is).
    Check { target: String },
    /// Turn a switch or checkbox off (does nothing if it already is).
    Uncheck { target: String },
    /// System alerts such as permission prompts: show it, accept it, or dismiss it.
    Dialog {
        #[arg(default_value = "status")]
        action: DialogAction,
    },
    /// Change simulator settings: location, appearance, permission, statusbar, text-size, contrast.
    Set {
        #[command(subcommand)]
        what: SetCommand,
    },
    /// Send a push notification to an app (simulator).
    ///
    /// agent-phone push com.example.app --title "Hi" --body "New message" · push com.example.app payload.json
    Push {
        bundle_id: String,
        /// A JSON file, or JSON text, with an "aps" object.
        payload: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        badge: Option<u32>,
    },
    /// Save, restore or reset an app's data (documents, databases, preferences) (simulator).
    State {
        #[command(subcommand)]
        action: StateCommand,
    },
    /// Reset the simulator's keychain (saved passwords and tokens for every app).
    Keychain { action: KeychainAction },
    /// Add photos, videos or contacts (vCard) to the simulator.
    Media { action: MediaAction, paths: Vec<String> },
    /// Read or set the simulator's clipboard.
    Clipboard { action: ClipboardAction, text: Option<String> },
    /// Recent log lines from an app (the frontmost one by default).
    Logs {
        /// Bundle ID; defaults to the frontmost app.
        #[arg(long)]
        app: Option<String>,
        /// Every process, not just one app.
        #[arg(long)]
        all: bool,
        /// How far back: 30s, 5m, 1h.
        #[arg(long, default_value = "1m")]
        last: String,
        /// Only errors and faults.
        #[arg(long)]
        errors: bool,
        /// Only lines containing this text.
        #[arg(long)]
        grep: Option<String>,
        /// Keep printing new lines until interrupted.
        #[arg(long)]
        follow: bool,
        /// Print at most this many lines (the newest).
        #[arg(long, default_value_t = 200)]
        lines: usize,
    },
    /// Recent crash reports from simulator apps.
    Crashes {
        /// App name or bundle ID.
        #[arg(long)]
        app: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Check the screen for accessibility problems: missing labels, small tap targets, duplicates.
    Audit,
    /// Record the screen to a video: `record start [path]`, then `record stop`.
    Record {
        action: RecordAction,
        #[arg(default_value = "recording.mp4")]
        path: String,
    },
    /// Compare a screenshot with the screen now (or with another screenshot).
    Diff {
        baseline: String,
        /// The image to compare with; defaults to a new screenshot.
        current: Option<String>,
        /// Write a picture with the changed pixels in red.
        #[arg(long)]
        out: Option<String>,
        /// How much a colour channel must change (0-255) for a pixel to count.
        #[arg(long, default_value_t = 24)]
        threshold: u8,
    },
    /// Find elements by role, text, label, id, placeholder or value, and optionally act on one.
    ///
    /// find role button --name "Done" click · find id login-field fill "me@x.com" · find text Cell count
    Find {
        by: By,
        value: String,
        /// What to do with the match. Without one, the matches are listed with refs.
        action: Option<FindAction>,
        /// The text for `fill`.
        text: Option<String>,
        /// With `role`: the element's text must also contain this.
        #[arg(long)]
        name: Option<String>,
        /// Match the whole text, not part of it.
        #[arg(long)]
        exact: bool,
        /// Use the nth match (from 1) when several match.
        #[arg(long)]
        nth: Option<usize>,
        /// Act even if the element is disabled.
        #[arg(long)]
        force: bool,
    },
    /// Read one thing: text, value, label, id, role or frame of a target; count of matches;
    /// the focused element; the frontmost app; the screen size.
    Get { what: GetWhat, target: Option<String> },
    /// Check a target's state: visible, enabled, focused, selected, on. Prints true or false.
    Is { what: IsWhat, target: String },
    /// Wait for milliseconds, an element (or text with --text), or a still screen with --stable.
    Wait {
        /// Milliseconds, or a target to wait for (@e3, "Done", id=spinner).
        what: Option<String>,
        #[arg(long)]
        text: Option<String>,
        /// Wait for it to disappear instead.
        #[arg(long)]
        gone: bool,
        /// Wait until the screen stops changing.
        #[arg(long)]
        stable: bool,
        /// Give up after this many seconds.
        #[arg(long, default_value_t = 10.0)]
        timeout: f64,
    },
    /// Save a PNG screenshot.
    Screenshot {
        #[arg(default_value = "screenshot.png")]
        path: String,
        /// Draw each interactive element's ref on the picture.
        #[arg(long)]
        annotate: bool,
    },
    /// Run several commands in order, from arguments or one per line on stdin.
    ///
    /// agent-phone batch "click @e4" "wait --text Done" "snapshot -i"
    Batch {
        commands: Vec<String>,
        /// Keep going after a command fails.
        #[arg(long)]
        keep_going: bool,
    },
    /// Print the agent guide (SKILL.md) built into this binary.
    Skills {
        /// `get` (the default) prints it.
        #[arg(default_value = "get")]
        action: String,
    },
    /// Serve agent-phone as an MCP server over stdio.
    Mcp,
    /// Manage the background daemons that keep simulator connections open.
    Daemon { action: DaemonAction },
    #[command(name = "__daemon", hide = true)]
    DaemonServe { key: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn opposite(self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum FindAction {
    List,
    Click,
    Fill,
    Text,
    Count,
}

#[derive(Clone, Copy, ValueEnum)]
enum GetWhat {
    Text,
    Value,
    Label,
    Id,
    Role,
    Frame,
    Count,
    Focused,
    App,
    Size,
}

#[derive(Clone, Copy, ValueEnum)]
enum IsWhat {
    Visible,
    Enabled,
    Focused,
    Selected,
    #[value(alias = "checked")]
    On,
}

#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum DialogAnswer {
    Accept,
    Dismiss,
}

#[derive(Clone, Copy, ValueEnum, PartialEq)]
enum DialogAction {
    Status,
    Accept,
    Dismiss,
}

#[derive(Subcommand)]
enum SetCommand {
    /// Simulated GPS position: `set location 37.3349,-122.0090`, or `set location clear`.
    Location { value: String },
    /// Light or dark mode.
    Appearance { mode: Appearance },
    /// `set permission location grant --app com.example.app`. Services: all, calendar, contacts,
    /// contacts-limited, location, location-always, photos, photos-add, media-library,
    /// microphone, motion, reminders, siri.
    Permission {
        service: String,
        action: PermissionAction,
        /// Bundle ID (needed for grant and revoke).
        #[arg(long)]
        app: Option<String>,
    },
    /// `clean` (9:41, full signal and battery) or `clear`.
    Statusbar { mode: StatusBar },
    /// Dynamic Type size: extra-small … extra-extra-extra-large, accessibility-medium …
    /// accessibility-extra-extra-extra-large, increment or decrement.
    TextSize { size: String },
    /// Increase Contrast on or off.
    Contrast { mode: OnOff },
}

#[derive(Clone, Copy, ValueEnum)]
enum Appearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, ValueEnum)]
enum PermissionAction {
    Grant,
    Revoke,
    Reset,
}

#[derive(Clone, Copy, ValueEnum)]
enum StatusBar {
    Clean,
    Clear,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Subcommand)]
enum StateCommand {
    /// Copy an app's data under a name (the app is stopped first).
    Save {
        name: String,
        /// Bundle ID; defaults to the frontmost app.
        #[arg(long)]
        app: Option<String>,
    },
    /// Put saved data back (the app is stopped; open it again).
    Load { name: String },
    /// Saved states.
    List,
    /// Forget a saved state.
    Delete { name: String },
    /// Empty an app's data, as if freshly installed (the keychain is separate).
    Reset {
        #[arg(long)]
        app: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum KeychainAction {
    Reset,
}

#[derive(Clone, Copy, ValueEnum)]
enum MediaAction {
    Add,
}

#[derive(Clone, Copy, ValueEnum)]
enum ClipboardAction {
    Get,
    Set,
}

#[derive(Clone, Copy, ValueEnum)]
enum RecordAction {
    Start,
    Stop,
}

#[derive(Clone, Copy, ValueEnum)]
enum DaemonAction {
    Status,
    Stop,
}

/// What a command printed, so it can run here, in the daemon, in a batch, or for MCP.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

struct Ctx {
    json: bool,
    device_arg: Option<String>,
    dialogs: Option<DialogAnswer>,
    out: RefCell<String>,
    code: Cell<i32>,
}

enum Backend {
    Sim { udid: String, native: native::Target },
    Mirror(RefCell<mirror::Mirror>),
}

struct Phone {
    backend: Backend,
    /// SpringBoard's process, which owns the home screen and system alerts.
    springboard: OnceCell<Option<i64>>,
}

/// Buttons that say yes, and no, on iOS system alerts, best first.
const ACCEPT: &[&str] = &[
    "Allow While Using App", "Allow", "Allow Full Access", "Allow Access to All Photos", "OK", "Continue", "Open", "Yes",
    "Allow Paste", "Done", "Allow Once",
];
const DISMISS: &[&str] = &[
    "Don’t Allow", "Don't Allow", "Ask App Not to Track", "Cancel", "Not Now", "No", "Close", "Later", "Don’t Allow Paste",
    "Keep Current Selection",
];

/// How a command names an element.
enum Locator {
    Point((f64, f64)),
    Ref(String),
    Text(String),
    Query(Query, Option<usize>),
}

impl Locator {
    fn parse(target: &str) -> Self {
        if let Some(point) = parse_point(target) {
            Locator::Point(point)
        } else if let Some(reference) = target.strip_prefix('@') {
            Locator::Ref(reference.to_string())
        } else if let Some(id) = target.strip_prefix("id=") {
            Locator::Query(Query { by: By::Id, value: id.to_string(), name: None, exact: true }, None)
        } else {
            Locator::Text(target.to_string())
        }
    }
}

#[derive(Default, Clone, Copy)]
struct LocateOptions {
    /// Prefer text fields (for `fill`).
    editable: bool,
    /// Act on disabled elements too.
    force: bool,
}

const SIM_ONLY: &str = "is only available on a simulator; iPhone Mirroring can't do it";

thread_local! {
    /// Simulator connections, reused for the life of the process (the daemon, a batch, MCP).
    static PHONES: RefCell<HashMap<String, Rc<Phone>>> = RefCell::new(HashMap::new());
}

pub fn forget_phones() {
    PHONES.with(|p| p.borrow_mut().clear());
}

impl Ctx {
    fn mirror(&self) -> bool {
        mirror::selected(self.device_arg.as_deref())
    }

    fn phone(&self) -> Result<Rc<Phone>> {
        if self.mirror() {
            let m = mirror::Mirror::open()?;
            return Ok(Rc::new(Phone { backend: Backend::Mirror(RefCell::new(m)), springboard: OnceCell::new() }));
        }
        let phone = self.sim_phone()?;
        if let Some(answer) = self.dialogs {
            if let Some(note) = phone.answer_dialog(answer == DialogAnswer::Accept, false)? {
                if self.json {
                    eprintln!("{note}");
                } else {
                    self.out.borrow_mut().push_str(&format!("({note})\n"));
                }
            }
        }
        Ok(phone)
    }

    fn sim_phone(&self) -> Result<Rc<Phone>> {
        let key = self.device_arg.clone().unwrap_or_default();
        if let Some(phone) = PHONES.with(|p| p.borrow().get(&key).cloned()) {
            return Ok(phone);
        }
        let device = device::resolve(self.device_arg.as_deref())?;
        if !device.booted() {
            bail!("{} is not booted. Run `agent-phone boot \"{}\"`", device.name, device.name);
        }
        let native = native::Target::new(&device::developer_dir()?, &device.udid)?;
        let phone = Rc::new(Phone { backend: Backend::Sim { udid: device.udid, native }, springboard: OnceCell::new() });
        PHONES.with(|p| p.borrow_mut().insert(key, phone.clone()));
        Ok(phone)
    }

    /// The simulator for commands a mirrored iPhone can't do.
    fn sim(&self, what: &str) -> Result<Rc<Phone>> {
        if self.mirror() {
            bail!("`{what}` {SIM_ONLY}");
        }
        self.phone()
    }

    /// Prints `text` for people, or `data` (with ok: true unless it says otherwise) for --json.
    fn done(&self, text: impl AsRef<str>, data: serde_json::Value) {
        let mut out = self.out.borrow_mut();
        if self.json {
            let mut data = data;
            if let Some(map) = data.as_object_mut() {
                map.entry("ok").or_insert(json!(true));
            }
            out.push_str(&format!("{data}\n"));
        } else {
            out.push_str(text.as_ref());
            if !text.as_ref().ends_with('\n') {
                out.push('\n');
            }
        }
    }
}

impl Phone {
    fn udid(&self) -> &str {
        match &self.backend {
            Backend::Sim { udid, .. } => udid,
            Backend::Mirror(_) => "iphone-mirroring",
        }
    }

    fn sim(&self) -> (&str, &native::Target) {
        match &self.backend {
            Backend::Sim { udid, native } => (udid, native),
            Backend::Mirror(_) => unreachable!("simulator-only call on a mirrored iPhone"),
        }
    }

    fn is_mirror(&self) -> bool {
        matches!(self.backend, Backend::Mirror(_))
    }

    fn snapshot(&self) -> Result<Snapshot> {
        match &self.backend {
            Backend::Sim { native, .. } => Snapshot::parse(&native.snapshot(5.0)?),
            Backend::Mirror(m) => m.borrow_mut().snapshot(),
        }
    }

    fn screen(&self) -> Result<(f64, f64)> {
        match &self.backend {
            Backend::Sim { native, .. } => native.screen().map(|s| (s.width, s.height)),
            Backend::Mirror(m) => Ok(m.borrow().size()),
        }
    }

    fn touch(&self, points: &[(f64, f64)], hold_ms: u32, step_ms: u32) -> Result<()> {
        match &self.backend {
            Backend::Sim { native, .. } => native.touch(points, hold_ms, step_ms),
            Backend::Mirror(m) => m.borrow().touch(points, hold_ms, step_ms),
        }
    }

    fn screenshot(&self, path: &str) -> Result<()> {
        match &self.backend {
            Backend::Sim { udid, .. } => device::screenshot(udid, path),
            Backend::Mirror(m) => m.borrow_mut().screenshot(path),
        }
    }

    /// The element `locator` names on `snapshot`, or why there is none. Points name no element.
    fn find_in(&self, snapshot: &Snapshot, locator: &Locator, editable: bool, saved: Option<&Element>) -> Result<Option<Element>> {
        Ok(Some(match locator {
            Locator::Point(_) => return Ok(None),
            Locator::Ref(reference) => {
                let saved = saved.context("no saved element for the ref")?;
                snapshot.relocate(saved).cloned().with_context(|| {
                    format!("@{reference} ({}) is no longer on screen. Run `agent-phone snapshot` for fresh refs", saved.describe())
                })?
            }
            Locator::Text(text) => snapshot
                .find_text(text, editable)
                .cloned()
                .with_context(|| format!("nothing on screen matches \"{text}\". Run `agent-phone snapshot` to see what is there"))?,
            Locator::Query(query, nth) => {
                let mut matches = snapshot.query(query);
                if editable {
                    matches.sort_by_key(|e| !e.editable());
                }
                let index = nth.unwrap_or(1).max(1) - 1;
                match matches.get(index) {
                    Some(e) => (*e).clone(),
                    None if matches.is_empty() => {
                        bail!("nothing on screen matches {}. Run `agent-phone snapshot` to see what is there", query.describe())
                    }
                    None => bail!("only {} elements match {}", matches.len(), query.describe()),
                }
            }
        }))
    }

    fn saved_for(&self, locator: &Locator) -> Result<Option<Element>> {
        match locator {
            Locator::Ref(reference) => Ok(Some(Session::load(self.udid())?.get(reference)?.clone())),
            _ => Ok(None),
        }
    }

    /// Resolves a target to an element (if it names one) and a point to touch. An element off
    /// screen is scrolled into view first; a disabled one is refused unless `force`.
    fn locate(&self, locator: &Locator, options: LocateOptions) -> Result<(Option<Element>, (f64, f64))> {
        if let Locator::Point(point) = locator {
            return Ok((None, *point));
        }
        let saved = self.saved_for(locator)?;
        let (width, height) = self.screen()?;
        let first = self.snapshot()?;
        let found = self.find_in(&first, locator, options.editable, saved.as_ref()).map_err(|e| match self.system_alert(&first) {
            Some(_) => e.context(format!(
                "a system alert is in front (\"{}\"). Answer it with `agent-phone dialog accept` or `dialog dismiss`",
                Self::alert_text(&first)
            )),
            None => e,
        })?;
        let mut element = found.context("no element")?;
        for attempt in 0.. {
            let (x, y) = element.center();
            let off = x < 0.0 || y < 0.0 || x > width || y > height;
            if !off {
                break;
            }
            if self.is_mirror() || attempt >= 5 {
                bail!("{} is off screen; scroll to it first", element.describe());
            }
            let (direction, gap) = if y > height {
                (Direction::Down, y - height / 2.0)
            } else if y < 0.0 {
                (Direction::Up, height / 2.0 - y)
            } else if x > width {
                (Direction::Right, x - width / 2.0)
            } else {
                (Direction::Left, width / 2.0 - x)
            };
            let limit = if matches!(direction, Direction::Up | Direction::Down) { height * 0.5 } else { width * 0.6 };
            self.scroll(direction, None, Some(gap.min(limit)))?;
            std::thread::sleep(Duration::from_millis(450));
            element = self.find_in(&self.snapshot()?, locator, options.editable, saved.as_ref())?.context("no element")?;
        }
        if !element.enabled && !options.force {
            bail!("{} is disabled; pass --force to tap it anyway", element.describe());
        }
        let point = element.tap_point();
        Ok((Some(element), point))
    }

    fn type_text(&self, text: &str, keystrokes: bool) -> Result<()> {
        match &self.backend {
            Backend::Sim { udid, native } => match keys::text(text) {
                Some(events) => native.keys(&events),
                None => {
                    // Characters a US keyboard cannot type go through the pasteboard.
                    device::set_pasteboard(udid, text)?;
                    native.keys(&keys::command('v'))
                }
            },
            Backend::Mirror(m) if keystrokes => m.borrow().type_keys(text),
            Backend::Mirror(m) => m.borrow().paste(text),
        }
    }

    fn clear_field(&self) -> Result<()> {
        match &self.backend {
            Backend::Sim { native, .. } => {
                native.keys(&keys::command('a'))?;
                native.keys(&keys::backspace())
            }
            Backend::Mirror(m) => {
                let m = m.borrow();
                m.keys(&keys::mac_chord(&["cmd"], 'a').context("no key for a")?)?;
                m.keys(&keys::mac_named("backspace").context("no backspace key")?)
            }
        }
    }

    fn press(&self, key: &str) -> Result<()> {
        let unknown = || {
            anyhow::anyhow!(
                "unknown key \"{key}\". Use home, lock, switcher, spotlight, enter, tab, backspace, escape, space, up, down, left or right"
            )
        };
        let name = key.to_lowercase();
        match &self.backend {
            Backend::Sim { native, .. } => match name.as_str() {
                "home" => native.button(native::Button::Home),
                "lock" | "power" => native.button(native::Button::Lock),
                other => native.keys(&keys::named(other).ok_or_else(unknown)?),
            },
            Backend::Mirror(m) => {
                let m = m.borrow();
                // iPhone Mirroring's own shortcuts for the system controls.
                let shortcut = match name.as_str() {
                    "home" => Some('1'),
                    "switcher" | "app-switcher" | "recents" => Some('2'),
                    "spotlight" | "search" => Some('3'),
                    "lock" | "power" => bail!("iPhone Mirroring can't lock the phone"),
                    _ => None,
                };
                let events = match shortcut {
                    Some(c) => keys::mac_chord(&["cmd"], c),
                    None => keys::mac_named(&name),
                };
                m.keys(&events.ok_or_else(unknown)?)?;
                if shortcut.is_some() {
                    std::thread::sleep(Duration::from_millis(800));
                }
                Ok(())
            }
        }
    }

    fn start_point(&self, from: Option<&str>) -> Result<(f64, f64)> {
        match from {
            Some(target) => Ok(self.locate(&Locator::parse(target), LocateOptions { force: true, ..Default::default() })?.1),
            None => {
                let (w, h) = self.screen()?;
                Ok((w / 2.0, h / 2.0))
            }
        }
    }

    fn swipe(&self, direction: Direction, from: Option<&str>, distance: Option<f64>) -> Result<(f64, f64, f64, f64)> {
        let (width, height) = self.screen()?;
        let (x0, y0) = self.start_point(from)?;
        let vertical = matches!(direction, Direction::Up | Direction::Down);
        let distance = distance.unwrap_or(if vertical { height * 0.4 } else { width * 0.6 });
        let (dx, dy) = match direction {
            Direction::Up => (0.0, -distance),
            Direction::Down => (0.0, distance),
            Direction::Left => (-distance, 0.0),
            Direction::Right => (distance, 0.0),
        };
        let clamp = |v: f64, max: f64| v.clamp(1.0, max - 1.0);
        let (x1, y1) = (clamp(x0 + dx, width), clamp(y0 + dy, height));
        let steps = 12;
        let points: Vec<(f64, f64)> = (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)
            })
            .collect();
        self.touch(&points, 0, if self.is_mirror() { 25 } else { 16 })?;
        Ok((x0, y0, x1, y1))
    }

    /// Scrolls content: a drag on a simulator, scroll-wheel events through iPhone Mirroring.
    fn scroll(&self, direction: Direction, from: Option<&str>, distance: Option<f64>) -> Result<()> {
        match &self.backend {
            Backend::Sim { .. } => self.swipe(direction.opposite(), from, distance).map(|_| ()),
            Backend::Mirror(m) => {
                let at = self.start_point(from)?;
                let (w, h) = self.screen()?;
                let d = distance.unwrap_or(match direction {
                    Direction::Up | Direction::Down => h * 0.5,
                    _ => w * 0.6,
                });
                let (dx, dy) = match direction {
                    Direction::Down => (0.0, d),
                    Direction::Up => (0.0, -d),
                    Direction::Right => (d, 0.0),
                    Direction::Left => (-d, 0.0),
                };
                m.borrow().scroll(at, dx, dy)
            }
        }
    }

    fn open(&self, target: &str, fresh: bool) -> Result<()> {
        match &self.backend {
            Backend::Sim { udid, .. } if target.contains("://") => device::open_url(udid, target),
            Backend::Sim { udid, .. } => device::launch(udid, target, fresh),
            Backend::Mirror(_) if target.contains("://") => {
                bail!("iPhone Mirroring can't open URLs directly; open Safari and type it instead")
            }
            Backend::Mirror(m) => m.borrow().open_app(target),
        }
    }

    fn click(&self, ctx: &Ctx, locator: &Locator, label: &str, hold: u32, force: bool) -> Result<()> {
        let (element, point) = self.locate(locator, LocateOptions { force, ..Default::default() })?;
        self.touch(&[point], hold.max(30), 0)?;
        let verb = if hold >= 400 { "long-pressed" } else { "clicked" };
        ctx.done(format!("{verb} {}", target_label(&element, point, label)), element_json(&element, point));
        Ok(())
    }

    fn fill(&self, ctx: &Ctx, locator: &Locator, label: &str, text: &str, keystrokes: bool, force: bool) -> Result<()> {
        let (element, point) = self.locate(locator, LocateOptions { editable: true, force })?;
        self.touch(&[point], 30, 0)?;
        std::thread::sleep(Duration::from_millis(350));
        self.clear_field()?;
        self.type_text(text, keystrokes)?;
        ctx.done(format!("filled {}", target_label(&element, point, label)), element_json(&element, point));
        Ok(())
    }

    /// Whether `locator` names something on screen right now. For text and queries, any
    /// matching element counts, not only the best match.
    fn present(&self, snapshot: &Snapshot, locator: &Locator, saved: Option<&Element>) -> bool {
        let (w, h) = self.screen().unwrap_or((f64::MAX, f64::MAX));
        let on_screen = |e: &Element| {
            let (x, y) = e.center();
            x >= 0.0 && y >= 0.0 && x <= w && y <= h
        };
        match locator {
            Locator::Point(_) => true,
            Locator::Text(text) => {
                snapshot.query(&Query { by: By::Text, value: text.clone(), name: None, exact: false }).into_iter().any(on_screen)
            }
            Locator::Query(query, None) => snapshot.query(query).into_iter().any(on_screen),
            _ => matches!(self.find_in(snapshot, locator, false, saved), Ok(Some(e)) if on_screen(&e)),
        }
    }
}

impl Phone {
    fn springboard(&self) -> Option<i64> {
        *self.springboard.get_or_init(|| match &self.backend {
            Backend::Sim { udid, .. } => sim::springboard_pid(udid).ok().flatten(),
            Backend::Mirror(_) => None,
        })
    }

    /// A system alert (from SpringBoard) is in front: a few buttons and some text, nothing else.
    fn system_alert<'a>(&self, snapshot: &'a Snapshot) -> Option<Vec<&'a Element>> {
        if snapshot.pid.is_none() || snapshot.pid != self.springboard() {
            return None;
        }
        let buttons: Vec<&Element> = snapshot.elements.iter().filter(|e| e.interactive).collect();
        let alert = !buttons.is_empty()
            && buttons.len() <= 6
            && buttons.iter().all(|e| e.role == "button")
            && snapshot.elements.iter().any(|e| e.role == "text");
        alert.then_some(buttons)
    }

    fn alert_text(snapshot: &Snapshot) -> String {
        snapshot.elements.iter().filter(|e| e.role == "text").filter_map(|e| e.name()).collect::<Vec<_>>().join(" ")
    }

    /// Taps the yes (or no) button of a system alert, if one is up. Returns what it did.
    fn answer_dialog(&self, accept: bool, required: bool) -> Result<Option<String>> {
        if self.is_mirror() {
            return Ok(None);
        }
        let snapshot = self.snapshot()?;
        let Some(buttons) = self.system_alert(&snapshot) else {
            if required {
                bail!("no system alert is on screen");
            }
            return Ok(None);
        };
        let wanted = if accept { ACCEPT } else { DISMISS };
        let button = wanted
            .iter()
            .find_map(|w| buttons.iter().find(|b| b.label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(w))))
            .with_context(|| {
                let names: Vec<String> = buttons.iter().map(|b| format!("\"{}\"", b.name().unwrap_or("?"))).collect();
                format!("can't tell which button {} this alert; click one: {}", if accept { "accepts" } else { "dismisses" }, names.join(", "))
            })?;
        self.touch(&[button.center()], 30, 0)?;
        std::thread::sleep(Duration::from_millis(500));
        Ok(Some(format!("{} \"{}\" on the alert \"{}\"", if accept { "accepted with" } else { "dismissed with" }, button.name().unwrap_or("?"), Self::alert_text(&snapshot))))
    }

    /// The frontmost app's bundle ID.
    fn frontmost(&self, snapshot: &Snapshot) -> Result<Option<String>> {
        let Backend::Sim { udid, .. } = &self.backend else { return Ok(None) };
        let Some(pid) = snapshot.pid else { return Ok(None) };
        if Some(pid) == self.springboard() {
            return Ok(Some("com.apple.springboard".into()));
        }
        Ok(sim::running(udid)?.into_iter().find(|r| r.pid == pid).map(|r| r.bundle_id))
    }

    fn frontmost_app(&self, what: &str) -> Result<String> {
        let bundle = self.frontmost(&self.snapshot()?)?.context("can't tell which app is in front")?;
        if bundle == "com.apple.springboard" {
            bail!("no app is open (the home screen is in front); pass --app for `{what}`");
        }
        Ok(bundle)
    }

    fn back(&self) -> Result<String> {
        let snapshot = self.snapshot()?;
        let (w, h) = self.screen()?;
        let named = snapshot.elements.iter().find(|e| {
            e.role == "button"
                && (e.label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("back")) || e.identifier.as_deref() == Some("BackButton"))
        });
        let corner = || {
            snapshot.elements.iter().find(|e| {
                let (x, y) = e.center();
                e.role == "button" && e.enabled && y > 0.0 && y < 130.0 && x < w * 0.25
            })
        };
        if let Some(button) = named.or_else(corner) {
            self.touch(&[button.center()], 30, 0)?;
            return Ok(format!("went back with {} (@{})", button.describe(), button.reference));
        }
        let y = h * 0.45;
        let points: Vec<(f64, f64)> = (0..=16).map(|i| (1.0 + (w * 0.8) * i as f64 / 16.0, y)).collect();
        self.touch(&points, 0, 12)?;
        Ok("went back with a swipe from the left edge".into())
    }

    /// Turns a switch or checkbox on or off, then checks it took.
    fn set_switch(&self, target: &str, on: bool) -> Result<String> {
        let locator = Locator::parse(target);
        let (element, point) = self.locate(&locator, LocateOptions::default())?;
        let element = element.context("give a switch, not a point")?;
        if !matches!(element.role.as_str(), "switch" | "checkbox" | "radio") && element.value.is_none() {
            bail!("{} is not a switch or checkbox", element.describe());
        }
        let state = if on { "on" } else { "off" };
        if element.on() == on {
            return Ok(format!("{} is already {state}", element.describe()));
        }
        self.touch(&[point], 30, 0)?;
        std::thread::sleep(Duration::from_millis(450));
        let after = self.snapshot()?;
        if let Some(now) = after.relocate(&element) {
            if now.on() != on {
                bail!("tapped {} but it is still {}", element.describe(), if on { "off" } else { "on" });
            }
        }
        Ok(format!("turned {} {state}", element.describe()))
    }
}

/// Accessibility problems on the screen: unlabelled controls, small tap targets, duplicates.
fn audit(snapshot: &Snapshot) -> Vec<serde_json::Value> {
    let mut issues = Vec::new();
    let interactive: Vec<&Element> = snapshot.elements.iter().filter(|e| e.interactive).collect();
    for e in &interactive {
        if e.label.is_none() && e.placeholder.is_none() {
            issues.push(json!({ "ref": e.reference, "issue": "no-label", "detail": format!("{} has no accessibility label; VoiceOver reads nothing useful", e.describe()) }));
        }
        // Small both ways, or a sliver: frames that are long in one direction are usually rows
        // whose real hit area is larger.
        let (w, h) = (e.frame[2], e.frame[3]);
        let small = !e.editable() && e.role != "cell" && w > 0.0 && ((w < 44.0 && h < 44.0) || w.min(h) < 20.0);
        if small {
            issues.push(json!({ "ref": e.reference, "issue": "small-target", "detail": format!("{} is {:.0}x{:.0} pt; tap targets should be at least 44x44", e.describe(), e.frame[2], e.frame[3]) }));
        }
    }
    let mut seen: HashMap<(String, String), Vec<&Element>> = HashMap::new();
    for e in &interactive {
        if let Some(label) = &e.label {
            seen.entry((e.role.clone(), label.clone())).or_default().push(e);
        }
    }
    let mut duplicates: Vec<_> = seen.into_iter().filter(|(_, v)| v.len() > 1).collect();
    duplicates.sort_by_key(|(_, v)| v[0].reference.trim_start_matches('e').parse::<usize>().unwrap_or(0));
    for ((role, label), elements) in duplicates {
        let refs: Vec<String> = elements.iter().map(|e| format!("@{}", e.reference)).collect();
        issues.push(json!({ "ref": elements[0].reference, "issue": "duplicate-label", "detail": format!("{} {role} elements are labelled \"{label}\" ({}); VoiceOver users can't tell them apart", elements.len(), refs.join(", ")) }));
    }
    issues
}

/// Makes a path absolute against the current folder, for tools that run elsewhere.
fn absolute(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir().map(|d| d.join(p).to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string())
    }
}

fn parse_point(target: &str) -> Option<(f64, f64)> {
    let (x, y) = target.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn element_json(element: &Option<Element>, point: (f64, f64)) -> serde_json::Value {
    json!({ "element": element, "x": point.0, "y": point.1 })
}

fn target_label(element: &Option<Element>, point: (f64, f64), target: &str) -> String {
    match element {
        Some(e) if target.starts_with('@') => format!("{} ({target})", e.describe()),
        Some(e) => format!("{} (@{})", e.describe(), e.reference),
        None => format!("{:.0},{:.0}", point.0, point.1),
    }
}

fn mirror_state_line(s: &mirror::Status) -> String {
    match s.state.as_str() {
        "ready" => "● connected".into(),
        "blocked" => format!("paused: {}", s.blocked_text.join(" / ")),
        "unknown" => "needs permissions (run doctor)".into(),
        other => other.replace('-', " "),
    }
}

/// Splits a batch line into words the way a shell would, with quotes and backslashes.
fn split_words(line: &str) -> Result<Vec<String>> {
    let (mut words, mut word, mut in_word, mut quote) = (Vec::new(), String::new(), false, None::<char>);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => word.extend(chars.next()),
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    in_word = true;
                }
                '\\' => {
                    word.extend(chars.next());
                    in_word = true;
                }
                c if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                c => {
                    word.push(c);
                    in_word = true;
                }
            },
        }
    }
    if quote.is_some() {
        bail!("unclosed quote in: {line}");
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

fn doctor(ctx: &Ctx) -> Result<()> {
    let mut lines = Vec::new();
    let mut checks = Vec::new();
    let mut check = |name: &str, ok: bool, detail: String| {
        lines.push(format!("{} {name}: {detail}", if ok { "✓" } else { "✗" }));
        checks.push(json!({ "check": name, "ok": ok, "detail": detail }));
    };
    if ctx.mirror() {
        let s = mirror::status()?;
        let app = mirror::host_app().unwrap_or_else(|| "the app you run agent-phone from".into());
        check(
            "accessibility",
            s.accessibility,
            if s.accessibility {
                format!("{app} may send taps and keys")
            } else {
                format!("grant {app} Accessibility: open \"x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility\"")
            },
        );
        check(
            "screen recording",
            s.screen_recording,
            if s.screen_recording {
                format!("{app} may capture the phone window")
            } else {
                format!("grant {app} Screen Recording, then restart it: open \"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture\"")
            },
        );
        check(
            "iPhone Mirroring",
            s.running,
            if s.running { "running".into() } else { "not running; `agent-phone --device iphone connect` opens it".into() },
        );
        if s.running {
            let detail = match s.state.as_str() {
                "ready" => match &s.window {
                    Some(w) => format!("connected, phone screen {:.0}x{:.0} pt", w.w, w.h),
                    None => "connected".into(),
                },
                "blocked" => format!("not live: \"{}\". Lock the iPhone, keep it nearby, then `connect`", s.blocked_text.join(" / ")),
                "no-window" => "no phone window; open iPhone Mirroring and pair your iPhone".into(),
                _ => "can't tell without Accessibility".into(),
            };
            check("session", s.state == "ready", detail);
        }
    } else {
        match device::developer_dir() {
            Ok(dir) => check("xcode", true, dir),
            Err(e) => check("xcode", false, format!("{e:#}")),
        }
        match device::resolve(ctx.device_arg.as_deref()) {
            Ok(d) => check("simulator", d.booted(), format!("{} ({}){}", d.name, d.runtime, if d.booted() { "" } else { " is not booted" })),
            Err(e) => check("simulator", false, format!("{e:#}")),
        }
    }
    let all_ok = checks.iter().all(|c| c["ok"] == json!(true));
    ctx.done(lines.join("\n"), json!({ "ok": all_ok, "checks": checks }));
    if !all_ok {
        ctx.code.set(1);
    }
    Ok(())
}

fn run(ctx: &Ctx, cli: Cli) -> Result<()> {
    match cli.command {
        Command::Devices => {
            let devices = device::list().unwrap_or_default();
            let mut text = String::new();
            let mirrored = mirror::status().ok();
            if let Some(s) = &mirrored {
                text.push_str(&format!("{:<24} {:<9} {:<36}  {}\n", "iPhone (Mirroring)", "", "--device iphone", mirror_state_line(s)));
            }
            for d in &devices {
                let mark = if d.booted() { "● booted" } else { "" };
                text.push_str(&format!("{:<24} {:<9} {}  {}\n", d.name, d.runtime, d.udid, mark));
            }
            let mut data: Vec<_> = devices
                .iter()
                .map(|d| json!({"name": d.name, "udid": d.udid, "runtime": d.runtime, "booted": d.booted(), "kind": "simulator"}))
                .collect();
            if let Some(s) = &mirrored {
                data.insert(0, json!({"name": "iPhone (Mirroring)", "udid": "iphone", "kind": "mirroring", "state": s.state}));
            }
            ctx.done(text, json!({ "devices": data }));
        }
        Command::Doctor => doctor(ctx)?,
        Command::Connect { timeout } => {
            if !ctx.mirror() && ctx.device_arg.is_some() {
                bail!("`connect` is for iPhone Mirroring; use `--device iphone` (simulators use `boot`)");
            }
            let s = mirror::connect(timeout)?;
            let size = s.window.as_ref().map(|w| format!(" ({:.0}x{:.0} pt)", w.w, w.h)).unwrap_or_default();
            ctx.done(format!("iPhone connected through iPhone Mirroring{size}"), json!({ "state": s.state, "window": s.window.map(|w| json!({"w": w.w, "h": w.h})) }));
        }
        Command::Boot { name } => {
            if ctx.mirror() {
                bail!("a real iPhone doesn't boot from here; use `agent-phone --device iphone connect`");
            }
            let wanted = name.or(ctx.device_arg.clone());
            let device = match wanted {
                Some(w) => device::resolve(Some(&w))?,
                None => device::list()?
                    .into_iter()
                    .find(|d| d.name.starts_with("iPhone"))
                    .context("no iPhone simulators are installed. Add one in Xcode > Settings > Components")?,
            };
            let started = Instant::now();
            device::boot(&device)?;
            daemon::stop_all();
            ctx.done(
                format!("booted {} ({}) in {:.1}s", device.name, device.udid, started.elapsed().as_secs_f64()),
                json!({ "name": device.name, "udid": device.udid }),
            );
        }
        Command::Shutdown => {
            if ctx.mirror() {
                bail!("`shutdown` {SIM_ONLY}");
            }
            let device = device::resolve(ctx.device_arg.as_deref())?;
            device::shutdown(&device)?;
            daemon::stop_all();
            ctx.done(format!("shut down {}", device.name), json!({ "udid": device.udid }));
        }
        Command::Apps { running: true } => {
            let phone = ctx.sim("apps")?;
            let apps = sim::running(phone.sim().0)?;
            let text: String = apps.iter().map(|a| format!("{:<8} {}\n", a.pid, a.bundle_id)).collect();
            ctx.done(text, json!({ "apps": apps }));
        }
        Command::Apps { .. } => {
            let phone = ctx.sim("apps")?;
            let apps = device::apps(phone.sim().0)?;
            let mut text = String::new();
            for app in &apps {
                let kind = if app.user { "user" } else { "system" };
                text.push_str(&format!("{:<28} {:<7} {}\n", app.name, kind, app.bundle_id));
            }
            ctx.done(text, json!({ "apps": apps }));
        }
        Command::Open { target, fresh } => {
            let phone = ctx.phone()?;
            phone.open(&target, fresh)?;
            ctx.done(format!("opened {target}"), json!({ "target": target }));
        }
        Command::Close { bundle_id } => {
            let phone = ctx.sim("close")?;
            device::terminate(phone.sim().0, &bundle_id)?;
            ctx.done(format!("closed {bundle_id}"), json!({ "bundle_id": bundle_id }));
        }
        Command::Install { path } => {
            let phone = ctx.sim("install")?;
            device::install(phone.sim().0, &path)?;
            ctx.done(format!("installed {path}"), json!({ "path": path }));
        }
        Command::Snapshot { raw: true, .. } => {
            let phone = ctx.sim("snapshot --raw")?;
            ctx.done(phone.sim().1.snapshot(5.0)?, json!({}));
        }
        Command::Snapshot { interactive, depth, scope, diff, .. } => {
            let phone = ctx.phone()?;
            let snapshot = phone.snapshot()?;
            let before = Session::load(phone.udid())?.elements;
            Session::save(phone.udid(), &snapshot)?;
            if diff {
                let (text, added, removed) = snapshot.diff(&before, interactive);
                ctx.done(text, json!({ "added": added, "removed": removed, "diff": snapshot.diff(&before, interactive).0 }));
                return Ok(());
            }
            let elements: Vec<&Element> = match &scope {
                Some(target) => {
                    let locator = Locator::parse(target);
                    let saved = phone.saved_for(&locator)?;
                    let root = phone.find_in(&snapshot, &locator, false, saved.as_ref())?.context("--scope needs an element, not a point")?;
                    snapshot.subtree(&root)
                }
                None => snapshot.elements.iter().collect(),
            };
            let options = RenderOptions { interactive, max_depth: depth, ..Default::default() };
            let text = snapshot.render_with(&elements, &options);
            let listed: Vec<&&Element> = elements.iter().filter(|e| !interactive || e.interactive).collect();
            ctx.done(text, json!({ "app": snapshot.app, "elements": listed }));
        }
        Command::Click { target, hold, force } => {
            ctx.phone()?.click(ctx, &Locator::parse(&target), &target, hold, force)?;
        }
        Command::Fill { target, text, keystrokes, force } => {
            ctx.phone()?.fill(ctx, &Locator::parse(&target), &target, &text, keystrokes, force)?;
        }
        Command::Type { text, keystrokes } => {
            let phone = ctx.phone()?;
            phone.type_text(&text, keystrokes)?;
            ctx.done(format!("typed {} characters", text.chars().count()), json!({ "length": text.chars().count() }));
        }
        Command::Press { key } => {
            let phone = ctx.phone()?;
            phone.press(&key)?;
            ctx.done(format!("pressed {key}"), json!({ "key": key }));
        }
        Command::Swipe { direction, from, distance } => {
            let phone = ctx.phone()?;
            let (x0, y0, x1, y1) = phone.swipe(direction, from.as_deref(), distance)?;
            ctx.done(
                format!("swiped from {x0:.0},{y0:.0} to {x1:.0},{y1:.0}"),
                json!({ "from": [x0, y0], "to": [x1, y1] }),
            );
        }
        Command::Scroll { direction, from, distance } => {
            let phone = ctx.phone()?;
            phone.scroll(direction, from.as_deref(), distance)?;
            ctx.done("scrolled", json!({}));
        }
        Command::Back => {
            let phone = ctx.phone()?;
            let note = phone.back()?;
            ctx.done(note, json!({}));
        }
        Command::Reload => {
            let phone = ctx.sim("reload")?;
            let bundle = phone.frontmost_app("reload")?;
            device::launch(phone.sim().0, &bundle, true)?;
            ctx.done(format!("relaunched {bundle}"), json!({ "bundle_id": bundle }));
        }
        Command::Dblclick { target } => {
            let phone = ctx.phone()?;
            let (element, point) = phone.locate(&Locator::parse(&target), LocateOptions::default())?;
            phone.touch(&[point], 30, 0)?;
            std::thread::sleep(Duration::from_millis(70));
            phone.touch(&[point], 30, 0)?;
            ctx.done(format!("double-clicked {}", target_label(&element, point, &target)), element_json(&element, point));
        }
        Command::Drag { from, to, hold, duration } => {
            let phone = ctx.phone()?;
            let any = LocateOptions { force: true, ..Default::default() };
            let (_, a) = phone.locate(&Locator::parse(&from), any)?;
            let (_, b) = phone.locate(&Locator::parse(&to), any)?;
            let steps = (duration / 16).max(4);
            let mut points = vec![a];
            points.extend((1..=steps).map(|i| {
                let t = i as f64 / steps as f64;
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            }));
            // Rest on the target briefly so drop targets register the finger.
            points.extend([b, b, b]);
            phone.touch(&points, hold, 16)?;
            ctx.done(format!("dragged from {:.0},{:.0} to {:.0},{:.0}", a.0, a.1, b.0, b.1), json!({ "from": [a.0, a.1], "to": [b.0, b.1] }));
        }
        Command::Check { target } | Command::Uncheck { target } if ctx.mirror() => {
            bail!("`check` {SIM_ONLY} (switch states can't be read from the mirrored screen); click {target} instead")
        }
        Command::Check { target } => {
            let note = ctx.phone()?.set_switch(&target, true)?;
            ctx.done(note, json!({ "on": true }));
        }
        Command::Uncheck { target } => {
            let note = ctx.phone()?.set_switch(&target, false)?;
            ctx.done(note, json!({ "on": false }));
        }
        Command::Dialog { action } => {
            let phone = ctx.sim("dialog")?;
            match action {
                DialogAction::Status => {
                    let snapshot = phone.snapshot()?;
                    match phone.system_alert(&snapshot) {
                        Some(buttons) => {
                            let text = Phone::alert_text(&snapshot);
                            Session::save(phone.udid(), &snapshot)?;
                            let list = snapshot.render_with(&buttons, &RenderOptions { flat: true, ..Default::default() });
                            ctx.done(format!("system alert: \"{text}\"\n{list}"), json!({ "alert": true, "text": text, "buttons": buttons }));
                        }
                        None => ctx.done("no system alert", json!({ "alert": false })),
                    }
                }
                DialogAction::Accept | DialogAction::Dismiss => {
                    let note = phone.answer_dialog(action == DialogAction::Accept, true)?.unwrap_or_default();
                    ctx.done(note, json!({}));
                }
            }
        }
        Command::Set { what } => {
            let phone = ctx.sim("set")?;
            let udid = phone.sim().0;
            let note = match what {
                SetCommand::Location { value } if value == "clear" => {
                    sim::clear_location(udid)?;
                    "cleared the simulated location".to_string()
                }
                SetCommand::Location { value } => {
                    let (lat, lon) = parse_point(&value).context("give latitude,longitude, e.g. `set location 37.3349,-122.0090`")?;
                    sim::set_location(udid, lat, lon)?;
                    format!("location set to {lat},{lon}")
                }
                SetCommand::Appearance { mode } => {
                    let mode = match mode {
                        Appearance::Light => "light",
                        Appearance::Dark => "dark",
                    };
                    sim::ui(udid, "appearance", mode)?;
                    format!("appearance set to {mode}")
                }
                SetCommand::Permission { service, action, app } => {
                    let action = match action {
                        PermissionAction::Grant => "grant",
                        PermissionAction::Revoke => "revoke",
                        PermissionAction::Reset => "reset",
                    };
                    let app = match app {
                        Some(app) => Some(app),
                        None if action == "reset" => None,
                        None => Some(phone.frontmost_app("set permission")?),
                    };
                    sim::privacy(udid, action, &service, app.as_deref())?;
                    format!("{action} {service}{}", app.map(|a| format!(" for {a}")).unwrap_or_default())
                }
                SetCommand::Statusbar { mode: StatusBar::Clean } => {
                    sim::status_bar_clean(udid)?;
                    "status bar set to 9:41 with full signal and battery".into()
                }
                SetCommand::Statusbar { mode: StatusBar::Clear } => {
                    sim::status_bar_clear(udid)?;
                    "status bar back to normal".into()
                }
                SetCommand::TextSize { size } => {
                    sim::ui(udid, "content_size", &size)?;
                    format!("text size set to {size}")
                }
                SetCommand::Contrast { mode } => {
                    let on = matches!(mode, OnOff::On);
                    sim::ui(udid, "increase_contrast", if on { "enabled" } else { "disabled" })?;
                    format!("Increase Contrast {}", if on { "on" } else { "off" })
                }
            };
            ctx.done(note, json!({}));
        }
        Command::Push { bundle_id, payload, title, body, badge } => {
            let phone = ctx.sim("push")?;
            let payload: serde_json::Value = match payload {
                Some(p) if p.trim_start().starts_with('{') => serde_json::from_str(&p).context("the payload is not valid JSON")?,
                Some(p) => serde_json::from_slice(&std::fs::read(&p).with_context(|| format!("could not read {p}"))?).context("the payload is not valid JSON")?,
                None => {
                    let mut aps = json!({ "alert": { "title": title.unwrap_or_default(), "body": body.unwrap_or_default() }, "sound": "default" });
                    if let Some(badge) = badge {
                        aps["badge"] = json!(badge);
                    }
                    json!({ "aps": aps })
                }
            };
            if payload.get("aps").is_none() {
                bail!("the payload needs an \"aps\" object");
            }
            sim::push(phone.sim().0, &bundle_id, &payload)?;
            ctx.done(format!("pushed a notification to {bundle_id}"), json!({ "payload": payload }));
        }
        Command::State { action } => {
            let phone = ctx.sim("state")?;
            let udid = phone.sim().0;
            match action {
                StateCommand::Save { name, app } => {
                    let bundle = match app {
                        Some(a) => a,
                        None => phone.frontmost_app("state save")?,
                    };
                    let dir = sim::save_state(udid, &bundle, &name)?;
                    ctx.done(format!("saved {bundle}'s data as \"{name}\" (the app was stopped)"), json!({ "name": name, "bundle_id": bundle, "path": dir }));
                }
                StateCommand::Load { name } => {
                    let bundle = sim::load_state(udid, &name)?;
                    ctx.done(format!("restored \"{name}\" into {bundle}; `agent-phone open {bundle}` to use it"), json!({ "name": name, "bundle_id": bundle }));
                }
                StateCommand::List => {
                    let states = sim::list_states()?;
                    let text = if states.is_empty() {
                        "no saved states".to_string()
                    } else {
                        states.iter().map(|(n, m)| format!("{n:<20} {}", m.bundle_id)).collect::<Vec<_>>().join("\n")
                    };
                    let data: Vec<_> = states.iter().map(|(n, m)| json!({ "name": n, "bundle_id": m.bundle_id, "saved_at": m.saved_at })).collect();
                    ctx.done(text, json!({ "states": data }));
                }
                StateCommand::Delete { name } => {
                    sim::delete_state(&name)?;
                    ctx.done(format!("deleted \"{name}\""), json!({ "name": name }));
                }
                StateCommand::Reset { app } => {
                    let bundle = match app {
                        Some(a) => a,
                        None => phone.frontmost_app("state reset")?,
                    };
                    sim::reset_state(udid, &bundle)?;
                    ctx.done(format!("emptied {bundle}'s data (the app was stopped)"), json!({ "bundle_id": bundle }));
                }
            }
        }
        Command::Keychain { action: KeychainAction::Reset } => {
            let phone = ctx.sim("keychain")?;
            sim::keychain_reset(phone.sim().0)?;
            ctx.done("reset the keychain", json!({}));
        }
        Command::Media { action: MediaAction::Add, paths } => {
            let phone = ctx.sim("media")?;
            if paths.is_empty() {
                bail!("give one or more files: `agent-phone media add photo.jpg clip.mov`");
            }
            let paths: Vec<String> = paths.iter().map(|p| absolute(p)).collect();
            sim::add_media(phone.sim().0, &paths)?;
            ctx.done(format!("added {} file{}", paths.len(), if paths.len() == 1 { "" } else { "s" }), json!({ "paths": paths }));
        }
        Command::Clipboard { action, text } => {
            let phone = ctx.sim("clipboard")?;
            match action {
                ClipboardAction::Get => {
                    let text = device::pasteboard(phone.sim().0)?;
                    ctx.done(text.clone(), json!({ "text": text }));
                }
                ClipboardAction::Set => {
                    let text = text.context("give the text: `agent-phone clipboard set \"hello\"`")?;
                    device::set_pasteboard(phone.sim().0, &text)?;
                    ctx.done(format!("clipboard set ({} characters)", text.chars().count()), json!({}));
                }
            }
        }
        Command::Logs { app, all, last, errors, grep, follow, lines } => {
            let phone = ctx.sim("logs")?;
            let udid = phone.sim().0;
            let bundle = match (all, app) {
                (true, _) => None,
                (false, Some(app)) => Some(app),
                (false, None) => Some(phone.frontmost_app("logs")?),
            };
            let process = bundle.as_deref().map(|b| sim::executable(udid, b)).transpose()?;
            let query = sim::LogQuery { process, last, errors, grep };
            if follow {
                query.follow(udid)?;
                return Ok(());
            }
            let text = query.show(udid)?;
            // Drop log's own header line, keep the newest `lines`.
            let body: Vec<&str> = text.lines().filter(|l| !l.starts_with("Timestamp ") && !l.trim().is_empty()).collect();
            let shown = &body[body.len().saturating_sub(lines)..];
            let out = if shown.is_empty() { "(no log lines)".to_string() } else { shown.join("\n") };
            ctx.done(out, json!({ "lines": shown }));
        }
        Command::Crashes { app, limit } => {
            let list = sim::crashes(app.as_deref(), limit)?;
            let text = if list.is_empty() {
                "no crash reports".to_string()
            } else {
                list.iter().map(|c| format!("{}  {}  {}\n  {}", c.time, c.app, c.exception, c.path)).collect::<Vec<_>>().join("\n")
            };
            ctx.done(text, json!({ "crashes": list }));
        }
        Command::Audit => {
            let phone = ctx.phone()?;
            if phone.is_mirror() {
                bail!("`audit` {SIM_ONLY}");
            }
            let snapshot = phone.snapshot()?;
            Session::save(phone.udid(), &snapshot)?;
            let issues = audit(&snapshot);
            let text = if issues.is_empty() {
                "no problems found on this screen".to_string()
            } else {
                issues.iter().map(|i| format!("@{} {}: {}", i["ref"].as_str().unwrap_or(""), i["issue"].as_str().unwrap_or(""), i["detail"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")
            };
            ctx.done(text, json!({ "issues": issues }));
        }
        Command::Record { action, path } => {
            let phone = ctx.sim("record")?;
            let udid = phone.sim().0;
            match action {
                RecordAction::Start => {
                    let path = absolute(&path);
                    sim::record_start(udid, &path)?;
                    ctx.done(format!("recording to {path}; `agent-phone record stop` to finish"), json!({ "path": path }));
                }
                RecordAction::Stop => {
                    let path = sim::record_stop(udid)?;
                    ctx.done(format!("saved {path}"), json!({ "path": path }));
                }
            }
        }
        Command::Diff { baseline, current, out, threshold } => {
            let current = match current {
                Some(c) => c,
                None => {
                    let phone = ctx.phone()?;
                    let path = std::env::temp_dir().join(format!("agent-phone-diff-{}.png", std::process::id())).to_string_lossy().into_owned();
                    phone.screenshot(&path)?;
                    path
                }
            };
            let ratio = annotate::diff(&baseline, &current, out.as_deref(), threshold)?;
            let percent = ratio * 100.0;
            let summary = if ratio == 0.0 { "identical".to_string() } else { format!("{percent:.2}% of pixels changed") };
            let text = match &out {
                Some(o) => format!("{summary}; changes marked in {o}"),
                None => summary,
            };
            ctx.done(text, json!({ "changed": ratio, "identical": ratio == 0.0, "out": out }));
        }
        Command::Find { by, value, action, text, name, exact, nth, force } => {
            let phone = ctx.phone()?;
            let query = Query { by, value, name, exact };
            let label = query.describe();
            let locator = Locator::Query(query.clone(), nth);
            match action.unwrap_or(FindAction::List) {
                FindAction::Click => phone.click(ctx, &locator, &label, 0, force)?,
                FindAction::Fill => {
                    let text = text.context("`find … fill` needs the text to type, e.g. `find id email fill \"me@x.com\"`")?;
                    phone.fill(ctx, &locator, &label, &text, false, force)?;
                }
                action => {
                    let snapshot = phone.snapshot()?;
                    Session::save(phone.udid(), &snapshot)?;
                    let mut matches = snapshot.query(&query);
                    if let Some(n) = nth {
                        matches = matches.into_iter().skip(n.max(1) - 1).take(1).collect();
                    }
                    match action {
                        FindAction::Count => ctx.done(matches.len().to_string(), json!({ "count": matches.len() })),
                        FindAction::Text => {
                            let e = matches.first().with_context(|| format!("nothing on screen matches {label}"))?;
                            let value = e.name().unwrap_or_default();
                            ctx.done(value, json!({ "value": value, "element": e }));
                        }
                        _ => {
                            if matches.is_empty() {
                                bail!("nothing on screen matches {label}. Run `agent-phone snapshot` to see what is there");
                            }
                            let text = snapshot.render_with(&matches, &RenderOptions { flat: true, ..Default::default() });
                            ctx.done(text, json!({ "elements": matches }));
                        }
                    }
                }
            }
        }
        Command::Get { what, target } => {
            let phone = ctx.phone()?;
            let (value, extra): (serde_json::Value, Option<&Element>);
            let snapshot;
            let element;
            match what {
                GetWhat::App => {
                    snapshot = phone.snapshot()?;
                    let bundle = phone.frontmost(&snapshot).ok().flatten();
                    value = match (&snapshot.app, &bundle) {
                        _ if phone.system_alert(&snapshot).is_some() => json!("System alert (com.apple.springboard)"),
                        (_, Some(b)) if b == "com.apple.springboard" => json!("Home Screen (com.apple.springboard)"),
                        (Some(name), Some(b)) => json!(format!("{name} ({b})")),
                        (name, b) => json!(name.clone().or(b.clone())),
                    };
                    extra = None;
                }
                GetWhat::Size => {
                    let (w, h) = phone.screen()?;
                    value = json!(format!("{w:.0}x{h:.0}"));
                    extra = None;
                }
                GetWhat::Focused => {
                    snapshot = phone.snapshot()?;
                    let focused = snapshot.elements.iter().find(|e| e.focused);
                    value = json!(focused.map(|e| format!("{} @{}", e.describe(), e.reference)));
                    extra = focused;
                    Session::save(phone.udid(), &snapshot)?;
                }
                GetWhat::Count => {
                    let target = target.context("`get count` needs a target, e.g. `get count \"Delete\"`")?;
                    snapshot = phone.snapshot()?;
                    let count = match Locator::parse(&target) {
                        Locator::Text(t) => snapshot.query(&Query { by: By::Text, value: t, name: None, exact: false }).len(),
                        Locator::Query(q, _) => snapshot.query(&q).len(),
                        locator => usize::from(phone.present(&snapshot, &locator, phone.saved_for(&locator)?.as_ref())),
                    };
                    value = json!(count);
                    extra = None;
                }
                _ => {
                    let target = target.context("this needs a target, e.g. `get text @e4`")?;
                    let locator = Locator::parse(&target);
                    snapshot = phone.snapshot()?;
                    let saved = phone.saved_for(&locator)?;
                    element = phone.find_in(&snapshot, &locator, false, saved.as_ref())?.context("a point has no text or state")?;
                    let e = &element;
                    value = match what {
                        GetWhat::Text => json!(e.name()),
                        GetWhat::Value => json!(e.value),
                        GetWhat::Label => json!(e.label),
                        GetWhat::Id => json!(e.identifier),
                        GetWhat::Role => json!(e.role),
                        GetWhat::Frame => json!(format!("{:.0},{:.0},{:.0},{:.0}", e.frame[0], e.frame[1], e.frame[2], e.frame[3])),
                        _ => unreachable!(),
                    };
                    extra = Some(e);
                }
            }
            let text = match &value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => "(none)".into(),
                other => other.to_string(),
            };
            ctx.done(text, json!({ "value": value, "element": extra }));
        }
        Command::Is { what, target } => {
            let phone = ctx.phone()?;
            let locator = Locator::parse(&target);
            let snapshot = phone.snapshot()?;
            let value = match what {
                // A ref that isn't in the last snapshot isn't visible either.
                IsWhat::Visible => match phone.saved_for(&locator) {
                    Ok(saved) => phone.present(&snapshot, &locator, saved.as_ref()),
                    Err(_) => false,
                },
                _ => {
                    let saved = phone.saved_for(&locator)?;
                    let e = phone.find_in(&snapshot, &locator, false, saved.as_ref())?.context("a point has no state")?;
                    match what {
                        IsWhat::Enabled => e.enabled,
                        IsWhat::Focused => e.focused,
                        IsWhat::Selected => e.selected || e.on(),
                        IsWhat::On => e.on(),
                        IsWhat::Visible => unreachable!(),
                    }
                }
            };
            ctx.done(value.to_string(), json!({ "value": value }));
        }
        Command::Wait { what, text, gone, stable, timeout } => {
            let started = Instant::now();
            if let Some(ms) = what.as_deref().and_then(|w| w.parse::<u64>().ok()) {
                std::thread::sleep(Duration::from_millis(ms));
                ctx.done(format!("waited {ms}ms"), json!({ "ms": ms }));
                return Ok(());
            }
            let phone = ctx.phone()?;
            if stable {
                let mut last = String::new();
                loop {
                    // An empty tree is an app still launching, not a settled screen.
                    let now = phone.snapshot().ok().filter(|s| !s.elements.is_empty()).map(|s| s.render(false)).unwrap_or_default();
                    if !now.is_empty() && now == last {
                        let secs = started.elapsed().as_secs_f64();
                        ctx.done(format!("screen is still after {secs:.1}s"), json!({ "seconds": secs }));
                        return Ok(());
                    }
                    if started.elapsed().as_secs_f64() > timeout {
                        bail!("the screen was still changing after {timeout}s");
                    }
                    last = now;
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
            let (locator, label) = match (what, text) {
                (Some(target), None) => (Locator::parse(&target), target),
                (None, Some(text)) => (Locator::Query(Query { by: By::Text, value: text.clone(), name: None, exact: false }, None), format!("\"{text}\"")),
                _ => bail!("pass milliseconds (`wait 500`), a target (`wait @e4`, `wait \"Done\"`), --text or --stable"),
            };
            if let Locator::Point(_) = locator {
                bail!("`wait` can't wait for a point; pass milliseconds, a ref or text");
            }
            let saved = phone.saved_for(&locator)?;
            loop {
                let present = phone.snapshot().map(|s| phone.present(&s, &locator, saved.as_ref())).unwrap_or(false);
                if present != gone {
                    let state = if gone { "gone" } else { "visible" };
                    let secs = started.elapsed().as_secs_f64();
                    ctx.done(format!("{label} is {state} after {secs:.1}s"), json!({ "target": label, "seconds": secs }));
                    break;
                }
                if started.elapsed().as_secs_f64() > timeout {
                    let state = if gone { "still on screen" } else { "not on screen" };
                    bail!("{label} was {state} after {timeout}s");
                }
                std::thread::sleep(Duration::from_millis(400));
            }
        }
        Command::Screenshot { path, annotate } => {
            let phone = ctx.phone()?;
            if !annotate {
                phone.screenshot(&path)?;
                ctx.done(format!("saved {path}"), json!({ "path": path }));
                return Ok(());
            }
            let snapshot = phone.snapshot()?;
            Session::save(phone.udid(), &snapshot)?;
            phone.screenshot(&path)?;
            let (w, h) = phone.screen()?;
            let shown: Vec<&Element> = snapshot
                .elements
                .iter()
                .filter(|e| e.interactive)
                .filter(|e| {
                    let (x, y) = e.center();
                    x >= 0.0 && y >= 0.0 && x <= w && y <= h
                })
                .collect();
            annotate::draw(&path, &shown, w)?;
            let legend = snapshot.render_with(&shown, &RenderOptions { flat: true, ..Default::default() });
            ctx.done(format!("saved {path} with {} refs\n{legend}", shown.len()), json!({ "path": path, "elements": shown }));
        }
        Command::Batch { commands, keep_going } => {
            let lines: Vec<String> = if commands.is_empty() {
                std::io::stdin().lines().collect::<std::io::Result<_>>()?
            } else {
                commands
            };
            let mut results = Vec::new();
            let mut text = String::new();
            let mut failed = 0;
            for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let mut words = split_words(line)?;
                if words.first().map(String::as_str) == Some("agent-phone") {
                    words.remove(0);
                }
                let mut args = vec!["agent-phone".to_string()];
                if ctx.json {
                    args.push("--json".into());
                }
                if let Some(device) = &ctx.device_arg {
                    args.extend(["--device".to_string(), device.clone()]);
                }
                args.extend(words);
                let out = dispatch(args);
                if ctx.json {
                    let value: serde_json::Value = out.stdout.lines().last().and_then(|l| serde_json::from_str(l).ok()).unwrap_or(json!({}));
                    results.push(json!({ "command": line, "result": value, "code": out.code }));
                } else {
                    text.push_str(&format!("$ {line}\n{}{}", out.stdout, out.stderr));
                }
                if out.code != 0 {
                    failed += 1;
                    if !keep_going {
                        text.push_str("(stopped; pass --keep-going to run the rest)\n");
                        break;
                    }
                }
            }
            ctx.done(text, json!({ "ok": failed == 0, "results": results }));
            if failed > 0 {
                ctx.code.set(1);
            }
        }
        Command::Skills { action } => match action.as_str() {
            "get" | "core" | "show" => ctx.done(SKILL, json!({ "skill": SKILL })),
            "list" => ctx.done("agent-phone  the core guide (`agent-phone skills get`)", json!({ "skills": ["agent-phone"] })),
            other => bail!("unknown skills action \"{other}\"; use `get` or `list`"),
        },
        Command::Mcp => mcp::serve()?,
        Command::Daemon { action } => match action {
            DaemonAction::Status => {
                let list = daemon::status();
                let text = if list.is_empty() {
                    "no daemons running".to_string()
                } else {
                    list.iter().map(|d| format!("{:<20} pid {:<7} {} commands, idle {}s", d.key, d.pid, d.served, d.idle_secs)).collect::<Vec<_>>().join("\n")
                };
                ctx.done(text, json!({ "daemons": list }));
            }
            DaemonAction::Stop => {
                let stopped = daemon::stop_all();
                ctx.done(format!("stopped {stopped} daemon{}", if stopped == 1 { "" } else { "s" }), json!({ "stopped": stopped }));
            }
        },
        Command::DaemonServe { key } => daemon::serve(&key)?,
    }
    Ok(())
}

/// Runs a parsed command in this process and returns what it printed.
pub fn execute(cli: Cli) -> Output {
    let ctx = Ctx { json: cli.json, device_arg: cli.device.clone(), dialogs: cli.dialogs, out: RefCell::new(String::new()), code: Cell::new(0) };
    match run(&ctx, cli) {
        Ok(()) => Output { stdout: ctx.out.take(), stderr: String::new(), code: ctx.code.get() },
        Err(error) => {
            // The device may have changed under us; reconnect next time.
            forget_phones();
            let mut stdout = ctx.out.take();
            let mut stderr = String::new();
            if ctx.json {
                stdout.push_str(&format!("{}\n", json!({ "ok": false, "error": format!("{error:#}") })));
            } else {
                stderr = format!("error: {error:#}\n");
            }
            Output { stdout, stderr, code: 1 }
        }
    }
}

/// Parses and runs `args` (starting with the program name) in this process.
pub fn execute_args(args: Vec<String>) -> Output {
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli),
        Err(e) => {
            let text = e.render().to_string();
            let code = e.exit_code();
            if e.use_stderr() {
                Output { stdout: String::new(), stderr: text, code }
            } else {
                Output { stdout: text, stderr: String::new(), code }
            }
        }
    }
}

/// Runs `args` through the daemon when the command uses one, otherwise here.
fn dispatch(args: Vec<String>) -> Output {
    if let Ok(cli) = Cli::try_parse_from(&args) {
        if cli.wants_daemon() {
            if let Ok(out) = daemon::forward(&cli.with_env_flags(args.clone()), cli.device.as_deref()) {
                return out;
            }
        }
        return execute(cli);
    }
    execute_args(args)
}

impl Cli {
    /// `args` with settings that came from the environment spelled out, since the daemon has its own.
    fn with_env_flags(&self, mut args: Vec<String>) -> Vec<String> {
        let given = args.iter().any(|a| a == "--dialogs" || a.starts_with("--dialogs="));
        if let (Some(answer), false) = (self.dialogs, given) {
            let value = if answer == DialogAnswer::Accept { "accept" } else { "dismiss" };
            args.splice(1..1, ["--dialogs".to_string(), value.to_string()]);
        }
        args
    }

    /// Commands that talk to a simulator go through its daemon.
    fn wants_daemon(&self) -> bool {
        if self.no_daemon || mirror::selected(self.device.as_deref()) {
            return false;
        }
        matches!(
            self.command,
            Command::Apps { .. }
                | Command::Back
                | Command::Reload
                | Command::Dblclick { .. }
                | Command::Drag { .. }
                | Command::Check { .. }
                | Command::Uncheck { .. }
                | Command::Dialog { .. }
                | Command::Audit
                | Command::Clipboard { .. }
                | Command::Diff { .. }
                | Command::Open { .. }
                | Command::Close { .. }
                | Command::Snapshot { .. }
                | Command::Click { .. }
                | Command::Fill { .. }
                | Command::Type { .. }
                | Command::Press { .. }
                | Command::Swipe { .. }
                | Command::Scroll { .. }
                | Command::Find { .. }
                | Command::Get { .. }
                | Command::Is { .. }
                | Command::Wait { .. }
                | Command::Screenshot { .. }
        )
    }
}

fn emit(out: Output) -> ! {
    use std::io::Write;
    print!("{}", out.stdout);
    eprint!("{}", out.stderr);
    let _ = std::io::stdout().flush();
    std::process::exit(out.code);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cli = Cli::parse();
    if cli.wants_daemon() {
        if let Ok(out) = daemon::forward(&cli.with_env_flags(args), cli.device.as_deref()) {
            emit(out);
        }
    }
    emit(execute(cli));
}
