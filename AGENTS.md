# Warcry — working notes

Warcry turns what you say into keystrokes in a game. Three things were added on top of that
engine: a **phone companion** that hears a game window and alerts a phone, a **command editor**
in the desktop app, and a **phone deck** -- a button on the phone for any command you can say.

The companion lives *inside the Tauri app*. `warcry-rig listen` still exists and still works,
but nothing requires it: the app embeds the same LAN server and the same detection pipeline, so
the phone connects straight to the desktop's local address. The optional hub in `hub/` is the
only piece meant to leave the network, and it is still not deployed.

Built and proven end to end on macOS 15.1 against World of Warcraft (`_forever_`, interface
16001) on 2026-11-12. The next target is a Windows multibox rig.

## The invariant that shapes everything

**Warcry must never automate gameplay.** It is an accessibility aid. The detection path
reports; a human acts. This is structural, not a convention:

- `ears` (all detection) has no dependency that can reach the input layer. `cargo tree -p
  warcry-ears` has no `enigo`. Keep it that way.
- A keystroke can only originate from a `Press` that arrived over the network and passed
  `warcry-relay::host::Hosted`, which checks a pairing token, rejects replayed sequence
  numbers, and rate-limits to 10/s. The host decides what a press *does* through
  `companion::Fire`; `Hosted` decides whether it happens at all. Both callers debounce one
  (rig, slot) to 600ms.
- A `Press` names an *action slot*, never a key. The desktop decides what slot 0 means.
- There is deliberately no message meaning "reel when you detect a bite". Do not add one.

WoW exposes no event for a bobber splash — deliberately, to stop fishing bots. That is why
the bite is *heard* rather than queried, and why the addon only ever plays sounds. The addon
reads only what the sanctioned Lua API offers and calls `PlaySoundFile`. Nothing reads memory,
injects input, or touches the install.

## Crates

| crate | job |
| --- | --- |
| `warcry` (root) | the voice engine. `keys`, `input` (enigo), `window` (focus, `BackgroundSink` on Windows), `profile`, `matcher`, `engine`. |
| `app/` | Tauri 2 tray app: engine control, **command editor**, **addon installer**, **the companion itself** (`app/src/companion.rs`). |
| `link/` | wire protocol, pairing token, admission gate. Postcard for native, JSON for browsers. |
| `ears/` | per-window audio capture and all detection. Cannot reach input. |
| `relay/` | LAN server: serves the companion page and phone websocket. `host::Hosted` holds the admission rules. |
| `rig/` | `companion::Companion` -- taps plus relay plus press routing, used by both the CLI and the app. `deck` turns `pad = true` commands into action slots. Also the fishing CLI. |
| `hub/` | optional public fan-out service. Unit-tested, never deployed. |

## Part one: the fishing companion (works, don't break it)

`warcry-rig listen` taps a WoW window's audio, decodes beacon tones from the addon, detects
the bobber splash, and serves a page the phone loads. The phone shows a glowing button, chimes
on a bite, and its press becomes a keystroke.

```sh
warcry-rig windows
warcry-rig listen --pid <pid> --serve --port 8137 \
  --sounds sounds/bobber.json --token-file hack/token \
  --advertise <reachable-ip> --focus --record hack/casts
warcry-rig teach-from hack/casts/*.wav --name bobber --out sounds/bobber.json
warcry-rig refine a.json b.json --name bobber --min 0.65 --out sounds/bobber.json
warcry-rig replay hack/casts/*.wav --sounds sounds/bobber.json
warcry-rig analyze hack/casts/*.wav
```

`teach-from` finds the sound automatically in the 3.0s–1.0s window before each cast ended,
blanking anything near a decoded beacon tone so it cannot train on Warcry's own beeps.

**`replay` is the regression test for the detector. Run it before and after any change to
`ears::library` or any retrain.** A threshold was once set so high that 0 of 11 casts detected
and it went unnoticed for an hour, because nobody measured.

### Detector approaches that failed — do not retry

