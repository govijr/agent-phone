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
    #[serde(default)]
    selected: serde_json::Value,
    frame: [f64; 4],
    #[serde(default)]
    children: Vec<RawNode>,
    #[serde(default)]
    pid: Option<i64>,
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
    #[serde(default)]
    pub selected: bool,
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

    /// Where to tap. A switch whose frame covers its whole row (as in Settings) is tapped on the
    /// toggle at the right end, since the label part doesn't flip it.
    pub fn tap_point(&self) -> (f64, f64) {
        let (x, y) = self.center();
        if self.role == "switch" && self.frame[2] > 90.0 {
            return (self.frame[0] + self.frame[2] - 26.0, y);
        }
        (x, y)
    }

    /// A switch or checkbox that is on.
    pub fn on(&self) -> bool {
        matches!(self.value.as_deref(), Some("1" | "true" | "on" | "On"))
    }

    pub fn name(&self) -> Option<&str> {
        self.label
            .as_deref()
            .or(self.placeholder.as_deref())
            .or(self.identifier.as_deref())
            .or(self.value.as_deref())
    }

    pub fn same_element(&self, other: &Element) -> bool {
        self.role == other.role && self.label == other.label && self.identifier == other.identifier
            && self.placeholder == other.placeholder
    }

    pub fn texts(&self) -> impl Iterator<Item = &str> {
        [&self.label, &self.value, &self.placeholder, &self.identifier].into_iter().flatten().map(|s| s.as_str())
    }
}

pub struct Snapshot {
    pub elements: Vec<Element>,
    /// The frontmost app's name, when the tree says.
    pub app: Option<String>,
    /// The frontmost process (SpringBoard for the home screen and system alerts).
    pub pid: Option<i64>,
}

/// What `find` searches by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    Role,
    Text,
    Label,
    Id,
    Placeholder,
    Value,
}

/// A semantic locator: `find role button --name Done`, `find id login-button`.
#[derive(Clone, Debug)]
pub struct Query {
    pub by: By,
    pub value: String,
    /// With `role`: the element's label (or other text) must also match this.
    pub name: Option<String>,
    pub exact: bool,
}

impl Query {
    fn matches(&self, e: &Element) -> bool {
        let fit = |text: Option<&str>, wanted: &str| -> bool {
            let Some(text) = text else { return false };
            let (t, w) = (text.to_lowercase(), wanted.to_lowercase());
            if self.exact {
                t == w
            } else {
                t.contains(&w)
            }
        };
        let primary = match self.by {
            By::Role => e.role.eq_ignore_ascii_case(&self.value),
            By::Text => e.texts().any(|t| fit(Some(t), &self.value)),
            By::Label => fit(e.label.as_deref(), &self.value),
            By::Id => fit(e.identifier.as_deref(), &self.value),
            By::Placeholder => fit(e.placeholder.as_deref(), &self.value),
            By::Value => fit(e.value.as_deref(), &self.value),
        };
        primary
            && match &self.name {
                Some(name) => e.texts().any(|t| fit(Some(t), name)),
                None => true,
            }
    }

    pub fn describe(&self) -> String {
        let by = format!("{:?}", self.by).to_lowercase();
        match &self.name {
            Some(name) => format!("{by} {} named \"{name}\"", self.value),
            None => format!("{by} \"{}\"", self.value),
        }
    }
}

