# Installer-technology benchmark: TigerSetup 0.7.1 vs Inno Setup 7.1.0 vs NSIS 3.12

Three Windows installer technologies — **TigerSetup 0.7.1**, **Inno Setup
7.1.0** and **NSIS 3.12** — packaging the same four real, open-source
Windows applications with each, plus a minimal no-payload installer per
technology, measured statically on the build machine and at runtime on
TigerWinLab's clean Windows 11 baseline (`TigerWinLab-Win11-Clean`).

The reproducible harness — acquisition, canonical payloads, package
definitions, build, measurement, the lab campaign driver and this report's
generator — lives under [`benchmark/`](benchmark/README.md). Every number
below is read from `benchmark/results/*.json` by
`benchmark/scripts/New-Report.ps1`; nothing is transcribed by hand.

## How this report came to be

The benchmark was first run against TigerSetup 0.7.0. That prototype run
exposed two problems before any runtime number could be trusted, and both
were corrected before the measurement campaign this report is built from:

- **A TigerSetup capability gap.** qBittorrent's real installer writes
  `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled`, and
  TigerSetup 0.7.0's typed `[[registry]]` resource reached only the scope's
  `Software` root, so the prototype package fell back to a custom action
  running `reg.exe` from a hard-coded `System32` path — the wrong
  abstraction, and not a portable package definition. TigerSetup 0.7.1
  added explicit registry locations to the typed resource (`root = "HKLM"`),
  with the pre-installation state recorded and restored, and the package
  now declares the value as the typed resource it is.
- **A benchmark harness defect.** The prototype's NSIS rows for the
  per-user applications ran the installer without `/S`, so the wizard waited
  invisibly on the desktop-less job account until the timeout; the
  prototype also timed an NSIS uninstall by the lifetime of `Uninstall.exe`,
  which copies itself to `%TEMP%` and exits while the copy does the work,
  and papered over the gap with a fixed sleep. The harness now waits for the
  actual completion — the hand-off process's exit and the install root's
  removal — and no artificial delay is anywhere in the measured path.

TigerSetup was then frozen at **0.7.1** (engine
`5e77289c7d299053e043c02cfed43d281d8c5b49609af44887a758a455602f2a`, commit `f991a24`, uncommitted working tree),
every installer was rebuilt from the definitions in this repository, and
**every measurement below comes from that single clean campaign**
(`benchmark/results/lab-results.json`, started 2026-09-19 14:35 +01:00). No
prototype measurement was kept.

## Scope and methodology

**Question.** With real applications and real measurements, how does
TigerSetup's packaging output compare with two established installer
technologies — on installer size, on install and uninstall time on a clean
Windows 11, on what each installer actually leaves on the machine, and on
how much of each application's installer-level contract each technology
can express natively rather than by script or custom action?

**Applications.** Four open-source Windows applications whose upstream
installers are themselves built with the technologies under comparison —
so the `Upstream tech` column is an independent reference point, not a
benchmark artifact — chosen together for the installer features they
exercise: per-user and per-machine scope, file associations, a URL
protocol, classic and COM context-menu integration, a firewall rule, PATH,
and a system-wide registry setting.

| App | Version | Upstream tech | Licence | Canonical source |
|---|---|---|---|---|
| ShareX | 21.0.0 | Inno Setup | GPL-3.0 | `ShareX-21.0.0-portable-x64.zip` (208,850,419 bytes, SHA-256 `8124938fa718d702…`, matches the published hash) |
| WinMerge | 2.16.58.2 | Inno Setup | GPL-2.0-or-later | `winmerge-2.16.58.2-x64-exe.zip` (24,493,933 bytes, SHA-256 `5732474add39283f…`, matches the published hash) |
| qBittorrent | 5.2.3 | NSIS | GPL-2.0-or-later (NOASSERTION on GitHub due to OpenSSL exception wording) | `qbittorrent_5.2.3_x64_setup.exe` (43,100,219 bytes, SHA-256 `ff508e2f912d59c9…`) |
| VLC | 3.0.23 | NSIS | GPL-2.0-or-later | `vlc-3.0.23-win64.zip` (79,893,405 bytes, SHA-256 `992d19dbd0b8a7cd…`, matches the published hash) |

