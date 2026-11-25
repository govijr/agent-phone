---
name: agent-phone
description: Drive the iOS Simulator, or the user's real iPhone through iPhone Mirroring, from the command line - open apps, read the screen with refs, tap, type, swipe, wait and take screenshots. Use when asked to test, use or check an iOS app, an iPhone simulator, or the user's own iPhone.
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
- Targets: `@e4`, visible text such as `"Sign in"`, `id=login-button` (accessibility id), or points such as `120,480`.
- `agent-phone find role button --name "Done" click` and `agent-phone find id email fill "me@example.com"` act without a snapshot first. `find text Delete count` counts matches.
- `agent-phone get text @e4` / `get value @e4` reads one element. `get focused`, `get app`. `agent-phone is enabled @e4` prints true or false.
- `agent-phone snapshot -i --diff` prints only what changed since your last snapshot. `snapshot -s @e7` prints one element and its contents.
- `agent-phone wait @e4`, `wait "Done" --gone`, or `wait --stable` (screen stopped changing) instead of fixed sleeps.
- `agent-phone screenshot --annotate shot.png` labels each interactive element with its ref, so you can act on what you see.
- `agent-phone batch "click @e4" "wait --text Done" "snapshot -i"` runs several commands in one call.
- `agent-phone back` goes back a screen. `agent-phone reload` relaunches the app in front.
- `agent-phone check "Wi-Fi"` / `uncheck` set a switch and confirm it changed. `drag @e5 @e9 --hold 600` reorders; without `--hold` it moves sliders.
- A permission prompt or other system alert blocks taps on the app. `agent-phone dialog` shows it; `dialog accept` / `dialog dismiss` answers it. Or skip the prompt with `set permission camera grant --app <bundle-id>`.
- Test setup: `set location 37.33,-122.01`, `set appearance dark`, `set text-size extra-large`, `push <bundle-id> --title Hi --body Test`, `media add photo.jpg`, `clipboard set "text"`.
- `state save logged-in --app <bundle-id>` after signing in, then `state load logged-in` to start from there next time. `state reset --app <bundle-id>` makes the app fresh.
- Debugging: `logs --errors --last 2m`, `crashes`, `audit` (accessibility), `record start` / `record stop`, `diff baseline.png --out changes.png`.
- `agent-phone scroll down` reveals more content. `agent-phone swipe left --from @e7` swipes on an element.
- `agent-phone click @e3 --hold 700` long-presses.
- `agent-phone press home|lock|enter|tab|backspace|escape|up|down|left|right`
- `agent-phone screenshot out.png` shows you the screen when the tree isn't enough, for example with images, games or maps.
- Add `--json` for machine-readable output. Add `--device "<name>"` when several simulators are booted.

## Real iPhone (`--device iphone`)

Use this when the user asks for their own phone, not a simulator. It goes through macOS iPhone Mirroring.

- Start with `agent-phone --device iphone doctor`. If it reports a problem, run `agent-phone --device iphone connect`. Permissions are something only the user can grant, so tell them which ones are missing.
- `snapshot` is text recognized from the screen: there are no roles, and unlabeled icons are missing. Use `screenshot` when you need to see icons.
- Open apps by name: `agent-phone --device iphone open "Notes"` (through Spotlight).
- `press home`, `press switcher` and `press spotlight` work. `lock`, `apps`, `install` and `close` don't.
- `type` and `fill` paste through the Mac clipboard. Add `--keystrokes` if a field rejects pasting.
- This is the user's real phone. Ask before anything that sends, buys, posts or deletes.

## When things fail

- `is no longer on screen`: the screen changed. Run `snapshot` again and pick the new ref.
- `is off screen`: agent-phone scrolls simulators for you; on an iPhone run `scroll` toward it first.
- `a system alert is in front`: run `agent-phone dialog accept` or `dialog dismiss`.
- `is disabled`: the app won't accept the tap yet. Fill the required fields first; use `--force` only on purpose.
- An empty snapshot usually means the app is still loading. Wait, then try again.
