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
MyApp-1.0.0-Setup.exe        one file: loader + compressed engine + solid payload + compressed metadata
        ↓
tiger-setup inspect / verify
```

What a generated installer gives you:

- **One file, no prerequisites.** `Setup.exe` runs on a plain Windows 10 1809+,
  Windows 11 or Windows Server 2019+ x64 machine. It needs no .NET, no
  PowerShell 7, no WinGet, no runtime of any kind — and never the Internet
  unless a declared dependency has to be downloaded.
- **Transactional, and optimized for the shortest reliable transaction.**
  Every change is journaled before it is made. An install or upgrade ends
  installed or fully rolled back; an interrupted one — process kill, reboot,
  power cut — converges to exactly one valid version on the next run. The
  payload is one solid Zstandard stream chosen for how fast it decodes inside
  that transaction, not for the smallest download.
- **Unattended by design.** `Setup.exe install --quiet --json` has stable exit
  codes and machine-readable output; the wizard is a client of the same
  engine, not a second implementation.
- **Native, small, localized.** A plain Win32 wizard that follows the Windows
  theme (light and dark), scales at 100–200 % DPI and speaks `en-US` and
  `pl-PL`, adding about 0.4 MB to a 2.3 MB engine.
- **Inspectable.** The installer format is designed to be decomposed and
  verified without running it: `tiger-setup inspect` reads it, `verify`
  checks it, `inspect --output-payload` / `--output-meta` write the embedded
  blocks out exactly as the file carries them, `--output-zip` reconstructs
  the payload as an ordinary archive and `--output-engine` extracts the
  engine the installer runs.

TigerSetup is at version **0.11.0**.

## Getting `tiger-setup`

TigerSetup is three executables: `tiger-setup.exe`, the builder;
`tigersetup-loader.exe`, the small native C loader every generated
`Setup.exe` begins with; and `tigersetup-setup.exe`, the installer engine the
loader unpacks and runs. The builder takes the loader and the engine from the
files beside itself, so keep the three together.

**From a TigerSetup release installer** — `TigerSetup-<version>-Setup.exe`
installs both binaries (per user by default) and adds them to `PATH`:

```powershell
TigerSetup-0.11.0-Setup.exe                    # the wizard
TigerSetup-0.11.0-Setup.exe install --quiet    # unattended
```

**From source** — stable Rust (1.98 or later) for `x86_64-pc-windows-msvc`
with the Visual Studio C++ build tools (`cl.exe` and `link.exe`, which also
compile the C loader) and the Windows SDK (`rc.exe`):

```powershell
git clone <this repository>
cd TigerSetup
cargo build --release
# target\x86_64-pc-windows-msvc\release\tiger-setup.exe
# target\x86_64-pc-windows-msvc\release\tigersetup-loader.exe
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
tiger-setup inspect MyApp-1.0.0-Setup.exe --output-payload payload.zst     # the compressed payload block, byte for byte
tiger-setup inspect MyApp-1.0.0-Setup.exe --output-zip payload.zip         # the payload's files as an ordinary ZIP
tiger-setup inspect MyApp-1.0.0-Setup.exe --output-meta metadata.pb        # the embedded metadata, byte for byte
tiger-setup inspect MyApp-1.0.0-Setup.exe --output-meta-json metadata.json # the metadata decoded
tiger-setup inspect MyApp-1.0.0-Setup.exe --output-engine engine.exe       # the engine the installer runs, decompressed
```

`inspect` reads the file without executing it:

```text
Package:   MyApp (Contoso.MyApp) 1.0.0 by Contoso
Scopes:    user
Root:      user=%LOCALAPPDATA%\Programs\MyApp
Engine:    TigerSetup 0.11.0 sha256 3db9…b78f
Block:     sha256 b3e9…b225 (2518016 B compressed to 1158659 B)
Loader:    sha256 e0d4…2da5 (74752 B)
Windows:   MyApp Setup · MyApp 1.0.0 · Contoso · MyApp-1.0.0-Setup.exe
Icon:      64×64, 48×48, 32×32, 24×24, 20×20, 16×16
Layout:    loader 74752 B | engine 1158659 B @ 74752 | payload 121262 B @ 1233411 | metadata 489 B @ 1354673 | footer @ 1355162
Metadata:  sha256 bada…a94c (489 B in one zstd block from 658 B; 2 files in 1 batches)
Payload:   sha256 1bea…b023 (2 files, 2 entries, 121262 B in one zstd stream from 254471 B)
             0       254464 76f1b3b1 MyApp.exe
        254464            7 9bbbb9a8 README.txt
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

