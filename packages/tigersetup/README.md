# TigerSetup — the self-hosted package

TigerSetup packages itself. The generated `Setup.exe` installs the three
release binaries the project ships — `tiger-setup.exe` (the builder),
`tigersetup-setup.exe` (the engine) and `tigersetup-loader.exe` (the loader
every generated `Setup.exe` begins with) — and the installed help, and it is
built by TigerSetup's own current release, on the normal release-quality path
(`TigerSetup-Design.md` §8.4). This is the dogfooding the design calls for: the
installer TigerSetup produces for every other product is the one that installs
TigerSetup.

| File | Role |
|---|---|
| `TigerSetup.toml` | the package: TigerSetup's identity, the builder whose VERSIONINFO the product metadata is read from, both scopes, the payload, the Start Menu folder, the PATH option, the Add/Remove Programs registration, and the WinGet metadata |
| `Build-Package.ps1` | stages the release binaries and the help into `stage/` and runs `tiger-setup build` |

`stage/` is a working directory and is not committed.

## What it installs

```text
<install root>\tiger-setup.exe
<install root>\tigersetup-setup.exe
<install root>\tigersetup-loader.exe
<install root>\help\TigerSetup-Help.md     docs\TigerSetup-Help.md, as it is
<install root>\help\TigerSetup-Help.pdf    rendered from that Markdown at build time

Start Menu\TigerSetup\TigerSetup Shell             tiger-setup.exe shell
Start Menu\TigerSetup\TigerSetup Help              help\TigerSetup-Help.pdf
Start Menu\TigerSetup\TigerSetup Help (Markdown)   help\TigerSetup-Help.md
```

Only TigerSetup Shell wears TigerSetup's icon; the two help shortcuts show
their documents' file-type icons.

The install root is `%LOCALAPPDATA%\Programs\TigerSetup` per user (the default)
or `%ProgramFiles%\TigerSetup` for everyone; the PATH option (on by default)
adds it to that scope's `PATH`. TigerSetup Shell works either way: it opens
`%ComSpec%` with the install root first on that window's `PATH` only.

## Building

```powershell
pwsh -File packages\tigersetup\Build-Package.ps1              # release quality
pwsh -File packages\tigersetup\Build-Package.ps1 -Fast        # iteration only
pwsh -File packages\tigersetup\Build-Package.ps1 -SkipBuild   # reuse the built binaries
```

The result is `artifacts\tigersetup\TigerSetup-<version>-Setup.exe`.

The help PDF needs `tiger-mark`, the TigerMarkView command line, which the
script resolves as the registered `TigerMarkView` tool through TigerAiCore
(`eng\TigerAiCore.psm1`); `-TigerMarkPath <tiger-mark.exe>` overrides it for one
run. The PDF is generated on every build from the staged Markdown and never
edited or committed anywhere; `docs\TigerSetup-Help.md` is its only source.

The product version is never typed here: `[metadata] source = "exe"` reads it —
with the copyright and company — from the packaged `tiger-setup.exe`, whose
VERSIONINFO comes from the workspace `Cargo.toml` version. The generated
installer's own Windows VERSIONINFO and icon identify TigerSetup, and its
embedded metadata still records the engine that built it:

```powershell
tiger-setup inspect artifacts\tigersetup\TigerSetup-<version>-Setup.exe --json
```

## WinGet

```powershell
tiger-setup winget prepare packages\tigersetup\TigerSetup.toml --installer artifacts\tigersetup\TigerSetup-<version>-Setup.exe --output artifacts\tigersetup\winget
tiger-setup winget finalize artifacts\tigersetup\winget --url <published url> --installer artifacts\tigersetup\TigerSetup-<version>-Setup.exe
```

`lab\Invoke-SelfInstallerRows.ps1 -Rows winget-user,winget-machine,moderator
-ManifestDirectory artifacts\tigersetup\winget` exercises the finished set in
the lab (`lab/README.md`). Nothing is submitted from here.
