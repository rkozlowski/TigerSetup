# WinMerge 2.16.58.2 — benchmark functional contract

Package identity note: upstream's real version is the four-part
`2.16.58.2`; TigerSetup requires a strict `major.minor.patch` version
(`AGENTS.md` version discipline), so all three benchmark packages for this
app use `2.16.58` for identity parity across technologies.

Derived from WinMerge's real upstream Inno Setup script
(`Installer/InnoSetup/WinMergeX64.is6.iss`, read at the pinned tag).

| Feature | Upstream default | Benchmark contract |
|---|---|---|
| Install scope | `PrivilegesRequiredOverridesAllowed=dialog` -> user/machine choice at install time | both `user` and `machine` scopes |
| Payload | full WinMerge x64 tree (~82 MB canonical) | identical canonical payload, all three technologies |
| Start Menu shortcut | always | always |
| Desktop shortcut | optional task, unchecked | optional, default **off** |
| Explorer context menu | optional task, **checked** by default, registers the real `ShellExtensionX64.dll` via `regsvr32` | optional, default **on**; `regsvr32 /s` at post-install, `regsvr32 /u /s` at pre-uninstall (a genuine custom lifecycle action in every technology -- none of the three has a typed "register a COM shell extension" primitive). Each technology resolves `regsvr32` its own way rather than by a path written into the package: IS `{sys}`, NSIS `ExecWait` by name, TS a packaged one-line batch action the command shell resolves |
| Add to PATH | optional task, unchecked | optional, default **off** |
| File association | always, `.WinMerge` project file -> `WinMerge.Project.File` -> `WinMergeU.exe "%1"` | always |
| App Paths | always, `WinMergeU.exe` (and classic `WinMerge.exe` alias) | always, `WinMergeU.exe` only -- the benchmark payload does not ship a separate `WinMerge.exe` alias binary, so the alias entry is dropped rather than pointed at a file that would not exist |
| Component types (Typical/Full/Compact/Custom), ~45 language components, filter/plugin components | real installer offers granular components | excluded -- the whole canonical payload installs as one unit in all three technologies; a per-component install matrix would not be comparable across three very different component models and is not needed to prove the packaging/installer-technology point |
| TortoiseCVS/Git/SVN integration | conditional, only when detected | excluded -- environment-conditional, not a fixed installer behavior |
| Quick Launch shortcut | only pre-Windows 7 | excluded -- dead code path on the benchmark's Windows 11 baseline |

Every option defaults to the same value as the real WinMerge installer.
