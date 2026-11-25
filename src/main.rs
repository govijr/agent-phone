//! agent-phone: iPhone automation CLI for AI agents.

mod device;
mod keys;
mod native;
mod snapshot;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;
use snapshot::{Element, Session, Snapshot};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "agent-phone", version, about = "iPhone automation CLI for AI agents")]
#[command(after_help = "Targets: @e3 (a ref from `snapshot`), \"Sign in\" (text on screen), or 120,480 (points).")]
struct Cli {
    /// Simulator name or UDID. Defaults to the only booted simulator.
    #[arg(long, short = 'd', global = true, env = "AGENT_PHONE_DEVICE")]
    device: Option<String>,

    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List available simulators.
    Devices,
    /// Boot a simulator (headless) and wait until it is ready.
    Boot {
        /// Name or UDID; defaults to --device, then the newest iPhone.
        name: Option<String>,
    },
    /// Shut the simulator down.
    Shutdown,
    /// List installed apps and their bundle IDs.
    Apps,
    /// Launch an app by bundle ID, or open a URL or deep link.
    Open {
        target: String,
        /// Restart the app if it is already running.
        #[arg(long)]
        fresh: bool,
    },
    /// Terminate an app.
    Close { bundle_id: String },
    /// Install a .app bundle.
    Install { path: String },
    /// Print the screen's accessibility tree with refs.
    Snapshot {
        /// Only interactive elements (buttons, fields, switches…).
        #[arg(short, long)]
        interactive: bool,
    },
    /// Tap an element or point.
    #[command(visible_alias = "tap")]
    Click {
        target: String,
        /// Hold for this many milliseconds (a long press).
        #[arg(long, default_value_t = 0)]
        hold: u32,
    },
    /// Tap a text field, clear it, and type into it.
    Fill { target: String, text: String },
    /// Type text into the focused field.
    Type { text: String },
    /// Press a key or button: home, lock, enter, tab, backspace, escape, up, down, left, right.
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
    /// Wait for milliseconds, or until text appears (or disappears with --gone).
    Wait {
        /// Milliseconds to wait.
        ms: Option<u64>,
        #[arg(long)]
        text: Option<String>,
        /// Wait for --text to disappear instead.
        #[arg(long)]
        gone: bool,
        /// Give up after this many seconds.
        #[arg(long, default_value_t = 10.0)]
        timeout: f64,
    },
    /// Save a PNG screenshot.
    Screenshot {
        #[arg(default_value = "screenshot.png")]
        path: String,
    },
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

struct Ctx {
    json: bool,
    device_arg: Option<String>,
}

struct Phone {
    udid: String,
    native: native::Target,
}

impl Ctx {
    fn phone(&self) -> Result<Phone> {
        let device = device::resolve(self.device_arg.as_deref())?;
        if !device.booted() {
            bail!("{} is not booted. Run `agent-phone boot \"{}\"`", device.name, device.name);
        }
        let native = native::Target::new(&device::developer_dir()?, &device.udid)?;
        Ok(Phone { udid: device.udid, native })
    }

    /// Prints `text` for people, or `data` (with ok: true) for --json.
    fn done(&self, text: impl AsRef<str>, data: serde_json::Value) {
        if self.json {
            let mut data = data;
            if let Some(map) = data.as_object_mut() {
                map.insert("ok".into(), json!(true));
            }
            println!("{data}");
        } else {
            let text = text.as_ref();
            if text.ends_with('\n') {
                print!("{text}");
            } else {
                println!("{text}");
            }
        }
    }
}

impl Phone {
    fn snapshot(&self) -> Result<Snapshot> {
        Snapshot::parse(&self.native.snapshot(5.0)?)
    }

