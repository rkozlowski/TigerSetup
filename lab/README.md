# TigerSetup lab driver

TigerSetup's consumer side of the TigerWinLab contract
(`TigerWinLab-Requirements.md` §2). TigerWinLab is a separate project: this
directory drives it and never modifies it.

| File | Role |
|---|---|
| `TigerSetupLab.psm1` | resolve the lab through TigerAiCore's resource resolver; invoke a lab entry point as a child process with a result file and an outer timeout; read a package's own facts through `tiger-setup inspect --json`; generate the installer, WinGet and recovery specifications from those facts; run `Setup.exe` commands in the guest; prepare dependency state; flatten lab checks into TigerSetup row results |
| `guest/Invoke-SetupCommands.ps1` | the guest job entry script: stages files outside the job workspace, runs the requested `Setup.exe` commands where each one names — the job account, the signed-in user's desktop, or elevated on that same desktop — and collects every exit code, JSON document, log, directory inventory, registry key and both `PATH` values |
| `guest/Invoke-PrepareDependencies.ps1` | installs named runtimes with their vendors' own installers, which is what "prepared" means in the matrix, and reports what was present before and after |
| `Invoke-MatrixRows.ps1` | the acceptance matrix of `TigerSetup-Validation.md` §5.2 |
| `Invoke-RecoveryRows.ps1` | the interrupted-install, interrupted-upgrade and rollback rows of `TigerSetup-Validation.md` §3, on the synthetic package |
| `Invoke-UiCaptureRows.ps1` | wizard captures at a language × scale × theme combination, for visual review |
| `Invoke-FeatureRows.ps1` | the consolidated feature rows of `TigerSetup-Validation.md` §5.3 on the synthetic package: options, option-gated components, environment variables, integrations, the new shortcut kinds, firewall rules, the embedded prerequisite and the custom lifecycle actions, through install → upgrade → changed reinstall → failed upgrade → failed action → committed change → reinstall → repair → uninstall |
| `Invoke-ElevationRows.ps1` | the all-users/elevation acceptance on the self-hosted installer: the native shield on Next, the genuine UAC prompt answered on the secure desktop, the elevated child completing the machine-scope install and handing its outcome back to the wizard that asked, a safe refusal, and per-user install without a prompt |
| `Invoke-ExplicitRegistryRow.ps1` | the explicit registry location row of `TigerSetup-Validation.md` §5.3 on the real machine hive: a `[[registry]]` value outside `Software` written over what Windows had, kept and repaired, preserved where an administrator changed it, and put back at uninstall, on `packages/test-explicit-registry` or any machine-only package that declares one |
| `Test-LabScripts.ps1` | the static gate: every script parses, and no function reads a variable nothing declares |
| `results/` | row results, lab artifacts and generated specifications per run; ignored by Git |

**Where a guest command runs.** Every command in a request names its session
with `runAs`, or inherits the request's: `job` is the job account (`LabAdmin`,
session 0, an administrator with no desktop), `interactiveUser` the signed-in
standard user's desktop, and `elevatedUser` an elevated agent on that same
desktop. The driver asks the lab for whichever sessions a request needs
(`-Desktop`, `-ElevatedDesktop`) and the payload uses the lab's own; nothing of
the lab's is copied into `guest/`. It matters wherever Windows itself
distinguishes the two: the Restart Manager lists a holder from its open file
handles across sessions but closes a window by messaging it, which does not
cross one, so M6 launches the application as the signed-in user and runs the
elevated upgrade beside it.

**Every job is one lease on the baseline VM, and the lab's policies are the
whole lifecycle.** A job's `EntryPolicy` says what it starts from — `Baseline`
restores the lab's clean checkpoint and boots it, `DontCare` takes what the
previous job left — and its `ExitPolicy` what becomes of the VM afterwards —
`DontCare` hands it to the lab, which shuts it down, restores the checkpoint
and leaves it Off; `PreserveUntilSessionEndOrNextLease` keeps it, running and
reserved to the session, for the next job. Omitted, the policies are
`(Baseline, DontCare)`: a job that says nothing starts clean and leaves the
VM clean. A row is a chain of jobs on one state, so the drivers splat
`Get-TigerSetupRowStepPolicy` (`-FromBaseline` for the first step): every step
preserves its result for the next, and the run's session end hands the VM
back. Nothing in this directory resets, starts, stops or cleans up a VM.

