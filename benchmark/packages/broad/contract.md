# Broad-corpus benchmark — the common package contract

The broad campaign (`benchmark/report-0.10.0-broad.md`,
`benchmark/results/0.10.0-broad/`) measures how Inno Setup, NSIS and
TigerSetup handle *payload shape* — fifteen real application trees from a
few files to twelve thousand, from 2 MB to 1 GB — not how rich a script each
technology can carry for one application. So every application gets the same
deliberately small package, stated once here and implemented three times by
`scripts/New-BroadPackages.ps1`, which generates the definitions under
`packages/broad/<app>/{tigersetup,innosetup,nsis}/` from the campaign's
corpus record (`results/0.10.0-broad/corpus.json`). The functional-contract
packages of the first benchmark (`packages/<app>/contract.md`) are a
different experiment and are not used here.

## What every package does

| Aspect | Contract | TigerSetup | Inno Setup 7 | NSIS 3.12 |
|---|---|---|---|---|
| Payload | the exact canonical payload tree, every file byte-identical, nothing added to it | `[[files]] source = "<payload>/**"` | `[Files] Source: "<payload>\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion` | `SetOutPath "$INSTDIR"` + `File /r "<payload>\*.*"` |
| Install root | the directory the command line names (a row gives `C:\TigerSetupBenchmark\apps\<App>-<Tech>`) | `--install-root` | `/DIR=` | `/D=` |
| Scope | user (per-user hive and folders; the job account is an administrator, so no elevation happens in any technology) | `scopes = ["user"]` | `PrivilegesRequired=lowest` | `RequestExecutionLevel user`, `SetShellVarContext current`, `HKCU` |
| Identity | `Benchmark.<App>`, the application's three-part version, publisher `TigerSetup broad benchmark` | `[package] id/name/version/publisher` | `AppId=Benchmark.<App>`, `AppName`, `AppVersion`, `AppPublisher` | `Name`, VERSIONINFO keys, the uninstall key `Benchmark.<App>` |
| Add/Remove Programs | registered through the technology's own mechanism, with the uninstaller it names | automatic (`HKCU\...\Uninstall\Benchmark.<App>`; the uninstaller is the state directory's copy) | automatic (`HKCU\...\Uninstall\Benchmark.<App>_is1`; `{app}\unins000.exe`) | `WriteUninstaller` + `WriteRegStr HKCU ...\Uninstall\Benchmark.<App>` (DisplayName, DisplayVersion, Publisher, InstallLocation, UninstallString, QuietUninstallString, EstimatedSize, NoModify, NoRepair) |
| Uninstall | started from the path Add/Remove Programs registered, silently; removes the payload, the install root, the registration and every package-owned file | `uninstall --quiet` | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` | `/S` |
| Compression | each technology's production-quality solid setting, unchanged from the first benchmark | the release builder's default (solid zstd-19-w27, never `--fast`) | `Compression=lzma2/max`, `SolidCompression=yes` | `SetCompressor /SOLID lzma` |
| Anything else | **nothing**: no shortcut, option, file association, protocol, COM registration, firewall rule, PATH entry, dependency or custom action | — | — | — |

A payload that needed one of the excluded features merely to be installable
would get it in all three definitions and the exception would be recorded
here; none of the fifteen did.

## What the row measures against it

`scripts/Invoke-BroadBenchmarkLab.ps1` runs each package through the same
lifecycle on TigerWinLab's clean Windows 11 baseline, one lab session per
row, and judges it by this contract:

- **Install**: exit code 0; the installer process's lifetime plus any
  hand-off process of the installer's own name (the time), then — outside
  the timed interval — every canonical file present under the root with its
  size and SHA-256 (`payloadExact`), and what else the technology put under
  the root (`payloadExtras`, with their bytes: Inno Setup's `unins000.exe`
  and `unins000.dat`, NSIS's `Uninstall.exe`, nothing for TigerSetup, whose
  bookkeeping lives outside the root).
- **Registration**: the Add/Remove Programs key of the technology's own
  naming exists, its `DisplayName` names the application (Inno Setup's
  default is `<AppName> <AppVersion>`, its own convention, accepted as such),
  its `DisplayVersion` is the package version, and it names an uninstaller
  (`UninstallString`).
- **Package-owned state outside the root**: what the technology keeps
  elsewhere while the package is installed — TigerSetup's state directory
  (`%LOCALAPPDATA%\TigerSetup\Benchmark.<App>`: the database, the log and
  the uninstaller copy); Inno Setup and NSIS keep nothing outside the root
  and the registry key.
- **Uninstall**: started from the executable the `UninstallString` names;
  exit code 0; the uninstaller's lifetime plus the exit of what it hands off
  to (NSIS: `Au_.exe`; TigerSetup: any process of the uninstaller's own
  name) and the removal of the install root and of the package-owned state
  outside it (the time); then the root absent, the registration absent, the
  package-owned state absent, and any residue listed.
- **Verdict**: a row passes when every one of those holds. Expected
  bookkeeping under the root is not a failure; a missing or differing
  canonical file, a registration that does not appear or does not go away,
  a root or state directory left behind, is.
