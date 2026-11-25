//! Finding Xcode and the target simulator, and the `xcrun simctl` operations.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Command;

pub fn developer_dir() -> Result<String> {
    if let Ok(dir) = std::env::var("DEVELOPER_DIR") {
        if !dir.is_empty() {
            return Ok(dir);
        }
    }
    let out = Command::new("xcode-select").arg("-p").output().context("could not run xcode-select")?;
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || dir.is_empty() {
        bail!("Xcode not found. Install Xcode, open it once, then run `sudo xcode-select -s /Applications/Xcode.app`");
    }
    if dir.contains("CommandLineTools") {
        bail!("xcode-select points at the Command Line Tools ({dir}). Run `sudo xcode-select -s /Applications/Xcode.app`");
    }
    Ok(dir)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub state: String,
    #[serde(skip)]
    pub runtime: String,
}

impl Device {
    pub fn booted(&self) -> bool {
        self.state == "Booted"
    }
}

#[derive(Deserialize)]
struct DeviceList {
    devices: BTreeMap<String, Vec<Device>>,
}

/// Every available iOS simulator, newest runtime first.
pub fn list() -> Result<Vec<Device>> {
    let json = simctl(&["list", "devices", "available", "-j"])?;
    let list: DeviceList = serde_json::from_str(&json).context("could not parse `simctl list`")?;
    let mut devices = Vec::new();
    for (runtime, entries) in list.devices.into_iter().rev() {
        if !runtime.contains("iOS") {
            continue;
        }
        let version = runtime.rsplit("iOS-").next().unwrap_or(&runtime).replace('-', ".");
        for mut device in entries {
            device.runtime = format!("iOS {version}");
            devices.push(device);
        }
    }
    Ok(devices)
}

/// Resolves `--device` (a UDID or a name) or, when it is absent, the single booted simulator.
pub fn resolve(wanted: Option<&str>) -> Result<Device> {
    let devices = list()?;
    if let Some(wanted) = wanted {
        let lowered = wanted.to_lowercase();
        let mut matches: Vec<&Device> = devices
            .iter()
            .filter(|d| d.udid.eq_ignore_ascii_case(wanted) || d.name.to_lowercase() == lowered)
            .collect();
        // Prefer a booted one when a name matches several runtimes.
        matches.sort_by_key(|d| !d.booted());
        return matches
            .first()
            .map(|d| (*d).clone())
            .with_context(|| format!("no simulator named or with UDID \"{wanted}\". Run `agent-phone devices`"));
    }
    let booted: Vec<&Device> = devices.iter().filter(|d| d.booted()).collect();
    match booted.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no simulator is booted. Run `agent-phone boot` (or `agent-phone boot \"iPhone 17 Pro\"`)"),
        many => {
            let names: Vec<String> = many.iter().map(|d| format!("{} ({})", d.name, d.udid)).collect();
            bail!("{} simulators are booted; pick one with --device: {}", many.len(), names.join(", "))
        }
    }
}

pub fn boot(device: &Device) -> Result<()> {
    if !device.booted() {
        simctl(&["boot", &device.udid])?;
    }
    simctl(&["bootstatus", &device.udid, "-b"])?;
    Ok(())
}

pub fn shutdown(device: &Device) -> Result<()> {
    simctl(&["shutdown", &device.udid])?;
    Ok(())
}

pub fn launch(udid: &str, bundle_id: &str, fresh: bool) -> Result<()> {
    let mut args = vec!["launch"];
    if fresh {
        args.push("--terminate-running-process");
    }
    args.extend([udid, bundle_id]);
    simctl(&args)?;
    Ok(())
}

pub fn open_url(udid: &str, url: &str) -> Result<()> {
    simctl(&["openurl", udid, url])?;
    Ok(())
}

pub fn terminate(udid: &str, bundle_id: &str) -> Result<()> {
    simctl(&["terminate", udid, bundle_id])?;
    Ok(())
}

pub fn install(udid: &str, path: &str) -> Result<()> {
    simctl(&["install", udid, path])?;
    Ok(())
}

pub fn screenshot(udid: &str, path: &str) -> Result<()> {
    simctl(&["io", udid, "screenshot", "--type=png", path])?;
    Ok(())
}

pub fn set_pasteboard(udid: &str, text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("xcrun")
        .args(["simctl", "pbcopy", udid])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("could not run simctl pbcopy")?;
    child.stdin.take().context("no stdin for pbcopy")?.write_all(text.as_bytes())?;
    if !child.wait()?.success() {
        bail!("simctl pbcopy failed");
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct App {
    pub bundle_id: String,
    pub name: String,
    pub user: bool,
}

/// Installed apps, user apps first.
pub fn apps(udid: &str) -> Result<Vec<App>> {
    let plist = simctl(&["listapps", udid])?;
    let mut convert = Command::new("plutil")
        .args(["-convert", "json", "-r", "-o", "-", "--", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("could not run plutil")?;
    {
        use std::io::Write;
        convert.stdin.take().context("no stdin for plutil")?.write_all(plist.as_bytes())?;
    }
    let out = convert.wait_with_output()?;
    let map: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&out.stdout).context("could not parse `simctl listapps`")?;
    let mut apps: Vec<App> = map
        .into_iter()
        .map(|(bundle_id, info)| {
            let name = info
                .get("CFBundleDisplayName")
                .or_else(|| info.get("CFBundleName"))
                .and_then(|v| v.as_str())
                .unwrap_or(&bundle_id)
                .to_string();
            let user = info.get("ApplicationType").and_then(|v| v.as_str()) == Some("User");
            App { bundle_id, name, user }
        })
        .collect();
    apps.sort_by(|a, b| b.user.cmp(&a.user).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(apps)
}

fn simctl(args: &[&str]) -> Result<String> {
    let out = Command::new("xcrun")
        .arg("simctl")
        .args(args)
        .output()
        .context("could not run xcrun simctl")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let message = stderr
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("Underlying error"))
            .unwrap_or("unknown error")
            .trim()
            .to_string();
        bail!("simctl {}: {}", args[0], message);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