**One lab session per baseline.** Preserving state is only possible inside a
session, and a preserved VM stays running, reserved to the session, until the
session ends — which is what makes a VM boot once for a whole baseline's rows
instead of between them, and also what means a session preserving two
baselines holds two running VMs. The host runs a bounded number at a time and
the matrix spans three, so each driver ends the session for the baseline it is
leaving and opens one for the next; the ids are derived from the run's own
`-SessionId`, so they stay correlated. Every session ends in a `finally`.

**Ending a session hands the VM to the lab and returns; the lab normalizes it
on its own.** The close records what the session still held and whether the
lab's maintenance process started, and prints `OWED:` when it did not — the
VM then stays unavailable until the next lease on it performs the
normalization first. Nothing here waits for a guest to shut down. A driver
that must know the VM is back at the baseline and Off — the lease lifecycle
rows do — asks `Get-TigerSetupLabVmState`, which reads the lab's own state of
the VM (`Available`, `Leased`, `Preserved`, `Normalizing`, `Recovering`,
`Faulted`) with its reason. A lease a killed run abandoned and preserved state
past the lab's bound (an hour) are recovered by the lab at the next lease, and
`Close-TigerWinLabSession.ps1 -SessionId <id>` ends the session record a killed
run left open, as `Get-TigerWinLabSession.ps1` reports it. A VM the lab could
not normalize is `Faulted`, refused to every run, and recovered with
`Reset-TigerWinLab.ps1 -Baseline <name>`.

**Two things to do before any lab run, both of which take about a second and
each of which otherwise costs guest time to discover.**

1. `pwsh -File lab\Test-LabScripts.ps1` — every script parses, and no function
   reads a variable nothing declares. Under `Set-StrictMode -Version Latest`
   that mistake ends the row rather than the statement.
2. Build in this order: `cargo build --release`, **then** the installers, then
   the rows. A row measures the engine and the loader embedded in the
   installer, and `tiger-setup build` takes them from `tigersetup-setup.exe`
   and `tigersetup-loader.exe` beside itself — so rebuilding an installer does
   not pick up an engine change, and a whole matrix can be evidence about the
   wrong engine without saying so. The drivers refuse a mismatch of either up
   front, naming both hashes.

**Finding the lab.** `TigerAiCoreConfig` names the machine configuration; its
`core` value names TigerAiCore; and
`tools/Resolve-TigerAiCoreResource.ps1 -Lab TigerWinLab` turns the
`[labs.TigerWinLab]` registration into a path. That resolver is the only
mechanism — there is no sibling-directory guess, no filesystem scan and no
TigerSetup environment variable of its own, because a path that is right on one
machine is wrong on the next. `-TigerWinLabRoot` still points a single run at a
checkout under development; that is an override of a resolved location, not a
second way of discovering one. Resolver exit codes are `0` resolved,
`1` unavailable on this machine, `2` a broken configuration.

TigerSetup consumes **TigerWinLab only**. TigerHyperLab is the VM substrate
underneath it and is never called from here.

Requirements: PowerShell 7 on the host, a registered TigerWinLab, and built and
ready baselines (`Test-TigerWinLab.ps1 -All` in the lab). A baseline's VM has
one holder at a time, so a second consumer of the same baseline gets `BUSY`,
and one the lab is normalizing gets `UNAVAILABLE`; both are exit code 2. The guest
runs **Windows PowerShell 5.1**, so everything under `guest/` stays
5.1-compatible (no `ProcessStartInfo.ArgumentList`, no `Process.Kill(true)`); a
comma-joined list is accepted wherever a script takes a list, because
`pwsh -File` passes it as one string.

## The acceptance matrix

```powershell
pwsh -File lab\Invoke-MatrixRows.ps1 `
    -InstallerPath artifacts\TigerMarkView\TigerMarkView-0.8.2-Setup.exe `
    -PreviousInstallerPath artifacts\TigerMarkView\TigerMarkView-0.8.1-Setup.exe `
    -LegacyInstallerPath C:\Projects\TigerMarkView\artifacts\installer\TigerMarkView-0.8.1-win-x64-setup.exe `
    -ManifestDirectory artifacts\TigerMarkView\winget `
    -Rows checkpoint          # or: all, or M1,M5b,W1