- **RMS onset detection.** The splash is quieter than 0.02 rms while beacon tones are ~0.18,
  so an onset detector fires on Warcry's own beeps.
- **Log-mel patches with one global mean removed.** Spectral tilt dominates; arbitrary game
  music scores 0.95 against the bobber. Patches now centre *each band across time*.
- **Matching without an onset gate.** Normalised shape alone matches silence. Each `Sound`
  stores a `flux` floor learned from its training onsets.
- **Single-alignment matching.** Audio arrives at arbitrary frame alignment; `Library::aligned`
  tries `ALIGNMENTS` offsets and takes the best.

### What is not verified

- **Bite hit rate.** `sounds/bobber.json` is 12 exemplars at threshold 0.50. That threshold is
  a guess — the held-out casts were destroyed by a filename collision before measurement.
  Fish ~10 casts, then `replay`.
- **False positives** measured ~1 per 30s of idle audio at 0.50, with music playing. During a
  cast BetterFishing mutes music so it should be far lower; never measured separately.
- **Bags full and lure alerts.** Addon emits them, decoder reads them, `rig` maps them, and no
  phone has ever received one.
- **Fishing from inside the app.** The deck was driven end to end against the running app (pair,
  press, `actions::trigger` fires the named command). The *detection* half of the app companion
  has not been run against a live game window -- it is the same `Companion` the CLI uses, with
  the same taps and the same pipeline, but nobody has watched a bite arrive through the app yet.
- **The window cast/reel buttons in the app.** `choose` is unit-tested and `tap` is the CLI's
  own code, but no press has gone through the app into a game window.
- **The hub.** 9 unit tests, no integration test, never run against a real phone.

## Part one and a half: the phone deck

A command with `pad = true` gets a button on the phone. `deck::pads` numbers those commands
0, 1, 2... and `deck::rig` publishes them as one extra `RigInfo` with `kind: Deck` and the
reserved id `RigId::MAX`, so no protocol message had to be invented: the phone already knew how
to read a rig's action slots. Pressing a pad runs `actions::trigger` with the engine's own
`Shared` when the engine is up, which is what makes it the *same* command voice would run --
including `stop_others`. `rig/tests/deck.rs` drives this over a real websocket.

The phone's cast and reel buttons resolve through `Deck { cast, reel }` in `app.json`: a command
name, matched case-insensitively against `display_name()`. With no name set they fall back to
`companion::tap` -- one key, after `raise(pid)` on macOS, or through `BackgroundSink` on Windows.
That fallback is the path that was measured working for fishing, so it stays the default.

`app/src/companion.rs` owns the mapping (`choose`) and the alert wording (`told`), both
unit-tested. `WARCRY_HOME` overrides the app's config directory, which is how to run the app
against a scratch profile without touching your own.

The probe thread calls `companion::rescan` every 2s: if the set of World of Warcraft pids has
changed, the companion is torn down and restarted so a client started after the app still gets
heard. Restart is `halt` then `start`, and **`halt` only truly frees the port because
`Companion::drop` joins the companion threads.** It did not at first: `pressing` and `hearing`
hold `Arc<Relay>` clones, so setting the quit flag left the old listener bound while `start`
ran microseconds later, and the rebind died with `Address already in use (os error 48)`. The
companion then stayed dead, because `rescan` asked `is_some_and(...)`, which is false when
there is no companion at all -- so nothing ever retried. `rescan` now treats "no companion
while the deck is enabled" as stale and restarts it.

## Part two: the desktop app

The Commands tab edits commands: name, phrase to say, key to press, optional hotkey, which
window, and whether it works while not listening. `Profile::save` serialises, parses the
result back, and only then writes, so a profile that would fail to load can never replace a
working one.

A command whose actions are more than a single press or combo is shown but **not editable in
the UI** — flattening someone's macro into one keystroke silently would be worse than sending
them to the file.

### Targeting one window out of several

Multiboxing runs **one executable with one window title**, so nothing about a window's name
separates two clients. Do not suggest renaming the executable: it does not work (same binary)
and it means touching the game install.

