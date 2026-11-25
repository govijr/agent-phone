//! Accessibility trees, `@e` refs, and finding elements again on a changed screen.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct RawNode {
    role: Option<String>,
    subrole: Option<String>,
    label: Option<String>,
    value: Option<String>,
    identifier: Option<String>,
    placeholder: Option<String>,
    #[serde(default)]
    enabled: serde_json::Value,
    #[serde(default)]
    focused: serde_json::Value,
    frame: [f64; 4],
    #[serde(default)]
    children: Vec<RawNode>,
}

/// One element on screen, with the ref it was given.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    #[serde(rename = "ref")]
    pub reference: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub interactive: bool,
    /// x, y, width, height in screen points.
    pub frame: [f64; 4],
    pub depth: usize,
}

impl Element {
    pub fn center(&self) -> (f64, f64) {
        (self.frame[0] + self.frame[2] / 2.0, self.frame[1] + self.frame[3] / 2.0)
    }

    /// A short human description, such as `button "Sign in"`.
    pub fn describe(&self) -> String {
        match self.name() {
            Some(name) => format!("{} \"{}\"", self.role, name),
            None => self.role.clone(),
        }
    }

    pub fn editable(&self) -> bool {
        matches!(self.role.as_str(), "textbox" | "searchbox" | "password")
    }

    fn name(&self) -> Option<&str> {
        self.label
            .as_deref()
            .or(self.placeholder.as_deref())
            .or(self.identifier.as_deref())
            .or(self.value.as_deref())
    }

    fn same_element(&self, other: &Element) -> bool {
        self.role == other.role && self.label == other.label && self.identifier == other.identifier
            && self.placeholder == other.placeholder
    }

    fn texts(&self) -> impl Iterator<Item = &str> {
        [&self.label, &self.value, &self.placeholder, &self.identifier].into_iter().flatten().map(|s| s.as_str())
    }
}

pub struct Snapshot {
    pub elements: Vec<Element>,
}

const INTERACTIVE: &[&str] = &[
    "AXButton",
    "AXTextField",
    "AXSecureTextField",
    "AXTextArea",
    "AXSearchField",
    "AXSlider",
    "AXSwitch",
    "AXCheckBox",
    "AXLink",
    "AXPopUpButton",
    "AXComboBox",
    "AXIncrementor",
    "AXMenuItem",
    "AXRadioButton",
    "AXCell",
    "AXTab",
    "AXDisclosureTriangle",
    "AXSegmentedControl",
];

fn role_name(role: &str, subrole: Option<&str>) -> String {
    let name = match (role, subrole) {
        ("AXTextField", Some("AXSearchField")) | ("AXSearchField", _) => "searchbox",
        ("AXTextField", Some("AXSecureTextField")) | ("AXSecureTextField", _) => "password",
        ("AXTextField", _) | ("AXTextArea", _) => "textbox",
        ("AXStaticText", _) => "text",
        ("AXCheckBox", Some("AXSwitch")) | ("AXSwitch", _) => "switch",
        ("AXCheckBox", _) => "checkbox",
        ("AXPopUpButton", _) => "popup",
        ("AXRadioButton", _) => "radio",
        ("AXApplication", _) => "app",
        _ => return role.trim_start_matches("AX").to_lowercase(),
    };
    name.to_string()
}

fn truthy(value: &serde_json::Value, default: bool) -> bool {
    match value {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().map(|n| n != 0.0).unwrap_or(default),
        _ => default,
    }
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl Snapshot {
    pub fn parse(json: &str) -> Result<Self> {
        let root: RawNode = serde_json::from_str(json).context("could not parse the accessibility tree")?;
        let mut elements = Vec::new();
        for child in root.children {
            flatten(child, 0, &mut elements);
        }
        for (i, element) in elements.iter_mut().enumerate() {
            element.reference = format!("e{}", i + 1);
        }
        Ok(Self { elements })
    }

    pub fn render(&self, interactive_only: bool) -> String {
        let mut out = String::new();
        for e in self.elements.iter().filter(|e| !interactive_only || e.interactive) {
            let indent = if interactive_only { 0 } else { e.depth };
            out.push_str(&"  ".repeat(indent));
            out.push_str("- ");
            out.push_str(&e.describe());
            out.push_str(&format!(" [ref={}]", e.reference));
            if let Some(value) = &e.value {
                if e.label.is_some() || e.placeholder.is_some() {
                    out.push_str(&format!(" [value=\"{}\"]", value));
                }
            }
            if let (Some(id), Some(label)) = (&e.identifier, &e.label) {
                if id != label {
                    out.push_str(&format!(" [id={id}]"));
                }
            }
            if !e.enabled {
                out.push_str(" [disabled]");
            }
            if e.focused {
                out.push_str(" [focused]");
            }
            out.push('\n');
        }
        if out.is_empty() {
            out.push_str("(no elements; the app may still be loading. Try again or `agent-phone screenshot`)\n");
        }
        out
    }

    /// The element matching `saved` on this (possibly changed) screen, nearest its old position.
    pub fn relocate(&self, saved: &Element) -> Option<&Element> {
        let (sx, sy) = saved.center();
        self.elements.iter().filter(|e| e.same_element(saved)).min_by(|a, b| {
            let da = (a.center().0 - sx).powi(2) + (a.center().1 - sy).powi(2);
            let db = (b.center().0 - sx).powi(2) + (b.center().1 - sy).powi(2);
            da.total_cmp(&db)
        })
    }

    /// An element whose text is `query`: exact matches first, then contains; interactive first.
    /// With `editable`, text fields win over everything else.
    pub fn find_text(&self, query: &str, editable: bool) -> Option<&Element> {
        let q = query.to_lowercase();
        let rank = |e: &&Element| -> Option<(u8, u8, u8)> {
            let exact = e.texts().any(|t| t.to_lowercase() == q);
            let contains = e.texts().any(|t| t.to_lowercase().contains(&q));
            if !exact && !contains {
                return None;
            }
            let field = if editable && e.editable() { 0 } else { 1 };
            Some((field, if exact { 0 } else { 1 }, if e.interactive { 0 } else { 1 }))
        };
        self.elements.iter().filter_map(|e| rank(&e).map(|r| (r, e))).min_by_key(|(r, _)| *r).map(|(_, e)| e)
    }

    pub fn contains_text(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.elements.iter().any(|e| e.texts().any(|t| t.to_lowercase().contains(&q)))
    }
}

fn flatten(node: RawNode, depth: usize, out: &mut Vec<Element>) {
    let role_raw = node.role.clone().unwrap_or_default();
    let label = clean(node.label);
    let value = clean(node.value);
    let identifier = clean(node.identifier);
    let placeholder = clean(node.placeholder);
    let interactive = INTERACTIVE.contains(&role_raw.as_str()) || node.subrole.as_deref() == Some("AXSwitch");
    let visible = node.frame[2] > 0.0 && node.frame[3] > 0.0;
    let has_text = label.is_some() || value.is_some() || identifier.is_some() || placeholder.is_some();
    let keep = visible && (interactive || has_text);
    let child_depth = if keep { depth + 1 } else { depth };
    if keep {
        out.push(Element {
            reference: String::new(),
            role: role_name(&role_raw, node.subrole.as_deref()),
            label,
            value,
            identifier,
            placeholder,
            enabled: truthy(&node.enabled, true),
            focused: truthy(&node.focused, false),
            interactive,
            frame: node.frame,
            depth,
        });
    }
    for child in node.children {
        flatten(child, child_depth, out);
    }
}

/// The refs from the last snapshot of a device, kept between commands.
#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    pub elements: Vec<Element>,
}

