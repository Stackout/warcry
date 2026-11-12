# Warcry companion — working notes

A phone companion for Warcry. Warcry listens to a game window's audio, works out when
something happened, and tells a phone. A human presses a button on the phone; the desktop
forwards that press to the game as a keystroke.

Built and proven end to end on macOS 15.1 against World of Warcraft (`_forever_`, interface
16001) on 2026-11-12. Next target is a Windows multibox rig.

## The invariant that shapes everything

**Warcry must never automate gameplay.** It is an accessibility aid. The detection path
reports; a human acts. This is structural, not a convention:

- `ears` (all detection) has no dependency that can reach the input layer. `cargo tree -p
  warcry-ears` has no `enigo`. Keep it that way.
- A keystroke can only originate from a `Press` that arrived over the network and passed
  `warcry-relay::host::Hosted`, which checks a pairing token, rejects replayed sequence
  numbers, and rate-limits to 10/s.
- A `Press` names an *action slot*, never a key. The desktop decides what slot 0 means, so a
  hostile client can ask for "the thing the user called cast" and nothing else.
- There is deliberately no message meaning "reel when you detect a bite". Do not add one.

WoW exposes no event for a bobber splash — that is deliberate on Blizzard's part. That is why
the bite is *heard* rather than queried, and why the addon only ever plays sounds.

## Crates

| crate | job |
| --- | --- |
| `warcry` (root) | the original voice engine. Provides `keys`, `input` (enigo), `window` (focus, and `BackgroundSink` on Windows). |
| `app/` | Tauri 2 tray app. Does not know about the companion yet. |
| `link/` | wire protocol, pairing token, admission gate. Postcard for native, JSON for browsers (`Token` serialises as hex when the format is human-readable). |
| `ears/` | per-window audio capture and all detection. Cannot reach input. |
| `relay/` | LAN server: serves the PWA and the phone websocket. `host::Hosted` holds the admission rules and is shared with the hub path. |
| `rig/` | ties it together. The CLI you actually run. Also the only place a press becomes a keystroke. |
| `hub/` | optional public fan-out service. Written and unit-tested, never deployed. |

## How a bite is found

1. The `WarcryBeacon` addon plays a short pure tone when a fishing cast starts, ends, lands
   loot, when bags are full, and when a lure runs down. Eight frequencies, `ears::tone::FREQS`,
   each an exact analysis-bin multiple. Decoded with a Goertzel bank.
2. `ears::detect::Listener` arms only inside a cast window (~30s), is deaf for the first 600ms
   (the cast sound itself is loud), and reports at most one bite per cast.
3. `ears::library` matches learned sound templates. A `Sound` holds several exemplar `Patch`es
   and matches on the best of them (nearest neighbour, not a centroid — WoW has several splash
   variants and averaging them produces mush).

Things that were tried and failed, so don't retry them:

- **RMS onset detection.** The splash is quieter than 0.02 rms while the beacon tones are ~0.18,
  so an onset detector fires on Warcry's own beeps. Dead end.
- **Log-mel patches with one global mean removed.** Spectral tilt dominates, so arbitrary game
  music scores 0.95 against the bobber. Patches now centre *each band across time*
  (`Patch::from_spectra`), which encodes how bands move rather than their absolute shape.
- **Matching without an onset gate.** Normalised shape alone matches silence. Each `Sound`
  stores a `flux` floor learned from its training onsets; matching requires that much spectral
  rise, aligned to the same frame offset used in training.
- **Single-alignment matching.** Audio arrives at arbitrary frame alignment. `Library::aligned`
  tries `ALIGNMENTS` offsets and takes the best.

## Commands

```sh
warcry-rig windows                       # find WoW pids
warcry-rig listen --pid <pid> --serve --port 8137 \
  --sounds sounds/bobber.json --token-file hack/token \
  --advertise <reachable-ip> --focus --record hack/casts
warcry-rig teach-from hack/casts/*.wav --name bobber --out sounds/bobber.json
warcry-rig refine a.json b.json --name bobber --min 0.65 --out sounds/bobber.json
warcry-rig replay hack/casts/*.wav --sounds sounds/bobber.json   # hit/miss per cast
warcry-rig analyze hack/casts/*.wav                              # rank spectral onsets
warcry-rig teach --pid <pid> --name <sound> --want 8             # human-labelled, press Enter
```

`teach-from` is the good one: it finds the sound automatically in the 3.0s–1.0s window before
each cast ended (the reel moment), blanking anything near a decoded beacon tone so it cannot
train on Warcry's own beeps. No labelling needed.