`Target` now carries an optional `pid`. The editor lists open windows and the command
remembers the chosen one. `window::find` prefers that exact pid while it exists and falls back
to title matching after a relaunch, so a dead pid degrades rather than breaks.

That covers "I have one healer window". It does **not** reliably pick between N clients across
restarts. The real answer there is the identity beacon, which needs, in order:

1. `ears/src/capture/windows.rs` — WASAPI per-process loopback
2. an identity listener in the app, mapping pid → character from the addon's login tones
3. the editor offering `Death-Tailor` instead of `World of Warcraft`

Steps 2 and 3 are small; step 1 is the work. The addon already emits identity (mark tone plus
three digits, 9 bits) and `rig::names` already resolves it against the `WTF` character folders.

### The addon installer

The app embeds the addon with `include_bytes!` and installs it into any WoW it finds.
**Bump `## Version:` in `addon/WarcryBeacon/WarcryBeacon.toc` whenever the addon changes**, or
the UI cannot tell there is an update. `hack/install-addon.sh` is the macOS shell equivalent
and is gitignored; a PowerShell version would help on Windows.

## Windows: what the next session needs to do

0. The app now hosts the companion, so everything below applies to the app as much as the CLI.
   `companion::tap` already prefers `BackgroundSink` on Windows, so a phone press into one client
   should not steal focus from the others once capture works.
1. **`ears/src/capture/windows.rs`.** Mirror the macOS backend: `pub fn open(pid: u32, tx:
   Sender<Vec<f32>>) -> Result<Tap>` with teardown in `Drop`. Use `ActivateAudioInterfaceAsync`
   with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS
   { TargetProcessId: pid, ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE }`.
   Windows 10 2004+. `capture/mod.rs` has the `cfg` seam and a stub that errors.
2. **`rig/src/find.rs` is macOS-only** — it shells out to `ps`. On Windows use
   `warcry::window::list()`, which has a real Windows implementation. The process is `Wow.exe`.
3. **Use `BackgroundSink`** (`src/window/windows.rs`, `#[cfg(windows)]`) for presses instead of
   raise-then-type. It posts keys to a specific window without stealing focus, which is the
   whole reason multiboxing works properly on Windows. Drop `--focus` and `rig::raise()` there.
4. **Turn on "Sound in Background"** in WoW's sound options, or unfocused clients emit no audio
   and nothing can be detected. The addon warns if the cvar is 0.
5. `sounds/bobber.json` should transfer — the templates are of WoW's own sounds. Verify with
   `replay` on Windows recordings before trusting it.

## Gotchas

- **Toolchain**: asdf provides Rust without `rustup`, so `rustup target add` is unavailable and
  iOS cross-compilation is blocked. Set `ASDF_RUST_VERSION=1.96.1`. Edition 2024, MSRV 1.85.
- **`cmake` is required** to build the app at all — `whisper-rs-sys` needs it. On Windows,
  winget or the VS build tools.
- **`rtk` wrapper**: a hook rewrites shell commands, swallows `cargo test` summaries and mangles
  multi-file `grep`. Use `rtk proxy cargo test ...` to see real output -- and note that a plain
  `cargo clippy` through the hook can report "No issues found" while clippy is failing.
- **Never pipe a long build into `tail` and background it.** The `whisper-rs-sys` build script
  deadlocked for 26 minutes writing into a full pipe whose reader had been killed: no compiler
  children, no file writes, holding `.cargo-lock` the whole time. Redirect to a log file instead.
- **A process is only a client if it runs from the app bundle.** `rig::find` used to match on the
  basename alone, so any file named "World of Warcraft" counted -- two copies in a scratch
  directory were enumerated as rigs 0 and 1 ahead of the real client. It now also requires
  `/World of Warcraft.app/Contents/MacOS/`.
- **CI stops at the first crate that fails to lint.** A clean lint of the relay said nothing
  about rig, hub or app, which had eight more errors between them. Always lint `--workspace`.
