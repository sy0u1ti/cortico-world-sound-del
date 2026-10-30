# Changelog

## 0.1.4

First public release, based on the locally tested 0.1.4 runtime.

- Trigger playback muting from the existing recording shortcut without patching Coopanion, remapping input or injecting keys.
- Use a passive keyboard hook, bounded ordered transitions and immediate supervisor wakeups; preserve rapid double taps.
- Prewarm Core Audio interfaces, invalidate topology on changes, and avoid duplicate initial scans and owned-device readbacks.
- Keep durable write-ahead recovery, existing mute state, detected manual overrides, cancellation, lease expiry and a separate hang watchdog.
- Distinguish request receipt from successful mute readback with bounded, privacy-conscious completion diagnostics.
- Include a prebuilt Windows x64 helper, source, tests, documentation and third-party notices in release packages.
- Declare the built-in desktop-pet voice-module dependency and experimental hardware-compatibility status.

## 0.1.3

Local development release; not previously published on npm.

- Preserve a held gesture when the host's recording-start notification is delayed.
- Allow a fresh gesture after an earlier release timeout.
- Batch durable recovery writes and keep successful restoration receipts when clearing the journal fails.
- Prefer the default playback endpoint and distinguish individually blocked calls from a progressing multi-device pass.

## 0.1.2

Local development release; not recommended for installation.

- Add direct shortcut detection, host eligibility, hold/toggle gestures and notification fallback.
- Hide the internal loopback URL from ordinary settings.

## 0.1.1

Local development release.

- Add acquisition-scoped volume notifications, bounded callback state, supervised workers and recovery verification.

## 0.1.0

Local prototype.

- Add notification-driven playback muting and restoration for Windows.