No application was built from source; every payload comes from an official
release artifact pinned in `benchmark/results/apps.json` and verified in
`downloads.json`. WinMerge's real version is the four-part `2.16.58.2`;
TigerSetup requires a strict three-part version, so all three benchmark
packages for WinMerge use `2.16.58` for identity parity.

**Canonical payloads.** For a given app, all three technologies install
byte-identical files: the canonical payload, derived from the official
artifact by `benchmark/scripts/New-CanonicalPayload.ps1` with every
normalization recorded, and inventoried file by file with SHA-256
(`benchmark/results/canonical-<App>.json`).

| App | Canonical files | Canonical bytes | Normalizations |
|---|---:|---:|---|
| ShareX | 1,230 | 541,619,146 | Removed the "Portable" marker file (portable-mode flag; a real install never has it). |
| WinMerge | 472 | 81,636,621 | Flattened the archive's nested "WinMerge\" directory (zip-packaging artifact, not part of the app tree). |
| qBittorrent | 38 | 231,534,246 | Payload derived by extracting the official NSIS installer (7-Zip's NSIS reader), the only available official artifact. Excluded $PLUGINSDIR (NSIS installer scratch space, never written to $INSTDIR). Excluded uninst.exe (NSIS's generated uninstaller stub; each benchmark technology generates its own uninstaller natively). Kept qbittorrent.pdb (debug symbols): present in the official installer beside qbittorrent.exe, as the upstream installer.nsh File /r glob ships it -- observed upstream behaviour, not a benchmark artifact. |
| VLC | 583 | 191,373,841 | Flattened the archive's nested "vlc-3.0.23\" directory (zip-packaging artifact, not part of the app tree). Excluded msi\ (VLC's own WiX/.wxs source for building an MSI -- an alternate installer technology's build sources shipped inside the zip, not an application file). |

**Functional contracts.** Each application's installer-level contract is
derived from its real upstream installer script and written down once in
`benchmark/packages/<app>/contract.md`, with every simplification stated
(VLC's 125+ associations reduced to a representative six; WinMerge's
component matrix collapsed to one full install). All three package
definitions of an app implement that contract, every option defaulting to
what the real installer defaults to, and each technology uses its own
natural primitive for each feature — a native primitive where it has one,
its script where it has not, a custom action where none of the three has
a typed form.

**Compression.** Each technology's commonly recommended production-quality
setting: Inno Setup `Compression=lzma2/max, SolidCompression=yes`; NSIS `SetCompressor /SOLID lzma`;
TigerSetup's release build (its default compression search — never
`--fast`, which the project's release discipline excludes from anything
measured or published).

**Runtime measurement.** Every row is one TigerWinLab session on one VM:
the VM is restored to the `TigerWinLab-Win11-Clean` checkpoint, the installer is
staged and run silently as the job account (an administrator with no
interactive desktop), the machine is read — the install root, the
registry keys, shortcuts, firewall rules and PATH the contract names — the
uninstaller is run silently, the machine is read again, the session is
closed, and the next row starts only after the lab reports the VM
normalized and available. Rows never overlap. Technology order rotates
between applications. **Timing boundaries are identical across
technologies**: install: the installer process lifetime; uninstall: the uninstaller process lifetime plus the exit of the processes it hands off to (NSIS: Au_.exe) and the removal of the install root; no artificial delay. One measurement per row: the lab's
restore-and-boot cost per row makes repeated samples materially more
expensive, so the figures are single controlled observations, not
distributions — treat differences of a second or two as noise.

**The installed payload is a control, not a score.** Every installer must
leave exactly the canonical files under the install root, plus its own
uninstaller bookkeeping; identical installed sizes are the expected result.

## Toolchain

| Technology | Version | Raw engine / stub | Bytes | SHA-256 |
|---|---|---|---:|---|
| TigerSetup | 0.7.1 | `tigersetup-setup.exe` (embedded verbatim in every installer) | 2,509,312 | `5e77289c7d299053e043c02cfed43d281d8c5b49609af44887a758a455602f2a` |
| Inno Setup | 7.1.0 | `Setup.e64` (native x64 engine; embedded LZMA2-compressed behind `SetupLdr.e64`, 1,416,192 bytes) | 6,915,072 | `ad12a06d09afefa9d1283c6616ef4d56289dc14a7137a12b24653a12637216bb` |
| NSIS | 3.12 | `Stubs\lzma_solid-x86-unicode` (32-bit exehead of the chosen compressor) | 38,912 | `3507903b63ab7517fb9079e07b706c73ceeb668e99bbca269de3fef0c7376023` |