- **`.gitattributes` keeps the served assets LF.** Without it, git hands Windows CRLF copies of
  `relay/ui/*` and anything that reasons about the HTTP body by splitting on a blank line breaks.
- **Postcard is not self-describing**, so serde internally-tagged enums (`#[serde(tag = "...")]`)
  silently fail to decode. All protocol enums are externally tagged. Do not add a tag.
- **Never hand enigo a `Key::Unicode` on macOS.** It resolves the character by asking the Text
  Services Manager for the current layout, and `TSMGetInputSourceProperty` calls
  `dispatch_assert_queue(main)`. Every command runs on its own thread, so the assert fired and
  the whole app died with `EXC_BREAKPOINT` -- an abort, not a catchable panic. `Key::Char` now
  maps through `MAC_KEYCODES` to a raw CGKeyCode and goes out as `Key::Other`, which enigo
  passes straight through. This also removed 256 TSM round-trips per keypress. Keycodes are
  positional (US ANSI), which is what a game binding wants; an unmapped character is now a
  clean "not supported" error instead of a crash. It hid for so long because the shipped
  profile targeted a window that never matched, and fishing only ever sent right-clicks.
- **macOS permissions**: `enigo` needs Accessibility and `osascript` needs Automation, both
  granted to the *terminal* that launched Warcry, not to Warcry. `rig` prints
  `accessibility: granted|DENIED` at startup. Screenshotting needs Screen Recording too, or
  `screencapture` silently returns wallpaper with every window stripped out.

### Never block the main thread, and know how to find it when you do

Five separate hangs came from blocking work on a thread that could not afford it. When
something freezes, **`sample <pid> 3` first** — it found the last one in thirty seconds after
two rounds of guessing.

- `snapshot` is a *synchronous* Tauri command, so it runs on the main thread. It used to call
  `window::find` (shells out to `osascript` to enumerate every window, ~190ms) and
  `audio::devices()`. A background thread now probes every 2 seconds and `snapshot` reads the
  cache. **Never put I/O in a sync command.**
- `stop` dropped an `Engine`, whose `Drop` **joins the engine thread**. A session still opening
  cannot see the quit signal while loading a speech model, so the join never returned and the
  webview could not even paint. The engine is now dropped on a background thread.
- `warcry::window::focus()` polls `active()` up to 30 times and each call enumerates every
  window via AppleScript — up to 15s per keypress. `rig::raise()` does one `osascript` in 0.19s.
- `snapshot` held `engine`'s lock for its whole body while building the companion view, whose
  rigs closure wants that same lock to read the engine's profile. The main thread deadlocked
  holding it, and the phone's websocket thread then blocked in `Hosted::welcome`, so the symptom
  was "the page loads but nothing ever answers". Read what you need out of a lock and drop the
  guard; `companion::current` additionally uses `try_lock` and falls back to the profile on disk.
- `osascript` had no deadline. A single AppleEvent that never came back left the `probe` thread
  parked in `poll` on the child's pipe forever, so the window list and the device list stopped
  updating and the app had to be force-quit. `sample <pid> 3` named the frame in one shot:
  `spawn_probe` -> `window::list` -> `macos::osascript` -> `Command::output` -> `poll`. Every
  shell-out now dies after `DEADLINE` (5s) and the probe keeps its last good answer instead of
  replacing it with an empty list. **This is macOS-only** -- the Windows `list()` calls
  `EnumWindows` and spawns nothing.
- **Never swallow a shell-out error.** `unwrap_or_default()` hid an `osascript` failure and
  turned `--focus` into a silent no-op for an hour.

## Code style

No comments. No `///`, no `//!`, no `//`. Avoid `else` unless genuinely required (`let ... else`
for early return is fine). Keep methods short and extract a helper past about three steps.
Prefer `match`, `then`, `then_some`, `map`, `filter_map`, `is_some_and` and iterator chains over
imperative branching. Match the surrounding code — the original `src/` is entirely comment-free.
