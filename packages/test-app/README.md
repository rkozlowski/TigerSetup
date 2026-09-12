# TigerSetupTestApp — the synthetic package

The synthetic test package the transactional tests, the recovery rows and the
wizard captures use: about sixty files in nested directories, several of
2–6 MB so that a write window is real, in two versions with
overlapping-unchanged, changed, removed and added files, and one of every
other resource kind — two installer options, a Start Menu and an option-gated
desktop shortcut, an option-gated PATH entry, two product registry values and
an Add/Remove Programs registration. It has no dependency, so an interactive
run reaches every wizard page in seconds. Product `IT-Tiger.TigerSetupTestApp`,
name `TigerSetupTestApp`, both scopes: user, whose default install root is
`%LOCALAPPDATA%\Programs\TigerSetupTestApp`, and machine, whose default install
root is `%PROGRAMFILES%\TigerSetupTestApp` and which needs an administrator.

`New-TestAppPayload.ps1` generates the payload deterministically (PowerShell 7);
the payload directories are not committed. `1.0.0/TigerSetup.toml` and
`1.1.0/TigerSetup.toml` are the manifests.

## Building both installers

From the repository root:

```powershell
cargo build --release
pwsh -File packages\test-app\New-TestAppPayload.ps1
$bin = "target\x86_64-pc-windows-msvc\release"
& "$bin\tiger-setup.exe" build packages\test-app\1.0.0\TigerSetup.toml --output artifacts\test-app
& "$bin\tiger-setup.exe" build packages\test-app\1.1.0\TigerSetup.toml --output artifacts\test-app
& "$bin\tiger-setup.exe" inspect artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe
& "$bin\tiger-setup.exe" verify  artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe
& "$bin\tiger-setup.exe" verify  artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe
```

The results are `artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe` and
`artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe`. `tiger-setup build` takes
the engine bytes from `tigersetup-setup.exe` beside itself; pass
`--engine <path>` to use another engine. Building twice from the same input
and engine gives identical bytes.

## Operation order

The engine journals operations in plan order: directories and files first,
then the product registry keys and values, then the PATH entry, then the
shortcuts, and the Add/Remove Programs registration last, because a
registration means "installed" to Windows. Removals follow in the reverse
resource order, so an uninstall unregisters first and removes the install
root last.

For 1.0.0 that is: operation 1 creates the install root, operations 2-11
create the ten directories (`bin`, `data`, `doc`, `lib`, `locale`, then
`bin\x64`, `data\tables`, `doc\legacy`, `doc\manual`, `lib\plugins`), and the
64 files follow in byte order of their install-relative path from operation
12. Resources come after the files, so **operation 30 is still
`data\big-4.bin`, a 6 MB file** — the operation the recovery scenarios
interrupt with `--fault <point>@30:<action>`. Operations 76-95 are the
resources: two keys (`HKCU\Software\IT Tiger` and its `TigerSetupTestApp`
child), the two product values, the PATH entry, the Start Menu shortcut, the
registration key and its thirteen values. The installer's log confirms the
order: each `operation_applied` line carries `sequence=` and `target=`.

An upgrade (`TigerSetupTestApp-1.1.0-Setup.exe install --quiet` over an installed
1.0.0) plans from the state database: kept files are journaled `applied` at
once and never walked, so its sequence numbers cover the directories first
(kept or created), then 1.1.0's files in byte order (kept, replaced or
added), then the resources (kept, or set where their data changed), then the
removals of 1.0.0-only files, then `doc\legacy`. The `transaction_started`
log line gives the counts (`kept=54 replaced=5 added=5 removed=5
directories_created=1 directories_removed=1 resources=...`), and the
`operation_applied` lines the sequence of each replaced, added or removed
resource for `--fault <point>@<sequence>`.

## Installer options

`path` (on by default) adds `<install root>\bin` as the last entry of the
scope's `Path` — `HKCU\Environment` for user scope, the machine hive's
`Session Manager\Environment` for machine scope; `desktop-shortcut` (off by
default) adds the desktop link, the user's or the shared one. `--option <name> <on|off>` sets either; a reinstall of the same version
with an explicit option reconciles the installation, and an upgrade keeps
whatever was chosen last.

## Exercising the installer

```powershell
$setup = "artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe"
& $setup install --quiet --scope user --json --log install.log
& $setup verify --json
& $setup inspect --json
& $setup install --quiet --scope user --fault after_write_before_flush@30:crash --log crash.log
& $setup install --quiet --scope user --json --log recover.log      # recovers forward, then finds it installed
& "artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe" install --quiet --scope user --json --log upgrade.log   # 1.0.0 → 1.1.0
& $setup install --quiet --scope user --option path off --json          # reconciles: the PATH entry goes
& $setup repair --quiet --scope user --json --log repair.log            # rewrites whatever is missing or changed
& $setup uninstall --quiet --scope user --json --log uninstall.log      # any version's installer uninstalls what is installed
& "$env:LOCALAPPDATA\TigerSetup\IT-Tiger.TigerSetupTestApp\uninstall.exe" uninstall --quiet --json   # and so does the copy Add/Remove Programs calls
```

`--scope machine` installs under `%PROGRAMFILES%` with its state under
`%ProgramData%\TigerSetup\IT-Tiger.TigerSetupTestApp`, registers in `HKLM`,
and uses the shared Start Menu and desktop folders. From an unelevated
console the installer raises the elevation prompt, runs the installation
elevated and reports the elevated run's document and exit code as its own;
from an elevated console it proceeds directly. `verify` and `inspect` never
elevate: a standard user can read the machine-scope database.

The installer writes that copy — the engine block, the same metadata marked
as the uninstaller of the scope, and no payload — into the state directory
before it opens a transaction, and `UninstallString` points at it. Running it
removes the product and then the state directory, itself included; `install`
and `repair` from it fail with `payload_unavailable`.

Fault switch: `--fault <point>[@<sequence>]:<action>[:<seconds>][:skip_flush]`
with points `after_prepare`, `after_applying`, `after_write_before_flush`,
`after_flush_before_rename`, `after_rename`, `after_applied`, `before_commit`,
`after_commit_before_cleanup` (and `after_rollback_undo` inside a rollback),
actions `crash`, `hold` (default 60 s) and `fail`. The switch may be repeated.
