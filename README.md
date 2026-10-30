# Sound Del

Mute Windows playback when you press Coopanion's recording shortcut, then restore it when recording ends.

**Keep stock Coopanion and your existing shortcut. No host patch, input remapping, injected keys, virtual audio driver, or microphone interception.**

[中文说明](README_zh.md) · [Releases](https://github.com/sy0u1ti/cortico-world-sound-del/releases) · [Report an issue](https://github.com/sy0u1ti/cortico-world-sound-del/issues)

## Requirements

- Windows x64; Cortico World API 6.
- Coopanion's built-in `desktop-pet` voice module with push-to-talk enabled.
- Node.js 22.12 or newer when running outside Coopanion's bundled environment.

This is a third-party extension, not an official Pal AI Lab product. It follows the
existing desktop-pet voice panel's local state and WebSocket endpoints. That
dependency is declared as `requires-extension`; it is not a standalone recorder.

## Install

In Coopanion's **World extensions > Extension market**, search for
`cortico-world-sound-del` (Chinese display name: **收音静音**) after npm publication
and index synchronization. Install, restart if prompted, then enable the World.

For a local install, download the ready-to-install ZIP from
[Releases](https://github.com/sy0u1ti/cortico-world-sound-del/releases), extract it,
and choose the directory containing `package.json`. Do not choose `dist` or `bin`.
GitHub's automatic source-code archives are source only; build those first.

For an update, disable the old World and verify playback is restored before
replacing or switching its package. Do not enable two copies. Restarting the
old installed directory does not switch to a separately downloaded version.

The npm package and release ZIP include the Windows helper; users do not need
Rust. No install-time script downloads or builds a native executable.

## How It Works

The extension reads the configured shortcut rather than asking you to enter it
again. For `LeftAlt*2` in hold mode, the first tap does not mute; the second,
held press triggers mute.

1. A passive low-level keyboard hook keeps only relevant chord transitions and
   immediately wakes a separate supervisor. It always forwards input.
2. A separate audio worker uses Windows Core Audio. Interfaces are prepared while
   idle, but current mute state is read again before capture.
3. A durable recovery record is committed before playback changes. SetMute is
   followed by GetMute verification.
4. Release, cancellation, stale host state, shutdown and duration limits trigger
   restoration. A separate watchdog handles an unresponsive audio worker.

The hook callback does no audio or disk work. A bounded queue preserves rapid
tap edges; missing events and suspended input are resynchronized. An unavailable
hook or a mouse-button chord uses polling instead. Host notifications remain a
fallback, not the sole start trigger.

Hold and toggle modes, one to three taps, and supported Windows key names are
recognized. Toggle key-up does not end recording. Hold key-up permits up to
350 ms of tail protection while waiting for the host's close event.

## Scope And Safety

- Mutes **all active playback endpoints**, including Coopanion's own playback.
  It does not selectively mute other apps, change numeric volume, or mute the microphone.
- Preserves pre-existing mute and detected manual unmute overrides.
- Uses local HTTP/WebSocket state only. It does not record microphone audio,
  transcripts or arbitrary keyboard input, or send telemetry to an external service.
- Exposes no model tools or events and causes no model calls of its own.
- Keeps recovery and bounded diagnostics in `%LOCALAPPDATA%\CoopanionSoundDel`,
  outside the installed package.
- Session/directory locks prevent competing helpers. A stuck worker is terminated
  before at most one recovery-only worker takes ownership. Unknown restoration
  stays visibly unknown and retains its recovery record.

The ordinary settings page only exposes the maximum mute duration: 300 seconds
by default, configurable from 10 to 900. The World list controls enabled state.

The internal `worlds.sound-del.webUiUrl` defaults to `http://127.0.0.1:17788`.
It is retained for non-default ports but hidden from ordinary settings. If the
local WebUI requires authentication, set `CORTICO_SOUND_DEL_WEB_PASSWORD` or
`CORTICO_WEB_PASSWORD` in the host's existing secret mechanism. Restart the World
after changing configuration; never publish credentials with the extension.

## Recovery And Diagnostics

After a terminal helper fault, restart this World. An enabled switch does not
by itself mean a failed helper is running.

If restoration remains pending, disable the World, reconnect affected devices,
and run from the installed package directory:

```powershell
npm run probe
npm run recover
```

`probe` is read-only. `recover` may unmute devices recorded in the recovery
journal. Do not run recovery alongside an active World or delete the journal to
clear an error. Hard failures can prevent observation of later manual changes.

Diagnostic files:

| File | Meaning |
| --- | --- |
| `last-failure.json` | Terminal fault and recent bounded operation timings |
| `last-slow-operation.json` | Successful calls taking at least 100 ms |
| `last-latency.json` | Trigger-to-worker and verified-mute completion measurements |

Timing distinguishes `hook`, `poll`, `host` and `simulation`, with the audio
backend recorded separately. `dispatchMs` only means worker receipt.
`firstMuteMs` requires successful SetMute and GetMute; `allMutedMs` requires the
initial full pass. Missing completion is `null`, not zero.

## Limits

**This cannot guarantee physical zero latency or that every microphone sample
starts after silence.** Stock Coopanion and the plugin respond independently to
the same key. Scheduling, durable recovery writes, device buffers and room
reverberation still matter. Software readback is not an acoustic measurement.

The first public release is marked `experimental`: the author's Windows setup
has been manually tried, but different drivers, exclusive-mode playback,
Bluetooth devices and sustained system load have not all been qualified.
[VALIDATION.md](VALIDATION.md) includes both passing tests and a retained
disk-heavy simulation failure. Test your own playback/recording setup before
depending on it.

## Development

Build in a separate checkout, never over an enabled installation:

```powershell
npm ci
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component llvm-tools-preview,rustfmt,clippy
npm run build
npm run typecheck
npm test
npm run check:extension
npm run verify:package
```

Runtime sources are under `src/` and `native/src/`; tests use isolated simulated
audio. They never synthesize system keystrokes or mute the actual Windows device.
The native listener lifecycle test installs an unconfigured passive hook only.

Packaging, privacy checks and the official market flow:
[PUBLISHING.md](PUBLISHING.md). License: [MIT](LICENSE); third-party notices:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
