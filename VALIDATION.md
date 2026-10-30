# Validation

Runtime baseline: 0.1.4. Date: 2026-10-30.

Version 0.1.5 updates documentation, package descriptions and version metadata;
the TypeScript and Rust runtime sources are unchanged. The baseline evidence
below is retained rather than presented as a new hardware trial.

The 0.1.5 build passed 55 Rust tests, 71 TypeScript/contract/process tests,
typecheck, Clippy and the same upstream extension checker again before
publication. Its rebuilt helper SHA-256 is
`4ffe5f30316a3acb4e33096ac31d18c22b4bca9c161480ccf1aa3ca25d86874e`.

Version 0.1.6 changes the default package description to Chinese and adds a
metadata regression test; runtime sources remain unchanged. Its build passed
55 Rust tests, 72 TypeScript/contract/process tests, typecheck, Clippy and the
same upstream extension checker. The rebuilt helper SHA-256 is
`5730ace500a63ff6a878e504695dba979e71ad458c38faf450ee79fdbc277fbf`.

## Evidence

| Check | Result |
| --- | --- |
| Rust tests | 55 passed |
| TypeScript, Cortico contract and cross-process tests | 71 passed |
| TypeScript typecheck | Passed |
| Clippy, all targets with warnings denied | Passed |
| Rapid double-tap regression | Old loop missed batched transitions; new loop triggered and restored |
| Disk-backed simulation stress | 600 cycles completed with an empty recovery journal and confirmed stop |
| Read-only Windows probe | Three playback endpoints; no SetMute calls |
| Manual trial | The user reported no apparent remaining issues in their Windows x64 / Coopanion 0.2.16 setup |
| Official upstream extension checker | Passed against Cortico commit `78e43502acbb082d6b7a587273784f359e554505` (World API 6) |
| Runtime dependency audit | No known production dependency vulnerabilities reported by npm at publication preparation |

The manual trial is user-reported acceptance, not a controlled acoustic benchmark.
It does not establish a physical key-to-silence latency bound.

The baseline 0.1.4 helper SHA-256 is:

```text
cbf3187e7b2357aac2692ec62398a6c42c967767301444afe3c1202a67fdde61
```

## Separated Measurements

On one Windows machine, 100 double-tap cycles with already-muted simulated
outputs isolated control/status latency: the median fell from 13.74 ms in 0.1.3
to 1.08 ms in 0.1.4 (p95: 20.15 ms to 1.62 ms). These cycles did not SetMute or
commit a recovery record. They are **not a real audio-muting benchmark**.

Getter-only Core Audio profiling over three real outputs used identical debug
build settings. Warm endpoint enumeration median was 6.779 ms without the cache
and 0.0018 ms on a cache hit. GetMute median was 0.0235 ms and 0.0182 ms.
Cold enumeration and device changes do not receive the cache-hit guarantee.

Thirty separate system-disk recovery-journal cycles measured a median durable
acquisition write of 8.764 ms (p95 11.511 ms). That safety cost remains before
the first playback mutation.

The 600-cycle simulation run reported a median start/status round trip of
8.65 ms, p95 10.17 ms and maximum 20.57 ms. Supervisor/worker handle samples
stayed at 120/114. Those figures describe the simulated backend and do not
include an actual keyboard hook event, real SetMute or room acoustics.

## Retained Failure

A different stress run placed both the simulated audio file and recovery file
on the system disk and performed dense, repeated durable writes:

- 0.1.3 completed 200 cycles, but p95 start/status latency reached 169 ms and the maximum was about 817 ms.
- 0.1.4 completed 110 cycles before the disk-backed simulated `audio.restore` stopped making progress and triggered the watchdog. Start measurements also had long tails.
- The helper reported one unconfirmed restoration instead of claiming success.
  A subsequent isolated recovery of that simulated directory succeeded.

The simulated setter itself writes, flushes and renames a file. This does not
identify a real Core Audio or driver defect, nor prove that the recovery-journal
API alone was responsible. The failure has not been hidden by increasing the
watchdog limit or removing durable recovery.

## What Is Not Guaranteed

Automated checks do not record the microphone, synthesize physical input or
mute the real Windows playback endpoint. Listener lifecycle tests install only
an unconfigured passive hook. Different drivers, exclusive-mode playback,
Bluetooth, sleep/resume and sustained real-world contention require additional
hardware testing.

Software mute readback does not prove acoustic silence. A third-party plugin
cannot impose a capture barrier on unmodified Coopanion. Treat this initial
public release as experimental outside the tested setup.
