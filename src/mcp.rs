//! `agent-phone mcp`: the CLI as an MCP server over stdio (newline-delimited JSON-RPC).
//! Each tool call runs the matching command in this process, so the simulator connection stays
//! open for the whole MCP session.

use crate::execute_args;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A positional argument.
    Arg,
    /// `--name value`.
    Flag,
    /// `--name` when true.
    Switch,
}

struct Param {
    name: &'static str,
    ty: &'static str,
    kind: Kind,
    required: bool,
    about: &'static str,
    choices: &'static [&'static str],
}

const fn p(name: &'static str, ty: &'static str, kind: Kind, required: bool, about: &'static str) -> Param {
    Param { name, ty, kind, required, about, choices: &[] }
}

const fn choice(name: &'static str, kind: Kind, required: bool, about: &'static str, choices: &'static [&'static str]) -> Param {
    Param { name, ty: "string", kind, required, about, choices }
}

struct Tool {
    name: &'static str,
    command: &'static str,
    about: &'static str,
    params: &'static [Param],
}

use Kind::*;

const TARGET: &str = "@e3 (a ref from snapshot), visible text such as \"Sign in\", id=<accessibility id>, or x,y in points";
const DIRECTIONS: &[&str] = &["up", "down", "left", "right"];

const TOOLS: &[Tool] = &[
    Tool {
        name: "snapshot",
        command: "snapshot",
        about: "The screen's accessibility tree with refs like [ref=e4]. Run it before acting and again after the screen changes.",
        params: &[
            p("interactive", "boolean", Switch, false, "Only interactive elements (buttons, fields, switches, cells)"),
            p("depth", "integer", Flag, false, "Only this many levels deep"),
            p("scope", "string", Flag, false, "Only this element and what is inside it (a target)"),
            p("diff", "boolean", Switch, false, "Only what changed since the last snapshot"),
        ],
    },
    Tool {
        name: "click",
        command: "click",
        about: "Tap an element or point. Off-screen elements are scrolled into view first.",
        params: &[
            p("target", "string", Arg, true, TARGET),
            p("hold", "integer", Flag, false, "Hold for this many milliseconds (600 is a long press)"),
            p("force", "boolean", Switch, false, "Tap even if the element is disabled"),
        ],
    },
    Tool {
        name: "fill",
        command: "fill",
        about: "Tap a text field, clear it, and type into it.",
        params: &[p("target", "string", Arg, true, TARGET), p("text", "string", Arg, true, "Text to type")],
    },
    Tool { name: "type", command: "type", about: "Type into the focused field.", params: &[p("text", "string", Arg, true, "Text to type")] },
    Tool {
        name: "press",
        command: "press",
        about: "Press a key or button.",
        params: &[p("key", "string", Arg, true, "home, lock, switcher, spotlight, enter, tab, backspace, escape, space, up, down, left, right")],
    },
    Tool {
        name: "scroll",
        command: "scroll",
        about: "Scroll content in a direction.",
        params: &[
            choice("direction", Arg, true, "Which way the content moves into view", DIRECTIONS),
            p("from", "string", Flag, false, "Where to scroll: a target (default: the middle of the screen)"),
            p("distance", "number", Flag, false, "Points to move"),
        ],
    },
    Tool {
        name: "swipe",
        command: "swipe",
        about: "Move a finger in a direction (for carousels, swipe actions, dismissing sheets).",
        params: &[
            choice("direction", Arg, true, "Which way the finger moves", DIRECTIONS),
            p("from", "string", Flag, false, "Where to start: a target"),
            p("distance", "number", Flag, false, "Points to move"),
        ],
    },
    Tool { name: "back", command: "back", about: "Go back: taps the navigation bar's back button, or swipes from the left edge.", params: &[] },
    Tool {
        name: "drag",
        command: "drag",
        about: "Drag from one target to another (sliders, maps). Add hold=600 to pick an item up first (reordering, home screen icons).",
        params: &[
            p("from", "string", Arg, true, TARGET),
            p("to", "string", Arg, true, TARGET),
            p("hold", "integer", Flag, false, "Hold at the start for this many milliseconds"),
        ],
    },
    Tool { name: "check", command: "check", about: "Turn a switch or checkbox on; does nothing if it already is.", params: &[p("target", "string", Arg, true, TARGET)] },
    Tool { name: "uncheck", command: "uncheck", about: "Turn a switch or checkbox off; does nothing if it already is.", params: &[p("target", "string", Arg, true, TARGET)] },
    Tool {
        name: "dialog",
        command: "dialog",
        about: "System alerts (permission prompts and the like): status shows the alert and its buttons; accept or dismiss answers it.",
        params: &[choice("action", Arg, false, "What to do (default: status)", &["status", "accept", "dismiss"])],
    },
    Tool {
        name: "set",
        command: "set",
        about: "Change simulator settings, e.g. [\"location\", \"37.33,-122.01\"], [\"appearance\", \"dark\"], [\"permission\", \"camera\", \"grant\", \"--app\", \"com.x\"], [\"statusbar\", \"clean\"], [\"text-size\", \"extra-large\"], [\"contrast\", \"on\"].",
        params: &[p("args", "array", Arg, true, "The setting and its value")],
    },
    Tool {
        name: "logs",
        command: "logs",
        about: "Recent log lines from an app (the frontmost one unless app is given).",
        params: &[
            p("app", "string", Flag, false, "Bundle ID"),
            p("last", "string", Flag, false, "How far back: 30s, 5m (default 1m)"),
            p("errors", "boolean", Switch, false, "Only errors and faults"),
            p("grep", "string", Flag, false, "Only lines containing this"),
            p("lines", "integer", Flag, false, "At most this many lines (default 200)"),
        ],
    },
    Tool { name: "audit", command: "audit", about: "Accessibility problems on the screen: unlabelled controls, small tap targets, duplicate labels.", params: &[] },
    Tool {
        name: "find",
        command: "find",
        about: "Find elements by role, text, label, id, placeholder or value; list them, count them, read one, or click or fill one.",
        params: &[
            choice("by", Arg, true, "What to match", &["role", "text", "label", "id", "placeholder", "value"]),
            p("value", "string", Arg, true, "The role (button, textbox, cell, switch…) or the text to match"),
            choice("action", Arg, false, "What to do (default: list)", &["list", "click", "fill", "text", "count"]),
            p("text", "string", Arg, false, "Text for fill"),
            p("name", "string", Flag, false, "With by=role: the element's text must also contain this"),
            p("exact", "boolean", Switch, false, "Match the whole text"),
            p("nth", "integer", Flag, false, "Use the nth match (from 1)"),
        ],
    },
    Tool {
        name: "get",
        command: "get",
        about: "Read one thing: text, value, label, id, role or frame of a target; count of matches; the focused element; the frontmost app; the screen size.",
        params: &[
            choice("what", Arg, true, "What to read", &["text", "value", "label", "id", "role", "frame", "count", "focused", "app", "size"]),
            p("target", "string", Arg, false, TARGET),
        ],
    },
    Tool {
        name: "is",
        command: "is",
        about: "Check a target's state. Returns true or false.",
        params: &[
            choice("what", Arg, true, "The state", &["visible", "enabled", "focused", "selected", "on"]),
            p("target", "string", Arg, true, TARGET),
        ],
    },
    Tool {
        name: "wait",
        command: "wait",
        about: "Wait for an element or text to appear (or disappear with gone), for the screen to settle (stable), or for milliseconds.",
        params: &[
            p("what", "string", Arg, false, "Milliseconds, or a target to wait for"),
            p("text", "string", Flag, false, "Text to wait for"),
            p("gone", "boolean", Switch, false, "Wait for it to disappear"),
            p("stable", "boolean", Switch, false, "Wait until the screen stops changing"),
            p("timeout", "number", Flag, false, "Seconds before giving up (default 10)"),
        ],
    },
    Tool {
        name: "screenshot",
        command: "screenshot",
        about: "A picture of the screen. With annotate, each interactive element is boxed and labelled with its ref.",
        params: &[p("annotate", "boolean", Switch, false, "Draw refs on the picture")],
    },
    Tool {
        name: "open",
        command: "open",
        about: "Launch an app by bundle ID (simulator) or name (iPhone), or open a URL or deep link.",
        params: &[p("target", "string", Arg, true, "Bundle ID, app name or URL"), p("fresh", "boolean", Switch, false, "Restart the app if it is running")],
    },
    Tool { name: "close", command: "close", about: "Terminate an app (simulator).", params: &[p("bundle_id", "string", Arg, true, "Bundle ID")] },
    Tool { name: "apps", command: "apps", about: "Installed apps and bundle IDs (simulator).", params: &[] },
    Tool { name: "devices", command: "devices", about: "Simulators and the mirrored iPhone.", params: &[] },
    Tool {
        name: "batch",
        command: "batch",
        about: "Run several agent-phone commands in order, e.g. [\"click @e4\", \"wait --text Done\", \"snapshot -i\"]. Stops at the first failure.",
        params: &[p("commands", "array", Arg, true, "Command lines without the agent-phone prefix")],
    },
];

