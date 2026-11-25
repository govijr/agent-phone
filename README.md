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

It also drives **your real iPhone** through macOS iPhone Mirroring, with the same commands:

```sh
agent-phone --device iphone connect
agent-phone --device iphone open Settings     # via Spotlight
agent-phone --device iphone click "General"
```

## Install

Requirements: an Apple silicon Mac and Xcode (opened once) with an iOS simulator runtime.

```sh
curl -fsSL https://raw.githubusercontent.com/govijr/agent-phone/main/install.sh | sh
```

This downloads the latest release, checks its SHA-256, and puts `agent-phone` in `~/.local/bin` (set `AGENT_PHONE_INSTALL_DIR` for somewhere else, or `AGENT_PHONE_VERSION=v0.2.0` for a specific release).

Or build it yourself, with Rust:

```sh
git clone https://github.com/govijr/agent-phone && cd agent-phone
cargo install --path .
```

`agent-phone upgrade` installs the newest release over the current binary, after checking its SHA-256 (`--check` only looks).

## Commands

| Command | What it does |
|---|---|
| `devices` | List simulators and the mirrored iPhone |
| `doctor` | Check Xcode and the simulator, or iPhone Mirroring and its permissions |
| `connect` | iPhone: open iPhone Mirroring in the background and wait for the phone |
| `boot [name]` | Boot headless and wait until ready (defaults to the newest iPhone) |
| `shutdown` | Shut the simulator down |
| `apps [--running]` | Installed apps and bundle IDs, or the running ones with process IDs |
| `open <bundle-id\|url> [--fresh]` | Launch an app, or open a URL or deep link |
| `close <bundle-id>` | Terminate an app |
| `install <path.app>` | Install a build |
| `snapshot [-i] [--depth n] [-s target] [--diff]` | Accessibility tree with refs; `-i` lists only interactive elements, `-s` one element and its contents, `--diff` only what changed since the last snapshot |
| `click <target> [--hold ms] [--force]` | Tap (alias `tap`); `--hold 600` is a long press |
| `fill <target> <text>` | Tap a field, clear it, type |
| `back` | The navigation bar's back button, or a swipe from the left edge |
| `reload` | Relaunch the frontmost app |
| `dblclick <target>` | Double-tap |
| `drag <from> <to> [--hold ms]` | Drag (sliders, maps); `--hold 600` picks an item up first, for reordering |
| `check <target>` / `uncheck <target>` | Turn a switch on or off, and confirm it changed (does nothing if it already is) |
| `dialog [accept\|dismiss]` | Show, accept or dismiss a system alert such as a permission prompt |
| `find <role\|text\|label\|id\|placeholder\|value> <value> [click\|fill\|text\|count] [--name N] [--exact] [--nth n]` | Semantic locators: `find role button --name Done click`, `find id email fill "me@x.com"` |
| `get <what> [target]` | `text`, `value`, `label`, `id`, `role`, `frame`, `count`; also `focused`, `app`, `size` |
| `is <visible\|enabled\|focused\|selected\|on> <target>` | Prints `true` or `false` |
| `type <text>` | Type into the focused field |
| `press <key>` | `home`, `lock`, `switcher`, `spotlight`, `enter`, `tab`, `backspace`, `escape`, `space`, arrows |
| `swipe <dir> [--from target] [--distance pt]` | Move a finger up, down, left or right |
| `scroll <dir>` | Scroll content (the finger moves the opposite way) |
| `wait <ms\|target>` / `wait --text T` / `wait --stable` | Sleep, wait for an element or text to appear (or go with `--gone`), or for the screen to stop changing |
| `screenshot [path] [--annotate]` | PNG of the screen; `--annotate` draws each interactive element's ref on it |
| `set location <lat,lon\|clear>` | Simulated GPS position |
| `set appearance <light\|dark>`, `set text-size <size>`, `set contrast <on\|off>` | Dark mode, Dynamic Type, Increase Contrast |
| `set permission <service> <grant\|revoke\|reset> [--app id]` | Camera, photos, location, contacts… without the prompt |
| `set statusbar <clean\|clear>` | 9:41 with full bars for screenshots, or back to normal |
| `push <bundle-id> [payload] [--title T --body B]` | Send a push notification |
| `state save\|load\|list\|delete <name>` / `state reset` | Save an app's data (for example logged in) and put it back later, or empty it |
| `keychain reset` | Clear the simulator's keychain |
| `media add <files…>` | Add photos, videos or contacts |
| `clipboard get` / `clipboard set <text>` | Read or set the clipboard |
| `logs [--app id] [--last 5m] [--errors] [--grep T] [--follow]` | Log lines from the frontmost app, or another |
| `crashes [--app name]` | Recent crash reports from simulator apps |
| `audit` | Accessibility problems: missing labels, small tap targets, duplicate labels |
| `record start [path]` / `record stop` | Record the screen to an MP4 |
| `diff <baseline.png> [current.png] [--out diff.png]` | How much of the screen changed against a baseline, with the changes marked in red |
| `stream [--port 7300]` | Watch the simulator live in your browser; click to tap, drag to swipe |
| `perf [--app id]` | CPU and memory of an app, sampled over a few seconds |
| `profile start [path] [--template time\|allocations\|leaks\|hitches…]` / `profile stop` | Record an Instruments trace of an app |
| `confirm [id]` / `deny <id>` | Run or drop an action the policy held back (no id lists them) |
| `config [show\|init]` | Show the merged `agent-phone.json` settings, or write a starter file |
| `upgrade [--check]` | Install the newest release |
| `batch [cmd…]` | Run commands in order, from arguments or one per line on stdin; stops at the first failure unless `--keep-going` |
| `skills get` | Print the agent guide built into the binary |
| `mcp` | Run as an MCP server over stdio |
| `daemon status\|stop` | See or stop the background daemons |

