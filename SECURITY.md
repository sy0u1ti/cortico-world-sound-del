# Security And Privacy

Sound Del controls playback mute, not microphone capture. A passive global
keyboard hook receives system keyboard callbacks, filters to the configured
shortcut, and retains only bounded chord transitions. It always forwards input
and does not record text or provide general keyboard access to a model.

The only runtime network target is a validated loopback origin used to follow
Coopanion's existing voice state. Authentication remains in the host's secret
store. Recovery files contain endpoint identifiers; diagnostic files intentionally
omit them, transcripts and credentials.

Do not attach an entire host configuration, `.env`, `.npmrc`, recording or
recovery journal to a public issue. Review diagnostic files before sharing.
Use GitHub's private vulnerability reporting for sensitive findings when it is
enabled; ordinary reproducible bugs can use a public issue.

Supported security-fix line: latest 0.1.x release. Older local prototypes are
not distributed as supported npm versions.