fn schema(tool: &Tool) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for param in tool.params {
        let mut prop = json!({ "type": param.ty, "description": param.about });
        if param.ty == "array" {
            prop["items"] = json!({ "type": "string" });
        }
        if !param.choices.is_empty() {
            prop["enum"] = json!(param.choices);
        }
        props.insert(param.name.into(), prop);
        if param.required {
            required.push(param.name);
        }
    }
    props.insert(
        "device".into(),
        json!({ "type": "string", "description": "Simulator name or UDID, or \"iphone\" for iPhone Mirroring. Defaults to the booted simulator." }),
    );
    json!({ "type": "object", "properties": props, "required": required })
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The command line for a tool call.
fn argv(tool: &Tool, input: &Value) -> Vec<String> {
    let mut args = vec!["agent-phone".to_string(), "--no-daemon".to_string()];
    if let Some(device) = input.get("device").and_then(Value::as_str) {
        args.extend(["--device".into(), device.into()]);
    }
    args.push(tool.command.into());
    for param in tool.params {
        let Some(value) = input.get(param.name).filter(|v| !v.is_null()) else { continue };
        let flag = format!("--{}", param.name.replace('_', "-"));
        match param.kind {
            Arg => match value {
                Value::Array(items) => args.extend(items.iter().map(text_of)),
                other => args.push(text_of(other)),
            },
            Flag => args.extend([flag, text_of(value)]),
            Switch => {
                if value.as_bool().unwrap_or(false) {
                    args.push(flag);
                }
            }
        }
    }
    args
}

