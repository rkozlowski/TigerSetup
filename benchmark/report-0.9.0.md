# TigerSetup 0.9.0 against the benchmark baseline: Inno Setup 7.1.0, NSIS 3.12, TigerSetup 0.7.1

The installer-technology benchmark ([`report.md`](../report.md)) measured
**TigerSetup 0.7.1**, **Inno Setup 7.1.0** and **NSIS 3.12**
packaging the same four open-source applications, statically on the build
machine and at runtime on TigerWinLab's clean Windows 11 baseline. That
report is frozen: its numbers are historical evidence and are not regenerated
here. This report re-runs the TigerSetup side of the same experiment for
**TigerSetup 0.9.0** — the same canonical payloads, the same package
definitions, the same lab baseline, contracts and timing boundaries — and
adds the build-side evidence the first campaign lacked: every one of the
fifteen installer definitions rebuilt on one host under one method, with
wall clock, CPU time and peak memory of the compiler's whole process tree.

Every number below is read from `benchmark/results/*.json` (historical)
and `benchmark/results/0.9.0/*.json` (fresh) by
`benchmark/scripts/New-ComparisonReport.ps1`; nothing is transcribed by
hand. The three classes of evidence are kept apart throughout:

| Class | What | Where measured | When |
|---|---|---|---|
| **A. Historical** | Inno Setup, NSIS and TigerSetup 0.7.1: installer bytes, build wall clock, the runtime rows | the build machine; `TigerWinLab-Win11-Clean` | 2026-09-19 14:35 +01:00 (lab); 2026-09-19 15:06 +01:00 (builds) |
| **B. Fresh host builds** | all 15 installers rebuilt: bytes, wall, CPU, peak memory | this host (below), serial, one run each | 2026-09-20 19:10 +01:00 → 2026-09-20 19:16 +01:00 |
| **C. Fresh runtime** | TigerSetup 0.9.0 only: the four application rows | `TigerWinLab-Win11-Clean` | 2026-09-20 19:20 +01:00 |

A comparison across classes is a comparison across campaigns: package
bytes are exact and compare freely; build figures compare within class B
(one host, one method, one afternoon); runtime figures compare across A and
C because the lab, the baseline checkpoint, the contracts and the timing
boundaries are the same, but they remain single observations on a VM and
differences of a second or two are noise.

## Provenance

**Product under test.** TigerSetup 0.9.0, commit `ac77e6a` (uncommitted working tree at build time: the benchmark scripts of this campaign),
built with `cargo build --release`: builder `tiger-setup.exe`
3,008,000 bytes `e2dd09d058cfa627b62332de67a105bb1439ea16f286313f0e17b35bc209351f`; engine
`tigersetup-setup.exe` 2,489,344 bytes `9a9272a8021a9fbb02dae4e6dae1e8be2bdcf1f77480d577e9b2719faefd31e7`; loader `tigersetup-loader.exe` 74,752 bytes `3ba78d2ae6ec64a47f0f91bde9c865e59612dffd68eefaf2246080d608d639af`.
Every TigerSetup installer built here was checked with `tiger-setup inspect`
to carry exactly that engine. The historical TigerSetup 0.7.1 engine was
`5e77289c7d299053e043c02cfed43d281d8c5b49609af44887a758a455602f2a` (commit `f991a24`); it was not rebuilt or rerun.

**Compilers.** Inno Setup 7.1.0 (`C:\Program Files\Inno Setup 7\ISCC.exe`,
`d06ebd38f38e3cee60a3c50cc45bd449d77e0bc6a5cabc607ea9886808e4de1a`; engine `Setup.e64` `ad12a06d09afefa9…`) and
NSIS 3.12 (`C:\Program Files (x86)\NSIS\makensis.exe` `b043e554afefbfc56315669d0b4779793aeae67f0f2a7a790e2ea91f05298eff`, the compiler it launches `Bin\makensis.exe` `25d1aa7081db1a9d…`; exehead `3507903b63ab7517…`).
Both are byte-identical to the compilers the historical campaign used.
Compression settings are the first campaign's: Inno Setup `Compression=lzma2/max, SolidCompression=yes`, NSIS `SetCompressor /SOLID lzma`,
TigerSetup's release build (its default compression search; never `--fast`).

