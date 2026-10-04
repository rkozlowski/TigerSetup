# TigerSetup privacy statement

This statement describes what TigerSetup does with information on your
computer and what it sends over the network. It covers the current release of
TigerSetup for Windows; earlier versions of this statement are in this file's
history in the repository.

TigerSetup is published by **IT Tiger** (<https://www.ittiger.net/>). Questions
about this statement can be asked at
<https://github.com/rkozlowski/TigerSetup/issues>; a security concern is
reported privately as [`SECURITY.md`](SECURITY.md) describes.

## In short

- TigerSetup has **no telemetry, no analytics, no crash reporting, no
  advertising, no accounts, no licence activation, no update checks and no
  self-update**. IT Tiger receives nothing from it.
- Everything TigerSetup records stays on your computer, in the places listed
  below, and a successful uninstall removes all of it.
- TigerSetup contacts the network only to obtain a **prerequisite a package
  declares and the computer lacks** — from the WinGet catalog, or from the
  download address the package declares — as described below, and never for
  TigerSetup's own installation.

## What this statement covers

TigerSetup is a tool for building installers. This statement covers:

- **TigerSetup itself**: the `tiger-setup` builder, the installer engine and
  loader it installs, the installed help, and the installer that installs
  TigerSetup;
- **the TigerSetup code inside every `Setup.exe` built with it**: the loader,
  the engine and the uninstaller copy it leaves for Add/Remove Programs.

An application installed by a `Setup.exe` built with TigerSetup is its own
publisher's software, with its own privacy terms; so are any programs or
scripts that publisher's package runs during installation. This statement
describes only what TigerSetup's own code does.

## Building installers (`tiger-setup`)

`tiger-setup build`, `metadata`, `inspect`, `verify` and `winget` work on
files you name. They read the `TigerSetup.toml` manifest and the files it
names — the application's files, icons, licence text, prerequisite
installers, custom-action programs — and, where the manifest takes product
metadata from them, the version information of an executable. They write the
`Setup.exe` you asked for, and the WinGet manifests or exported files you
asked for, where you asked for them.

A built `Setup.exe` contains the application's files and the package
description resolved from the manifest. File locations in it are relative to
the installation folder; the build machine's own folders are not recorded.

- **MSBuild metadata (optional).** A manifest that takes product metadata
  from an MSBuild project makes `tiger-setup` run `dotnet msbuild` on that
  project, which needs the .NET SDK. The .NET SDK is Microsoft's software and
  behaves according to its own settings and terms, including the .NET SDK's
  own telemetry (which `DOTNET_CLI_TELEMETRY_OPTOUT` controls). TigerSetup
  does not change those settings and does not send anything itself.
- **`tiger-setup shell`** opens a command prompt; it changes nothing on the
  computer.
- **While it resolves a WinGet prerequisite** (see *Network access*),
  `tiger-setup build` keeps the downloaded catalog index in a folder of its
  own under `%TEMP%` and removes it when the build ends.

## Installing, repairing and uninstalling

When TigerSetup's installer — or any `Setup.exe` built with TigerSetup —
installs, upgrades, repairs or uninstalls a product, it keeps the following on
the computer, for that product only:

- **The state directory**: `%LOCALAPPDATA%\TigerSetup\<product id>\` for a
  per-user installation, `%ProgramData%\TigerSetup\<product id>\` for an
  all-users installation (writable by administrators only). It holds:
  - the **state database** (`state.db`): the product's identifier, version
    and scope; the installation folder; every file, folder, registry value,
    PATH entry, shortcut and other resource the installation added, by path
    or name; the installer options chosen; a fingerprint (SHA-256) of the
    licence text accepted in the wizard, if one was accepted; the record of
    the transaction in progress; the prerequisites considered, with the
    version, download address and SHA-256 of any that were downloaded; and
    the custom actions run, with their exit codes;
  - the **uninstaller** (`uninstall.exe`) that Add/Remove Programs runs, and
    copies of the product's uninstall-time programs, if it declares any;
  - **logs** of each install and repair (`logs\`).
- **Logs** record what each run did: the time, the product and version, the
  scope, the installation folder, each file or resource changed, prerequisites
  considered, and — when files are in use — the names and process IDs of the
  applications Windows' Restart Manager reported holding them, and whether
  they were closed and restarted. Output that a package's custom actions
  print is recorded too. Paths in logs and in the database can include your
  Windows account name, because that is part of your profile folder's path.
- **While a run is in progress** TigerSetup uses temporary files of its own:
  the engine the loader extracts (under `%TEMP%\TigerSetup\`, or a protected
  folder under `%SystemRoot%\Temp\` for an administrator's run), downloaded
  prerequisite installers (in the state directory), and, for an uninstall, the
  temporary uninstaller copy and the uninstall's log. They are removed when
  the run ends.
- **Copy log path.** The wizard's completion page offers a *Copy log path*
  link. Only when you choose it does TigerSetup put the log's path on the
  clipboard. TigerSetup never reads the clipboard.

TigerSetup records only what it needs to upgrade, repair, verify and remove
the installation. It reads nothing else of yours: no documents, no browser
data, no credentials.

### Retention and deletion

The state directory and its logs are kept while the product is installed, so
that upgrade, repair, verification and uninstall work from what is really
installed. **A successful uninstall removes everything TigerSetup added**: the
product's files and resources, the state database, the logs, the uninstaller,
the Start Menu entries, the PATH entry and the Add/Remove Programs entry, the
uninstall's own temporary files and log, and the `TigerSetup` folders when no
other product's state is left in them. Something that existed before the
installation — a PATH entry, a folder with your own files in it — is left as
it was, and so is an installed file you changed afterwards, which the
uninstall reports rather than deletes.

A run that fails keeps its log, and the state the next run needs to finish or
roll back the interrupted change, so that the problem can be diagnosed. A log
written to a path you choose with `--log` is yours, and TigerSetup does not
remove it.

## Network access

TigerSetup's own installer declares no prerequisites, so installing,
repairing or uninstalling TigerSetup makes **no network request at all**.

The network is used only to obtain a **prerequisite a package declares** —
for example the .NET Desktop Runtime or the WebView2 Runtime — in one of two
ways the package chooses:

- **From the WinGet community catalog** (the default for a prerequisite with
  a WinGet identity). When `tiger-setup build` runs for such a package, it
  reads the catalog to find the prerequisite's current installer and records
  that installer's download address and SHA-256 in the `Setup.exe`.
  `tiger-setup build --offline` makes no network request; the installer then
  resolves the prerequisite itself when it needs it.
- **From a download address the package declares**, with the SHA-256 the
  installer must have. The catalog is not involved.

**When a built `Setup.exe` runs** on a computer where a declared prerequisite
is missing, it downloads that prerequisite's installer: from the address
recorded at build time while that record is fresh (two weeks by default), and
otherwise — or when that download fails — from the address the WinGet catalog
names now, after reading the catalog again. A prerequisite the package carries
inside the `Setup.exe` is never downloaded. If the prerequisite is already
present, nothing is downloaded at all. `Setup.exe install
--no-dependency-install` never downloads: it fails with `dependency_missing`
instead.

What these requests send:

- The catalog is read over HTTPS from Microsoft's WinGet content servers
  (`cdn.winget.microsoft.com`): the catalog index, then the version list and
  the manifest of the prerequisite. The request names the prerequisite's
  WinGet package identifier.
- A prerequisite's installer is downloaded from the address the catalog names
  for it — that prerequisite's publisher's server, for example Microsoft's
  download servers — or from the address the package's publisher declared.
- Every request carries the `User-Agent` `TigerSetup`. Like any download, it
  reveals your computer's IP address to the server it goes to, and goes
  through Windows' proxy settings. No identifier of you, your computer or the
  product being installed is added, and no cookie is stored. Those servers
  are operated by their owners, under their own privacy terms.

The downloaded catalog is accepted only when it carries Microsoft's
signature, and a downloaded installer only when its SHA-256 matches the one
recorded for it — by the catalog, or by the package. A package's custom
actions may contact the network themselves; they are that package's
publisher's programs.
## Changes to this statement

This statement changes when TigerSetup's behaviour changes, in the same
release. The version that applies to a release is the one in that release's
source; the current one is at
<https://github.com/rkozlowski/TigerSetup/blob/main/PRIVACY.md>.