`replay` is the regression test for the detector. **Run it before and after any change to
`ears::library` or a retrain.** Measuring beats tuning; a threshold was once set so high that
0 of 11 casts detected and nobody noticed for an hour.

## State of play

Working and verified on macOS: beacon decode, cast arming, bite detection, phone alerts over
LAN, and phone press → `'` keystroke in game.

Not verified:

- **Bite hit rate.** The library in `sounds/bobber.json` is 12 exemplars at threshold 0.50.
  That threshold is a guess — the held-out casts were destroyed by a filename collision before
  they could be measured. Fish ~10 casts, then `replay`.
- **False positive rate** was ~1 per 30s of idle game audio at threshold 0.50, measured with
  music playing. During a cast BetterFishing mutes music, so it should be far lower, but that
  was never measured separately.
- **Bags full and lure alerts.** The addon emits them, the decoder reads them, `rig` maps them
  to alerts, and no phone has ever received one.
- **The hub.** `hub/` passes 9 unit tests. Its HTTP/websocket layer has no integration test and
  it has never run against a real phone.

## Windows: what the next session needs to do

1. **`ears/src/capture/windows.rs`.** Mirror the macOS backend's shape: `pub fn open(pid: u32,
   tx: Sender<Vec<f32>>) -> Result<Tap>`, with teardown in `Drop`. Use
   `ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` and
   `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: pid, ProcessLoopbackMode:
   PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE }`. Windows 10 2004+. This is what makes
   N windows work: each WoW client is its own pid with its own stream.
   `capture/mod.rs` already has the `cfg` seam and a stub that errors.
2. **`rig/src/find.rs` is macOS-only** — it shells out to `ps` and matches
   `world of warcraft`. On Windows the process is `Wow.exe`; use `warcry::window::list()`
   (which has a real Windows implementation) instead of adding a second shell-out.
3. **Use `BackgroundSink`** (`src/window/windows.rs`, `#[cfg(windows)]`) for the press instead
   of raise-then-type. It posts keys to a specific window without stealing focus, which is the
   whole reason multiboxing works properly on Windows and only approximately on macOS. Drop
   `--focus` and the `raise()` osascript path there.
4. **Install the addon** to each `World of Warcraft\<flavor>\Interface\AddOns\WarcryBeacon`.
   `hack/install-addon.sh` is macOS-only and gitignored; a PowerShell equivalent would help.
5. **Turn on "Sound in Background"** in WoW's sound options, or unfocused clients emit no audio
   at all and nothing can be detected. The addon prints a warning if the cvar is 0.
6. `sounds/bobber.json` should transfer — the templates are of WoW's own sounds, not of this
   Mac. Verify with `replay` on Windows recordings before trusting it.

## Gotchas

- **Toolchain**: asdf provides Rust without `rustup`, so `rustup target add` is unavailable and
  iOS cross-compilation is blocked. Set `ASDF_RUST_VERSION=1.96.1`. Edition 2024, MSRV 1.85.
- **`rtk` wrapper**: a hook rewrites shell commands and swallows `cargo test` summaries and
  mangles multi-file `grep`. Use `rtk proxy cargo test ...` to see real output.
- **Postcard is not self-describing**, so serde internally-tagged enums (`#[serde(tag = "...")]`)
  silently fail to decode. All protocol enums are externally tagged. Don't add a tag.
- **macOS permissions**: `enigo` needs Accessibility and `osascript` needs Automation, both
  granted to the *terminal* that launched Warcry, not to Warcry. `rig` prints
  `accessibility: granted|DENIED` at startup.
- **Don't call `warcry::window::focus()` per keypress.** It polls `active()` up to 30 times and
  each call enumerates every window via AppleScript — up to 15s per press. `rig::raise()` does
  one `osascript` in 0.19s.
- **Never swallow a shell-out error.** Two bugs here were `unwrap_or_default()` hiding an
  `osascript` failure, which turned `--focus` into a silent no-op.
- `hack/` is gitignored and holds the pairing token and recordings. **The token is a secret;
  don't commit it.** The trained library lives in `sounds/` so it is tracked.

## Code style

No comments. No `///`, no `//!`, no `//`. Avoid `else` unless genuinely required (`let ... else`
for early return is fine). Keep methods short and extract a helper past about three steps.
Prefer `match`, `then`, `then_some`, `map`, `filter_map`, `is_some_and` and iterator chains over
imperative branching. Match the surrounding code — the original `src/` is entirely comment-free.