**Host (class B).** Microsoft Windows 11 Pro 25H2 (build 26200);
AMD Ryzen 7 5700X 8-Core Processor, 8 cores / 16 logical processors;
32,692.8 MiB RAM;
repository on C: KINGSTON SNV3S2000G (SSD, NVMe, 1,907,729.1 MiB);
PowerShell 7.6.6. Single-run measurements on a live workstation, not an isolated
benchmark host; the OS file cache was not flushed between builds, and the
technology order rotates between applications so no technology always
builds first into a cold cache or last into a warm one.

**Lab (class C).** TigerWinLab baseline `TigerWinLab-Win11-Clean`, the checkpoint the
historical rows used; one session per row, the VM restored to the checkpoint
before the install job, the uninstall job on the same VM, the session closed
and the VM reported `Available` before the next row. Every row runs as the
lab's job account (an administrator with no interactive desktop), silently.

**Canonical payloads.** Verified before anything was built
(2026-09-20 18:47 +01:00): all four application payloads on disk are byte-identical to the committed inventories
(`benchmark/results/0.9.0/canonical-verification.json`, from `Test-CanonicalPayload.ps1`):

| App | Files | Bytes | Against inventory |
|---|---:|---:|---|
| ShareX | 1,230 | 541,619,146 | identical |
| WinMerge | 472 | 81,636,621 | identical |
| qBittorrent | 38 | 231,534,246 | identical |
| VLC | 583 | 191,373,841 | identical |

**Package definitions.** Unchanged from the first campaign, with one build-side
exception: each NSIS script's `OutFile` now takes `/DOUTFILE=` so a campaign
can build into artifacts of its own (the default path is the definition's
own; the installer's bytes do not depend on where it is written).

## 1. Package size

Exact bytes, class A for Inno Setup, NSIS and TigerSetup 0.7.1 (the
historical installers) and class B for TigerSetup 0.9.0. The Inno Setup
and NSIS installers were rebuilt in class B as well; the `rebuilt` column
says whether the fresh build reproduced the historical bytes exactly.

| App | Inno Setup | NSIS | TigerSetup 0.7.1 | TigerSetup 0.9.0 | 0.9.0 vs 0.7.1 | 0.9.0 vs smallest of IS/NSIS | rebuilt IS / NSIS identical |
|---|---:|---:|---:|---:|---:|---:|---|
| Minimal | 2,097,202 | 38,332 | 2,510,441 | **1,221,714** | −51.3% (-1,288,727) | +3,087.2% vs NSIS | yes / yes |
| ShareX | 139,596,731 | 140,047,177 | 205,630,804 | **151,478,421** | −26.3% (-54,152,383) | +8.5% vs IS | yes / yes |
| WinMerge | 18,785,970 | 17,231,634 | 27,405,232 | **18,988,950** | −30.7% (-8,416,282) | +10.2% vs NSIS | yes / yes |
| qBittorrent | 44,446,345 | 43,455,720 | 81,045,247 | **48,055,185** | −40.7% (-32,990,062) | +10.6% vs NSIS | yes / yes |
| VLC | 46,894,502 | 45,717,687 | 82,303,489 | **42,413,898** | −48.5% (-39,889,591) | −7.2% vs NSIS | yes / yes |

`Minimal` is the fixed cost of a technology before any payload: its
TigerSetup installer carries one 38-byte file. The installer-to-payload
ratio, for the four applications:

| App | Canonical bytes | Inno Setup | NSIS | TigerSetup 0.7.1 | TigerSetup 0.9.0 |
|---|---:|---:|---:|---:|---:|
| ShareX | 541,619,146 | 25.8% | 25.9% | 38.0% | 28.0% |
| WinMerge | 81,636,621 | 23.0% | 21.1% | 33.6% | 23.3% |
| qBittorrent | 231,534,246 | 19.2% | 18.8% | 35.0% | 20.8% |
| VLC | 191,373,841 | 24.5% | 23.9% | 43.0% | 22.2% |

## 2. Fresh host build statistics (class B)

All fifteen builds on the host above, serial, one run each, every compiler
under the same meter (`benchmark/scripts/ProcessTreeMeter.psm1`): the
compiler starts suspended inside a job object of its own and is resumed, so
every process it creates is measured with it (NSIS's `makensis.exe` is a
launcher for `Bin\makensis.exe`). **Wall** runs from the resume until the job
holds no process. **CPU** is the job's kernel-maintained accounting (user +
kernel time of every process that ever belonged to it). **Peak commit** is
the job's `PeakJobMemoryUsed` — the highest commit charge of all its
processes together at any instant — also kernel-maintained and exact.
**Peak working set** is sampled every 20 ms: the largest sum of the
processes' working sets seen in one sample (the tree) and the largest
Windows-tracked per-process peak read while that process was alive (the
largest single process). Windows keeps no job-wide working-set peak, so the
sampled figures can miss a spike shorter than the interval or a process that
lives and dies between two samples; the sample count and the processes seen
against the job's total say how much a build was covered — a build of a few
tens of milliseconds is covered by its commit figures, not by its samples.

