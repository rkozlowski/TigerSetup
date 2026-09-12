# TigerSetup

<p align="center">
  <img src="docs/assets/TigerSetup.png" alt="TigerSetup project icon" width="128"/>
</p>

> **Tell TigerSetup what the application consists of; get a `Setup.exe`.**

TigerSetup builds small, self-contained Windows installers for ordinary
desktop applications. You describe the installation in a `TigerSetup.toml`,
`tiger-setup build` produces a single `Setup.exe`, and on the target machine a
per-installation SQLite database records what that installation actually owns
— so upgrade, repair, uninstall, verification and crash recovery all plan from
what is really installed, never from a re-read manifest.

```text
application payload (publish/…)
        ↓
TigerSetup.toml
        ↓
tiger-setup build TigerSetup.toml
        ↓
MyApp-1.0.0-Setup.exe        one file: engine + metadata + ZIP payload
        ↓
tiger-setup inspect / verify
```

What a generated installer gives you:

- **One file, no prerequisites.** `Setup.exe` runs on a plain Windows 10 1809+,
  Windows 11 or Windows Server 2019+ x64 machine. It needs no .NET, no
  PowerShell 7, no WinGet, no runtime of any kind — and never the Internet
  unless a declared dependency has to be downloaded.
- **Transactional.** Every change is journaled before it is made. An install
  or upgrade ends installed or fully rolled back; an interrupted one — process
  kill, reboot, power cut — converges to exactly one valid version on the next
  run.
- **Unattended by design.** `Setup.exe install --quiet --json` has stable exit
  codes and machine-readable output; the wizard is a client of the same
  engine, not a second implementation.
- **Native, small, localized.** A plain Win32 wizard that follows the Windows
  theme (light and dark), scales at 100–200 % DPI and speaks `en-US` and
  `pl-PL`, adding about 0.4 MB to a 2.3 MB engine.
- **Inspectable.** The installer format is designed to be decomposed and
  verified without running it.

TigerSetup is at version **0.5.2**.

## Getting `tiger-setup`

TigerSetup is two executables: `tiger-setup.exe`, the builder, and
`tigersetup-setup.exe`, the installer engine that becomes the head of every
generated `Setup.exe`. The builder takes the engine from the file beside
itself, so keep the two together.

**From a TigerSetup release installer** — `TigerSetup-<version>-Setup.exe`
installs both binaries (per user by default) and adds them to `PATH`:

```powershell
TigerSetup-0.5.2-Setup.exe                    # the wizard
TigerSetup-0.5.2-Setup.exe install --quiet    # unattended
```

**From source** — stable Rust (1.98 or later) for `x86_64-pc-windows-msvc`
with the Visual Studio C++ build tools and the Windows SDK (`rc.exe`):

```powershell
git clone <this repository>
cd TigerSetup
cargo build --release
# target\x86_64-pc-windows-msvc\release\tiger-setup.exe
# target\x86_64-pc-windows-msvc\release\tigersetup-setup.exe
```

## Your first installer

Put the files to install in a directory — here `publish/` — and write a
`TigerSetup.toml` beside it:

```toml
[package]
id = "Contoso.MyApp"        # stable identity: the Add/Remove Programs key and the WinGet id
name = "MyApp"
version = "1.0.0"
publisher = "Contoso"

[[files]]
source = "publish/**"

[[shortcuts]]
location = "start-menu"
target = "MyApp.exe"
```

Build, inspect and verify:

```powershell
tiger-setup build TigerSetup.toml          # → MyApp-1.0.0-Setup.exe in the current directory
tiger-setup inspect MyApp-1.0.0-Setup.exe  # what the installer contains and claims
tiger-setup verify MyApp-1.0.0-Setup.exe   # hashes, CRCs and the declared files check out
```

`inspect` reads the file without executing it:

```text
Package:   MyApp (Contoso.MyApp) 1.0.0 by Contoso
Scopes:    user
Root:      user=%LOCALAPPDATA%\Programs\MyApp
Engine:    TigerSetup 0.5.2 sha256 2054…7a82
Windows:   MyApp Setup · MyApp 1.0.0 · Contoso · MyApp-1.0.0-Setup.exe
Layout:    engine 2280960 B | metadata 335 B @ 2280960 | payload 233485 B @ 2281295 | footer @ 2514780
Payload:   sha256 6072…9759 (2 files, 2 entries)
      360448     233266 Deflated 93c930a0 MyApp.exe
           7          7 Stored   46ce8aac README.txt
Registers: Contoso.MyApp · DisplayName MyApp · DisplayVersion 1.0.0
Shortcut:  start-menu MyApp → MyApp.exe
Verify:    ok
```