While iterating, `tiger-setup build TigerSetup.toml --fast` uses a fast
compression level instead of the release profile (Zstandard 19 with a 128 MiB
window): the installer is functionally identical, larger, and builds in a
fraction of the time. The default build is what you publish.

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

[[options]]                            # choices the user makes: wizard check boxes and radio
name = "path"                          # buttons, --option <name> <value> on the command line
kind = "path"                          # "path" | "desktop-shortcut" | "custom" (default) | "choice"
default = true

[[options]]
name = "desktop-shortcut"
kind = "desktop-shortcut"

[[options]]
name = "samples"
label = { "en-US" = "Install the sample documents", "pl-PL" = "Zainstaluj przykładowe dokumenty" }

[[options]]                            # a choice option: exactly one of its values
name = "mode"
kind = "choice"
default = "standard"
label = { "en-US" = "Installation type" }
choices = [
  { value = "standard", label = { "en-US" = "Standard" } },
  { value = "full",     label = { "en-US" = "Full, with the extras" } },
]

[[files]]                              # an optional component: files gated by an option
source = "extras/**"
when = { option = "mode", equals = "full" }

[[shortcuts]]
location = "start-menu"                # "start-menu" | "desktop" | "startup" | "send-to"
target = "MyApp.exe"                   # install-relative
app_user_model_id = "Contoso.MyApp"    # optional: groups the taskbar buttons
# name, arguments, description, icon, folder, working_directory are optional

[[shortcuts]]
location = "desktop"
target = "MyApp.exe"
option = "desktop-shortcut"            # only when the option is on (the same as
                                       # when = { option = "desktop-shortcut", equals = true })
[[shortcuts]]
location = "start-menu"
name = "MyApp Documentation"
url = "https://example.com/myapp/docs" # an Internet shortcut instead of an installed file

[[path]]
entry = "."                            # install-relative directory added to the scope's PATH
option = "path"

[[environment]]                        # a variable of the scope's environment, apart from PATH
name = "MYAPP_HOME"
value = "%INSTALLROOT%"
expandable = true                      # REG_EXPAND_SZ; default is a plain string

[[registry]]                           # values under the scope's Software root (HKCU or HKLM)
key = "Contoso\\MyApp"
name = "InstallRoot"
kind = "expand-string"                 # "string" | "expand-string" | "dword"
data = "%INSTALLROOT%"                 # %INSTALLROOT% and %VERSION% expand at install time

[[registry]]                           # or at an explicit location in the scope's hive:
root = "HKLM"                          # "HKLM" needs scopes = ["machine"], "HKCU" scopes = ["user"]
key = "SYSTEM\\CurrentControlSet\\Control\\FileSystem"
name = "LongPathsEnabled"
kind = "dword"
data = "1"                             # what was there before comes back when the value is removed
when = { option = "long-paths", equals = true }

[[file_associations]]                  # a handler for these types — never the default
prog_id = "MyApp.Document"
extensions = [".myapp"]
description = "MyApp document"
executable = "MyApp.exe"               # arguments default to "%1"; icon defaults to the executable

[[url_protocols]]                      # a handler for myapp: links
scheme = "myapp"
description = "MyApp link"
executable = "MyApp.exe"

[[app_paths]]                          # Win+R, ShellExecute and `start` find MyApp.exe by name
executable = "MyApp.exe"
add_directory = true

[[context_menu]]                       # a classic Explorer verb
target = "files"                       # "files" (all, or `extensions`) | "directories" | "directory-background"
verb = "open-with-myapp"
label = "Open with MyApp"
executable = "MyApp.exe"

[[firewall]]                           # a Windows Firewall rule for an installed program
name = "MyApp"
program = "MyApp.exe"
direction = "in"                       # "in" | "out"
action = "allow"                       # "allow" | "block"
protocol = "tcp"                       # optional: "tcp" | "udp"; needed for local_ports
local_ports = "8080"

[[actions]]                            # a custom lifecycle action — see Custom actions
name = "build-cache"
phase = "post-install"                 # "pre-install" | "post-install" | "pre-uninstall" | "post-uninstall"
run_on = ["install", "upgrade", "reinstall", "repair"]   # default: the phase's operations without repair
kind = "powershell"                    # "exe" | "powershell" | "cmd"
source = "actions/build-cache.ps1"     # packaged with the installer; or command = "%INSTALLROOT%\\MyApp.exe"
arguments = ["-Root", "%INSTALLROOT%"]
timeout_seconds = 120                  # default 300
on_failure = "fail"                    # "fail" (default) | "continue"

