# TigerSetup — the self-hosted package

TigerSetup packages itself. The generated `Setup.exe` installs the two release
binaries the project ships — `tiger-setup.exe` (the builder) and
`tigersetup-setup.exe` (the engine) — and it is built by TigerSetup's own
current release engine, on the normal release-quality path
(`TigerSetup-Design.md` §8.4). This is the dogfooding the design calls for: the
installer TigerSetup produces for every other product is the one that installs
TigerSetup.

| File | Role |
|---|---|
| `TigerSetup.toml` | the package: TigerSetup's identity, the builder whose VERSIONINFO the product metadata is read from, both scopes, the two binaries, the PATH option, the Add/Remove Programs registration, and the WinGet metadata |
| `Build-Package.ps1` | stages the release binaries into `stage/` and runs `tiger-setup build` |

`stage/` is a working directory and is not committed.

## Building

```powershell
pwsh -File packages\tigersetup\Build-Package.ps1              # release quality
pwsh -File packages\tigersetup\Build-Package.ps1 -Fast        # iteration only
pwsh -File packages\tigersetup\Build-Package.ps1 -SkipBuild   # reuse the built binaries
```

The result is `artifacts\tigersetup\TigerSetup-<version>-Setup.exe`.

The product version is never typed here: `[metadata] source = "exe"` reads it —
with the copyright and company — from the packaged `tiger-setup.exe`, whose
VERSIONINFO comes from the workspace `Cargo.toml` version. The generated
installer's own Windows VERSIONINFO and icon identify TigerSetup, and its
embedded metadata still records the engine that built it:

```powershell
tiger-setup inspect artifacts\tigersetup\TigerSetup-<version>-Setup.exe --json
```