```

**Nothing about the product is written twice.** The expectations of a row —
the install root, the registration key, the shortcuts, the `PATH` entries,
the declared options and dependencies — are read from the installer itself
through `tiger-setup inspect --json`, so a package that changes its manifest
changes its lab specification with it. What genuinely belongs to the product
lives in `packages/<name>/lab-matrix.json`: the smoke commands, the files a
row asserts by name, the settings file that must survive an uninstall, the
runtimes a "prepared" row installs first with their vendors' URLs, the
legacy installer's switches, and the WinGet identity. A second product needs
those two files, not a second driver.

`-Rows checkpoint` is M1, M2, M5a and W1 — the smoke subset run between
changes. `-Rows all` is the whole matrix, in series.

A row's result is `results/<run>/<row>.json`: `status`, the lab's environment
block, every check with its stable `code` (lab checks prefixed by the step
that produced them), and the evidence the verdict was read from — the engine
log tails, the `verify --json` / `inspect --json` documents, the PATH values,
the captures. A missing or unreadable lab result is a failing check, never a
pass.

`results/<run>/summary.json` is one line per row: its status, its check
counts and how long it took. A row that threw instead of finishing has
status `ERROR` and carries the message, the failing statement and the script
stack, because re-running a row to find out where it broke costs minutes of
guest time.

## The recovery rows

```powershell
pwsh -File lab\Invoke-RecoveryRows.ps1 `
    -InstallerPath artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe `
    -UpgradeInstallerPath artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe `
    -Rows all
```

**The interruption is triggered by the boundary, not by a clock.** Each row
passes `--fault-signal` to the engine, so the injected fault creates a file the
moment it reaches its journal boundary and then holds there; the row's
interruption specification carries a `signal` trigger on that file, which the
lab polls every 100 ms. The cut therefore lands inside the window by
construction. This matters most for the unflushed-write rows: a renamed file
comes back at full length with different content only if the power goes about a
second after the unflushed rename, because Windows' lazy writer flushes the
pages itself within a few seconds — which is also why the hold at the fault
point may not be lengthened to "make sure" (`LESSONS_LEARNED.md`). An
`install-poweroff-skipflush*` row whose file is intact anyway still reports WARN
("proved nothing"), never FAIL: an interruption that damaged nothing is absence
of evidence.

## The feature rows

```powershell
pwsh -File lab\Invoke-FeatureRows.ps1 `
    -InstallerPath artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe `
    -UpgradeInstallerPath artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe `
    -Rows all            # lifecycle-user, standard-user, machine on Win11; win10
```

The consolidated acceptance of the optional-resource model
(`TigerSetup-Validation.md` §5.3), on both synthetic installers built from the
same engine (`packages/test-app/Build-Package.ps1`). Each row chains guest
jobs that preserve the VM for the next, so one job's state is the next job's
starting point,
and after every step the same evidence is read: the engine's `verify --json`
and `inspect --json`, the install root, the state directory and the
prerequisite's directory, every integration key, both PATH values, the
firewall rule as the firewall service holds it, each shortcut (the target
the link stores, and the working directory and AppUserModelID as the shell
reads them; the URL of an Internet shortcut) and the environment variable in
both hives. A link's target is read from the file rather than resolved,
because the shell relocates a per-user target through its known folder and a
standard user's link read from the job account would answer with the wrong
profile. What the row expects is
derived from the option selection it made (`Add-ResourceChecks -Selected`),
so a resource is asserted present or absent by the same code.

**The shell probe runs where Windows would look.** `App Paths` is proved by a
real `ShellExecute` of the bare executable name, issued as a guest command in
the session whose registry the installer wrote — not from a collector in the
job process, which is elevated and therefore never consults `HKCU\…\App Paths`
(`LESSONS_LEARNED.md`). The `standard-user` row proves the per-user
registration from the signed-in standard user's desktop, the `machine` row
the machine one from the job; the elevated per-user rows assert the key alone.

**The failed upgrade is a real one.** Step 4 of `lifecycle-user` changes
several options and injects `--fault before_commit:fail`; the row then reads
the machine and requires the previous run's choices and resources, to the
value, and `transaction_rolled_back` in the log. Step 7 destroys owned
resources with `reg.exe`, `del` and `Remove-NetFirewallRule` and requires
`verify` to name each loss before `repair` restores it.

**The actions are read from what they wrote and from what the engine
recorded.** The package's five custom actions (`packages/test-app/README.md`)
write their markers, a record line per run and the cache under
`C:\ProgramData\TigerSetupTestActions`, which every step collects as logs
and inventory beside the state directory that keeps the uninstall programs;
`Add-ActionChecks` requires the outcome's `actions[]` (name and status, in
order, on the operation the step is), the cache for the installed version,
the markers that must and must not exist, and the stored programs under
`actions\<sha256>\` with the hashes `tiger-setup inspect` reports for the
package. `lifecycle-user` turns the `preflight` option on so the pre-install
phase runs on the install and the upgrade, adds a step 4b whose post-install
action fails on purpose — the run rolls back with `action_failed`, the
committed choices and resources stay, the program's own marker stays and the
outcome records `action_not_reverted` — deletes the cache in step 7 so the
repair proves the action that opted into repair rebuilds it, and reads the
uninstall log for the phase order: the pre-uninstall script is operation 1
and the post-uninstall program follows the removal of the install root.

## The wizard captures

```powershell
pwsh -File lab\Invoke-UiCaptureRows.ps1 `
    -ExecutablePath artifacts\TigerMarkView\TigerMarkView-0.8.2-Setup.exe `
    -TitlePattern TigerMarkView -LanguageArgumentTemplate '--lang {lang}'