[launch]                               # the completion page's "Launch MyApp" — see Launch after install
executable = "MyApp.exe"               # an installed .exe; started as the signed-in user, never elevated
arguments = ["--welcome"]              # separate arguments; %INSTALLROOT%, %VERSION% and the known folders expand
working_directory = "."                # optional, install-relative; default: the executable's own directory
checked = true                         # the check box's initial state (default true)

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

[[dependencies]]                       # a prerequisite carried inside the installer
id = "Contoso.Runtime"
detect = { kind = "file-version", path = "%PROGRAMFILES%\\Contoso\\runtime.dll" }
acquire = { file = "prerequisites/contoso-runtime-setup.exe" }
install = { arguments = ["/S"], success_codes = [0], reboot_codes = [3010] }

[winget]                               # what a WinGet manifest needs and the installer does not
moniker = "myapp"
commands = ["myapp"]
tags = ["example"]
package_url = "https://example.com/myapp"
short_description = "Does the thing."
```

Every optional resource — `[[files]]`, `[[shortcuts]]`, `[[path]]`,
`[[registry]]`, `[[environment]]`, the integrations, `[[firewall]]`,
`[[actions]]` and `[[dependencies]]` — takes the same one predicate,
`when = { option = "<name>", equals = <value> }`: a boolean for a boolean
option, a value for a choice.
There is deliberately nothing more; a resource wanted under two values is
declared twice. A component is nothing but an option that gates files:
turning it off during an upgrade or reinstall removes the files it owned,
with the same conservative rules as an uninstall (a file the user changed is
preserved and reported), and turning it on installs them.

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
| Start Menu, desktop, Startup or Send To shortcut | `[[shortcuts]]` with `location`, `target`; `folder` puts it in a subfolder, `working_directory` and `app_user_model_id` set what the link runs with, `url` makes it an Internet shortcut, `when` (or `option`) makes it a choice. Send To is per-user only. |
| Add a directory to `PATH` | `[[path]] entry = "."` (or a subdirectory) — added to the user or machine `PATH` by scope, never duplicated, never claimed if it already existed. |
| Set an environment variable | `[[environment]]` — set for the scope; a value that was there before is restored when the variable is removed, and a value the user changed afterwards is preserved and reported. |
| Registry values | `[[registry]]` under the scope's `Software` root, or with `root = "HKLM"` / `"HKCU"` at an explicit location in the hive the package's only scope writes; ownership is per value, a value that was there before is restored when the value is removed, and a value the user changed afterwards is preserved and reported. |
| Open a file type or a `scheme:` link | `[[file_associations]]`, `[[url_protocols]]` — registered as a handler the user can pick (Open With, Settings › Default apps), never as the default over their choice; a scheme another application owns is left alone and reported. |
| Be found by name | `[[app_paths]]` — an `App Paths` entry for an installed executable. |
| Explorer context menu | `[[context_menu]]` — a classic verb on files, directories or the directory background. Modern (COM) shell extensions are out of scope. |
| Firewall rule | `[[firewall]]` — one rule for an installed program, created through the Windows Firewall API; needs an administrator, so a per-user install by a standard user reports it skipped. |
| Let the user choose | `[[options]]` — `path` and `desktop-shortcut` are labelled by the wizard in its own language; a `custom` option carries its own `label` per language, `en-US` required; a `choice` option offers its `choices` as radio buttons. Choices are remembered by the installation and kept by upgrades. |
| Optional component | An option plus `[[files]] … when = { option = "…", equals = … }` — see above. |
| Carry a prerequisite inside the installer | `[[dependencies]] acquire = { file = "…" }` — see [Dependencies](#dependencies). |
| Prerequisites (.NET, WebView2, …) | `[[dependencies]]` — see [Dependencies](#dependencies). |
| Offer to start the application when setup finishes | `[launch]` — a check box on the completion page of an interactive install or upgrade; see [Launch after install](#launch-after-install). |
| Run your own program or script at install or uninstall time | `[[actions]]` — a packaged or installed executable, PowerShell or batch script at `pre-install`, `post-install`, `pre-uninstall` or `post-uninstall`; see [Custom actions](#custom-actions). Reach for a typed resource first: an action is not rolled back. |
| Replace an Inno Setup installation | `[legacy]` — see [Upgrades, repair and uninstall](#upgrades-repair-and-uninstall). |
| Custom install root | `[install] user_root` / `machine_root`; the user can still choose another root in the wizard or with `--install-root` unless the manifest pins one. |
| Branding | `[package] icon`, `[installer] icon` — see [Branding and installer identity](#branding-and-installer-identity). |
| WinGet manifests | `tiger-setup winget prepare` / `finalize` — see [Distribution](#distribution). |

## Build, inspect, verify

```text
tiger-setup build    <TigerSetup.toml> [--output <dir|file.exe>] [--engine <path>] [--loader <path>]
                                        [--property <Name=Value>]... [--offline] [--fast]