**Targets** can be a ref from the last snapshot (`@e4`), visible text (`"Sign in"`), an accessibility identifier (`id=login-button`), or a point in screen points (`120,480`). With text targets, exact matches beat partial ones and interactive elements beat static text. `fill` prefers text fields.

**Refs are checked before every action.** agent-phone reads the screen again and finds the element by role, label and id. If it moved, the tap goes to its new position. If it's gone, the command fails with `is no longer on screen` and never taps something else. An element below or above the screen is scrolled into view first, and a disabled one is refused unless you pass `--force`.

**Global flags:** `--device <name|udid>` (or `AGENT_PHONE_DEVICE`) picks a simulator when several are booted. `--json` prints `{"ok": true, ...}` or `{"ok": false, "error": ...}`. `--no-daemon` (or `AGENT_PHONE_NO_DAEMON=1`) runs the command in-process. `--dialogs accept|dismiss` (or `AGENT_PHONE_DIALOGS`) answers system alerts before each command, so permission prompts don't block an agent.

**The daemon.** The first simulator command starts a background process for that device, which keeps the simulator connection open, so later commands take tens of milliseconds instead of a few hundred. It exits after 30 idle minutes (`AGENT_PHONE_DAEMON_IDLE` in seconds), and restarts itself when you rebuild agent-phone. `boot` and `shutdown` stop it. iPhone Mirroring commands don't use it.

## Config and policy

`agent-phone.json` in your project folder (or any parent), and `~/.agent-phone/config.json` for every project, set defaults and limit what agents may do. Project settings win; flags and environment variables win over both. `agent-phone config init` writes a starter file.

```json
{
  "device": "iPhone 17 Pro",
  "dialogs": "accept",
  "policy": {
    "allowApps": ["com.example.*"],
    "confirm": true,
    "deny": ["Delete Account"],
    "humanConfirm": false
  }
}
```

- **`allowApps`**: the only apps `open` may launch and taps may land in (bundle IDs, `*` wildcards; app names on an iPhone). Links that would open other apps are refused.
- **`confirm`**: taps on elements whose text has one of these words are held instead of sent. `true` uses the built-in list (buy, pay, order, subscribe, send, post, share, delete, remove, transfer, sign out, call…). The command fails with an id; `agent-phone confirm <id>` runs it and `deny <id>` drops it. Held actions expire after 10 minutes. **On a real iPhone the built-in list is on unless you set `"confirm": false`.**
- **`deny`**: words that are always refused.
- **`humanConfirm`**: `confirm` only works in an interactive terminal, so an agent can't approve its own actions.