Then run it. Double-clicking `MyApp-1.0.0-Setup.exe` opens the wizard;
automation uses the command line:

```powershell
MyApp-1.0.0-Setup.exe install --quiet --json      # install, or upgrade what is installed
MyApp-1.0.0-Setup.exe verify --json               # compare the installation with what it owns
MyApp-1.0.0-Setup.exe uninstall --quiet --json
```

The installation lands in `%LOCALAPPDATA%\Programs\MyApp`, registers in
Add/Remove Programs, and keeps its state in
`%LOCALAPPDATA%\TigerSetup\Contoso.MyApp\` — the database, the logs and the
uninstaller copy that Add/Remove Programs calls.

While iterating, `tiger-setup build TigerSetup.toml --fast` skips the
compression search: the installer is functionally identical, larger, and
builds in a fraction of the time. The default build is what you publish.

## Manifest essentials

`TigerSetup.toml` is developer-facing source; it is not shipped. The builder
validates it, resolves the file set and dependency metadata, and embeds the
result as compact runtime metadata. Every section other than `[package]` and
`[[files]]` is optional.

```toml
[package]
id = "Contoso.MyApp"
name = "MyApp"
version = "1.0.0"                      # or from [metadata]
publisher = "Contoso"
description = "Does the thing."        # Add/Remove Programs and the shortcuts
copyright = "© 2026 Contoso"
license = "MIT"                        # SPDX identifier where one applies
license_file = "publish/LICENSE.txt"   # shown by the wizard's licence page
icon = "assets/MyApp.ico"              # the product's branding icon
website = "https://example.com/myapp"
support = "https://example.com/myapp/support"
help    = "https://example.com/myapp/help"

[metadata]                             # where version, description, copyright come from
source = "static"                      # "static" (default) | "msbuild" | "exe"

[install]
scopes = ["user", "machine"]           # allowed scopes; the first is the default
existing_scope = "preserve"            # "preserve" (default) | "allow-parallel" | "error"
architecture = "x64"
# user_root    = "%LOCALAPPDATA%\\Programs\\MyApp"   (the defaults)
# machine_root = "%PROGRAMFILES%\\MyApp"
# minimum_build = 17763

[installer]
icon = "branding"                      # the Setup.exe's own icon (see Branding)

[[files]]
source = "publish/**"                  # glob relative to the manifest; the part before
exclude = ["*.pdb", "*.xml"]           # the first wildcard is the install-relative base

[[options]]                            # on/off choices: wizard checkboxes, --option on the command line
name = "path"
kind = "path"                          # "path" | "desktop-shortcut" | "custom" (default)
default = true

[[options]]
name = "desktop-shortcut"
kind = "desktop-shortcut"

[[options]]
name = "samples"
label = { "en-US" = "Install the sample documents", "pl-PL" = "Zainstaluj przykładowe dokumenty" }

[[shortcuts]]
location = "start-menu"                # "start-menu" | "desktop"
target = "MyApp.exe"                   # install-relative
# name, arguments, description, icon, folder, option are optional

[[shortcuts]]
location = "desktop"
target = "MyApp.exe"
option = "desktop-shortcut"            # only when the option is on

[[path]]
entry = "."                            # install-relative directory added to the scope's PATH
option = "path"

[[registry]]                           # values under the scope's Software root (HKCU or HKLM)
key = "Contoso\\MyApp"
name = "InstallRoot"
kind = "expand-string"                 # "string" | "expand-string" | "dword"
data = "%INSTALLROOT%"                 # %INSTALLROOT% and %VERSION% expand at install time

[registration]                         # Add/Remove Programs
display_icon = "MyApp.exe"
# key_name, display_name, display_version default to the package id, name and version

[legacy]                               # an installation by another technology to replace
installer_type = "inno"
registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"