tiger-setup metadata <TigerSetup.toml> [--property <Name=Value>]... [--json]
tiger-setup inspect  <Setup.exe> [--json] [--output-payload <file>] [--output-zip <file>]
                                 [--output-meta <file>] [--output-engine <file>]
                                 [--output-meta-json <file>]
tiger-setup verify   <Setup.exe>
tiger-setup winget prepare  <TigerSetup.toml> --installer <Setup.exe> --output <dir>
tiger-setup winget finalize <manifest dir> --url <url> --installer <Setup.exe>
```

- `build` writes `<name>-<version>-Setup.exe` into `--output` (a directory, or
  the file itself when it ends in `.exe`). `--engine` and `--loader` name
  another engine or loader executable; `--property` passes a global MSBuild
  property to metadata evaluation; `--offline` resolves nothing from the
  WinGet catalog, leaving dependency hints for the installer to resolve at
  install time.
- `inspect` decodes the footer, the metadata and the payload listing and
  checks the hashes; `--json` is the same as one document with stable field
  names. `verify` is the yes/no form: exit `0` when everything checks out,
  `1` when it does not, `2` when the file is not an installer.
- `inspect` also takes the installer apart. `--output-payload` writes the
  compressed payload block byte for byte as the file carries it — not
  re-packed, not re-encoded — so the SHA-256 of the exported file is the
  payload hash the footer records and `inspect` reports; `--output-meta`
  writes the embedded Protocol Buffers metadata decompressed, whose SHA-256
  is the metadata hash. `--output-zip` reconstructs the payload's
  files as an ordinary ZIP archive of stored entries, in stream order, for
  any archive tool; `--output-engine` writes the engine executable the
  installer runs, decompressed, whose SHA-256 is the engine block hash
  `inspect` reports. `--output-meta-json` writes the metadata decoded as
  readable JSON: every field of the embedded message tree under its proto
  name, enumerations as stable names; not the `--json` report, which
  describes the whole file. The options work alone or together, each names a
  file that must not exist yet, and nothing is written for an installer that
  fails verification.
- The same inputs, the same engine and the same loader give byte-identical
  output, so a build can be reproduced and compared.

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
  with an explicit `--option` it applies that choice, and everything the
  choice gates follows: a component's files, a shortcut, a PATH entry, an
  environment variable, an integration, a firewall rule. Every option value —
  explicit for this run, else the last committed one, else the manifest
  default — is committed with the transaction, so a run that fails or is
  cancelled leaves the previous values exactly as they were.
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

**Licence acceptance.** The wizard shows `license_file` on the first
interactive install and asks the person to accept it. An interactive upgrade
or reinstall skips the page while the installation records acceptance of
exactly that text, and asks again when the text changes — an edited
copyright year included. `--quiet` never shows or waits for the page, and
records no acceptance on anyone's behalf.

**Running applications.** Before replacing or removing files, the installer
asks the Windows Restart Manager which applications hold them, asks those
applications to close, and restarts afterwards what it stopped. An
application that will not close ends the run with `package_in_use` and nothing
changed; nothing is ever killed. Only files something actually holds are put
to the Restart Manager — a file that can be renamed this moment is not
registered — so an upgrade of a thousand files nobody has open costs the
Restart Manager nothing.

To upgrade cleanly, an application should respond promptly to the normal
Windows close/session-ending request (a service stops cleanly through the
Service Control Manager instead), release its file handles promptly, save
whatever state it needs before exiting, and avoid a watchdog or
background-helper that relaunches it while it is being closed for servicing.
An application that wants Restart Manager to bring it back afterwards calls
`RegisterApplicationRestart`. See
[`TigerSetup-Design.md` §5.10](TigerSetup-Design.md#510-running-applications-and-files-in-use)
for the full mechanism.

**Applications the Restart Manager cannot close.** A process with no window
to message — a tray helper, a background worker — is listed as a holder and
never closed, so its upgrades would end `package_in_use`. Declare how the
package stops it, and how it comes back:

```toml
[[quiescence]]
name = "helper"
not_running_codes = [3]               # stop exit codes that mean "nothing was running"
# run_on = ["upgrade", "reinstall", "repair", "uninstall"]   # the default

[quiescence.stop]                     # the custom action's envelope, without a phase
kind = "exe"
command = "%INSTALLROOT%\MyHelper.exe"
arguments = ["--quit"]
timeout_seconds = 30

