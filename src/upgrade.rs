//! `agent-phone upgrade`: replaces this binary with the newest GitHub release, after checking
//! the download against its published SHA-256.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub enum Outcome {
    UpToDate(String),
    Available(String),
    Upgraded { from: String, to: String, path: PathBuf },
}

fn repo() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").trim_end_matches('/')
}

fn curl(url: &str, out: Option<&Path>) -> Result<Vec<u8>> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--retry", "2", "-H", "User-Agent: agent-phone-upgrade", url]);
    if let Some(out) = out {
        cmd.arg("-o").arg(out);
    }
    let output = cmd.output().context("could not run curl")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.contains("404") {
            bail!("404");
        }
        bail!("download failed: {}", err.trim());
    }
    Ok(output.stdout)
}

fn parse(version: &str) -> Vec<u64> {
    version.trim_start_matches('v').split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn run(check_only: bool) -> Result<Outcome> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let exe = std::env::current_exe()?.canonicalize()?;
    let shown = exe.to_string_lossy();
    if shown.contains("/Cellar/") {
        bail!("agent-phone was installed with Homebrew; run `brew upgrade agent-phone`");
    }
    if shown.contains("/target/") {
        bail!("this is a development build ({shown}); update with `git pull && cargo install --path .`");
    }
    let api = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let release: serde_json::Value = match curl(&api, None) {
        Ok(body) => serde_json::from_slice(&body).context("GitHub sent a release agent-phone can't read")?,
        Err(e) if e.to_string() == "404" => bail!("no releases are published yet at github.com/{}", repo()),
        Err(e) => return Err(e),
    };
    let tag = release["tag_name"].as_str().context("the latest release has no tag")?.to_string();
    if parse(&tag) <= parse(&current) {
        return Ok(Outcome::UpToDate(current));
    }
    if check_only {
        return Ok(Outcome::Available(tag));
    }
    let name = format!("agent-phone-{tag}-aarch64-apple-darwin.tar.gz");
    let asset = |file: &str| -> Result<String> {
        release["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == file))
            .and_then(|x| x["browser_download_url"].as_str())
            .map(String::from)
            .with_context(|| format!("release {tag} has no {file}"))
    };
    let dir = std::env::temp_dir().join(format!("agent-phone-upgrade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let archive = dir.join(&name);
    curl(&asset(&name)?, Some(&archive))?;
    let expected = String::from_utf8(curl(&asset(&format!("{name}.sha256"))?, None)?)?;
    let expected = expected.split_whitespace().next().context("empty checksum file")?.to_lowercase();
    let actual = Command::new("shasum").args(["-a", "256"]).arg(&archive).output()?;
    let actual = String::from_utf8_lossy(&actual.stdout).split_whitespace().next().unwrap_or_default().to_lowercase();
    if actual != expected {
        bail!("the download's checksum doesn't match the release ({actual} vs {expected}); nothing was changed");
    }
    let status = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&dir).status()?;
    if !status.success() {
        bail!("could not unpack {name}");
    }
    let fresh = dir.join("agent-phone");
    let reported = Command::new(&fresh).arg("--version").output().context("the new binary doesn't run")?;
    if !String::from_utf8_lossy(&reported.stdout).contains(tag.trim_start_matches('v')) {
        bail!("the new binary reports the wrong version; nothing was changed");
    }
    // Copy next to the old binary, then rename over it, so a failure never leaves it half-written.
    let staged = exe.with_extension("new");
    std::fs::copy(&fresh, &staged).with_context(|| format!("could not write next to {shown} (try with sudo?)"))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&staged, &exe)?;
    let _ = std::fs::remove_dir_all(&dir);
    crate::daemon::stop_all();
    Ok(Outcome::Upgraded { from: current, to: tag, path: exe })
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::parse("v0.10.0") > super::parse("0.9.3"));
        assert!(super::parse("v0.1.0") <= super::parse("0.1.0"));
        assert_eq!(super::repo(), "govijr/agent-phone");
    }
}
