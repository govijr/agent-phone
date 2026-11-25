# agent-phone

iPhone automation CLI for AI agents.

Agents drive the iOS Simulator through its accessibility tree with short refs, without screenshots:

```sh
agent-phone open com.apple.Preferences
agent-phone snapshot -i
# - button "General" [ref=e4] [id=com.apple.settings.general]
# - searchbox "Search" [ref=e14]
agent-phone click @e4
agent-phone fill "Search" "Wallpaper"
agent-phone wait --text "About"
agent-phone screenshot done.png
```

It's a single native binary. It runs headless, so you don't need Simulator.app open, and it has no Python, idb or WebDriverAgent dependencies.

## Install

Requirements: an Apple silicon Mac, Xcode (opened once) with an iOS simulator runtime, and Rust to build.

```sh
git clone https://github.com/govijr/agent-phone && cd agent-phone
cargo install --path .
```

## Commands

| Command | What it does |
|---|---|
| `devices` | List simulators |
| `boot [name]` | Boot headless and wait until ready (defaults to the newest iPhone) |
| `shutdown` | Shut the simulator down |
| `apps` | Installed apps and bundle IDs |
| `open <bundle-id\|url> [--fresh]` | Launch an app, or open a URL or deep link |
| `close <bundle-id>` | Terminate an app |
| `install <path.app>` | Install a build |
| `snapshot [-i]` | Accessibility tree with refs; `-i` lists only interactive elements |
| `click <target> [--hold ms]` | Tap (alias `tap`); `--hold 600` is a long press |
| `fill <target> <text>` | Tap a field, clear it, type |
| `type <text>` | Type into the focused field |
| `press <key>` | `home`, `lock`, `enter`, `tab`, `backspace`, `escape`, `space`, arrows |
| `swipe <dir> [--from target] [--distance pt]` | Move a finger up, down, left or right |
| `scroll <dir>` | Scroll content (the finger moves the opposite way) |
| `wait <ms>` / `wait --text T [--gone] [--timeout s]` | Sleep, or wait for text to appear or disappear |
| `screenshot [path]` | PNG of the screen |

**Targets** can be a ref from the last snapshot (`@e4`), visible text (`"Sign in"`), or a point in screen points (`120,480`). With text targets, exact matches beat partial ones and interactive elements beat static text. `fill` prefers text fields.

**Refs are checked before every action.** agent-phone reads the screen again and finds the element by role, label and id. If it moved, the tap goes to its new position. If it's gone, the command fails with `is no longer on screen` and never taps something else.

**Global flags:** `--device <name|udid>` (or `AGENT_PHONE_DEVICE`) picks a simulator when several are booted. `--json` prints `{"ok": true, ...}` or `{"ok": false, "error": ...}`.

## With an agent

Point your agent at [`skills/agent-phone/SKILL.md`](skills/agent-phone/SKILL.md), or tell it:

> Use `agent-phone` to drive the iOS Simulator. Run `agent-phone snapshot -i` to see the screen, act with `click`/`fill`/`press` using `@e` refs, and run `snapshot` again after each action.

## How it works

- **Input** goes through SimulatorKit's HID client as Indigo touch, button and keyboard messages, the same path Simulator.app uses.
- **The tree** comes from AccessibilityPlatformTranslation's `AXPTranslator`, fed by CoreSimulator's accessibility requests, so frames are in screen points.
- **App lifecycle and screenshots** use `xcrun simctl`.

The private frameworks are loaded at runtime from your Xcode, and every call goes through a small Objective-C shim (`native/shim.m`) so no Objective-C exception reaches Rust. Tested on Xcode 27.2 with iOS 27.2.

## Roadmap

- Real iPhones over iPhone Mirroring
- A background daemon to make repeated commands faster
- Non-US keyboard layouts (other text falls back to paste today)

## License

Apache-2.0. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for idb (MIT).