[quiescence.resume]                   # optional: started detached after the run
kind = "exe"
command = "%INSTALLROOT%\MyHelper.exe"
arguments = ["--background"]
```

The stop program runs before the Restart Manager is asked and before any
file is touched. Its exit code says what it found: a success code (`0` by
default) means the application was running and is now stopped; a
`not_running_codes` code means nothing was running; anything else fails the
run before it mutates anything (`quiescence_failed`), or with
`on_failure = "continue"` is recorded and the Restart Manager has its turn.
**Only what was stopped is resumed**, and it is resumed on every path that
leaves the product on the machine: after a successful install, upgrade,
reinstall or repair, and after a failure or rollback — a Restart Manager
refusal over some other holder included. An uninstall stops and never
resumes. The `resume` program is started detached with the run's token, so it
is the one program the installer starts and does not wait for. `stop` and
`resume` take a packaged `source` instead of a `command` like an action does;
an uninstall runs the entry the installation recorded when it was installed.

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
- **Embedded.** `acquire = { file = "<manifest-relative .exe or .msi>" }`
  carries the prerequisite's installer inside `Setup.exe`, for a fully offline
  package. The builder embeds the exact bytes and records their SHA-256 and
  size; at install time the engine extracts them to the state directory only
  when detection says the dependency is missing, refuses bytes that do not
  match (`dependency_unacquirable`, `hash_mismatch`), runs them with the
  declared `install` switches under the same outcome model as a download,
  and removes the extracted file afterwards. `tiger-setup verify` checks the
  embedded bytes against the recorded hash without running anything, and
  `inspect --json` reports the dependency's `source` as `embedded` with the
  payload entry, hash and size it carries.
- **Optional.** A dependency with `when = { option = "…", equals = … }` is a
  requirement only while the option holds; the outcome records it as
  `not_required` otherwise.
- **Policy.** Online, a missing dependency is acquired and installed before
  the product, unattended and interactive alike; `--no-dependency-install`
  fails with `dependency_missing` instead. Offline with a missing dependency
  the run fails cleanly with `dependency_unacquirable` before any product
  change. A dependency whose installer needs elevation makes an unattended
  user-scope run fail with `dependency_requires_elevation`; the wizard asks
  for elevation for the dependency alone. A dependency installer that asks
  for a reboot still installs the product and the run exits `3010`.

## Launch after install

The completion page can offer to start your application — the familiar
"Launch MyApp" check box above Finish:

```toml
[launch]
executable = "MyApp.exe"                      # an .exe the package installs (the build checks it)
arguments = ["--welcome", "%INSTALLROOT%\\samples"]   # each element is exactly one argument
working_directory = "."                       # install-relative; default: the executable's own directory
checked = true                                # the box's initial state; default true
```

It is not a custom action and not part of the installation. What it does and
does not do:

- **When.** The box appears only on the completion page of an interactive
  `install` — a first install, an upgrade or a reinstall — that ended
  installed. A failed, rolled-back or cancelled run, a repair and an
  uninstall never show it. Pressing Finish with the box checked starts the
  program; closing the window on that page is Finish.
- **Never silent.** A `--quiet` run has no completion page, so it never starts
  anything, whatever the manifest says: an unattended caller did not ask for a
  window.
- **Never elevated.** The program always runs as the person signed in to the
  desktop, with their ordinary, unelevated token — also when the wizard is the
  elevated child of a UAC prompt, when the administrator who approved the
  prompt is another account, and when the installer was started elevated.
  An unelevated wizard starts it with its own token; an elevated one asks the
  desktop's shell (Explorer) to start it, so the program runs as the shell's
  user with the shell's environment. Where no such context exists — no shell
  is running, or the shell is itself elevated because User Account Control is
  off — nothing is started, and nothing ever falls back to the wizard's
  administrator token.
- **In front.** The wizard hands its foreground right to the program and stays
  up until the program's window appears (up to 15 seconds, less for a program
  that shows none, such as a tray application), then puts that window in the
  foreground.
- **Failure is not an installation failure.** A program that cannot start is
  reported in a box of the wizard's own ("MyApp is installed, but it could not
  be started: …"); the run still ends installed, with exit code `0`.

Arguments are passed as separate arguments and never joined by a shell, with
`%INSTALLROOT%`, `%VERSION%` and the known folders expanded as in an action's
arguments; a `%` that opens no placeholder stays. The outcome document
(`--json`) of an interactive run reports what happened under `launch` —
`status` `started`, `declined`, `failed` (`launch_failed`) or `unavailable`
(`launch_unavailable`), the resolved program and arguments, `method`
(`own_token` or `shell`), the `pid` and whether its window reached the
`foreground` — and the run's log carries the same as a `launch_*` line. The
installation's own `outcome`, `code` and exit code never change because of it.

## Custom actions

Everything above is a typed resource: TigerSetup owns it, journals it, rolls
it back, verifies it, repairs it and removes it. Some products also need work
no typed resource can express — building a cache, registering with a service
the product ships, migrating a settings store, removing what the product
generated while it ran. A **custom action** is the way to do that work: a
program TigerSetup packages and verifies, starts at a defined point of the
run under a controlled envelope, and records. Prefer a typed resource
wherever one exists, because the difference is not cosmetic:

> TigerSetup can roll back the resources it owns and understands. It cannot
> guarantee rollback of arbitrary side effects produced by a custom action.

```toml
[[actions]]
name = "build-cache"                          # stable identity; lower-case words joined by '-'
phase = "post-install"                        # when it runs (below)
run_on = ["install", "upgrade", "reinstall", "repair"]   # default: the phase's operations without repair
kind = "powershell"                           # "exe" | "powershell" | "cmd"
source = "actions/build-cache.ps1"            # packaged: embedded, hashed, verified, extracted to run
arguments = ["-Root", "%INSTALLROOT%", "-Version", "%VERSION%"]
working_directory = "%INSTALLROOT%"           # default: the program's own directory
timeout_seconds = 120                         # default 300; the program and everything it started are killed
success_codes = [0]                           # default
reboot_codes = [3010]                         # success with a reboot pending, like a dependency's
on_failure = "fail"                           # "fail" (default) | "continue"
when = { option = "cache", equals = true }    # the same one predicate as every resource