- Frozen for the benchmark; built with cargo build --release. The engine is embedded verbatim (uncompressed) in every generated installer.
- Setup.e64 is the native x64 setup engine an x64-target installer embeds (LZMA2-compressed, behind the SetupLdr.e64 loader); the minimal installer is therefore smaller than the uncompressed engine image.
- Stock NSIS ships only x86 (32-bit) exeheads; every NSIS installer here is a 32-bit process installing a 64-bit payload. The exehead named is the one SetCompressor /SOLID lzma with Unicode true selects; makensis additionally compiles the script into an "EXE header" (bytecode, strings, resources) it reports in its build log.

**Minimal installers** (identity and metadata only; TigerSetup's carries one
38-byte file because its builder requires a file set):

| Technology | Installer | Bytes | SHA-256 |
|---|---|---:|---|
| NSIS | `Minimal-NSIS.exe` | 38,332 | `36a49ca27dbc10ad2b6efadae83c3b3d6d34241c1d47166fe4fa838ab3d9df3e` |
| Inno Setup | `Minimal-InnoSetup.exe` | 2,097,202 | `363821daa2b717b6426dd0d5f3974ca7ab498d5af61c4b0795b8ab401edff459` |
| TigerSetup | `Minimal-TigerSetup.exe` | 2,510,441 | `2ef5eb2a1c432bbf99bf83c48044bed8c87badb3f652dc6dd2f3244366c19481` |

## Installer sizes

Exact bytes are the data; MiB is shown for readability. `Ratio` is
installer bytes over canonical payload bytes.

| App | Technology | Installer bytes | MiB | Ratio | vs. smallest | SHA-256 |
|---|---|---:|---:|---:|---:|---|
| ShareX | Inno Setup | 139,596,731 | 133.1 | 25.8% | smallest | `267107bcd00333ce376154f5bb71d95112292b195d56391eb445cc845046fa32` |
| ShareX | NSIS | 140,047,177 | 133.6 | 25.9% | +450,446 (+0.3%) | `faa86f804fd49151c03f94ea4420a3b4b7d63ba60f15934eb70b1b4ae6569d27` |
| ShareX | TigerSetup | 205,630,804 | 196.1 | 38.0% | +66,034,073 (+47.3%) | `c84df6038dfdf4b5332959e48294513a2399dce1caf02fd2eae3fe9d92fbcec3` |
| WinMerge | NSIS | 17,231,634 | 16.4 | 21.1% | smallest | `e006b174c572fdcf4a01f370028b9a629582fcc7592422085f842ba1979a4322` |
| WinMerge | Inno Setup | 18,785,970 | 17.9 | 23.0% | +1,554,336 (+9.0%) | `88c32702550f2f4d1c380735895cc194bc99f12661437189e534fd479c1ee0fa` |
| WinMerge | TigerSetup | 27,405,232 | 26.1 | 33.6% | +10,173,598 (+59.0%) | `4e8956cd98508635f3a5beda0a9ef273743e0b301aac5bb0046dc006d44431c2` |
| qBittorrent | NSIS | 43,455,720 | 41.4 | 18.8% | smallest | `23effaf8842cc172e850b883c8978ee1920d9955a40ad9243cb8066b31e06ff6` |
| qBittorrent | Inno Setup | 44,446,345 | 42.4 | 19.2% | +990,625 (+2.3%) | `4c90e1b31c675ff25210a468537470fa475bcadc8051e7d9c9f140138b6a1029` |
| qBittorrent | TigerSetup | 81,045,247 | 77.3 | 35.0% | +37,589,527 (+86.5%) | `e3cfa82be904a3316ec72b3cdbc82a5e955fbdf3c0127748b99980423fef1dc6` |
| VLC | NSIS | 45,717,687 | 43.6 | 23.9% | smallest | `0277ed54331ffa3aed825b8bf3166d153c7bc739a5f93057da389fa4c77c0895` |
| VLC | Inno Setup | 46,894,502 | 44.7 | 24.5% | +1,176,815 (+2.6%) | `00c86e1b66b0cb0aef73ad206984be75312843849d165f47cfb156aa3d244ba2` |
| VLC | TigerSetup | 82,303,489 | 78.5 | 43.0% | +36,585,802 (+80.0%) | `3d9cb2dabc7ea4626776e93936d7b493d7ede7f0c410fc6808e412a3a5aefee6` |