fn session_path(udid: &str) -> Result<PathBuf> {
    let base = match std::env::var_os("AGENT_PHONE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".agent-phone"),
    };
    std::fs::create_dir_all(base.join("sessions"))?;
    Ok(base.join("sessions").join(format!("{udid}.json")))
}

impl Session {
    pub fn save(udid: &str, snapshot: &Snapshot) -> Result<()> {
        let session = Session { elements: snapshot.elements.clone() };
        std::fs::write(session_path(udid)?, serde_json::to_vec(&session)?)?;
        Ok(())
    }

    pub fn load(udid: &str) -> Result<Self> {
        let path = session_path(udid)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
            Err(_) => Ok(Session::default()),
        }
    }

    pub fn get(&self, reference: &str) -> Result<&Element> {
        match self.elements.iter().find(|e| e.reference == reference) {
            Some(e) => Ok(e),
            None if self.elements.is_empty() => bail!("no snapshot yet. Run `agent-phone snapshot` to get refs"),
            None => bail!("@{reference} is not in the last snapshot. Run `agent-phone snapshot` for fresh refs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"{"role":"AXApplication","frame":[0,0,402,874],"children":[
        {"role":"AXStaticText","label":"Welcome","frame":[10,10,100,20],"enabled":1,"children":[]},
        {"role":"AXGroup","frame":[0,40,402,200],"children":[
            {"role":"AXTextField","placeholder":"Email","frame":[10,50,300,40],"enabled":1,"focused":1,"children":[]},
            {"role":"AXButton","label":"Sign in","frame":[10,100,300,44],"enabled":0,"children":[]}
        ]},
        {"role":"AXButton","label":"Hidden","frame":[0,0,0,0],"children":[]}
    ]}"#;

    #[test]
    fn assigns_refs_and_skips_empty_or_invisible_nodes() {
        let s = Snapshot::parse(TREE).unwrap();
        let refs: Vec<String> = s.elements.iter().map(|e| e.describe()).collect();
        assert_eq!(refs, vec!["text \"Welcome\"", "textbox \"Email\"", "button \"Sign in\""]);
        assert_eq!(s.elements[2].reference, "e3");
    }

    #[test]
    fn renders_interactive_elements_with_state() {
        let s = Snapshot::parse(TREE).unwrap();
        assert_eq!(
            s.render(true),
            "- textbox \"Email\" [ref=e2] [focused]\n- button \"Sign in\" [ref=e3] [disabled]\n"
        );
    }

    #[test]
    fn relocates_a_moved_element_and_finds_text() {
        let s = Snapshot::parse(TREE).unwrap();
        let mut saved = s.elements[2].clone();
        saved.frame[1] = 400.0;
        assert_eq!(s.relocate(&saved).unwrap().reference, "e3");
        assert_eq!(s.find_text("sign", false).unwrap().reference, "e3");
        assert_eq!(s.find_text("email", false).unwrap().reference, "e2");
        assert!(s.find_text("nope", false).is_none());
    }

    #[test]
    fn fill_prefers_text_fields_over_buttons_with_the_same_text() {
        let tree = r#"{"role":"AXApplication","frame":[0,0,402,874],"children":[
            {"role":"AXButton","label":"Search","frame":[0,100,402,44],"children":[]},
            {"role":"AXTextField","subrole":"AXSearchField","label":"Search","frame":[0,800,402,44],"children":[]}
        ]}"#;
        let s = Snapshot::parse(tree).unwrap();
        assert_eq!(s.find_text("Search", false).unwrap().reference, "e1");
        assert_eq!(s.find_text("Search", true).unwrap().reference, "e2");
    }
}