/// How much of the tree `render` prints.
#[derive(Default, Clone)]
pub struct RenderOptions {
    pub interactive: bool,
    /// One element per line with no indenting.
    pub flat: bool,
    /// Deepest level to print, counted from the top (or from the scope).
    pub max_depth: Option<usize>,
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
        let app = clean(root.label.clone());
        let pid = root.pid;
        let mut elements = Vec::new();
        for child in root.children {
            flatten(child, 0, &mut elements);
        }
        for (i, element) in elements.iter_mut().enumerate() {
            element.reference = format!("e{}", i + 1);
        }
        Ok(Self { elements, app, pid })
    }

    /// A snapshot from recognized text (iPhone Mirroring), in reading order. Text can't say
    /// whether it is a button, so every line counts as something to tap.
    pub fn from_text(mut items: Vec<(String, [f64; 4])>) -> Self {
        // Group into rows (centers within half a line height of the row's first item), then read
        // each row left to right.
        let center = |f: &[f64; 4]| f[1] + f[3] / 2.0;
        items.sort_by(|a, b| center(&a.1).total_cmp(&center(&b.1)));
        let mut rows: Vec<Vec<(String, [f64; 4])>> = Vec::new();
        for item in items {
            match rows.last_mut() {
                Some(row) if (center(&item.1) - center(&row[0].1)).abs() < row[0].1[3].min(item.1[3]) / 2.0 => row.push(item),
                _ => rows.push(vec![item]),
            }
        }
        let elements = rows
            .into_iter()
            .flat_map(|mut row| {
                row.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
                row
            })
            .enumerate()
            .map(|(i, (text, frame))| Element {
                reference: format!("e{}", i + 1),
                role: "text".into(),
                label: Some(text.trim().to_string()),
                value: None,
                identifier: None,
                placeholder: None,
                enabled: true,
                focused: false,
                selected: false,
                interactive: true,
                frame,
                depth: 0,
            })
            .collect();
        Self { elements, app: None, pid: None }
    }

    pub fn render(&self, interactive_only: bool) -> String {
        self.render_with(&self.elements.iter().collect::<Vec<_>>(), &RenderOptions { interactive: interactive_only, ..Default::default() })
    }

    /// Prints `elements` (all of them, or a subtree) as an indented list.
    pub fn render_with(&self, elements: &[&Element], options: &RenderOptions) -> String {
        let base = elements.iter().map(|e| e.depth).min().unwrap_or(0);
        let mut out = String::new();
        for e in elements {
            let level = e.depth - base;
            if options.interactive && !e.interactive {
                continue;
            }
            if options.max_depth.is_some_and(|d| level > d) {
                continue;
            }
            let indent = if options.interactive || options.flat { 0 } else { level };
            out.push_str(&"  ".repeat(indent));
            out.push_str(&line(e, Some(&e.reference)));
            out.push_str(&states(e));
            out.push('\n');
        }
        if out.is_empty() {
            out.push_str("(no elements; the app may still be loading. Try again or `agent-phone screenshot`)\n");
        }
        out
    }

    /// `element` and everything inside it, in tree order.
    pub fn subtree(&self, element: &Element) -> Vec<&Element> {
        let Some(start) = self.elements.iter().position(|e| e.reference == element.reference) else {
            return Vec::new();
        };
        let mut out = vec![&self.elements[start]];
        for e in &self.elements[start + 1..] {
            if e.depth <= element.depth {
                break;
            }
            out.push(e);
        }
        out
    }

    /// Every element `query` matches, in tree order.
    pub fn query(&self, query: &Query) -> Vec<&Element> {
        self.elements.iter().filter(|e| query.matches(e)).collect()
    }

    /// What changed since `before`, as `+` (new) and `-` (gone) lines. Unchanged lines are left out.
    pub fn diff(&self, before: &[Element], interactive_only: bool) -> (String, usize, usize) {
        let keep = |e: &&Element| !interactive_only || e.interactive;
        let old: Vec<&Element> = before.iter().filter(keep).collect();
        let new: Vec<&Element> = self.elements.iter().filter(keep).collect();
        // Diff lines drop the list's "- " so they read `+ button "Done" [ref=e4]`.
        let key = |e: &Element| format!("{}{}", &line(e, None)[2..], states(e));
        let (a, b): (Vec<String>, Vec<String>) = (old.iter().map(|e| key(e)).collect(), new.iter().map(|e| key(e)).collect());
        // Longest common subsequence, then walk it to emit the edits.
        let (n, m) = (a.len(), b.len());
        let mut lcs = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
            }
        }
        let (mut i, mut j, mut out, mut added, mut removed) = (0, 0, String::new(), 0, 0);
        while i < n || j < m {
            if i < n && j < m && a[i] == b[j] {
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                out.push_str(&format!("+ {}{}\n", &line(new[j], Some(&new[j].reference))[2..], states(new[j])));
                added += 1;
                j += 1;
            } else {
                out.push_str(&format!("- {}\n", a[i]));
                removed += 1;
                i += 1;
            }
        }
        if out.is_empty() {
            out.push_str("(no changes since the last snapshot)\n");
        }
        (out, added, removed)
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

}

/// `- button "Sign in" [ref=e3]` with its value and id.
fn line(e: &Element, reference: Option<&str>) -> String {
    let mut out = format!("- {}", e.describe());
    if let Some(r) = reference {
        out.push_str(&format!(" [ref={r}]"));
    }
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
    out
}

fn states(e: &Element) -> String {
    let mut out = String::new();
    if !e.enabled {
        out.push_str(" [disabled]");
    }
    if e.selected {
        out.push_str(" [selected]");
    }
    if e.focused {
        out.push_str(" [focused]");
    }
    out
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
            selected: truthy(&node.selected, false),
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

/// Where agent-phone keeps its state: `$AGENT_PHONE_HOME`, or `~/.agent-phone`.
pub fn home() -> Result<PathBuf> {
    Ok(match std::env::var_os("AGENT_PHONE_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".agent-phone"),
    })
}

fn session_path(udid: &str) -> Result<PathBuf> {
    let base = home()?;
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
    fn text_snapshots_read_in_rows_then_left_to_right() {
        let s = Snapshot::from_text(vec![
            ("Done".into(), [300.0, 52.0, 40.0, 20.0]),
            ("Notes".into(), [20.0, 100.0, 60.0, 30.0]),
            ("Cancel".into(), [20.0, 50.0, 60.0, 20.0]),
        ]);
        let order: Vec<&str> = s.elements.iter().map(|e| e.label.as_deref().unwrap()).collect();
        assert_eq!(order, vec!["Cancel", "Done", "Notes"]);
        assert_eq!(s.elements[2].reference, "e3");
    }

    #[test]
    fn queries_scopes_and_diffs() {
        let s = Snapshot::parse(TREE).unwrap();
        let q = Query { by: By::Role, value: "button".into(), name: Some("sign".into()), exact: false };
        assert_eq!(s.query(&q).len(), 1);
        let q = Query { by: By::Placeholder, value: "Email".into(), name: None, exact: true };
        assert_eq!(s.query(&q)[0].reference, "e2");
        let mut before = s.elements.clone();
        before.remove(0);
        before[1].enabled = true;
        let (out, added, removed) = s.diff(&before, false);
        assert_eq!((added, removed), (2, 1), "{out}");
        assert!(out.contains("+ text \"Welcome\" [ref=e1]"));
        assert!(s.diff(&s.elements, false).0.contains("no changes"));
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