[[actions]]
name = "unregister"
phase = "pre-uninstall"
kind = "exe"
command = "%INSTALLROOT%\\MyApp.exe"          # an installed program, as a template
arguments = ["--unregister"]
```

**Phases.** `pre-install` runs after the dependencies are satisfied and
before any product resource is touched — the first operations of the
transaction; `post-install` after every resource is applied and every removal
done — the last operations before the commit; `pre-uninstall` before any
owned resource is removed; `post-uninstall` after every owned resource is
removed, the install root included. An action of an install phase may run on
`install`, `upgrade`, `reinstall` and `repair`; one of an uninstall phase on
`uninstall` only, and the builder refuses any other combination. **Repair is
opt-in**: an action runs during a repair only when its `run_on` names it,
because an arbitrary program is not necessarily safe to repeat. An upgrade
runs the new package's install-phase actions and nothing of the previous
installation's uninstall actions; a rerun that has nothing to reconcile runs
no action at all.

**Programs.** `source` is a manifest-relative file the builder packages
inside `Setup.exe` (an `.exe`, `.ps1`, or `.cmd`/`.bat`, matching `kind`),
hashed like an embedded prerequisite: `tiger-setup verify` checks the bytes,
`inspect` shows them, and the engine verifies them again after extracting
them and refuses to run bytes that do not match. `command` names a program
already on the target machine as a template — `%INSTALLROOT%`, `%VERSION%`
and the known folders (`%PROGRAMDATA%`, `%LOCALAPPDATA%`, …) expand, and a
`%` that opens no placeholder is kept, so `100%` is an argument. Exactly one
of the two. A `pre-install` action cannot assume the product's files are
there yet, and a `post-uninstall` action cannot use `%INSTALLROOT%` at all —
the builder refuses it — so those two phases package what they run. Scripts
run through their interpreter explicitly and non-interactively:
`powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File`
(Windows PowerShell, on every supported Windows; the package, not the
machine's execution policy, is the trust boundary) and `cmd.exe /d /s /c`.
Nothing is ever downloaded and run.

**The envelope.** `arguments` are passed as separate arguments after
expansion, never joined by a shell. The process starts hidden, with no
standard input, both streams captured into the log (`action_output`, line by
line) and the outcome (the tail of each). It sees `TIGERSETUP_INSTALL_ROOT`,
`TIGERSETUP_VERSION`, `TIGERSETUP_PRODUCT_ID`, `TIGERSETUP_SCOPE`,
`TIGERSETUP_OPERATION`, `TIGERSETUP_PHASE`, `TIGERSETUP_ACTION` and
`TIGERSETUP_QUIET` (`1` unattended, `0` in the wizard) in its environment.
The action and everything it starts live in a job object: `timeout_seconds`
ends the whole tree, and so does the action's own exit — a program that must
keep running belongs to your application, not to its installer. The exit
code is judged by `success_codes`, `reboot_codes` (success with a reboot
pending: the run reports `reboot_required` and exits `3010`), and anything
else — a timeout, a program that cannot be started — is a failure.
`on_failure = "fail"` fails the run, which rolls back what TigerSetup owns;
`"continue"` records the failure (`action_failed_continued` in the findings,
the action's status in the outcome) and goes on. `continue` never turns a
failure into a silent success.

**Execution context.** An action runs with the token of the run: elevated in
a machine-scope install or uninstall, as the user in a per-user one. There is
no per-action elevation, and TigerSetup never elevates a single action behind
the caller's back. Quiet and interactive runs start an action identically,
and TigerSetup manufactures no prompt: **whether your program can run
unattended is your responsibility**, which is what `TIGERSETUP_QUIET` is
for. An action can run arbitrary code, in an elevated installer: it is not
sandboxed, and the package that carries it has to be trusted as a whole,
which is why `inspect` makes every action visible.

**Failure and rollback.** When an action with `on_failure = "fail"` fails,
the run rolls back every resource TigerSetup applied and reports
`action_failed` (`action_timed_out`, `action_launch_failed`) with the
action's name, exit code and output. What the program changed before it
failed stays changed, and the rollback records `action_not_reverted` for
every action that ran rather than pretending otherwise.

**Crash and retry.** Every execution is recorded — `started` before the
process exists, its status and exit code after — so a crash or power cut
while an action runs is told from an action that never ran. The next run of
the same package recovers forward: it reports the earlier run as
`action_interrupted`, **runs the action again**, and completes the
installation; a different package rolls the transaction back and runs
nothing. An interrupted action has unknown side effects, and TigerSetup
claims nothing about them. So write actions that are **idempotent, safe to
retry, bounded, non-interactive, and able to detect work they already did**
— TigerSetup will run them again after a crash, and on every operation they
name.

**Uninstall actions outlive the installer.** By the time a product is
uninstalled, the `Setup.exe` that installed it is usually gone, and the copy
Add/Remove Programs runs carries no payload. So the installation keeps its
`pre-uninstall` and `post-uninstall` actions itself: their definitions in the
state database and the packaged programs under
`<state directory>\actions\<sha256>\`, recorded by the same transaction that
installs the product. A successful upgrade makes the new version's uninstall
actions current; a failed upgrade leaves the previous version's exactly as
they were. Uninstall verifies each stored program against its hash before
running it; `verify` reports one that is missing or modified
(`action_program_missing`, `action_program_modified`) and `repair` restores
it.

**Evidence.** The log carries the launch with its command line and envelope
(`action_started`), the captured output, and the verdict
(`action_completed`, `action_failed`, `action_timed_out`,
`action_launch_failed`); the outcome document lists every action the run
executed under `actions[]` with its name, phase, operation, kind, resolved
program, status, exit code, duration, timeout, policy, output tail and hash;
`Setup.exe inspect --json` lists the declared actions under
`package.actions[]` and, for an installed product, the stored uninstall
actions under `owned.actions[]`. Arguments are logged as passed — pass a
secret through a file the action reads, not on the command line.

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
                    [--option <name> <value>]... [--no-dependency-install]
                    [--lang <tag>] [--log <path>] [--json]
Setup.exe uninstall [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Setup.exe repair    [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Setup.exe verify    [--scope user|machine] [--json]
Setup.exe inspect   [--scope user|machine] [--json]
```

