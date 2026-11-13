<p align="center"><img src="assets/icon.png" width="160" alt="Warcry"></p>

<h1 align="center">Warcry</h1>

<p align="center"><b>Speak, and Warcry presses the keys.</b></p>

Warcry is an open-source, cross-platform voice command engine for games and apps, in the spirit of VoiceAttack. Say "cast the line" and it right-clicks in your game, even while you're typing in another window. It's written in Rust, runs entirely offline with [whisper.cpp](https://github.com/ggml-org/whisper.cpp), and works on **Windows, macOS, and Linux**.

```
$ warcry run default.toml
  INFO speech model ~/.local/share/warcry/models/ggml-base.en.bin
  INFO microphone "Blue Yeti" at 48000 Hz
  INFO targeting window title~"World of Warcraft" (FocusRestore)
  INFO ready. Ctrl+C to quit
  INFO ▶ cast  (heard "Cast the line.", 100%)
  INFO ▶ reel  (heard "Reel it in!", 100%)
```

## Features

- **Offline speech recognition.** Your voice never leaves your machine. The model downloads automatically on first run.
- **Fuzzy matching.** "real in" still triggers "reel in", and "okay, throw my line please" still finds "throw my line".
- **Flexible phrases.** `[cast;throw] [the;my;] line` covers every combination with one pattern.
- **Window targeting.** Commands go to the game window, not whatever you're focused on, and focus comes back to you afterwards.
- **Background input (Windows).** Post keys and clicks straight to a window without focusing it at all.
- **Keyboard, mouse, and text actions.** Press, hold, combos, clicks, mouse holds, scrolling, typing, waits, random waits, and loops.
- **Stop anything instantly.** "stop" cancels running macros and releases any held key or button, so nothing gets stuck.
- **Push-to-talk** (hold or toggle) or always-listening, plus "start/stop listening" voice commands.
- **Hotkeys.** Any command can also fire from a key combo.
- **Text-to-speech** confirmations using your OS's built-in voice.
- **Hot reload.** Save the profile and changes apply immediately.

## Install

Grab the installer for your computer from [Releases](../../releases) and double-click it. No terminal needed.

| Platform | Download |
| --- | --- |
| Windows 10/11 | `Warcry_X.Y.Z_x64-setup.exe` (or the `.msi`) |
| macOS Apple Silicon | `Warcry_X.Y.Z_aarch64.dmg` |
| macOS Intel | `Warcry_X.Y.Z_x64.dmg` |
| Linux | `Warcry_X.Y.Z_amd64.deb` or `.AppImage` |

Warcry lives in your system tray. Click the tray icon to open the window, or right-click it to pause listening, start or stop, or quit. Closing the window keeps Warcry running in the tray.

**First run:** click **Create a profile**, then flip **Listen** on. The speech model (142 MB) downloads automatically. The **Target** menu in the toolbar picks the one window Warcry drives — by pid while that client is running, by title after a relaunch. **Test** tries a phrase without speaking, and **Open file** edits the profile in your text editor; saved changes apply immediately.

The icon rail on the left holds **Live** (what Warcry hears), **Commands**, **Phone**, **Log** and **Settings**, where the microphone, speech model, addon and startup options live.

### Your phone as a button deck

Open the **Phone** tab and press **Turn on**. Warcry serves a small page from your own
computer; point your phone's camera at the QR code to open it. No account, no cloud, nothing
leaves your network — the page talks straight to your machine over its local address, and the
link carries a pairing token so only a phone you showed the code to can press anything.

Every command you tick as a phone button gets a key on the deck, and pressing it runs exactly
what saying it would. Tick them in the **Commands** tab, or with **Give it a button on my
phone** in the editor.

If World of Warcraft is running with the [Warcry beacon addon](addon/) installed, the deck also
gets a cast and reel button per game window, and your phone chimes when a bobber bites — the
addon tells Warcry a cast started, Warcry listens for the splash, and *you* press reel. It
never fishes for you.

### Command-line version

Each release also includes a `warcry-cli-…` archive with the `warcry` command for scripting and power users. Or build it with `cargo install --git https://github.com/Stackout/warcry warcry`.

## Command-line quick start

```sh
warcry init                     # writes the default example to your config dir
warcry windows warcraft         # find the window you want to target
warcry check                    # validate the profile and list commands
warcry simulate ~/.config/warcry/profile.toml "cast the line" --dry-run
warcry run                      # start listening
```

`warcry init` writes to `%APPDATA%\warcry\profile.toml` on Windows, `~/Library/Application Support/warcry/profile.toml` on macOS, and `~/.config/warcry/profile.toml` on Linux. `warcry run` uses that file unless you pass a path.

## Profiles

A profile is a TOML file with settings, an optional target window, and commands.

```toml
name = "Fishing"

[settings]
min_confidence = 0.75
push_to_talk = "ctrl+alt"

[target]
title = "World of Warcraft"
mode = "focus_restore"

[[command]]
name = "cast"
phrase = "[cast;throw] [the;my;] [line;rod;pole;]"
actions = [{ do = "click", button = "right" }]

[[command]]
name = "auto fish"
phrase = "start [auto;] fishing"
stop_others = true
actions = [
  { do = "repeat", actions = [
    { do = "click", button = "right" },
    { do = "wait_random", min_ms = 15000, max_ms = 20000 },
    { do = "click", button = "right" },
  ] },
]

[[command]]
name = "stop"
phrase = "stop;cancel"
hotkey = "ctrl+shift+x"
always_active = true
no_target = true
actions = [{ do = "stop_all" }]
```

See [`profiles/`](profiles) for complete examples.

### Phrase syntax

| Pattern | Matches |
| --- | --- |
| `reel in` | "reel in" |
| `reel in;pull it in` | either phrase |
| `[cast;throw] line` | "cast line", "throw line" |
| `cast [the;] line` | "cast line", "cast the line" (an empty option makes the group optional) |
| `[reel [it;] in;pull]` | groups can nest |

Numbers are normalized, so a command for `slot 2` matches when whisper hears "slot two", and vice versa.

### Command options

| Key | Meaning |
| --- | --- |
| `name` | Name used in logs and by `execute`. Defaults to the phrase. |
| `phrase` | Voice trigger (syntax above). |
| `hotkey` | Keyboard trigger, e.g. `"ctrl+shift+f9"`. |
| `target` | Override the profile's `[target]` for this command. |
| `no_target` | Send input to whatever is focused, ignoring `[target]`. |
| `always_active` | Fires even while listening is turned off. Use for "start listening" and "stop". |
| `stop_others` | Cancel all running commands before starting. |
| `actions` | List of actions to run in order. |

### Actions

| `do =` | Fields | Notes |
| --- | --- | --- |
| `press` | `key`, `hold_ms` | Tap a key. Minimum hold is 30 ms so games register it. |
| `key_down` / `key_up` | `key` | Hold or release a key. |
| `combo` | `keys`, `hold_ms` | `"ctrl+shift+s"`: press in order, release in reverse. |
| `type` | `text` | Type text. |
| `click` | `button`, `count`, `hold_ms` | `left`, `right`, `middle`, `back`, `forward`. |
| `mouse_down` / `mouse_up` | `button` | Hold or release a mouse button. |
| `mouse_move` | `x`, `y`, `relative` | Absolute screen coordinates, or relative movement. |
| `scroll` | `amount`, `horizontal` | Positive is down/right. |
| `wait` | `ms` | Can be cancelled by "stop". |
| `wait_random` | `min_ms`, `max_ms` | Random delay, so repeated macros aren't perfectly regular. |
| `repeat` | `times`, `actions` | Leave out `times` to repeat until stopped. |
| `focus` | `title`, `process`, `class` | Bring a window to the front. |
| `run` | `program`, `args` | Launch a program. |
| `say` | `text` | Text-to-speech. |
| `log` | `message` | Print to the console. |
| `execute` | `command` | Run another command's actions by name. |
| `stop_all` | | Cancel every running command. |
| `listening` | `state` | `on`, `off`, or `toggle`. |

Key names: single characters (`a`, `1`, `/`), `space`, `enter`, `tab`, `esc`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown`, `up`/`down`/`left`/`right`, `shift`/`lshift`/`rshift`, `ctrl`/`lctrl`/`rctrl`, `alt`/`lalt`/`ralt`, `win`/`cmd`/`super`, `f1`–`f24`, `numpad0`–`numpad9`, `capslock`, `printscreen`, `pause`.

### Settings

| Key | Default | Meaning |
| --- | --- | --- |
| `min_confidence` | `0.78` | 0–1. How close speech must be to a phrase to fire. |
| `push_to_talk` | unset | Key or combo for push-to-talk. Unset means always listening. |
| `push_to_talk_mode` | `"hold"` | `hold` or `toggle`. |
| `start_listening` | `true` | Whether voice commands are active at startup. |
| `model` | `"base.en"` | Whisper model name or path to a ggml `.bin` file. |
| `language` | `"en"` | Language for multilingual models. |
| `input_device` | default mic | Substring of the microphone name (see `warcry devices`). |
| `vad_threshold` | `0.012` | Minimum loudness counted as speech. Raise it in noisy rooms. |
| `silence_ms` | `450` | Pause length that ends a phrase. |
| `max_utterance_ms` | `8000` | Longest phrase. |
| `speak_confirmations` | `false` | Say each command's name when it fires. |

## Window targeting

Run `warcry windows` (optionally with a filter) to see what can be targeted, then set `[target]` in your profile or `target` on a command. Every field you set must match, as a case-insensitive substring:

```toml
[target]
title = "World of Warcraft"
process = "wow.exe"
mode = "focus_restore"
```

| `mode` | Behavior |
| --- | --- |
| `focus_restore` (default) | Focus the target, run the command, then return focus to what you were doing. |
| `focus` | Focus the target and leave it there. |
| `background` | **Windows only.** Post input directly to the window without focusing it. Your keyboard and mouse stay completely free. Some games ignore posted input because they read raw input or DirectInput; use `focus_restore` for those. On other platforms this falls back to `focus_restore`. |

If the target window isn't found, the command doesn't run, so keys never land in the wrong app.

## Models

```sh
warcry model list
warcry model download small.en
warcry transcribe clip.wav my-profile.toml   # test recognition offline
```

`base.en` (142 MB) is the default and runs in real time on any modern CPU. Use `tiny.en` on slow machines, or `small.en` if you want better accuracy.

## Platform notes

**Windows**: works out of the box. Run Warcry as administrator if your game runs as administrator, because Windows blocks input from lower-privilege processes.

**macOS**: grant your terminal **Microphone**, **Accessibility**, and **Input Monitoring** permission (System Settings → Privacy & Security). Window targeting also asks for **Automation** permission the first time. Release binaries are signed with a Developer ID and notarized by Apple, so Gatekeeper runs them without warnings.

**Linux**: needs X11 or XWayland. Games under Proton/Wine and most apps run through XWayland and can be targeted. Native Wayland windows can't be targeted or sent synthetic input, because Wayland prevents that by design. Text-to-speech uses `spd-say` or `espeak-ng` if installed. Runtime libraries: `libasound2`, `libx11`, `libxi`, `libxtst`.

## Building from source

Requires Rust 1.85+, CMake, and Clang/libclang (for whisper.cpp). The tray app also needs the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and the Tauri CLI (`cargo install tauri-cli --version "^2"`).

```sh
# Debian/Ubuntu
sudo apt install libasound2-dev libx11-dev libxi-dev libxtst-dev cmake clang libclang-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
cargo test --workspace
cargo run -p warcry-app            # the tray app
cargo run -p warcry -- --help      # the command-line version
cd app && cargo tauri build        # installers for your platform
```

The repo is a workspace: the root crate is the engine library plus the `warcry` CLI, and `app/` is the Tauri tray app with a plain HTML/CSS/JS frontend in `app/ui`. Cargo features on the engine: `whisper` (speech recognition) and `download` (model downloads) are on by default; `--no-default-features` builds a hotkey-only binary with no C++ dependencies.

## Releasing

Push a tag that starts with `v`. GitHub Actions builds installers and command-line archives for Windows, macOS (Apple Silicon and Intel), and Linux, signs the macOS builds with a Developer ID, submits them for notarization, and publishes everything to a GitHub Release with SHA-256 checksums:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

## Fair play

Warcry sends ordinary keyboard and mouse input, like any macro tool. Some online games forbid automation such as unattended repeat loops. Check the rules of the game you're playing.

## License

MIT
