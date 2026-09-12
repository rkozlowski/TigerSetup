# TigerMarkView — the reference package

The package TigerSetup's acceptance standard is measured on: a
TigerSetup-generated TigerMarkView installer that replaces the Inno Setup one
(`TigerSetup-Validation.md` §4). It is also the worked example of a real
package — MSBuild metadata, both scopes, options, shortcuts, PATH, two
dependencies, a legacy installation to migrate from, and WinGet metadata.

| File | Role |
|---|---|
| `TigerSetup.toml` | the package: identity, the MSBuild project the version comes from, both scopes, the file set, the PATH and desktop-shortcut options, the Start Menu and desktop shortcuts, the Add/Remove Programs registration, the Inno footprint to migrate from, the two dependencies, and the WinGet metadata |
| `Build-Package.ps1` | clones the pinned TigerMarkView commit into `source/`, publishes into `publish/`, and runs `tiger-setup build` |
| `lab-matrix.json` | what the acceptance matrix needs that only the product knows: the files a row names, the smoke commands, the settings file that must survive, the runtimes a "prepared" row installs first, the legacy installer's switches, the WinGet identity |

`source/` and `publish/` are working directories and are not committed. The
TigerMarkView checkout named by the TigerAiCore configuration is **read,
never written**: `dotnet publish` writes intermediates under a project's own
`obj/`, so the build clones the pinned commit rather than publishing in place.

## Building

```powershell
cargo build --release                                  # the builder and the engine
pwsh -File packages\TigerMarkView\Build-Package.ps1     # clone, publish, build
pwsh -File packages\TigerMarkView\Build-Package.ps1 -Version 0.8.2   # a second version to upgrade to
```

The result is `artifacts\TigerMarkView\TigerMarkView-<version>-Setup.exe`.
`-SkipClone` and `-SkipPublish` reuse what is already there; `-Offline`
resolves no dependency hints from the catalog, which the engine then
refreshes at install time.

The version is never typed here: `[metadata] source = "msbuild"` evaluates
`Version.props` through the project, and the published `TigerMarkView.exe` is
validated against it — a stale publish is a build error, not a release
artifact.

```powershell
tiger-setup metadata packages\TigerMarkView\TigerSetup.toml   # every value and where it came from
tiger-setup winget prepare packages\TigerMarkView\TigerSetup.toml `
    --installer artifacts\TigerMarkView\TigerMarkView-0.8.1-Setup.exe `
    --output artifacts\TigerMarkView\winget
```

## Migration from the Inno Setup installer

`[legacy]` names the Inno registration key, so the first TigerSetup install
on a machine holding an Inno-installed TigerMarkView removes it with its own
quiet uninstaller before installing (`TigerSetup-Design.md` §5.12). The
registration identity is the package id, `ItTiger.TigerMarkView`; the WinGet
`PackageIdentifier` is unchanged, and the settings file outside the install
root survives — row M16 of the acceptance matrix asserts all of it.