| App | Technology | Installer bytes | Build wall | CPU | CPU / wall | Peak tree WS | Largest process WS | Peak commit (job) | Samples / processes |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Minimal | Inno Setup | 2,097,202 | 0.71 s | 0.83 s | 1.16 | 47 MB | 47 MB | 43 MB | 25 / 1 of 1 |
| Minimal | NSIS | 38,332 | 0.03 s | 0.03 s | 0.91 | 4 MB | 6 MB | 97 MB | 3 / 1 of 2 |
| Minimal | TigerSetup | 1,221,714 | 0.86 s | 0.55 s | 0.64 | 64 MB | 64 MB | 62 MB | 29 / 1 of 1 |
| ShareX | Inno Setup | 139,596,731 | 73.39 s | 129.64 s | 1.77 | 118 MB | 118 MB | 113 MB | 2356 / 1 of 1 |
| ShareX | NSIS | 140,047,177 | 145.17 s | 144.66 s | 1.00 | 142 MB | 136 MB | 100 MB | 4649 / 2 of 2 |
| ShareX | TigerSetup | 151,478,421 | 152.80 s | 151.83 s | 0.99 | 282 MB | 282 MB | 279 MB | 4895 / 1 of 1 |
| WinMerge | Inno Setup | 18,785,970 | 9.09 s | 15.06 s | 1.66 | 119 MB | 119 MB | 115 MB | 301 / 1 of 1 |
| WinMerge | NSIS | 17,231,634 | 18.46 s | 18.33 s | 0.99 | 142 MB | 136 MB | 100 MB | 613 / 2 of 2 |
| WinMerge | TigerSetup | 18,988,950 | 19.05 s | 18.77 s | 0.99 | 231 MB | 231 MB | 228 MB | 636 / 1 of 1 |
| qBittorrent | Inno Setup | 44,446,345 | 35.77 s | 55.91 s | 1.56 | 117 MB | 117 MB | 112 MB | 1147 / 1 of 1 |
| qBittorrent | NSIS | 43,455,720 | 64.65 s | 64.41 s | 1.00 | 141 MB | 135 MB | 99 MB | 2140 / 2 of 2 |
| qBittorrent | TigerSetup | 48,055,185 | 75.27 s | 74.73 s | 0.99 | 282 MB | 282 MB | 278 MB | 2458 / 1 of 1 |
| VLC | Inno Setup | 46,894,502 | 26.63 s | 44.69 s | 1.68 | 118 MB | 118 MB | 113 MB | 854 / 1 of 1 |
| VLC | NSIS | 45,717,687 | 53.48 s | 53.34 s | 1.00 | 141 MB | 135 MB | 99 MB | 1712 / 2 of 2 |
| VLC | TigerSetup | 42,413,898 | 48.99 s | 48.55 s | 0.99 | 282 MB | 282 MB | 279 MB | 1568 / 1 of 1 |

The historical campaign recorded a wall clock around each compiler
invocation (class A, 2026-09-19 15:06 +01:00, the same host a day earlier, no
memory or CPU figures; `not measured` where it did not). Shown beside the
fresh wall clock for orientation only — the two are different campaigns, not
repeated samples of one:

| App | Technology | Wall, historical campaign | Wall, this campaign | Peak memory, historical |
|---|---|---:|---:|---|
| Minimal | Inno Setup | 0.9 s | 0.7 s | not measured |
| Minimal | NSIS | 0.0 s | 0.0 s | not measured |
| Minimal | TigerSetup (0.7.1 → 0.9.0) | 0.4 s | 0.9 s | not measured |
| ShareX | Inno Setup | 65.6 s | 73.4 s | not measured |
| ShareX | NSIS | 150.2 s | 145.2 s | not measured |
| ShareX | TigerSetup (0.7.1 → 0.9.0) | 47.3 s | 152.8 s | not measured |
| WinMerge | Inno Setup | 8.3 s | 9.1 s | not measured |
| WinMerge | NSIS | 18.1 s | 18.5 s | not measured |
| WinMerge | TigerSetup (0.7.1 → 0.9.0) | 7.0 s | 19.0 s | not measured |
| qBittorrent | Inno Setup | 34.0 s | 35.8 s | not measured |
| qBittorrent | NSIS | 65.2 s | 64.7 s | not measured |
| qBittorrent | TigerSetup (0.7.1 → 0.9.0) | 16.3 s | 75.3 s | not measured |
| VLC | Inno Setup | 25.1 s | 26.6 s | not measured |
| VLC | NSIS | 54.8 s | 53.5 s | not measured |
| VLC | TigerSetup (0.7.1 → 0.9.0) | 18.6 s | 49.0 s | not measured |