The policy covers taps, long presses, double-taps, `fill`, `check` and `uncheck`, whether they name an element or a point. It doesn't read what you `type` or which key you `press`.

## Real iPhone (iPhone Mirroring)

`--device iphone` (or `AGENT_PHONE_DEVICE=iphone`) drives your own iPhone through the iPhone Mirroring app.

**Needs:** macOS 15+, an iPhone on iOS 18+ that's paired with iPhone Mirroring, signed in to the same Apple Account, nearby and locked. The app you run agent-phone from (your terminal, or the agent's own app) needs **Accessibility** and **Screen Recording** in System Settings > Privacy & Security. `agent-phone --device iphone doctor` checks all of this and tells you which app needs which permission.

**How it differs from the simulator:**

- **The screen is read with text recognition.** The Mirroring window is a video stream that macOS accessibility can't see into, so `snapshot` lists the text on screen with refs. Elements have no roles or states, and icons without labels don't show up. Use `screenshot` to see them.
- **Input is real Mac mouse and keyboard events.** The Mirroring window has to be in front. agent-phone brings it forward, confirms it's there, and refuses to send anything if another window covers the spot it would tap.
- **`open <App Name>` uses Spotlight**, since bundle IDs and URLs don't apply.
- **`type` and `fill` paste through the Mac clipboard** so iOS autocorrect can't change the words. Your clipboard keeps the typed text afterwards. Add `--keystrokes` to type key by key instead.
- **`press home`, `switcher` and `spotlight`** use iPhone Mirroring's own shortcuts. `lock`, `apps`, `install`, `close`, `boot`, `reload`, `check`, `dialog`, `set`, `push`, `state`, `keychain`, `media`, `clipboard`, `logs`, `audit` and `record` are simulator-only.
- **If the phone is in use or the session has paused**, commands stop with the message Mirroring shows. `connect` presses Connect for you when it's offered.

## With an agent

Point your agent at [`skills/agent-phone/SKILL.md`](skills/agent-phone/SKILL.md) (also printed by `agent-phone skills get`), or tell it:

> Use `agent-phone` to drive the iOS Simulator. Run `agent-phone snapshot -i` to see the screen, act with `click`/`fill`/`press` using `@e` refs, and run `snapshot` again after each action.

Or add it as an MCP server. Its tools are `snapshot`, `click`, `fill`, `type`, `press`, `scroll`, `swipe`, `find`, `get`, `is`, `wait`, `screenshot` (returns the image), `open`, `close`, `apps`, `devices` and `batch`:

```json
{ "mcpServers": { "agent-phone": { "command": "agent-phone", "args": ["mcp"] } } }
```

## How it works

- **Input** goes through SimulatorKit's HID client as Indigo touch, button and keyboard messages, the same path Simulator.app uses.
- **The tree** comes from AccessibilityPlatformTranslation's `AXPTranslator`, fed by CoreSimulator's accessibility requests, so frames are in screen points.
- **Screenshots and the live stream** read the simulator's framebuffer (an IOSurface) directly, which takes tens of milliseconds; `simctl io screenshot` takes over a second and is only the fallback.
- **App lifecycle, settings and recording** use `xcrun simctl`; profiling uses `xctrace`.

The private frameworks are loaded at runtime from your Xcode, and every call goes through a small Objective-C shim (`native/shim.m`) so no Objective-C exception reaches Rust. Tested on Xcode 27.2 with iOS 27.2.

For iPhone Mirroring (`native/mirror.m`), the window is found by bundle ID, capture uses `screencapture`, text recognition uses Apple's Vision framework on-device, and input is CoreGraphics events. Tested on macOS 26.7 with a real iPhone.

## Roadmap

- iPhone: tap and swipe without bringing the Mirroring window to the front
- Pinch and rotate gestures, and picker wheels
- Non-US keyboard layouts (other text falls back to paste today)

## License

Apache-2.0. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for idb and phone-harness (both MIT).