pwsh -File lab\Invoke-UiCaptureRows.ps1 `
    -ExecutablePath artifacts	igersetup\TigerSetup-0.8.0-Setup.exe `
    -TitlePattern TigerSetup -LanguageArgumentTemplate '--lang {lang}' `
    -AdvanceByPage '2=Menu+A;Return'      # the self-installer's licence page: accept, then Next
```

A combination is `<language>:<scale>[:<theme>]`, and the default set is the
Windows 11 UI matrix of `TigerSetup-Validation.md` §8: both languages, four
scales, and both light and dark. Each one starts from the baseline, because the
capture answers every page it photographs — including the last one, which means
a run that reaches the wizard's ready page installs the product, and the next
combination would otherwise photograph an upgrade wizard. The job is started with the lab's `-Desktop`,
so the lab establishes the interactive session and hands it to the payload —
nothing of the lab's own is copied into `guest/`. What a row asserts is the
session the lab *measured*: a capture taken at the wrong scale, in the wrong
theme, or with something else on top of the wizard fails its row rather than
passing quietly as evidence for a combination it does not show. A capture is of
the screen as it is composited, so the runner asks who owns the middle pixel of
the window before each shot and records anything that is not the wizard.

**A row is not started on a desktop worth nothing.** The job is asked for with
the lab's `-RequireClearDesktop`, so the lab clears the transient shell state on
its own interactive desktop before the wizard starts and stops the row at that
precondition when it cannot — a menu left open before the row would otherwise
be in every picture the row takes. `capture.desktop.precondition` records what
the lab found and cleared. Should something appear over the wizard anyway, the
run ends at the page it was found on: every page after it would be photographed
through the same obstruction, so capturing them spends the row to learn nothing.

**A page is answered when the wizard leaves it.** The wizard keeps its forward
control visible and disabled while it is busy, so keys pressed at a page that is
downloading a dependency or applying a transaction are lost rather than queued.
After answering a page the capture therefore waits for the page itself to change
— identified by the set of control ids it shows, which survives the label text a
progress page rewrites as it works — or for the process to exit, bounded by
`PageTimeoutSeconds`. A page that never changes ends the wizard with what it was
still showing, which names a wrong key rather than photographing the same page
until the page budget runs out.

**A capture from the session cannot read the secure desktop.** An elevation
prompt switches the input desktop to one the session may not open, and
`screenshot` fails there while every window the session enumerates still says
the wizard is in front. A capture row that must stay unelevated has to answer
the pages that decide that: W6 selects "Install for me only" on the scope page
precisely because the neighbouring choice would raise a prompt no session
capture could read. The elevation rows below are the ones that want that
prompt, and they capture it from the host instead.

## The elevation rows

```powershell
pwsh -File lab\Invoke-ElevationRows.ps1 `
    -InstallerPath artifacts\tigersetup\TigerSetup-0.8.0-Setup.exe `
    -Rows shield-refuse,complete-uac,complete-uac-admin,complete-elevated,complete-user   # all five by default
```

The all-users/elevation acceptance for the wizard, on the **self-hosted
TigerSetup installer** — the artifact under release, because the privilege
transition is a property of the bytes that ship, and the synthetic package
would prove the engine rather than those bytes. Five rows on one session:

