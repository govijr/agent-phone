---
name: agent-phone
description: Drive the iOS Simulator from the command line - open apps, read the screen as an accessibility tree with refs, tap, type, swipe, wait and take screenshots. Use when asked to test, use or check an iOS app or anything on an iPhone simulator.
---

# agent-phone

Drive the iOS Simulator with the `agent-phone` CLI.

## Loop

1. `agent-phone snapshot -i` to see what's on screen. Each element has a ref like `[ref=e4]`.
2. Act on one element: `agent-phone click @e4`, `agent-phone fill @e2 "me@example.com"`, `agent-phone press enter`.
3. Wait for the result with `agent-phone wait --text "Welcome"` instead of fixed sleeps.
4. Run `snapshot` again. Refs are numbered per snapshot, so never reuse refs from before the screen changed.

## Commands

- `agent-phone boot` boots the newest iPhone headless. Skip it if one is already booted.
- `agent-phone apps` lists bundle IDs. `agent-phone open <bundle-id> --fresh` restarts an app.
- `agent-phone open "myapp://path"` opens deep links and URLs.
- `agent-phone snapshot` (without `-i`) also shows static text. Use it to read the screen.
- Targets: `@e4`, visible text such as `"Sign in"`, or points such as `120,480`.
- `agent-phone scroll down` reveals more content. `agent-phone swipe left --from @e7` swipes on an element.
- `agent-phone click @e3 --hold 700` long-presses.
- `agent-phone press home|lock|enter|tab|backspace|escape|up|down|left|right`
- `agent-phone screenshot out.png` shows you the screen when the tree isn't enough, for example with images, games or maps.
- Add `--json` for machine-readable output. Add `--device "<name>"` when several simulators are booted.

## When things fail

- `is no longer on screen`: the screen changed. Run `snapshot` again and pick the new ref.
- `is off screen`: run `scroll` toward it first.
- An empty snapshot usually means the app is still loading. Wait, then try again.