    /// Resolves a target to an element (if it names one) and a point to touch.
    fn locate(&self, target: &str, editable: bool) -> Result<(Option<Element>, (f64, f64))> {
        if let Some(point) = parse_point(target) {
            return Ok((None, point));
        }
        let snapshot = self.snapshot()?;
        let element = if let Some(reference) = target.strip_prefix('@') {
            let session = Session::load(&self.udid)?;
            let saved = session.get(reference)?;
            snapshot.relocate(saved).cloned().with_context(|| {
                format!("@{reference} ({}) is no longer on screen. Run `agent-phone snapshot` for fresh refs", saved.describe())
            })?
        } else {
            snapshot
                .find_text(target, editable)
                .cloned()
                .with_context(|| format!("nothing on screen matches \"{target}\". Run `agent-phone snapshot` to see what is there"))?
        };
        let screen = self.native.screen()?;
        let (x, y) = element.center();
        if x < 0.0 || y < 0.0 || x > screen.width || y > screen.height {
            bail!("{} is off screen; scroll to it first", element.describe());
        }
        let point = element.center();
        Ok((Some(element), point))
    }

    fn type_text(&self, text: &str) -> Result<()> {
        match keys::text(text) {
            Some(events) => self.native.keys(&events),
            None => {
                // Characters a US keyboard cannot type go through the pasteboard.
                device::set_pasteboard(&self.udid, text)?;
                self.native.keys(&keys::command('v'))
            }
        }
    }