Without `--quiet`, `install`, `uninstall` and `repair` show the wizard; with
it, nothing waits for a person — the licence page included, which an
unattended run neither shows nor accepts. An option takes its value as a
separate argument (`--log C:\x.log`). A declared installer option is set
with `--option <name> <value>`: `on`/`off` (or `true`/`false`, `yes`/`no`,
`1`/`0`) for a boolean option, one of the declared values for a choice
option; anything else is refused with `option_value_invalid` before anything
happens, and `inspect --json` lists every option with its kind, default and
values under `package.options`. `--json` prints one machine-readable
document to stdout whose identifiers are never localized
(`"code": "dependency_missing"`); human text follows `--lang`, or the Windows
UI language, with English as the fallback.

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
  and target. The wizard's completion page offers the path with a
  **Copy log path** link (Alt+C) that puts the exact path on the clipboard.
- **`Setup.exe inspect --json`** describes the package — its declared custom
  actions included (`package.actions[]`) — every installation of it in
  either scope, what each owns — the recorded option values with their
  types (`"path-mode": "tools"`, `"desktop-shortcut": false`), registry
  values, PATH entries, shortcuts, environment variables, firewall rules, the
  stored uninstall actions (`owned.actions[]`) — each declared integration as
  the machine holds it (`integrations[]`: kind, enabled, values present and
  owned), any open transaction, and each declared dependency as this machine
  answers it. It never elevates and never writes, and reads a state database
  written by an older TigerSetup as it is.
