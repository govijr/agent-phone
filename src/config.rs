//! `agent-phone.json`: defaults and the action policy, from the project folder (or a parent)
//! and from `~/.agent-phone/config.json`. Project settings win over user settings, and flags and
//! environment variables win over both.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// Default `--device`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Default `--dialogs`: "accept" or "dismiss".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialogs: Option<String>,
    /// false runs every command in-process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<bool>,
    /// Seconds an idle daemon waits before exiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_idle_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
}

/// What agents may do. Words match whole words in an element's text, ignoring case.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// Apps that may be opened and used (bundle IDs, or names on an iPhone). `*` matches
    /// anything, as in "com.example.*". Empty allows every app.
    pub allow_apps: Vec<String>,
    /// Taps on elements with these words wait for `agent-phone confirm <id>`. `true` uses the
    /// built-in list (buy, pay, send, delete…); `false` turns it off.
    pub confirm: Option<WordList>,
    /// Taps on elements with these words are always refused.
    pub deny: Vec<String>,
    /// `confirm` must be typed by a person at a terminal, not run by an agent.
    pub human_confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WordList {
    Default(bool),
    Words(Vec<String>),
}

/// Words on controls that spend money, send things, or destroy data.
pub const RISKY: &[&str] = &[
    "buy", "purchase", "pay", "checkout", "place order", "order", "subscribe", "send", "post", "publish", "share",
    "delete", "remove", "erase", "transfer", "sign out", "log out", "call", "confirm",
];

impl Policy {
    /// The words that need confirmation. On a real iPhone the built-in list applies unless the
    /// policy turns it off; on a simulator only when asked for.
    pub fn confirm_words(&self, real_phone: bool) -> Vec<String> {
        match &self.confirm {
            Some(WordList::Words(w)) => w.clone(),
            Some(WordList::Default(true)) => RISKY.iter().map(|s| s.to_string()).collect(),
            Some(WordList::Default(false)) => Vec::new(),
            None if real_phone => RISKY.iter().map(|s| s.to_string()).collect(),
            None => Vec::new(),
        }
    }

    pub fn app_allowed(&self, app: &str) -> bool {
        self.allow_apps.is_empty() || self.allow_apps.iter().any(|pattern| glob(pattern, app))
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.to_lowercase(), text.to_lowercase());
    match p.split_once('*') {
        None => p == t,
        Some((head, tail)) => t.len() >= head.len() + tail.len() && t.starts_with(head) && t.ends_with(tail),
    }
}

/// The first word in `words` that appears as a whole word (or phrase) in `text`.
pub fn matching_word<'a>(text: &str, words: &'a [String]) -> Option<&'a str> {
    let text = text.to_lowercase();
    words.iter().map(String::as_str).find(|word| {
        let word = word.to_lowercase();
        text.match_indices(&word).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + word.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

fn user_path() -> Result<PathBuf> {
    Ok(crate::snapshot::home()?.join("config.json"))
}

/// The nearest `agent-phone.json` in this folder or a parent.
fn project_path() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("agent-phone.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn read(path: &Path) -> Result<Config> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("{} is not a valid agent-phone config", path.display()))
}

/// The merged config and the files it came from.
pub fn load() -> Result<(Config, Vec<PathBuf>)> {
    let mut sources = Vec::new();
    let mut config = Config::default();
    for path in [user_path().ok().filter(|p| p.is_file()), project_path()].into_iter().flatten() {
        let layer = read(&path)?;
        config.device = layer.device.or(config.device);
        config.dialogs = layer.dialogs.or(config.dialogs);
        config.daemon = layer.daemon.or(config.daemon);
        config.daemon_idle_seconds = layer.daemon_idle_seconds.or(config.daemon_idle_seconds);
        config.policy = layer.policy.or(config.policy);
        sources.push(path);
    }
    Ok((config, sources))
}

/// Applies config defaults as environment variables, which the flags read, unless already set.
pub fn apply_defaults(config: &Config) {
    let set = |key: &str, value: Option<String>| {
        if let (Some(value), true) = (value, std::env::var_os(key).is_none()) {
            std::env::set_var(key, value);
        }
    };
    set("AGENT_PHONE_DEVICE", config.device.clone());
    set("AGENT_PHONE_DIALOGS", config.dialogs.clone());
    set("AGENT_PHONE_NO_DAEMON", config.daemon.filter(|d| !d).map(|_| "1".to_string()));
    set("AGENT_PHONE_DAEMON_IDLE", config.daemon_idle_seconds.map(|s| s.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_whole_words_only() {
        let words: Vec<String> = RISKY.iter().map(|s| s.to_string()).collect();
        assert_eq!(matching_word("Buy Now", &words), Some("buy"));
        assert_eq!(matching_word("Send Message", &words), Some("send"));
        assert_eq!(matching_word("Calendar", &words), None);
        assert_eq!(matching_word("Settings", &words), None);
        assert_eq!(matching_word("Log Out of iCloud", &words), Some("log out"));
    }

    #[test]
    fn globs_bundle_ids() {
        let policy = Policy { allow_apps: vec!["com.example.*".into(), "com.apple.Preferences".into()], ..Default::default() };
        assert!(policy.app_allowed("com.example.app"));
        assert!(policy.app_allowed("com.apple.preferences"));
        assert!(!policy.app_allowed("com.apple.mobilesafari"));
        assert!(Policy::default().app_allowed("anything"));
    }

    #[test]
    fn parses_configs() {
        let c: Config = serde_json::from_str(r#"{"device":"iPhone 17","policy":{"allowApps":["com.x.*"],"confirm":true,"deny":["Delete Account"]}}"#).unwrap();
        let p = c.policy.unwrap();
        assert_eq!(p.confirm_words(false).len(), RISKY.len());
        assert!(serde_json::from_str::<Config>(r#"{"devise":"x"}"#).is_err());
    }
}