    fn swipe(&self, direction: Direction, from: Option<&str>, distance: Option<f64>) -> Result<(f64, f64, f64, f64)> {
        let screen = self.native.screen()?;
        let (x0, y0) = match from {
            Some(target) => self.locate(target, false)?.1,
            None => (screen.width / 2.0, screen.height / 2.0),
        };
        let vertical = matches!(direction, Direction::Up | Direction::Down);
        let distance = distance.unwrap_or(if vertical { screen.height * 0.4 } else { screen.width * 0.6 });
        let (dx, dy) = match direction {
            Direction::Up => (0.0, -distance),
            Direction::Down => (0.0, distance),
            Direction::Left => (-distance, 0.0),
            Direction::Right => (distance, 0.0),
        };
        let clamp = |v: f64, max: f64| v.clamp(1.0, max - 1.0);
        let (x1, y1) = (clamp(x0 + dx, screen.width), clamp(y0 + dy, screen.height));
        let steps = 12;
        let points: Vec<(f64, f64)> = (0..=steps)
            .map(|i| {
                let t = i as f64 / steps as f64;
                (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t)
            })
            .collect();
        self.native.touch(&points, 0, 16)?;
        Ok((x0, y0, x1, y1))
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

fn run(cli: Cli) -> Result<()> {
    let ctx = Ctx { json: cli.json, device_arg: cli.device.clone() };
    match cli.command {
        Command::Devices => {
            let devices = device::list()?;
            let mut text = String::new();
            for d in &devices {
                let mark = if d.booted() { "● booted" } else { "" };
                text.push_str(&format!("{:<24} {:<9} {}  {}\n", d.name, d.runtime, d.udid, mark));
            }
            let data: Vec<_> = devices
                .iter()
                .map(|d| json!({"name": d.name, "udid": d.udid, "runtime": d.runtime, "booted": d.booted()}))
                .collect();
            ctx.done(text, json!({ "devices": data }));
        }
        Command::Boot { name } => {
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
            ctx.done(
                format!("booted {} ({}) in {:.1}s", device.name, device.udid, started.elapsed().as_secs_f64()),
                json!({ "name": device.name, "udid": device.udid }),
            );
        }
        Command::Shutdown => {
            let device = device::resolve(ctx.device_arg.as_deref())?;
            device::shutdown(&device)?;
            ctx.done(format!("shut down {}", device.name), json!({ "udid": device.udid }));
        }
        Command::Apps => {
            let phone = ctx.phone()?;
            let apps = device::apps(&phone.udid)?;
            let mut text = String::new();
            for app in &apps {
                let kind = if app.user { "user" } else { "system" };
                text.push_str(&format!("{:<28} {:<7} {}\n", app.name, kind, app.bundle_id));
            }
            ctx.done(text, json!({ "apps": apps }));
        }
        Command::Open { target, fresh } => {
            let phone = ctx.phone()?;
            if target.contains("://") {
                device::open_url(&phone.udid, &target)?;
            } else {
                device::launch(&phone.udid, &target, fresh)?;
            }
            ctx.done(format!("opened {target}"), json!({ "target": target }));
        }
        Command::Close { bundle_id } => {
            let phone = ctx.phone()?;
            device::terminate(&phone.udid, &bundle_id)?;
            ctx.done(format!("closed {bundle_id}"), json!({ "bundle_id": bundle_id }));
        }
        Command::Install { path } => {
            let phone = ctx.phone()?;
            device::install(&phone.udid, &path)?;
            ctx.done(format!("installed {path}"), json!({ "path": path }));
        }
        Command::Snapshot { interactive } => {
            let phone = ctx.phone()?;
            let snapshot = phone.snapshot()?;
            Session::save(&phone.udid, &snapshot)?;
            let elements: Vec<&Element> =
                snapshot.elements.iter().filter(|e| !interactive || e.interactive).collect();
            ctx.done(snapshot.render(interactive), json!({ "elements": elements }));
        }
        Command::Click { target, hold } => {
            let phone = ctx.phone()?;
            let (element, point) = phone.locate(&target, false)?;
            phone.native.touch(&[point], hold.max(30), 0)?;
            let verb = if hold >= 400 { "long-pressed" } else { "clicked" };
            ctx.done(format!("{verb} {}", target_label(&element, point, &target)), element_json(&element, point));
        }
        Command::Fill { target, text } => {
            let phone = ctx.phone()?;
            let (element, point) = phone.locate(&target, true)?;
            phone.native.touch(&[point], 30, 0)?;
            std::thread::sleep(Duration::from_millis(350));
            phone.native.keys(&keys::command('a'))?;
            phone.native.keys(&keys::backspace())?;
            phone.type_text(&text)?;
            ctx.done(format!("filled {}", target_label(&element, point, &target)), element_json(&element, point));
        }
        Command::Type { text } => {
            let phone = ctx.phone()?;
            phone.type_text(&text)?;
            ctx.done(format!("typed {} characters", text.chars().count()), json!({ "length": text.chars().count() }));
        }
        Command::Press { key } => {
            let phone = ctx.phone()?;
            match key.to_lowercase().as_str() {
                "home" => phone.native.button(native::Button::Home)?,
                "lock" | "power" => phone.native.button(native::Button::Lock)?,
                other => match keys::named(other) {
                    Some(events) => phone.native.keys(&events)?,
                    None => bail!(
                        "unknown key \"{key}\". Use home, lock, enter, tab, backspace, escape, space, up, down, left or right"
                    ),
                },
            }
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
            let (x0, y0, x1, y1) = phone.swipe(direction.opposite(), from.as_deref(), distance)?;
            ctx.done("scrolled", json!({ "from": [x0, y0], "to": [x1, y1] }));
        }
        Command::Wait { ms, text, gone, timeout } => match (ms, text) {
            (Some(ms), None) => {
                std::thread::sleep(Duration::from_millis(ms));
                ctx.done(format!("waited {ms}ms"), json!({ "ms": ms }));
            }
            (None, Some(text)) => {
                let phone = ctx.phone()?;
                let started = Instant::now();
                loop {
                    let present = phone.snapshot().map(|s| s.contains_text(&text)).unwrap_or(false);
                    if present != gone {
                        let state = if gone { "gone" } else { "visible" };
                        let secs = started.elapsed().as_secs_f64();
                        ctx.done(format!("\"{text}\" is {state} after {secs:.1}s"), json!({ "text": text, "seconds": secs }));
                        break;
                    }
                    if started.elapsed().as_secs_f64() > timeout {
                        let state = if gone { "still on screen" } else { "not on screen" };
                        bail!("\"{text}\" was {state} after {timeout}s");
                    }
                    std::thread::sleep(Duration::from_millis(400));
                }
            }
            _ => bail!("pass milliseconds (`wait 500`) or --text (`wait --text \"Welcome\"`)"),
        },
        Command::Screenshot { path } => {
            let phone = ctx.phone()?;
            device::screenshot(&phone.udid, &path)?;
            ctx.done(format!("saved {path}"), json!({ "path": path }));
        }
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(error) = run(cli) {
        if json {
            println!("{}", json!({ "ok": false, "error": format!("{error:#}") }));
        } else {
            eprintln!("error: {error:#}");
        }
        std::process::exit(1);
    }
}
