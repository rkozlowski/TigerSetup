# TigerSetupTestApp — the synthetic package

The synthetic test package the transactional tests, the recovery rows, the
feature acceptance rows and the wizard captures use: about sixty core files in
nested directories, several of 2–6 MB so that a write window is real, in two
versions with overlapping-unchanged, changed, removed and added files, plus
one of every other resource kind, each behind the option the consolidated
acceptance names (`TigerSetup-Validation.md` §5.3):

```text
path-mode          choice: none | command (default) | tools — `tools` also installs the tools component
desktop-shortcut   off   the desktop link (declared with the older `option = "…"` spelling)
startup            on    the Startup link "TigerSetupTestApp Agent"
send-to            off   the Send To link (user scope only)
file-association   on    .tigertest → TigerSetupTestApp.Document, registered as a handler
url-protocol       on    tigersetuptest: → TigerSetupTestApp.tigersetuptest
context-menu       on    "Open with TigerSetupTestApp" on files, "TigerSetupTestApp here" on a folder background
environment        on    TIGERSETUPTESTAPP_HOME = %INSTALLROOT%
firewall           on    the inbound TCP 47110 rule "TigerSetupTestApp listener"
extras             off   the extras component (extras\**)
```

Always present: the Start Menu link (working directory `data`, AppUserModelID
`ITTiger.TigerSetupTestApp`), the documentation URL shortcut, the `App Paths`
entry, two product registry values, the Add/Remove Programs registration, and
the embedded prerequisite `IT-Tiger.TigerSetupTestPrereq` — the workspace's
own `TigerSetupTestPrereq.exe` (`crates/tigersetup-test-prereq`), which
installs itself as `%ProgramData%\TigerSetupTestPrereq\1.0.0` and exits with
the code it is told, so embedded-dependency behaviour is proven without the
Internet. Every identity is a test identity that cannot collide with a real
application. Product `IT-Tiger.TigerSetupTestApp`, name `TigerSetupTestApp`,
both scopes: user, whose default install root is
`%LOCALAPPDATA%\Programs\TigerSetupTestApp`, and machine, whose default install
root is `%PROGRAMFILES%\TigerSetupTestApp` and which needs an administrator.

`New-TestAppPayload.ps1` generates the payloads deterministically (PowerShell
7) — `payload\` (the core files), `payload-extras\` and `payload-tools\` (the
components) — and `Build-Package.ps1` copies the prerequisite beside them;
none of them is committed. `1.0.0/TigerSetup.toml` and `1.1.0/TigerSetup.toml`
are the manifests, and the process-level tests' fixture
(`crates/tigersetup-setup/tests/common/mod.rs`) declares the same package, so
a change to one is made in both.

## Building both installers

From the repository root:

```powershell
pwsh -File packages\test-app\Build-Package.ps1                    # cargo build --release, payloads, prerequisite, both installers
pwsh -File packages\test-app\Build-Package.ps1 -Fast -SkipBuild   # the iteration loop
$bin = "target\x86_64-pc-windows-msvc\release"
& "$bin\tiger-setup.exe" inspect artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe
& "$bin\tiger-setup.exe" verify  artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe
& "$bin\tiger-setup.exe" verify  artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe
```

The results are `artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe` and
`artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe`. `tiger-setup build` takes
the engine bytes from `tigersetup-setup.exe` beside itself; pass
`--engine <path>` to use another engine. Building twice from the same input
and engine gives identical bytes. `verify` checks the embedded prerequisite's
bytes against the hash the builder recorded, and `inspect --json` lists it
under `dependencies[]` with `acquisition.source = "embedded"`.

## Operation order

The engine journals operations in plan order: directories and files first,
then the registry keys and values (the product's, then the integrations'),
then the PATH entry, the environment variable, the shortcuts, the firewall
rule, and the Add/Remove Programs registration last, because a registration
means "installed" to Windows. Removals follow in the reverse resource order,
so an uninstall unregisters first and removes the install root last.

For 1.0.0 with the default options that is: operation 1 creates the install
root, operations 2-11 create the ten core directories (`bin`, `data`, `doc`,
`lib`, `locale`, then `bin\x64`, `data\tables`, `doc\legacy`, `doc\manual`,
`lib\plugins`), and the 64 core files follow in byte order of their
install-relative path from operation 12; a component's directory and files
appear only while its option is on. Resources come after the files, so
**operation 30 is still `data\big-4.bin`, a 6 MB file** — the operation the
recovery scenarios interrupt with `--fault <point>@30:<action>`. The
installer's log confirms the order: each `operation_applied` line carries
`sequence=` and `target=`.

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

`path-mode` decides what goes on the scope's `Path` — `HKCU\Environment` for
user scope, the machine hive's `Session Manager\Environment` for machine
scope: `none` nothing, `command` (the default) `<install root>\bin`, `tools`
`bin` and `tools`, the latter installing the tools component too. The boolean
options in the table above gate one resource each. `--option <name> <value>`
sets any of them (`on`/`off` for a boolean, the value for the choice); a
reinstall of the same version with an explicit option reconciles the
installation — the files, links, keys, variable and rule the option gates
follow — and an upgrade keeps whatever was chosen last.

## Exercising the installer

```powershell
$setup = "artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe"
& $setup install --quiet --scope user --json --log install.log
& $setup verify --json
& $setup inspect --json
& $setup install --quiet --scope user --fault after_write_before_flush@30:crash --log crash.log
& $setup install --quiet --scope user --json --log recover.log      # recovers forward, then finds it installed
& "artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe" install --quiet --scope user --json --log upgrade.log   # 1.0.0 → 1.1.0
& $setup install --quiet --scope user --option path-mode none --json    # reconciles: the PATH entry goes
& $setup install --quiet --scope user --option extras on --option firewall off --json   # the component appears, the rule goes
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