## 3. Runtime: install, uninstall, contract, payload

Class A rows (Inno Setup, NSIS, TigerSetup 0.7.1) and class C rows
(TigerSetup 0.9.0) side by side. Timing boundaries are the first
campaign's: install is the installer process's lifetime; uninstall is the
uninstaller's lifetime plus the exit of whatever it hands off to (NSIS's
`Au_.exe`) and the removal of the install root, the completion wait shown
as `(+wait)`. A TigerSetup 0.9.0 installer is a small loader that
extracts its engine and **waits for it**, so the installer process's lifetime
already spans the whole operation; the 0.9.0 rows additionally waited for
any process of the installer's own name (the engine keeps the package's
file name) and record that wait separately, so a hand-off could not have
escaped the clock. `Contract` is the number of install-time contract
probes that held, of those the application's contract names; `Payload
exact` is the installed payload against the canonical one: for the class C
rows every file's SHA-256 (the guest reader returned per-file hashes;
`extras` are what the technology added under the install root), for the
class A rows the file count and byte total the first campaign recorded.

| App | Technology | Class | Row | Install | Uninstall (+wait) | Contract | Payload exact | Extras under root | ARP | Start Menu link |
|---|---|---|---|---:|---:|---:|---|---:|---|---|
| ShareX | Inno Setup | A | ok | 18.34 s | 11.51 s (+0.62 s) | 4 / 4 | by count and bytes | 2 (4,768,493 B) | yes → yes | yes → yes |
| ShareX | NSIS | A | ok | 22.44 s | 1.81 s (+0.44 s) | 4 / 4 | by count and bytes | 1 (57,740 B) | yes → yes | yes → yes |
| ShareX | TigerSetup 0.7.1 | A | ok | 30.26 s | 24.04 s (+0.01 s) | 4 / 4 | by count and bytes | 0 (0 B) | yes → yes | yes → yes |
| ShareX | TigerSetup 0.9.0 | C | ok | 11.60 s | 2.42 s (+0.01 s) | 4 / 4 | yes (SHA-256, 1,230 files) | 0 (0 B) | yes → yes | yes → yes |
| WinMerge | Inno Setup | A | ok | 5.27 s | 2.20 s (+0.63 s) | 6 / 6 | by count and bytes | 2 (4,589,998 B) | yes → yes | yes → yes |
| WinMerge | NSIS | A | ok | 5.04 s | 1.00 s (+0.23 s) | 6 / 6 | by count and bytes | 1 (58,460 B) | yes → yes | yes → yes |
| WinMerge | TigerSetup 0.7.1 | A | ok | 10.90 s | 9.07 s (+0.01 s) | 6 / 6 | by count and bytes | 0 (0 B) | yes → yes | yes → yes |
| WinMerge | TigerSetup 0.9.0 | C | ok | 4.25 s | 1.07 s (+0.01 s) | 6 / 6 | yes (SHA-256, 472 files) | 0 (0 B) | yes → yes | yes → yes |
| qBittorrent | Inno Setup | A | ok | 7.22 s | 1.75 s (+0.63 s) | 7 / 7 | by count and bytes | 2 (4,490,496 B) | yes → yes | yes → yes |
| qBittorrent | NSIS | A | ok | 7.98 s | 1.28 s (+0.23 s) | 7 / 7 | by count and bytes | 1 (55,666 B) | yes → yes | yes → yes |
| qBittorrent | TigerSetup 0.7.1 | A | ok | 5.80 s | 11.31 s (+0.02 s) | 7 / 7 | by count and bytes | 0 (0 B) | yes → yes | yes → yes |
| qBittorrent | TigerSetup 0.9.0 | C | ok | 5.21 s | 0.58 s (+0.01 s) | 7 / 7 | yes (SHA-256, 38 files) | 0 (0 B) | yes → yes | yes → yes |
| VLC | Inno Setup | A | ok | 8.28 s | 1.83 s (+0.65 s) | 21 / 21 | by count and bytes | 2 (4,636,399 B) | yes → yes | yes → yes |
| VLC | NSIS | A | ok | 20.36 s | 1.08 s (+0.44 s) | 21 / 21 | by count and bytes | 1 (55,717 B) | yes → yes | yes → yes |
| VLC | TigerSetup 0.7.1 | A | ok | 19.29 s | 26.18 s (+0.01 s) | 21 / 21 | by count and bytes | 0 (0 B) | yes → yes | yes → yes |
| VLC | TigerSetup 0.9.0 | C | ok | 6.00 s | 1.28 s (+0.02 s) | 21 / 21 | yes (SHA-256, 583 files) | 0 (0 B) | yes → yes | yes → yes |

Read `ARP` and `Start Menu link` as *present after install → removed after
uninstall*. `Row` is the harness's verdict on the whole row: both jobs ran,
both processes exited 0, the completion wait was satisfied, every
install-time probe held, the payload was exact where it could be verified,
and every app-owned resource was gone after the uninstall.

**Contract probes after uninstall, TigerSetup 0.9.0** (a resource the
application owns must be gone; a *setting* is reported as observed; an
*absent* probe must still hold):

| App | Probe | After install | After uninstall |
|---|---|---|---|
| ShareX | Desktop shortcut (off by default) | ok — shortcut absent | still absent |
| ShareX | Send To entry (off by default) | ok — shortcut absent | still absent |
| ShareX | Start at sign-in (off by default) | ok — shortcut absent | still absent |
| ShareX | Explorer context menu (off by default) | ok — key absent | still absent |
| WinMerge | .WinMerge file association | ok — OpenWithProgids handler (never the default) | gone |
| WinMerge | WinMerge.Project.File ProgID | ok — ProgID class with shell\open\command | gone |
| WinMerge | App Paths entry | ok — App Paths entry | gone |
| WinMerge | Explorer context menu (COM shell extension) | ok — regsvr32 (DllRegisterServer, machine-wide HKCR) | gone |
| WinMerge | Add to PATH (off by default) | ok — PATH untouched | still absent |
| WinMerge | Desktop shortcut (off by default) | ok — shortcut absent | still absent |
| qBittorrent | .torrent file association | ok — OpenWithProgids handler (never the default) | gone |
| qBittorrent | qBittorrent.File.Torrent ProgID | ok — ProgID class with shell\open\command | gone |
| qBittorrent | magnet: URL protocol | ok — command under the scheme class | gone |
| qBittorrent | Firewall rule | ok — Windows Firewall rule | gone |
| qBittorrent | LongPathsEnabled = 1 | ok — registry value | observed: 0 |
| qBittorrent | Desktop shortcut (off by default) | ok — shortcut absent | still absent |
| qBittorrent | Startup shortcut (off by default) | ok — shortcut absent | still absent |
| VLC | App Paths entry | ok — App Paths entry | gone |
| VLC | .mp3 file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.mp3 ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .mp3 | ok — verb under Classes\SystemFileAssociations\.mp3 | gone |
| VLC | .flac file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.flac ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .flac | ok — verb under Classes\SystemFileAssociations\.flac | gone |
| VLC | .wav file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.wav ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .wav | ok — verb under Classes\SystemFileAssociations\.wav | gone |
| VLC | .mp4 file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.mp4 ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .mp4 | ok — verb under Classes\SystemFileAssociations\.mp4 | gone |
| VLC | .mkv file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.mkv ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .mkv | ok — verb under Classes\SystemFileAssociations\.mkv | gone |
| VLC | .avi file association | ok — OpenWithProgids handler (never the default) | gone |
| VLC | VLC.avi ProgID | ok — ProgID class with shell\open\command | gone |
| VLC | Play with VLC on .avi | ok — verb under Classes\SystemFileAssociations\.avi | gone |
| VLC | Play with VLC on a folder background | ok — verb under Classes\Directory\Background | gone |
| VLC | Desktop shortcut (off by default) | ok — shortcut absent | still absent |

**Totals over the four applications** (single measurements summed; a coarse
view, not a second measurement):

| Technology | Class | Install, all four | Uninstall, all four |
|---|---|---:|---:|
| Inno Setup | A | 39.11 s | 17.29 s |
| NSIS | A | 55.82 s | 5.17 s |
| TigerSetup 0.7.1 | A | 66.25 s | 70.60 s |
| TigerSetup 0.9.0 | C | 27.06 s | 5.35 s |

## 4. TigerSetup evolution: 0.7.1 → 0.9.0

The same packages, the same payloads, the same lab rows; class A against
class C for runtime, exact bytes for size.

| App | Installer bytes | Install | Uninstall (+wait) | Payload exact / extras under root | Where the bookkeeping is |
|---|---:|---:|---:|---|---|
| ShareX | 205,630,804 → 151,478,421 (−26.3%) | 30.26 s → 11.60 s (0.38×) | 24.04 s (+0.01 s) → 2.42 s (+0.01 s) (0.10×) | 0.7.1: by count/bytes, 0 extra; 0.9.0: SHA-256 exact, 0 extra | state database and uninstaller in the scope's TigerSetup state directory (both versions; not under the root, so not in `extras`) |
| WinMerge | 27,405,232 → 18,988,950 (−30.7%) | 10.90 s → 4.25 s (0.39×) | 9.07 s (+0.01 s) → 1.07 s (+0.01 s) (0.12×) | 0.7.1: by count/bytes, 0 extra; 0.9.0: SHA-256 exact, 0 extra | state database and uninstaller in the scope's TigerSetup state directory (both versions; not under the root, so not in `extras`) |
| qBittorrent | 81,045,247 → 48,055,185 (−40.7%) | 5.80 s → 5.21 s (0.90×) | 11.31 s (+0.02 s) → 0.58 s (+0.01 s) (0.05×) | 0.7.1: by count/bytes, 0 extra; 0.9.0: SHA-256 exact, 0 extra | state database and uninstaller in the scope's TigerSetup state directory (both versions; not under the root, so not in `extras`) |
| VLC | 82,303,489 → 42,413,898 (−48.5%) | 19.29 s → 6.00 s (0.31×) | 26.18 s (+0.01 s) → 1.28 s (+0.02 s) (0.05×) | 0.7.1: by count/bytes, 0 extra; 0.9.0: SHA-256 exact, 0 extra | state database and uninstaller in the scope's TigerSetup state directory (both versions; not under the root, so not in `extras`) |

**Where the 0.9.0 install time goes.** Each 0.9.0 row also brought back the
engine's own log and the guest's clock at the process's start and end,
so the loader's share of the lifetime is measured rather than assumed:
*before* is from the process start to the engine's first event
(`run_started`) — the loader locating the footer, decompressing and
verifying the engine, starting it, and the engine opening its log;
*engine* is the span from that event to `run_finished`; *after* is the
engine's exit and the loader's clean-up of its extraction directory.

| App | Step | Process lifetime | Before engine | Engine span | After engine | Loader share |
|---|---|---:|---:|---:|---:|---:|
| ShareX | install | 11.60 s | 5.47 s | 6.12 s | 0.01 s | 47.2% |
| ShareX | uninstall | 2.41 s | 0.89 s | 1.50 s | 0.01 s | 37.1% |
| WinMerge | install | 4.24 s | 1.96 s | 2.28 s | 0.01 s | 46.3% |
| WinMerge | uninstall | 1.06 s | 0.21 s | 0.83 s | 0.00 s | 20.5% |
| qBittorrent | install | 5.21 s | 2.72 s | 2.48 s | 0.01 s | 52.3% |
| qBittorrent | uninstall | 0.57 s | 0.21 s | 0.34 s | 0.01 s | 38.8% |
| VLC | install | 6.00 s | 2.68 s | 3.31 s | 0.00 s | 44.7% |
| VLC | uninstall | 1.26 s | 0.21 s | 1.04 s | 0.00 s | 16.5% |

*Before engine* on install is not the loader's decompression alone. It
grows with the installer's size — WinMerge 18.1 MiB: 1.96 s; VLC 40.4 MiB: 2.68 s; qBittorrent 45.8 MiB: 2.72 s; ShareX 144.5 MiB: 5.47 s —
while what the loader reads is the same engine block every time (the whole `Minimal` installer is 1.2 MiB) and
what the engine does before its first event is to open the package and
check the metadata block's hash, kilobytes. The uninstall, the same file
on the same VM a minute later, spends 0.21 s–0.89 s there. A cost that
scales with the file's bytes on its first launch and largely disappears on
its second is consistent with the platform's handling of a never-run
executable of that size on a clean Windows with real-time protection on;
this benchmark does not isolate it, and an Inno Setup or NSIS installer of
the same size goes through the same first launch inside its own install
time. What is TigerSetup's own in the column is the loader's work and the
engine's start-up, at most the uninstall's figure.

**Fixed overhead.** A TigerSetup 0.7.1 installer began with its
2,509,312-byte engine stored uncompressed
(`Minimal`: 2,510,441 bytes); a 0.9.0 installer begins with
a 74,752-byte C loader and the engine as one zstd block
(`Minimal`: 1,221,714 bytes). Against the smallest application payload
here (WinMerge, 81,636,621 bytes) that fixed cost is
6.4% of the 0.9.0 installer, where it was
9.2% of the 0.7.1 one; Inno Setup's fixed cost is
2,097,202 bytes and NSIS's 38,332.

## 5. Build-resource trade-off (class B)

What TigerSetup 0.9.0's `zstd-19-w27` solid payload costs at build time
against Inno Setup's `lzma2/max` and NSIS's `/SOLID lzma`, on one host
under one method, beside what it produces. Ratios are TigerSetup over the
named technology; a ratio above 1 means TigerSetup used more.

| App | Installer bytes: TS / IS / NSIS | Build wall: TS / IS / NSIS | TS wall vs IS / NSIS | CPU: TS / IS / NSIS | Peak commit: TS / IS / NSIS | TS commit vs IS / NSIS | Peak tree WS: TS / IS / NSIS |
|---|---|---|---|---|---|---|---|
| Minimal | 1,221,714 / 2,097,202 / 38,332 | 0.9 s / 0.7 s / 0.0 s | 1.20× / 25.26× | 0.5 s / 0.8 s / 0.0 s | 62 MB / 43 MB / 97 MB | 1.43× / 0.63× | 64 MB / 47 MB / 4 MB |
| ShareX | 151,478,421 / 139,596,731 / 140,047,177 | 152.8 s / 73.4 s / 145.2 s | 2.08× / 1.05× | 151.8 s / 129.6 s / 144.7 s | 279 MB / 113 MB / 100 MB | 2.46× / 2.79× | 282 MB / 118 MB / 142 MB |
| WinMerge | 18,988,950 / 18,785,970 / 17,231,634 | 19.0 s / 9.1 s / 18.5 s | 2.10× / 1.03× | 18.8 s / 15.1 s / 18.3 s | 228 MB / 115 MB / 100 MB | 1.99× / 2.28× | 231 MB / 119 MB / 142 MB |
| qBittorrent | 48,055,185 / 44,446,345 / 43,455,720 | 75.3 s / 35.8 s / 64.7 s | 2.10× / 1.16× | 74.7 s / 55.9 s / 64.4 s | 278 MB / 112 MB / 99 MB | 2.48× / 2.81× | 282 MB / 117 MB / 141 MB |
| VLC | 42,413,898 / 46,894,502 / 45,717,687 | 49.0 s / 26.6 s / 53.5 s | 1.84× / 0.92× | 48.5 s / 44.7 s / 53.3 s | 279 MB / 113 MB / 99 MB | 2.46× / 2.80× | 282 MB / 118 MB / 141 MB |

Over the four applications together: build wall TigerSetup 296.1 s, Inno Setup
144.9 s, NSIS 281.8 s (2.04× Inno Setup's, 1.05× NSIS's); CPU
293.9 s / 245.3 s / 280.7 s; the largest peak commit of any build 279 MB /
115 MB / 100 MB. The CPU-to-wall ratio in §2 says how each compiler
uses the host's cores: a ratio near 1 is one thread compressing; above it,
parallel compression.

## What the measurements answer

- **How much larger or smaller is TigerSetup 0.9.0 than Inno Setup and NSIS?**
  Against the smaller of the two on each application: ShareX +8.5%, WinMerge +10.2%, qBittorrent +10.6%, VLC -7.2%.
  The remaining difference is the fixed loader-plus-engine block
  (1,221,714 bytes against 38,332 for NSIS) and the codec: Zstandard
  level 19 with a 128 MiB window against LZMA/LZMA2, a trade the product
  made for decode speed inside the installation transaction
  (`TigerSetup-Design.md` §10.4, `benchmark/compression-spike/report.md`).
- **How much did TigerSetup shrink from 0.7.1?** ShareX -26.3%, WinMerge -30.7%, qBittorrent -40.7%, VLC -48.5%; `Minimal`
  −51.3%. Exact bytes, not an estimate.
- **How much longer does TigerSetup take to build?** On this host, per
  application, 1.8× to 2.1× Inno Setup's wall clock and 0.9× to 1.2× NSIS's;
  over the four applications 2.04× and 1.05×. In CPU time the
  picture is 1.20× Inno Setup's and 1.05× NSIS's: TigerSetup's
  compressor is single-threaded, Inno Setup's LZMA2 is not (§2, CPU / wall).
- **How much peak build memory does it consume?** Peak commit of the whole
  build tree: ShareX 279 MB, WinMerge 228 MB, qBittorrent 278 MB, VLC 279 MB — 2.0× to 2.5×
  Inno Setup's and 2.3× to 2.8× NSIS's on the same application (§5). Peak working
  set, sampled, is in §2 beside it.
- **How does 0.9.0 install time compare with 0.7.1, Inno Setup and NSIS?**
  Over the four applications: 27.06 s against 66.25 s for 0.7.1 (0.41×),
  39.11 s for Inno Setup (0.69×) and 55.82 s for NSIS (0.48×); per application in §3.
- **How does 0.9.0 uninstall time compare?** 5.35 s against 70.60 s for
  0.7.1 (0.08×), 17.29 s for Inno Setup (0.31×) and 5.17 s for NSIS (1.03×).
- **Did the transaction optimizations materially change runtime behaviour?**
  §4 is the answer per application: the install and uninstall ratios
  0.7.1 → 0.9.0, with the payload verified file by file and the same
  contract probes holding, and the split of each 0.9.0 lifetime into loader
  and engine. What changed between the versions is recorded in
  `benchmark/README.md` (the local A/B) and `TigerSetup-Design.md` §5.4,
  §5.10; this report measures the outcome on the clean VM.
- **Does the fixed loader/engine overhead remain significant on small
  packages?** In bytes, §4 (*Fixed overhead*); in time, the *before engine*
  and *after engine* columns of §4 on the smallest package here (WinMerge).
- **Which costs occur at build/distribution time versus during the
  installation transaction?** Build time, CPU and peak memory (§2, §5) are
  paid once per release on the build machine; installer bytes (§1) are paid
  per download; install and uninstall time (§3, §4) are paid per machine,
  inside the transaction. The benchmark measures each where it occurs and
  does not weigh one against another: which trade is right depends on how
  many machines install a release and how often it is built, which this
  experiment does not measure. Nor does it measure user preference.

## Anomalies, repeated rows and limitations of this campaign

- **ShareX builds repeated** — The first pass of the three ShareX builds (18:57–19:03 on 2026-09-20) overlapped the session's own editing of the lab scripts on the same host — short PowerShell and Python runs, one to three seconds of CPU each — so the whole ShareX package was rebuilt with the host otherwise idle and the repeated measurement replaced the first in build.json. First pass: TigerSetup 157.5 s wall / 156.8 s CPU / 293 MB peak commit; Inno Setup 76.3 s / 133.9 s / 119 MB; NSIS 157.4 s / 156.6 s / 105 MB. Idle pass (the figures reported): TigerSetup 152.8 s, Inno Setup 73.4 s, NSIS 145.2 s. All three rebuilt installers were byte-identical to the first pass. No other build was repeated.
- Every rebuilt Inno Setup and NSIS installer is byte-identical to the one the historical campaign measured (§1, the `rebuilt` column), and a TigerSetup 0.9.0 installer rebuilt twice came out byte-identical too: the fifteen definitions are reproducible to the byte on this toolchain, which is what lets a size comparison across campaigns be exact.
- The historical build wall clock (class A) was a stopwatch around the compiler invocation in the calling PowerShell, one day earlier on the same host; this campaign's wall clock (class B) is the job object's, from the resume of the suspended compiler to the last process's exit. The two definitions differ by process start-up and hand-off, a few milliseconds, and are shown side by side only for orientation.

## Limitations that apply to every number here

- Class B is a single run of each build on a live workstation, in one
  sitting, with the OS cache warm or cold as the rotation left it; a
  difference of ten per cent between two builds of a minute is within what a
  second sitting could move. The ratios between technologies on one
  application are the robust reading; the absolute seconds are this host's.
- Peak working set is sampled and can under-read a short build; peak commit
  is exact. Where the two disagree on a build of milliseconds, the commit
  figure is the one to trust.
- Class C is one measurement per row, as class A was; the boot-per-row
  cost of the lab makes repeats materially more expensive without changing
  what a second or two could show. The class A rows were measured on
  2026-09-19 14:35 +01:00 and the class C rows on 2026-09-20 19:20 +01:00: the same lab,
  baseline checkpoint and driver, a day apart, on a host that was not idle
  either time.
- Inno Setup and NSIS were not rerun in the lab; their runtime rows are the
  first campaign's, and nothing here says whether they would repeat to the
  second.
- Four applications are evidence about these four applications.