TigerSetup's payload block is a ZIP archive using `Stored` and `Deflate`
entries (`crates/tigersetup-format/src/payload.rs`); Inno Setup and NSIS
compress with LZMA2 and LZMA. That single format choice explains the
consistent size difference on every application; it is not a per-app
effect or a misconfiguration, and it is the benchmark's largest
quantitative finding about TigerSetup.

**Build times** (wall-clock around one compiler invocation on the build
machine, one run each; an engineering observation, not a compiler
benchmark):

| App | TigerSetup | Inno Setup | NSIS |
|---|---:|---:|---:|
| Minimal | 0.40 s | 0.90 s | 0.00 s |
| ShareX | 47.30 s | 65.60 s | 150.20 s |
| WinMerge | 7.00 s | 8.30 s | 18.10 s |
| qBittorrent | 16.30 s | 34.00 s | 65.20 s |
| VLC | 18.60 s | 25.10 s | 54.80 s |

## Runtime: install, uninstall, and what was left on the machine

Every row on the same clean Windows 11 baseline, silently, as described
above. `Installed` counts the files under the install root after the
install; `Extra` is what the technology added beyond the canonical
payload (Inno Setup's `unins000.exe`/`.dat`, NSIS's `Uninstall.exe`;
TigerSetup keeps its state outside the install root). `Uninstall` time
includes the completion wait the harness needed after the uninstaller
process exited, shown separately as `(+wait)`.

| App | Technology | Row | Install | Installed files / extra | Installed bytes / extra | Uninstall (+wait) | Root removed | ARP | Start Menu link |
|---|---|---|---:|---:|---:|---:|---|---|---|
| ShareX | Inno Setup | ok | 18.34 s | 1,232 / 2 | 546,387,639 / 4,768,493 | 11.51 s (+0.62 s) | yes | yes → yes | yes → yes |
| ShareX | NSIS | ok | 22.44 s | 1,231 / 1 | 541,676,886 / 57,740 | 1.81 s (+0.44 s) | yes | yes → yes | yes → yes |
| ShareX | TigerSetup | ok | 30.26 s | 1,230 / 0 | 541,619,146 / 0 | 24.04 s (+0.01 s) | yes | yes → yes | yes → yes |
| WinMerge | Inno Setup | ok | 5.27 s | 474 / 2 | 86,226,619 / 4,589,998 | 2.20 s (+0.63 s) | yes | yes → yes | yes → yes |
| WinMerge | NSIS | ok | 5.04 s | 473 / 1 | 81,695,081 / 58,460 | 1.00 s (+0.23 s) | yes | yes → yes | yes → yes |
| WinMerge | TigerSetup | ok | 10.90 s | 472 / 0 | 81,636,621 / 0 | 9.07 s (+0.01 s) | yes | yes → yes | yes → yes |
| qBittorrent | Inno Setup | ok | 7.22 s | 40 / 2 | 236,024,742 / 4,490,496 | 1.75 s (+0.63 s) | yes | yes → yes | yes → yes |
| qBittorrent | NSIS | ok | 7.98 s | 39 / 1 | 231,589,912 / 55,666 | 1.28 s (+0.23 s) | yes | yes → yes | yes → yes |
| qBittorrent | TigerSetup | ok | 5.80 s | 38 / 0 | 231,534,246 / 0 | 11.31 s (+0.02 s) | yes | yes → yes | yes → yes |
| VLC | Inno Setup | ok | 8.28 s | 585 / 2 | 196,010,240 / 4,636,399 | 1.83 s (+0.65 s) | yes | yes → yes | yes → yes |
| VLC | NSIS | ok | 20.36 s | 584 / 1 | 191,429,558 / 55,717 | 1.08 s (+0.44 s) | yes | yes → yes | yes → yes |
| VLC | TigerSetup | ok | 19.29 s | 583 / 0 | 191,373,841 / 0 | 26.18 s (+0.01 s) | yes | yes → yes | yes → yes |

