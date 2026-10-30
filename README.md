# Sound Del · 收音静音

**Automatically mute background playback while Coopanion records your voice, then restore it when you finish.**

Sound Del brings a familiar voice-input feature to Coopanion: mute while speaking, like tools such as WeChat Input (微信输入法) and 网易兔八哥. No need to pause your music or video manually before talking.

[中文说明](README_zh.md) · [Download](https://github.com/sy0u1ti/cortico-world-sound-del/releases) · [Report an issue](https://github.com/sy0u1ti/cortico-world-sound-del/issues)

## Features

- **Keep your shortcut.** Trigger muting directly from Coopanion's existing recording shortcut, without changing the host.
- **Restore afterward.** Finish or cancel recording to restore playback; already-muted devices stay muted.
- **Respect your controls.** Does not change the volume level or fight detected manual unmute overrides.

## Install

Requires **Windows x64** and Coopanion's built-in `desktop-pet` voice module with push-to-talk enabled.

1. In **World extensions → Extension market**, search for `cortico-world-sound-del`.
2. Install, restart if prompted, and enable **Sound Del**.
3. Use your usual recording shortcut. For `LeftAlt*2`, tap once, then press and hold; muting starts on the second press.

If the package is not yet visible in the market, download the Windows ZIP from [Releases](https://github.com/sy0u1ti/cortico-world-sound-del/releases). Extract it and select the whole folder containing `package.json`, not `dist` or `bin`. Disable the old World before updating.

The extension mutes all active playback outputs, including Coopanion's own audio. It is not microphone noise cancellation; response time depends on Windows and your audio device.

Third-party extension, not affiliated with the input methods mentioned. [Setup and recovery](USAGE.md) · [MIT License](LICENSE)