[[dependencies]]                       # see Dependencies
id = "Microsoft.DotNet.DesktopRuntime.10"
minimum = "10.0"
detect = { kind = "directory-version", path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App", pattern = "10.*" }

[winget]                               # what a WinGet manifest needs and the installer does not
moniker = "myapp"
commands = ["myapp"]
tags = ["example"]
package_url = "https://example.com/myapp"
short_description = "Does the thing."
```

Full descriptions of every key are in the builder's manifest module,
`crates/tigersetup-build/src/manifest.rs`; the design behind each section is
in [`TigerSetup-Design.md`](TigerSetup-Design.md).

### Product metadata from your build

Do not type the version twice. `[metadata]` reads it — with the description
and copyright — from the application itself, and validates a built executable
against the declared source, so a stale binary is a build error rather than a
release artifact:

```toml
[metadata]
source = "msbuild"                            # evaluated MSBuild properties, not parsed XML
project = "../src/MyApp/MyApp.csproj"
executable = "publish/MyApp.exe"              # validated against the project

[metadata]
source = "exe"                                # the VERSIONINFO of a built executable
executable = "publish/MyApp.exe"              # C++, Rust, anything with a version resource
```

A value typed in `[package]` wins over a provider's. `tiger-setup metadata
TigerSetup.toml` lists every value, where it came from, and what was
validated; `--json` gives the same with stable identifiers.

## Common packaging tasks

| Task | How |
|---|---|
| Install files | `[[files]] source = "publish/**"`; `exclude` globs match the install-relative path. Files keep their relative layout under the install root. |
| Start Menu / desktop shortcut | `[[shortcuts]]` with `location`, `target`; `folder` puts it in a subfolder, `option` makes it a choice. |
| Add a directory to `PATH` | `[[path]] entry = "."` (or a subdirectory) — added to the user or machine `PATH` by scope, never duplicated, never claimed if it already existed. |
| Registry values | `[[registry]]` under the scope's `Software` root; ownership is per value. |
| Let the user choose | `[[options]]` — `path` and `desktop-shortcut` are labelled by the wizard in its own language; a `custom` option carries its own `label` per language, `en-US` required. |
| Prerequisites (.NET, WebView2, …) | `[[dependencies]]` — see [Dependencies](#dependencies). |
| Replace an Inno Setup installation | `[legacy]` — see [Upgrades, repair and uninstall](#upgrades-repair-and-uninstall). |
| Custom install root | `[install] user_root` / `machine_root`; the user can still choose another root in the wizard or with `--install-root` unless the manifest pins one. |
| Branding | `[package] icon`, `[installer] icon` — see [Branding and installer identity](#branding-and-installer-identity). |
| WinGet manifests | `tiger-setup winget prepare` / `finalize` — see [Distribution](#distribution). |

## Build, inspect, verify

```text
tiger-setup build    <TigerSetup.toml> [--output <dir|file.exe>] [--engine <path>]
                                        [--property <Name=Value>]... [--offline] [--fast]
tiger-setup metadata <TigerSetup.toml> [--property <Name=Value>]... [--json]
tiger-setup inspect  <Setup.exe> [--json]
tiger-setup verify   <Setup.exe>
tiger-setup winget prepare  <TigerSetup.toml> --installer <Setup.exe> --output <dir>
tiger-setup winget finalize <manifest dir> --url <url> --installer <Setup.exe>
```

- `build` writes `<name>-<version>-Setup.exe` into `--output` (a directory, or
  the file itself when it ends in `.exe`). `--engine` names another engine
  executable; `--property` passes a global MSBuild property to metadata
  evaluation; `--offline` resolves nothing from the WinGet catalog, leaving
  dependency hints for the installer to resolve at install time.
- `inspect` decodes the footer, the metadata and the payload listing and
  checks the hashes; `--json` is the same as one document with stable field
  names. `verify` is the yes/no form: exit `0` when everything checks out,
  `1` when it does not, `2` when the file is not an installer.
- The same inputs and the same engine give byte-identical output, so a build
  can be reproduced and compared.

**Build once, validate exact bytes, publish those exact bytes.** The file that
passed your validation is the file you publish; never rebuild for a
submission.

## Installation scopes

A package declares which scopes it supports; the first is the default:

```toml
[install]
scopes = ["user", "machine"]
```

| | user | machine |
|---|---|---|
| Install root | `%LOCALAPPDATA%\Programs\<name>` | `%PROGRAMFILES%\<name>` |
| State | `%LOCALAPPDATA%\TigerSetup\<id>\` | `%ProgramData%\TigerSetup\<id>\` |
| Registration, registry values, `PATH` | `HKCU` | `HKLM` |
| Shortcuts | the user's folders | the shared folders |
| Elevation | none | the installer relaunches itself elevated through the UAC prompt |

The wizard offers the choice on its scope page and puts the UAC shield on
**Next** when the selected scope needs an administrator. On the command line,
`--scope user|machine` names the scope; from an unelevated console a
machine-scope install raises the prompt, waits, and reports the elevated
run's result as its own. `verify` and `inspect` never elevate.

**A run is about the installation the machine already holds.** Re-running
`Setup.exe` over an installed product upgrades or repairs that installation,
whichever scope it is in. `[install] existing_scope` decides what happens
when a run names the *other* scope:

```text
preserve         # default: continue with the existing installation;
                 # an explicit request for the other scope is refused (scope_conflict)
allow-parallel   # an explicit request for the other scope creates a second, independent installation
error            # refuse every run whose scope, named or defaulted, is not the installed one
```

With two installations present, a run that names no scope is
`scope_ambiguous`: the wizard asks which, automation gets the structured
refusal and must name one.

## Upgrades, repair and uninstall

- **Upgrade** — run the new version's installer. It plans from the state
  database: unchanged files are kept, changed ones replaced, removed ones
  removed, added ones installed; options keep the values the installation
  recorded. A downgrade is mechanically an upgrade to an older version.
- **Reinstall** — the same version's installer reconciles the installation;
  with an explicit `--option` it applies that choice.
- **Repair** — `Setup.exe repair` rewrites whatever is missing or changed
  from the installer's own payload.
- **Uninstall** — `Setup.exe uninstall`, or the uninstaller copy Add/Remove
  Programs calls. It removes what the installation owns and nothing else: a
  file you modified after installation is preserved and reported
  (`file_modified_preserved`), a `PATH` entry that existed before is not
  claimed, a directory with foreign content is left standing. A committed
  uninstall leaves nothing of TigerSetup's behind — no database, no logs, no
  uninstaller.
- **Migrating from another installer** — declare the legacy footprint and
  the first TigerSetup install removes it first, using that installer's own
  quiet uninstall command, then installs afresh. Inno Setup is the supported
  `installer_type`. The Add/Remove Programs identity becomes the package id
  (`[registration] key_name` keeps the old name where an external consumer
  requires it).

```toml
[legacy]
installer_type = "inno"
registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
```

**Running applications.** Before replacing or removing files, the installer
asks the Windows Restart Manager which applications hold them, asks those
applications to close, and restarts afterwards what it stopped. An
application that will not close ends the run with `package_in_use` and nothing
changed; nothing is ever killed.

**Interruptions.** Undo information is written durably before every change.
If an install or upgrade is interrupted, the next run of any installer of the
product — or of the uninstaller — finishes it forward or rolls it back, so an
upgrade ends as exactly the old version or exactly the new one, never a
mixture. `inspect --json` shows an open transaction and its state.

## Dependencies

A dependency is a requirement the installer satisfies, not something it
owns: installing WebView2 for your application does not mean uninstalling your
application removes WebView2.

```toml
[[dependencies]]
id = "Microsoft.DotNet.DesktopRuntime.10"          # a WinGet package identifier
name = ".NET Desktop Runtime 10"                   # shown to the user
minimum = "10.0"                                   # same major, not lower; absent means any
detect = { kind = "directory-version",
           path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App",
           pattern = "10.*" }

[[dependencies]]
id = "Microsoft.EdgeWebView2Runtime"
detect = { kind = "registry-version",
           keys = ["HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
                   "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"],
           value = "pv" }
```

- **Detection** is typed and declared by you: `directory-version` (child
  directories are versions), `registry-version` (a version value from the
  first key that has it), `file-version` (a file's version resource) or
  `registration` (an Add/Remove Programs entry by display name). Detection
  always runs first; a machine whose dependencies are present never touches
  the network.
- **Acquisition** defaults to the WinGet catalog entry for `id`: at build time
  the builder embeds the current installer URL, hash and silent switches as a
  hint; at install time the engine uses the hint while it is fresh
  (`acquire.max_age_days`, 14 by default) and otherwise refreshes it from the
  catalog over plain HTTPS — no `winget.exe` involved. A download is
  installed only when its SHA-256 matches. A dependency with no catalog entry
  declares `acquire = { url, sha256 }` and `install = { arguments,
  success_codes, reboot_codes }` and is never refreshed.
- **Policy.** Online, a missing dependency is acquired and installed before
  the product, unattended and interactive alike; `--no-dependency-install`
  fails with `dependency_missing` instead. Offline with a missing dependency
  the run fails cleanly with `dependency_unacquirable` before any product
  change. A dependency whose installer needs elevation makes an unattended
  user-scope run fail with `dependency_requires_elevation`; the wizard asks
  for elevation for the dependency alone. A dependency installer that asks
  for a reboot still installs the product and the run exits `3010`.

## Branding and installer identity

The wizard shows the product's name and icon; TigerSetup's own mark is a
small secondary brand mark. The generated `Setup.exe` presents the product in
Explorer too — its Windows version resource (`ProductName`, `FileDescription`,
`ProductVersion`, `CompanyName`, `LegalCopyright`, `OriginalFilename`) is
derived from `[package]`, so `MyApp-1.0.0-Setup.exe` reads as MyApp.

The executable's icon follows `[installer] icon`:

```text
omitted          [package].icon when one is declared, otherwise TigerSetup's icon
"branding"       [package].icon — a build error if none is declared, never a silent fallback
"tigersetup"     TigerSetup's icon even when a branding icon is declared
"<path>.ico"     that file, relative to the manifest
```

Every image in the `.ico` is carried, so Windows picks the right size at every
DPI.

**Product identity and engine provenance are kept apart.** The Windows
properties say what product the file installs; the embedded metadata records
which TigerSetup engine built it — the TigerSetup version, the SHA-256 of the
release engine (`engine_sha256`) and of this installer's own engine block
after the identity rewrite (`engine_block_sha256`). `tiger-setup inspect`
reports both, and `verify` checks the engine block against the recorded hash.

## Automation and unattended use

```text
Setup.exe                                   the root operation: install, interactive
Setup.exe install   [--quiet] [--scope user|machine] [--install-root <path>]
                    [--option <name> <on|off>]... [--no-dependency-install]
                    [--lang <tag>] [--log <path>] [--json]
Setup.exe uninstall [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Setup.exe repair    [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Setup.exe verify    [--scope user|machine] [--json]
Setup.exe inspect   [--scope user|machine] [--json]
```

Without `--quiet`, `install`, `uninstall` and `repair` show the wizard. An
option takes its value as a separate argument (`--log C:\x.log`). `--json`
prints one machine-readable document to stdout whose identifiers are never
localized (`"code": "dependency_missing"`); human text follows `--lang`, or
the Windows UI language, with English as the fallback.

| Exit | Meaning |
|---|---|
| `0` | success |
| `1` | failed and rolled back (`verify`: the installation differs from what it owns) |
| `2` | invalid arguments or package, including `scope_conflict` and `scope_ambiguous` |
| `3` | dependency missing or unacquirable |
| `4` | elevation required or refused |
| `5` | cancelled |
| `6` | package in use — an application would not close |
| `7` | an earlier transaction needs recovery and could not be completed |
| `8` | unsupported platform |
| `3010` | success, reboot required |

The uninstaller copy in the state directory takes the same commands with
`uninstall` as its root operation; `Setup.exe verify --json` and `inspect
--json` are the checks a pipeline or a package manager runs after it.

## Troubleshooting and diagnostics

- **Logs.** Every mutating run writes a log — `--log <path>`, or by default
  `<state directory>\logs\<timestamp>-<command>.log` (`%TEMP%\TigerSetup\`
  for an uninstall, which removes the state directory). Each event carries a
  stable code, and every `operation_applied` line names its sequence number
  and target.
- **`Setup.exe inspect --json`** describes the package, every installation of
  it in either scope, what each owns, any open transaction, and each declared
  dependency as this machine answers it. It never elevates and never writes.
- **`Setup.exe verify --json`** compares the installation with the database:
  `file_missing`, `file_modified`, `registry_value_missing`, … with counts per
  resource kind. Exit `0` only when everything matches.
- **Outcome documents** (`--json` on a mutating run) carry `outcome`, `code`,
  `exit_code`, the installation, the transaction, the dependencies
  considered, the applications closed and restarted, and the log path.
- **Recovery.** A run that finds an open transaction recovers it first
  (forward when the same package version is at hand, otherwise back), and the
  outcome says what it found. `recovery_incomplete` (exit `7`) means an
  earlier attempt could not be converged; the log and `inspect` say why.
- **`tiger-setup inspect`** on the installer file answers the build-side
  questions: what is embedded, which engine, whether the bytes are intact.

## Supported Windows versions

Generated installers run on **Windows 10 1809 x64 and later, Windows 11 x64,
and Windows Server 2019 x64 and later**; Windows Server 2016 is not supported.
`[install] minimum_build` raises the floor for a package whose application
needs a newer Windows. Whether the packaged *application* supports a given
Windows is the application's own concern.

The builder runs on 64-bit Windows.

## Distribution

The package id is the Add/Remove Programs identity and the WinGet package
identifier, so installer, registration and catalog metadata cannot drift.
A WinGet submission is a two-step workflow around the exact published bytes:

```powershell
tiger-setup winget prepare TigerSetup.toml --installer MyApp-1.0.0-Setup.exe --output winget
# publish MyApp-1.0.0-Setup.exe at an immutable URL, then
tiger-setup winget finalize winget --url https://example.com/releases/MyApp-1.0.0-Setup.exe --installer MyApp-1.0.0-Setup.exe
```

`prepare` writes the manifest set with the installer URL left unresolved;
`finalize` fills in the URL and the hash of the exact bytes and validates the
set for consistency. It never rebuilds anything. `[winget]` in the manifest
supplies what the community manifest needs and the installer does not.

A generated installer is also a conventional Windows installer — silent
switches, stable exit codes, correct registration — so a Chocolatey package
can wrap it thinly.

## Further reference

| Document | Owns |
|---|---|
| [`TigerSetup-Design.md`](TigerSetup-Design.md) | How TigerSetup works and why: the transactional model, ownership, scopes, dependencies, the installer format, the wizard, localization, technology choices, scope and open questions. |
| [`TigerSetup-Validation.md`](TigerSetup-Validation.md) | How it is proven: test levels, fault injection, the TigerWinLab acceptance matrix, the UI matrix. |
| [`TigerWinLab-Requirements.md`](TigerWinLab-Requirements.md) | What TigerSetup's acceptance needs from the TigerWinLab Windows lab, and how it consumes it. |
| [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) | Material redistributed inside a generated `Setup.exe`, with its licence notices. |
| [`LICENSE.txt`](LICENSE.txt) | MIT. |
| `packages/` | The packages TigerSetup builds: [its own installer](packages/tigersetup/README.md), [TigerMarkView](packages/TigerMarkView/README.md), and the [synthetic package](packages/test-app/README.md) the transactional tests and lab rows use. |

## Development

TigerSetup is a Rust workspace of five crates under `crates/`:
`tigersetup-format` (the installer format), `tigersetup-catalog` (the WinGet
catalog client), `tigersetup-engine` (state, journal, planner, transactions,
recovery, resources), `tigersetup-setup` (the engine executable with its
command-line client and the wizard) and `tigersetup-build` (the builder).
`proto/` holds the runtime-metadata schema, `lab/` the TigerWinLab
acceptance driver ([`lab/README.md`](lab/README.md)) and `docs/assets/` the
project artwork, including the `TigerSetup.ico` compiled into both executables.

The verification gate, the release discipline and the working rules for
contributors and AI agents are in [`AGENTS.md`](AGENTS.md).
[`LESSONS_LEARNED.md`](LESSONS_LEARNED.md) records the non-obvious things this
project paid to learn.

## Licence

TigerSetup is released under the [MIT License](LICENSE.txt). Material
redistributed inside a generated `Setup.exe` is listed with its licence
notices in [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

## Copyright & Project Sponsor

TigerSetup is a project by **IT Tiger**, © 2026 IT Tiger.  
🔗 https://www.ittiger.net/