- **`Setup.exe verify --json`** compares the installation with the database:
  `file_missing`, `file_modified`, `registry_value_missing`,
  `environment_variable_modified`, `firewall_rule_missing`,
  `action_program_modified`, … with counts per resource kind. Exit `0` only
  when everything matches.
- **Preserved, not destroyed.** A mutating run that leaves something alone
  says so: `file_modified_preserved`, `registry_value_modified_preserved`
  (a registry value somebody changed since it was written stays theirs
  through an upgrade, a reinstall and an uninstall alike),
  `environment_variable_modified_preserved`, `firewall_rule_modified_preserved`,
  `firewall_rule_name_in_use_preserved` (a stranger's rule of the same name),
  `url_protocol_scheme_in_use_preserved` (another application owns the
  scheme), `shortcut_location_unavailable` (Send To in machine scope),
  `firewall_rule_skipped_unelevated` (a per-user install without an
  administrator). Repair, which is asked for, is the one run that rewrites a
  resource the user changed.
- **Outcome documents** (`--json` on a mutating run) carry `outcome`, `code`,
  `exit_code`, the installation, the transaction, the dependencies
  considered, the custom actions executed (`actions[]`), the applications
  closed and restarted, and the log path.
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

TigerSetup is a Rust workspace of six crates under `crates/`:
`tigersetup-format` (the installer format), `tigersetup-catalog` (the WinGet
catalog client), `tigersetup-engine` (state, journal, planner, transactions,
recovery, resources), `tigersetup-loader` (the C Win32 loader every generated
`Setup.exe` begins with, compiled by its build script with the Visual Studio
C++ toolchain), `tigersetup-setup` (the engine executable with its
command-line client and the wizard) and `tigersetup-build` (the builder).
`proto/` holds the runtime-metadata schema, `lab/` the TigerWinLab
acceptance driver ([`lab/README.md`](lab/README.md)), `eng/` the developer
tooling and `docs/assets/` the project artwork: the small `TigerSetup.ico`
compiled into the loader, the engine and every generated installer, and the
richer `TigerSetup_32b.ico` of the builder.

The verification gate, the release discipline and the working rules for
contributors and AI agents are in [`AGENTS.md`](AGENTS.md).
[`LESSONS_LEARNED.md`](LESSONS_LEARNED.md) records the non-obvious things this
project paid to learn.

### Cleaning up

A working checkout accumulates gigabytes beyond Cargo's build output: lab
results, package builds, the synthetic test payload. `eng\Clean-TigerSetup.ps1`
knows which TigerSetup-generated locations are disposable and removes exactly
those, by its own explicit policy rather than by `.gitignore`:

```powershell
pwsh -File eng\Clean-TigerSetup.ps1 -Measure          # sizes only; nothing is removed
pwsh -File eng\Clean-TigerSetup.ps1                   # routine cleanup
pwsh -File eng\Clean-TigerSetup.ps1 -All              # ... plus artifacts\
pwsh -File eng\Clean-TigerSetup.ps1 -All -TestState   # ... plus HKCU\Software\TigerSetupTests
pwsh -File eng\Clean-TigerSetup.ps1 -WhatIf           # what a cleanup would remove
```

Routine cleanup removes Cargo's target directory (through `cargo clean`, so a
configured target directory is honored), `.is_screenshots\`, `lab\results\`,
`packages\test-app\*\payload\`, `packages\TigerMarkView\source\` and
`publish\`, and `packages\tigersetup\stage\`. It deliberately keeps
`artifacts\`, which may hold verified release installers — `-All` removes it —
and never touches `.claude\worktrees\`, `.git\`, tracked sources,
documentation, package definitions or anything outside the repository.
`-TestState` is the only thing that reaches outside the checkout: it removes
the registry namespace the Rust tests relocate their registry roots under, and
only that key. A junction or symbolic link in, at or on the way to a target is
skipped and reported rather than followed. The report shows the size of each
location, what was reclaimed, and any target that could not be removed; the
exit code is 0 only when everything requested was removed or already absent.
`eng\Test-CleanTigerSetup.ps1` tests the script against synthetic
repository-shaped directories.

## Licence

TigerSetup is released under the [MIT License](LICENSE.txt). Material
redistributed inside a generated `Setup.exe` is listed with its licence
notices in [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

## Copyright & Project Sponsor

TigerSetup is a project by **IT Tiger**, © 2026 IT Tiger.  
🔗 https://www.ittiger.net/