Read `ARP` and `Start Menu link` as *present after install → removed after
uninstall*. `Row` is the harness's verdict on the whole row: both jobs ran,
both processes exited 0, the completion wait was satisfied, every
install-time contract probe held, and every app-owned resource was gone
after the uninstall. A row measured after a definition fix carries its own
`finishedAt`: qBittorrent-NSIS (15:08), VLC-NSIS (15:10) — the NSIS packages of the two 64-bit machine-scope applications first wrote their Add/Remove Programs entry into the 32-bit registry view and were corrected with `SetRegView 64`, as the real installers do.

**Totals over the four applications** (the same single measurements,
summed; a coarse view of the same data, not a second measurement):

| Technology | Install, all four apps | Uninstall, all four apps |
|---|---:|---:|
| Inno Setup | 39.11 s | 17.29 s |
| NSIS | 55.82 s | 5.17 s |
| TigerSetup | 66.25 s | 70.60 s |

TigerSetup's uninstall is the slowest of the three on every application,
by a wide margin; its install ranks (fastest = 1) ShareX 3 of 3, WinMerge 3 of 3, qBittorrent 1 of 3, VLC 2 of 3.
The engine journals every operation durably before and after each
mutation — the crash-consistency design (`TigerSetup-Design.md` §5.4) —
and pays for it in per-file commits; Inno Setup and NSIS delete a tree
without a journal. This is a real cost of the transactional model at this
engine version, reported in the gaps below.

## Functional parity

For each application, each feature of its contract with how each
technology's package definition implements it — **native**: a declarative
primitive of the technology; **script**: installer-script code written for
this package; **action**: a custom executable or script the installer runs;
**workaround**: something done outside the technology's own model — and
what the lab read from the machine after install (the mechanism found)
and after uninstall. The implementation column is a fact about the files
under `benchmark/packages/<app>/`; the result columns are measurements.

### ShareX

| Feature | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| Files, install root, per-user or per-machine scope | native (`PrivilegesRequiredOverridesAllowed=dialog`) | script (`MultiUser.nsh`: mode page, `.onInit` re-exec, `SHCTX`) | native (`scopes = ["user", "machine"]`) |
| Start Menu shortcut | native (`[Icons]`) | native (`CreateShortCut`) | native (`[[shortcuts]]`) |
| Desktop / Send To / Startup shortcuts (optional, off) | native (`[Tasks]` + `[Icons]`) | native (`Section /o` + `CreateShortCut`) | native (`[[options]]` + `[[shortcuts]]`) |
| Explorer context menu "Upload with ShareX" (optional, off) | native (`[Registry]` on `Classes\*\shell`) | native (`WriteRegStr` on `Classes\*\shell`) | native (`[[context_menu]]`) |
| Add/Remove Programs registration | native (automatic) | script (`WriteRegStr` × 8 by hand) | native (automatic) |

What the lab read after install and after uninstall (each probe's
mechanism as found; *ok* = the contract held, *gone* = the resource was
removed, a **setting** is reported as observed):