- **complete-uac** is the row the others exist around: the whole real path on
  one wizard. The installer is started unelevated through the lab's tracked
  wrapper (so its exit code and what it prints are evidence), "for all users"
  is chosen, the Next button's region is compared with and without the shield,
  Next is pressed, and the genuine credential prompt the standard user gets is
  approved on the secure desktop through the lab's host console
  (`Approve-ElevationPrompt`: the lab types its administrator's password on the
  VM's console, never in the guest). The row then asserts the transition
  rather than assuming it: consent.exe left and the input desktop returned;
  exactly one process appeared elevated in the session and it is this
  installer, high integrity, started with `--elevated-result`; the child put up
  a visible wizard; the parent is alive, responsive and has stepped aside. The
  process the lab starts, tracks and reads the exit code of is the loader
  every generated `Setup.exe` begins with; the wizard, its pages and its
  liveness belong to the engine child the loader starts, which the guest
  scripts resolve from the loader's process id, both for the unelevated
  parent and for the elevated child (which is the loader again, elevated, with
  its engine under `%SystemRoot%\Temp`). The
  child is driven to its completion page through the lab's elevated agent (a
  high-integrity window cannot be clicked into from the standard session),
  closed, and the parent's exit code and printed document — the child's
  outcome, `installed` in `machine` scope — are read from the tracked run; the
  machine-scope state is verified with `inspect`/`verify --json`.
- **complete-uac-admin** is the same row on an administrator's desktop
  (`-InteractiveKind administrator`), where the prompt is the Yes/No consent
  prompt with *No* as its default button — the case a developer installing on
  their own machine meets — approved with the consent chord.
- **shield-refuse** drives the unelevated wizard. It captures the Next button
  with "for me only" and again with "for all users" and compares the button's
  own region: the native shield (`BCM_SETSHIELD`) appears only for all users,
  survives a hover and a repaint, and is cleared when "for me only" is chosen
  again. Then it presses Next, confirms the prompt this press raised, asserts
  the wizard is **still pumping messages** while the prompt is up
  (`window-responsive`), refuses the prompt on the secure desktop
  (`Deny-ElevationPrompt`, Escape — a person's No), dismisses the wizard's
  error dialog, and confirms the wizard is usable again and nothing was
  installed.
- **complete-elevated** drives the wizard in the lab's credential-backed
  elevated session (`-ElevatedDesktop`): all users completes with no shield
  and no prompt, and the machine-scope state is verified.
- **complete-user** drives the unelevated wizard through a per-user install
  with no elevation at any point, and verifies the user-scope state in the
  signed-in account's own session.

**Why one wizard end to end.** When correctness depends on a real privilege
transition, testing the halves — the prompt raised and refused with the wizard
responsive, an already-elevated wizard completing, a process test handing a
result document back — does not prove the handoff: every half can pass while
the unelevated wizard, after a genuine consent, waits for an elevated child
nobody can see. `complete-uac` and `complete-uac-admin` are the handoff
(`TigerSetup-Validation.md` §2, `LESSONS_LEARNED.md`). Nothing about UAC or
its secure desktop is changed to make the rows possible: the lab's keyboard is
keyboard hardware to the guest, and the prompt is the real one — the lab's
`console/uac-prompt.png` in each job is its own capture of that secure desktop.
The lab attributes the prompt it answers to the press that raised it by time
and exclusivity, because `consent.exe` does not name its requester; a prior or
ambiguous prompt is refused, not answered (`TigerWinLab-Requirements.md` §4).

Build order still binds (`cargo build --release`, then rebuild the installer):
a row measures the engine embedded in the installer, and the driver refuses a
stale engine up front.

## The explicit registry location row

```powershell
pwsh -File lab\Invoke-ExplicitRegistryRow.ps1                       # builds and runs packages\test-explicit-registry
pwsh -File lab\Invoke-ExplicitRegistryRow.ps1 -InstallerPath <a machine-only package that declares an explicit value>
```

One session, one VM, five chained jobs, every one read with the generic
guest reader so the registry is read as Windows holds it. `baseline` records
what the clean VM holds at every explicit location the package declares
(`tiger-setup inspect --json`, every `registry_values[]` entry whose `root`
is not `software`); `install` writes them, and `verify --json` and
`inspect --json` own them at their explicit paths; `reconcile` reinstalls
with an explicit option (which makes the run reconcile) and converges with
no finding, then turns the setting back off with `reg.exe` as an
administrator would, reinstalls again — the change is preserved and reported
(`registry_value_modified_preserved`, and `verify` says
`registry_value_modified`) — and repairs, which puts the package's value
back; `uninstall` gives the baseline back: a value Windows had holds what it
held, a value TigerSetup created is gone with the keys it created, and the
Windows keys above them stay.

The fixture pairs `LongPathsEnabled`, which every clean baseline holds as
`0` under a key Windows owns, with a marker under a key chain that does not
exist, so one row covers the pre-existing and the created case. The
process-level tests (`crates/tigersetup-setup/tests/explicit_registry.rs`)
cover the same lifecycle plus rollback and the option gate against relocated
roots; this row is the real hive, and the one thing those tests cannot say.