fn call(name: &str, input: &Value) -> Value {
    let Some(tool) = TOOLS.iter().find(|t| t.name == name) else {
        return json!({ "content": [{ "type": "text", "text": format!("unknown tool {name}") }], "isError": true });
    };
    let mut args = argv(tool, input);
    let shot = (tool.name == "screenshot").then(|| {
        std::env::temp_dir().join(format!("agent-phone-mcp-{}.png", std::process::id())).to_string_lossy().into_owned()
    });
    if let Some(path) = &shot {
        let at = args.iter().position(|a| a == "screenshot").unwrap_or(args.len() - 1) + 1;
        args.insert(at, path.clone());
    }
    let out = execute_args(args);
    let mut text = format!("{}{}", out.stdout, out.stderr);
    if text.trim().is_empty() {
        text = "done".into();
    }
    let mut content = vec![json!({ "type": "text", "text": text.trim_end() })];
    if let (Some(path), 0) = (&shot, out.code) {
        // Full-resolution PNGs run to megabytes; a 1200-pixel-tall JPEG reads just as well.
        let jpeg = format!("{path}.jpg");
        let small = std::process::Command::new("sips")
            .args(["-Z", "1200", "-s", "format", "jpeg", "-s", "formatOptions", "80", path, "--out", &jpeg])
            .output()
            .is_ok_and(|o| o.status.success());
        let (file, mime) = if small { (jpeg.as_str(), "image/jpeg") } else { (path.as_str(), "image/png") };
        if let Ok(bytes) = std::fs::read(file) {
            content.insert(0, json!({ "type": "image", "data": base64(&bytes), "mimeType": mime }));
        }
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(&jpeg);
    }
    json!({ "content": content, "isError": out.code != 0 })
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn reply(id: &Value, result: Result<Value, (i64, String)>) -> Value {
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    }
}

fn handle(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone(); // notifications get no reply
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let result = match message.get("method").and_then(Value::as_str).unwrap_or_default() {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "agent-phone", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Drive the iOS Simulator (or a real iPhone with device=\"iphone\"). Call snapshot with interactive=true, act on refs with click/fill/press, wait for results, and snapshot again after the screen changes.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": TOOLS.iter().map(|t| json!({ "name": t.name, "description": t.about, "inputSchema": schema(t) })).collect::<Vec<_>>()
        })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            Ok(call(name, params.get("arguments").unwrap_or(&json!({}))))
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(reply(&id, result))
}

pub fn serve() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&message),
            Err(e) => Some(reply(&Value::Null, Err((-32700, format!("parse error: {e}"))))),
        };
        if let Some(response) = response {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn builds_command_lines_from_tool_input() {
        let tool = TOOLS.iter().find(|t| t.name == "find").unwrap();
        let args = argv(tool, &json!({ "by": "role", "value": "button", "action": "click", "name": "Done", "exact": true, "device": "iPhone 17" }));
        assert_eq!(
            args,
            ["agent-phone", "--no-daemon", "--device", "iPhone 17", "find", "role", "button", "click", "--name", "Done", "--exact"]
        );
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn every_tool_names_a_real_command() {
        for tool in TOOLS {
            let mut args = vec!["agent-phone".to_string(), tool.command.to_string()];
            args.extend(tool.params.iter().filter(|p| p.required).flat_map(|p| match (p.choices.first(), tool.name) {
                (Some(c), _) => vec![c.to_string()],
                (None, "set") => vec!["appearance".to_string(), "dark".to_string()],
                (None, _) => vec!["1".to_string()],
            }));
            assert!(crate::Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }
}