| Probe | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| Desktop shortcut (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |
| Send To entry (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |
| Start at sign-in (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |
| Explorer context menu (off by default) | ok — key absent; still absent | ok — key absent; still absent | ok — key absent; still absent |

### WinMerge

| Feature | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| Files, install root, per-user or per-machine scope | native | script (`MultiUser.nsh`) | native |
| Start Menu shortcut; desktop shortcut (optional, off) | native | native | native |
| `.WinMerge` file association | native (`[Registry]`, extension default = ProgID) | native (`WriteRegStr`, extension default = ProgID) | native (`[[file_associations]]`, handler under `OpenWithProgids`; never the default) |
| App Paths entry | native (`[Registry]`) | native (`WriteRegStr`) | native (`[[app_paths]]`) |
| Explorer context menu: the real COM shell extension | action (`[Run]`/`[UninstallRun]` regsvr32 via `{sys}`) | action (`ExecWait regsvr32.exe`) | action (`[[actions]]` post-install/pre-uninstall running a packaged `regsvr32.cmd`) |
| Add to PATH (optional, off) | script (~35 lines of Pascal: read-modify-write of the `Path` value) | script (~15 lines: `ReadRegStr`/`WriteRegExpandStr`/broadcast) | native (`[[path]]`) |
| Add/Remove Programs registration | native | script | native |

What the lab read after install and after uninstall (each probe's
mechanism as found; *ok* = the contract held, *gone* = the resource was
removed, a **setting** is reported as observed):

| Probe | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| .WinMerge file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| WinMerge.Project.File ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| App Paths entry | ok — App Paths entry; gone | ok — App Paths entry; gone | ok — App Paths entry; gone |
| Explorer context menu (COM shell extension) | ok — regsvr32 (DllRegisterServer, machine-wide HKCR); gone | ok — regsvr32 (DllRegisterServer, machine-wide HKCR); gone | ok — regsvr32 (DllRegisterServer, machine-wide HKCR); gone |
| Add to PATH (off by default) | ok — PATH untouched; still absent | ok — PATH untouched; still absent | ok — PATH untouched; still absent |
| Desktop shortcut (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |

### qBittorrent

| Feature | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| Files, install root, machine scope | native (`PrivilegesRequired=admin`) | native (`RequestExecutionLevel admin`, `SetShellVarContext all`) | native (`scopes = ["machine"]`) |
| Start Menu shortcut; desktop and Startup shortcuts (optional, off) | native | native | native |
| `.torrent` file association | native (extension default = ProgID) | native (extension default = ProgID) | native (handler under `OpenWithProgids`) |
| `magnet:` URL protocol | native (`[Registry]`: scheme class + command) | native (`WriteRegStr`: scheme class + command) | native (`[[url_protocols]]`: handler ProgID + capability registration; the scheme class is claimed only when free) |
| Firewall rule (optional, on) | action (`[Run]`/`[UninstallRun]` netsh via `{sys}`) | action (`ExecWait netsh.exe`) | native (`[[firewall]]`, Windows Firewall API) |
| `LongPathsEnabled = 1` under `HKLM\SYSTEM` (optional, on) | native (`[Registry]`; the value is left at uninstall) | native (`WriteRegDWORD`; the value is left at uninstall) | native (`[[registry]] root = "HKLM"`, new in 0.7.1; the prior value is restored at uninstall) |
| Add/Remove Programs registration | native | script | native |

What the lab read after install and after uninstall (each probe's
mechanism as found; *ok* = the contract held, *gone* = the resource was
removed, a **setting** is reported as observed):

| Probe | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| .torrent file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| qBittorrent.File.Torrent ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| magnet: URL protocol | ok — command under the scheme class; gone | ok — command under the scheme class; gone | ok — command under the scheme class; gone |
| Firewall rule | ok — Windows Firewall rule; gone | ok — Windows Firewall rule; gone | ok — Windows Firewall rule; gone |
| LongPathsEnabled = 1 | ok — registry value; after uninstall: 1 | ok — registry value; after uninstall: 1 | ok — registry value; after uninstall: 0 |
| Desktop shortcut (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |
| Startup shortcut (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |

### VLC

| Feature | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| Files, install root, machine scope | native | native (`SetShellVarContext all`) | native |
| Start Menu shortcut (in a `VideoLAN` group); desktop shortcut (optional, off) | native | native | native (the benchmark manifest puts the link directly in Programs; `folder = "VideoLAN"` on the `[[shortcuts]]` entry would have matched the group) |
| App Paths entry | native | native | native |
| Six file associations (`.mp3 .flac .wav .mp4 .mkv .avi`) | native (30 `[Registry]` lines, one block per extension) | native (a `!macro` per extension) | native (six `[[file_associations]]`; handlers, never the default) |
| "Play with VLC" on those types and on a folder background (optional, on) | native (`[Registry]` verbs under each ProgID and `Directory\Background`) | native (`WriteRegStr` verbs, same keys) | native (two `[[context_menu]]` entries: `SystemFileAssociations\<.ext>\shell` and `Directory\Background\shell`) |
| Add/Remove Programs registration | native | script | native |

What the lab read after install and after uninstall (each probe's
mechanism as found; *ok* = the contract held, *gone* = the resource was
removed, a **setting** is reported as observed):

| Probe | Inno Setup | NSIS | TigerSetup |
|---|---|---|---|
| App Paths entry | ok — App Paths entry; gone | ok — App Paths entry; gone | ok — App Paths entry; gone |
| .mp3 file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.mp3 ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .mp3 | ok — verb under Classes\VLC.mp3; gone | ok — verb under Classes\VLC.mp3; gone | ok — verb under Classes\SystemFileAssociations\.mp3; gone |
| .flac file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.flac ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .flac | ok — verb under Classes\VLC.flac; gone | ok — verb under Classes\VLC.flac; gone | ok — verb under Classes\SystemFileAssociations\.flac; gone |
| .wav file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.wav ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .wav | ok — verb under Classes\VLC.wav; gone | ok — verb under Classes\VLC.wav; gone | ok — verb under Classes\SystemFileAssociations\.wav; gone |
| .mp4 file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.mp4 ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .mp4 | ok — verb under Classes\VLC.mp4; gone | ok — verb under Classes\VLC.mp4; gone | ok — verb under Classes\SystemFileAssociations\.mp4; gone |
| .mkv file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.mkv ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .mkv | ok — verb under Classes\VLC.mkv; gone | ok — verb under Classes\VLC.mkv; gone | ok — verb under Classes\SystemFileAssociations\.mkv; gone |
| .avi file association | ok — extension default = ProgID; gone | ok — extension default = ProgID; gone | ok — OpenWithProgids handler (never the default); gone |
| VLC.avi ProgID | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone | ok — ProgID class with shell\open\command; gone |
| Play with VLC on .avi | ok — verb under Classes\VLC.avi; gone | ok — verb under Classes\VLC.avi; gone | ok — verb under Classes\SystemFileAssociations\.avi; gone |
| Play with VLC on a folder background | ok — verb under Classes\Directory\Background; gone | ok — verb under Classes\Directory\Background; gone | ok — verb under Classes\Directory\Background; gone |
| Desktop shortcut (off by default) | ok — shortcut absent; still absent | ok — shortcut absent; still absent | ok — shortcut absent; still absent |

## Implementation observations

- **Where TigerSetup needed a custom action**: one feature across the four
  applications — WinMerge's COM shell extension, which no technology has a
  typed primitive for; all three run `regsvr32`. TigerSetup's action is a
  packaged one-line batch script so the command shell resolves `regsvr32`
  (a `kind = "exe"` action runs exactly the program it names, and a
  package must not carry a `System32` path); Inno Setup has `{sys}` and
  NSIS relies on `CreateProcess`'s search. Running elevated, all three
  register the extension machine-wide even for a per-user install, which
  is a property of `regsvr32`/COM, not of any installer.
- **Where Inno Setup and NSIS needed script or an action that TigerSetup
  did not**: PATH (both, by hand), the firewall rule (both, `netsh`),
  per-user/per-machine choice (NSIS, `MultiUser.nsh`), Add/Remove Programs
  registration (NSIS, eight `WriteRegStr` lines per package).
- **Different Windows mechanisms for the same feature.** For file
  associations and the URL protocol, Inno Setup and NSIS set the
  extension's or scheme's default handler; TigerSetup registers a handler
  (`OpenWithProgids`, capabilities) and never takes the default. For a
  context-menu verb on file types, Inno Setup and NSIS put the verb under
  the ProgID they own; TigerSetup puts it under
  `SystemFileAssociations\<.ext>`, where Windows shows it whatever the
  type's default handler is. The lab probes accept either; the tables
  above say which was found.
- **Conservative removal.** TigerSetup removes what it owns and restores
  what it changed: after its qBittorrent uninstall `LongPathsEnabled` is
  back at the baseline's `0`, where Inno Setup and NSIS leave `1` (as the
  real installer does). NSIS's `DeleteRegKey` on `Classes\.torrent` removes
  the whole extension key, other applications' `OpenWithProgids` entries
  included; Inno Setup's `uninsdeletevalue` removes only its own value;
  TigerSetup removes only the values and keys it created.
- **Definition size and shape.** The TigerSetup manifests are declarative
  throughout; the Inno Setup scripts are declarative except for PATH; the
  NSIS scripts are imperative throughout, longest for the two per-user
  applications because of `MultiUser.nsh`. This is visible in the
  implementation columns above; no numeric developer-experience score is
  offered.
- **Version identity.** TigerSetup's strict three-part version could not
  carry WinMerge's four-part `2.16.58.2`; Inno Setup and NSIS accept four.

## TigerSetup gaps and capabilities discovered

1. **Payload compression.** ZIP `Deflate` against LZMA/LZMA2 is the whole
   size story above and the one finding with a direct product implication
   (download size, WinGet submission size). Not changed during the
   benchmark: the product was frozen, and a compression format is an
   Architect decision.
2. **Install and uninstall time on large file sets.** The durable journal
   makes TigerSetup's uninstall several times slower than Inno Setup's or
   NSIS's on every application here (see the totals above), and its install
   slower on the two largest payloads. Whether the per-operation durability
   can be batched without weakening the crash-consistency guarantee is an
   engineering question for the engine, not changed during the benchmark.
3. **Explicit registry locations** — found by the 0.7.0 prototype, fixed
   in 0.7.1 before the campaign (`root = "HKLM"`/`"HKCU"` on
   `[[registry]]`, with the value's pre-installation state recorded and
   restored, and a hive that must be the one the package's only scope
   writes). A dual-scope package still cannot declare an `HKLM` value:
   there is no scope predicate, and none of the four applications needed one.
4. **A Start Menu group folder for the product's own link.** VLC and
   WinMerge put their link in a group folder; TigerSetup's `[[shortcuts]]`
   has `folder` for a subfolder, and the benchmark packages left the link
   directly in Programs. Not a gap in capability; noted because the
   parity table shows a different location.
5. **Custom-action program resolution.** A `kind = "exe"` action runs
   exactly the program it names and expands only the known folders; a
   Windows tool (`regsvr32`, `netsh`) therefore needs the `cmd` form. This
   is deliberate (deterministic semantics), and the benchmark records it as
   a property rather than a defect.

## Limitations

- Four applications are evidence about these four applications, not a
  verdict on any technology.
- Single measurement per row; the boot-per-row cost of the lab made repeats
  materially more expensive without changing what a second or two of
  difference could show.
- Silent/unattended paths only; no interactive wizard, UAC or upgrade rows
  here — TigerSetup's own acceptance suite covers those for TigerSetup.
- All rows run as an administrator (the lab's job account), including the
  per-user installs; a standard-user install path is not exercised.
- qBittorrent's and VLC's upstream installer scripts were read at a recent
  branch state rather than the exact pinned tag when the contracts were
  written.

## Conclusions

Grounded in this campaign's four applications and single measurements:

- **Size**: Inno Setup and NSIS land within 9% of each other on
  every application; TigerSetup's installer is the largest on every application, by the
  margin its ZIP/Deflate payload predicts, and the fixed engine overhead
  (2.4 MiB, uncompressed) is a second-order term next to any real
  payload. This is the actionable quantitative finding.
- **Runtime**: 12 of 12 rows installed and uninstalled the canonical
  payload on the clean baseline with every contract probe holding; the timing
  table above is the evidence, and its differences are engineering
  observations at single-sample precision.
- **Expressiveness**: for these four contracts, TigerSetup expressed
  everything but the COM shell extension declaratively; Inno Setup needed
  script for PATH and actions for the firewall and the shell extension;
  NSIS needed script for scope choice, PATH and ARP and actions for the
  firewall and the shell extension. Where TigerSetup differs in mechanism
  (handlers rather than defaults, verbs under `SystemFileAssociations`,
  restored rather than abandoned system settings) it differs towards the
  conservative end.

## A second round with IT Tiger applications

Not started. The harness is parameterized by `benchmark/results/apps.json`
and one `packages/<app>/` directory per application; a round with IT Tiger
applications needs a `contract.md` per application (from the application's
own requirements rather than an upstream installer script), the three
package definitions, and a rerun of the same acquisition → canonical →
build → measure → lab pipeline. That is an Architect decision.
