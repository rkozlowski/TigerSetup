# TigerSetup — Validation

This document owns how TigerSetup is proven correct: the validation levels,
what counts as acceptance evidence, fault injection, the acceptance standard,
the TigerWinLab acceptance matrix, and the TigerWinLab project boundary.

What TigerSetup is and how it works is owned by `TigerSetup-Design.md`; what
the acceptance needs from the lab, and how TigerSetup consumes it, by
`TigerWinLab-Requirements.md`; the drivers that run the rows are documented in
`lab/README.md`.

---

## 1. Validation is an architectural feature

Verification is not an afterthought or only an external test script. TigerSetup
makes machine verification cheap and explicit:

```text
Setup.exe inspect --json
Setup.exe verify --json
```

The product provides deterministic evidence consumable by TigerWinLab, CI and
release automation, support tooling, and package-manager validation.
Machine-readable output uses stable identifiers, never localized text
(`TigerSetup-Design.md` §6.3), so that a lab, a pipeline or an agent can run

```text
provision/reset TigerWinLab
→ install
→ inspect exit code and machine-readable result
→ verify
→ launch/smoke-test the application
→ upgrade
→ verify
→ uninstall
→ verify absence
```

without clicking dialogs or interpreting screenshots.

---

## 2. Validation levels, and which of them is acceptance

Two kinds of evidence exist, and they are not interchangeable:

- **Supporting evidence** — unit and local integration tests. They establish
  that a component does what it should when it is operated, on whatever
  machine `cargo test` runs on. They are fast, deterministic and run on every
  change, and they are the gate a change must pass *before* the expensive
  evidence is paid for.
- **Acceptance evidence** — the TigerWinLab rows of §5.2 and the UI matrix of
  §8, run against the exact installer bytes under validation, on a clean
  Windows the lab restored and measured. Whether the product is acceptable on
  Windows is established here and nowhere else. No screenshot, scale, theme
  or elevation observation taken on a developer's own desktop is evidence for
  a row.

**Where a requirement is end to end, acceptance exercises the actual
integration transition.** Passing the pieces separately is supporting
evidence for each piece; it is not evidence for the seam between them. The
rule matters most for privilege:

> **When product correctness depends on a real privilege transition, testing
> the pre-consent and post-elevation halves separately does not prove the
> handoff.**

A wizard that raises the UAC prompt correctly, an already-elevated wizard that
installs correctly, and a process test that hands a result document back can
all pass while the unelevated wizard, after a genuine consent, waits forever
for an elevated child nobody can see. The elevation rows of §5.2 therefore
run one wizard from the unelevated request, through the genuine prompt on the
secure desktop, to the outcome that comes back to it.

### 2.1 Unit tests

Deterministic internal logic: manifest parsing and validation, metadata
normalization and provenance, version semantics, the installer format
(footer, metadata, payload, compose, inspect, verify), planning and
reconciliation, ownership decisions, transaction state transitions,
dependency detection and selection, the catalog client against fixtures, the
Restart Manager holder logic, the wizard's layout, theme and glyph rules.

### 2.2 Local integration tests

Process-level tests in `crates/tigersetup-setup/tests/` build real installers
from the synthetic package (`packages/test-app/README.md`) and run them as
separate processes with `%LOCALAPPDATA%` and the registry hives redirected, so
they write only under `target\`:

- the whole lifecycle — install, verify, reinstall, option reconciliation,
  upgrade, repair, uninstall, the uninstaller copy — in user scope and, where
  this process can write both roots, machine scope;
- every resource kind: files, directories, registry keys and values, PATH
  entries with the pre-existing-lookalike and empty-segment vectors,
  shortcuts including a folder that moved, the registration;
- a registry value at an explicit location outside the scope's `Software`
  root, on a machine-only package against relocated hives: created where
  nothing was, written over a prior value and the prior value restored at
  uninstall, deleted where it was created, left as found where it already
  held the wanted data, preserved and reported where an administrator
  changed it (reinstall, upgrade and uninstall) and rewritten by a repair,
  rolled back with a failing install, gated by its option, owned at its
  explicit path in `inspect` and checked by `verify`; and the builder
  refusing an explicit root the package's scopes cannot write;
- crash and fault injection at every journal boundary of an install, an
  upgrade and an uninstall, including a skipped flush and a zero-filled
  target, each converging under recovery to a state `verify --json` confirms
  and the disk compared by content with exactly one version's payload;
- dependencies against a local HTTP server: present, acquired, hash mismatch,
  offline, a failing installer, a reboot request, elevation required, opt-out;
- custom actions (`TigerSetup-Design.md` §5.14) against the controlled
  `TigerSetupTestAction.exe` and real PowerShell and batch scripts: the
  envelope — expanded arguments with spaces, Unicode and a lone `%`, the
  working directory, the `TIGERSETUP_*` environment, captured output — the
  verdict on success, custom success and reboot codes, a failure that rolls
  the run back without reverting the program's own marker, `continue`, a
  timeout that kills the program, a program that cannot be started, phase
  order and `run_on` through install → upgrade → repair → reinstall →
  uninstall on the synthetic package, a failing action on a reinstall, the
  uninstall actions run from the state directory after the original
  installer is deleted, an upgrade that fails keeping the previous stored
  program and one that commits switching it, a tampered stored program found
  by `verify`, restored by `repair` and refused at uninstall, a crash while an
  action runs recovered forward (`action_interrupted`, run again) and back
  (never run), a packaged program whose bytes do not match failing
  `tiger-setup verify` and the run, and `inspect` listing every declared
  action;
- the cross-scope policy, elevation argument handling, legacy migration with
  a stub uninstaller, quiescence against a real holder process;
- package-declared quiescence (`TigerSetup-Design.md` §5.10) against a real
  application the Restart Manager cannot close — a console process with no
  message loop that ignores the control event, holding an installed file
  without delete sharing: stopped by the package's stop program before the
  Restart Manager is asked and resumed after the upgrade committed, left
  alone when it was not running, resumed when the Restart Manager then
  refused the run over a second holder (nothing mutated), stopped and not
  resumed by an uninstall run from the state directory, a failing stop
  program ending the run before any mutation or recorded under `continue`,
  and `inspect` listing the entries;
- the loader and the container: every process-level test runs the real
  `Setup.exe` — the C loader, compressed engine, solid payload, compressed
  metadata — so the interactive and silent paths, the elevated relaunch,
  argument and exit-code propagation, the temporary uninstaller copy and
  the cleanup of the extracted engine are exercised by all of them; a
  corrupted payload block, a corrupted engine block and a corrupted or
  mis-declared metadata block fail safely (`tiger-setup verify` names the
  problem, the engine refuses the bytes, the loader refuses to start an
  engine whose hash does not match). The loader's own tests
  (`crates/tigersetup-loader/tests`) run it against synthetic packages
  around a fake engine: the command-line tail and the exit code forwarded
  verbatim, every malformed footer and damaged or mis-declared engine block
  refused before anything executes, nothing left behind on any path, eight
  concurrent launches in their own directories, the stale sweep, a signed
  layout, and — run elevated, on the lab's elevation rows — the protected
  system-temp directory;
- the journal's commit groups: a crash inside a commit group — up to eight
  consecutive journal batches sharing one `applying` and one `applied`
  commit (`TigerSetup-Design.md` §5.4) — before its first mutation,
  part-way through its files or after every file is in place but before the
  group is acknowledged, is recovered by reconciling the group (the files
  already in place completed without a rewrite, the missing ones written),
  a group spans its batches and closes at its bound, and an interrupted
  upgrade group rolls back to a verified previous version; the `batched`
  fault modifier keeps the named operation inside its group, where a real
  interruption lands;
- the wizard, driven through window messages against its published UI
  Automation ids — a real window on whatever desktop `cargo test` runs on,
  answered with posted messages rather than pointer or keyboard input, so it
  neither takes the desktop over nor depends on it being left alone;
- launch after install (`TigerSetup-Design.md` §11.7), with the controlled
  `TigerSetupTestLaunch.exe` reporting how it was started: the builder's
  `[launch]` parsing and refusals (not an `.exe`, not installed, a working
  directory outside the install root, a command-line string instead of an
  argument list), the offer checked and unchecked as declared, Finish
  starting the program unelevated with its own token, in the declared
  directory, with nine hostile arguments — spaces, quotes, trailing and
  quote-adjacent backslashes, an empty one, non-ASCII, a bare `%`,
  `%VERSION%` — arriving exactly, an interactive upgrade offering it again
  with the new version, repair and uninstall never offering it, a cleared box
  recorded as declined, a program that cannot start reported in the wizard's
  own box while the run still ends installed with exit 0 and verifies, and a
  rolled-back, a cancelled and a quiet run starting nothing. The elevated
  paths and the foreground need a real prompt and a desktop nobody else is
  using, so they are the lab's launch rows (§5.2).

`cargo test --workspace` runs all of them; the process-level tests take several
minutes.

### 2.3 TigerWinLab end-to-end rows

Real Windows behaviour on restored baselines: user-scope and machine-scope
installation with elevation, PATH, ARP registration, dependency detection and
acquisition, running applications, upgrade, reinstall, silent operation,
crash and restart recovery, uninstall, shared-dependency preservation,
application launch and smoke tests, the wizard at every language × scale ×
theme combination, and the genuine UAC handoff. §5.2 and §8 are the rows.

---

## 3. Fault injection is a retained engineering capability

Crash consistency is TigerSetup's hardest problem, so fault injection is a real
testing mechanism compiled into every build rather than temporary debug code:
`--fault <point>[@<sequence>]:<action>[:<seconds>][:skip_flush]` affects only
the invoking run, so the bytes the interrupted rows validate are the bytes
that ship. The points are the journal/mutation boundaries —
`after_prepare`, `after_applying`, `after_write_before_flush`,
`after_flush_before_rename`, `after_rename`, `after_applied`, `before_commit`,
`after_commit_before_cleanup`, and `after_rollback_undo` inside a rollback —
and the actions are `crash`, `hold` and `fail`. `--fault-signal <path>`
creates a file the moment the fault reaches its boundary, so a harness can
interrupt the process, the guest or its power exactly there. The journal is
written in commit groups (`TigerSetup-Design.md` §5.4); an operation a fault
names is a commit group of its own, so a fault's boundary is exactly that
operation's — everything before it durably applied, nothing after it
started — and the rows below mean what they always meant.

```text
journal applying (with the undo record)
        ↓
      [FAIL]
        ↓
Windows mutation
        ↓
      [FAIL]
        ↓
journal applied
```

It gives machine-verifiable answers to:

- Was enough undo state durable before the mutation?
- Can an ambiguous operation be reconciled after restart?
- Is rollback idempotent?
- Does ownership remain correct after partial failure?
- Does an interrupted upgrade converge to exactly version A or exactly version
  B, never a mixture (`TigerSetup-Design.md` §5.4)?

**The recovery rows** (`lab/Invoke-RecoveryRows.ps1`) are the lab half of this
capability and run against the synthetic two-version package: process kill,
reboot and power-off during an install and during an upgrade at each
boundary, each ending with the product's own recovery in the guest the
interruption left, a passing `verify --json`, and — for an upgrade — exactly
1.0.0 or exactly 1.1.0; the old version's uninstaller against an open
upgrade rolls it back and removes the product to a verified absence; and the
unflushed-write rows, which skip the flush and cut the power about a second
after the rename, must show in the engine's own log that a file came back
without its content and was re-applied. **An interruption that damaged nothing
proved nothing**: such a row reports WARN, never PASS and never FAIL. The cut
is aimed by the product (`--fault-signal`) rather than by a clock, because
the symptom exists only in a window of about a second after the write.

---

## 4. The acceptance standard

TigerSetup is acceptable when:

> **The TigerSetup-generated TigerMarkView installer can replace the
> production installer for the behaviours in scope.**

Not "TigerSetup produced an `.exe`". The installer being replaced is the
behavioural oracle where one applies.

```text
TigerSetup
    ↓
packages/TigerMarkView/TigerSetup.toml
    ↓
TigerMarkView-<version>-Setup.exe
    ↓
TigerWinLab automated validation
    ↓
install → verify → upgrade → verify → uninstall → verify absence
    ↓
PASS
```

The matrix of §5.2 is the concrete form of that standard. It is fixed here so
that the standard is objective and is not redefined after the fact; reducing
it is an Architect decision.

### Exact artifact principle

The installer bytes that pass validation are the bytes that are published.
Release validation runs on the release's own artifacts, retrieved from the
draft release that will publish them, and never on a rebuild, however
equivalent (`RELEASING.md` for TigerSetup's own releases). A locally built
installer is evidence for a change, not a release.

> **Build once, validate exact bytes, publish those exact bytes.**

---

## 5. Validation dimensions

```text
OS                  Windows 11 x64 (primary) / Windows 10 22H2 x64 /
                    Windows Server 2019 x64
Identity            administrator / standard user
Install scope       user / machine
Language            en-US / pl-PL
DPI                 100% / 125% / 150% / 200%
Theme               light / dark
Network             online / offline
Dependency state    neither installed / .NET only / WebView2 only / both installed
Lifecycle           fresh install / reinstall / upgrade / failed or interrupted
                    install / recovery / uninstall
```

The matrix is not the Cartesian product. §5.2 is a compact covering matrix of
meaningful combinations, shaped by three facts: the two WebView2-absent
dependency states are observable only on the Server 2019 baseline, because
Windows 11 ships WebView2 inbox and Windows 10 22H2 has held it since its
September 2026 servicing — the lab's baselines follow current servicing
rather than being pinned to preserve a dependency state
(`TigerWinLab-Requirements.md` §3); `pl-PL` is exercised on Windows 11 only
(§5.1); and the lab runs every row in series on one lease, so a row that
needs a dependency present prepares it with a plain job and continues
without a second baseline restore.

Windows 10 **22H2** is the concrete build the Windows 10 rows run on. The
supported baseline is Windows 10 1809 and later as an API baseline
(`TigerSetup-Design.md` §10.1): the Windows Server 2019 rows run on build
17763, the same code base as Windows 10 1809, and together with the 22H2 rows
that is the platform evidence TigerSetup claims. No row runs on a Windows 10
1809 client, and no result claims to.

### 5.1 Language coverage is scoped by platform

`pl-PL` is a required TigerSetup installer language, not an optional extra.
What is scoped is where it is exercised:

```text
Windows 11 x64        primary platform → full en-US + pl-PL validation (§8)
Windows 10 22H2 x64   compatibility validation → en-US only
Windows Server 2019   compatibility validation → en-US only
```

Localization is a property of the installer, not of the Windows build it runs
on, so repeating the Polish UI matrix on the compatibility platforms buys
coverage the primary platform already provides. The compatibility rows exist
to prove the engine and installer behave on those Windows versions.

### 5.2 The covering matrix

The rows below are the acceptance standard's concrete form: the TigerMarkView
replacement passes when every row passes on the exact installer bytes that
will be published. Each row is one TigerWinLab invocation or a chain of them
on one baseline without a reset, driven from this repository by
`lab/Invoke-MatrixRows.ps1` as the lab's consumer contract requires
(`TigerWinLab-Requirements.md` §2). The expectations of a row — install root,
registration key, shortcuts, PATH entries, options, dependencies — are read
from the installer itself through `tiger-setup inspect --json`; what only the
product knows — smoke commands, the files a row names, the settings file that
must survive, the runtimes a "prepared" row installs first, the legacy
installer's switches, the WinGet identity — comes from the package's own
`lab-matrix.json`. A second product needs those two files, not a second
driver.

*Dependency state* names what the row starts from: **clean** is the freshly
restored baseline, and **prepared** means a plain job installed the named
runtime first. What the clean state contains is a fact about the baseline,
not about the row, and the four dependency states of §6 are therefore
provided by different baselines:

```text
Dependency state   Provided by                    As          Rows
neither            TigerWinLab-Server2019-Clean   clean       S1, S2, S3
.NET only          TigerWinLab-Server2019-Clean   prepared    S4, S5, S6
WebView2 only      TigerWinLab-Win11-Clean        clean       M1, M4, M7, M10, M11
                   TigerWinLab-Win10-Clean        clean       W1, W3, W4
both               TigerWinLab-Win11-Clean        prepared    M2, M3, M5a–c, M6, M8, M9, M12–M17
                   TigerWinLab-Win10-Clean        prepared    W2, W5
```

The .NET Desktop Runtime is absent on every clean baseline. WebView2 is
present on the clean Windows 11 baseline, where Windows ships it inbox, and
on the clean Windows 10 22H2 baseline, where the September 2026 servicing
delivers it and the lab's baseline maintenance keeps servicing current; it
is absent on Server 2019, which Windows does not give the runtime to. So
Server 2019 is authoritative for both WebView2-absent states — the
acquisition its rows prove is the product's and not something the fixture
manufactured by removing a runtime — and the two client baselines prove the
states the product's users actually start from. Neither the Windows 10 image
nor its servicing level is held back to recreate the absent state, and no
row assumes it. A row's premise is checked, not assumed: every lab step
reports the runtimes it found before its payload ran, and the driver fails
any row whose scenario did not start from the state the row declares
(`premise/dependency state`), because a run that had nothing to acquire is
no evidence of acquisition. Rows marked *interactive* run the installer
scenario's wizard phases with a screenshot per page.

**`TigerWinLab-Win11-Clean` — the primary platform**

| Row | Scope · account | Language · scale | Network | Dependency state | What the row runs | Requirements covered |
|---|---|---|---|---|---|---|
| M1 | machine · administrator | en-US | online | clean (WebView2 only) | silent: install (acquires .NET) → verify → smoke → reinstall → upgrade from the previous version → verify → uninstall → verify absence; .NET still present; the machine PATH entry is present after install, exactly one after reinstall and upgrade, absent after uninstall | §6 machine scope, silent lifecycle, smoke, ARP, PATH, reinstall, upgrade, WebView2-only, shared dependency preserved on uninstall |
| M2 | user · `LabUser` | en-US | online | prepared .NET (both) | silent lifecycle as M1 in user scope; `HKCU` registration and PATH; no elevation prompt | §6 per-user, PATH, both present, acceptable already-installed versions |
| M3 | machine · administrator | en-US | offline | prepared .NET (both) | silent install → verify → uninstall with no connectivity | §7 Scenario B |
| M4 | machine · administrator | en-US | offline | clean (.NET absent) | silent install fails with the dependency-unacquirable code; no install root, registration or state remains | §7 Scenario C on Windows 11 |
| M5a | machine · administrator | en-US | online | prepared .NET | recovery scenario: `powerOff` during install → recovery → verify | §6 interrupted install and recovery |
| M5b | machine · administrator | en-US | online | prepared .NET, previous version installed | recovery scenario: `powerOff` during upgrade → recovery → `inspect` reports exactly the old or the new version and `verify` passes for it | §6 interrupted upgrade, never a hybrid |
| M5c | machine · administrator | en-US | online | as M5b | recovery scenario: `reboot` during upgrade → same assertions | §6 reboot scenarios required by the transaction model |
| M6 | machine · administrator | en-US | online | prepared .NET, previous version installed and running | the application is launched on the signed-in user's desktop and the silent upgrade runs elevated on that same desktop, Restart Manager shutdown and restart evidence, verify | §6 upgrade while the application is running |
| M7 | machine · administrator | en-US · 100 % | online | clean (.NET absent) | interactive install — machine scope chosen on the scope page, the PATH option turned off, dependency progress — → verify (no PATH entry) → interactive upgrade → interactive uninstall | §8 `en-US` @ 100 %, scope page, PATH option, dependency progress page, upgrade and uninstall UI |
| M8 | user · `LabUserPL` | pl-PL · 150 % | online | prepared .NET | interactive install → verify → interactive uninstall as the standard user | §8 `pl-PL` @ 150 %; the user-scope wizard |
| M9 | user · `LabUser` | en-US · 200 % | online | prepared .NET | interactive install → verify → interactive uninstall | §8 `en-US` @ 200 % |
| M10 | machine · administrator (`pl-PL` account) | pl-PL · 100 % | online | clean (.NET absent) | interactive install — scope page and PATH option (left on) in Polish, dependency progress in Polish — → verify → interactive uninstall | §8 `pl-PL` @ 100 %, scope page, PATH option, dependency dialogs and messages localized |
| M11 | machine · administrator | en-US | online | clean (.NET absent) | two silent runs with fault injection: the dependency installer made to fail → clean failure, nothing installed; then the product transaction made to fail after .NET was acquired → rolled back, .NET remains | §6 dependency installation failure; application failure after a prerequisite was installed; shared dependency preserved on rollback |
| M12 | user · `LabUser` | en-US | online | prepared .NET | plain jobs: install → modify an owned file → seed the application's settings file outside the install root → uninstall → the modified owned file is preserved and reported as `file_modified_preserved` (`TigerSetup-Design.md` §5.6), the settings survive, everything else owned is gone | §6 modified owned files; settings preservation |
| M13 | machine · administrator | en-US | online | prepared .NET | WinGet scenario with the manifest set of the installer under validation | `TigerSetup-Design.md` §8.2; the release gate |
| M14 | machine · administrator | en-US · 125 % | online | prepared .NET | interactive install → verify → interactive uninstall | §8.1 and `TigerSetup-Design.md` §11.4: the 125 % scale |
| M15 | machine · administrator | en-US | online | prepared .NET | plain jobs seed the machine PATH with the two PATH regression vectors — a pre-existing lookalike of the install root's entry (`<root>\`), and an entry followed by an empty segment (`…;;`) — then silent install → inspect PATH → reinstall → inspect → uninstall → inspect: the lookalike is neither duplicated, claimed nor removed, the empty segment survives, exactly one TigerSetup entry exists after reinstall and none after uninstall, the value type is `REG_EXPAND_SZ` | §6 PATH behaviour; `TigerSetup-Design.md` §5.6 PATH ownership |
| M16 | machine · administrator | en-US | online | prepared .NET, the Inno 0.8.x installer installed | plain jobs: install the legacy Inno version → seed the settings file → silent TigerSetup install → the legacy registration is gone, the log records the legacy uninstall, exactly one registration and one PATH entry remain, `verify` passes, the settings survive | `TigerSetup-Design.md` §5.12 uninstall-first migration |
| M17 | machine · administrator | en-US | online | prepared .NET | plain jobs: a standard user creates `%ProgramData%\TigerSetup\<ProductId>` first → silent machine-scope install → the state directory is owned by Administrators and grants the standard user read and execute only, the run reports `state_directory_ownership_claimed`, and that user can neither write into the directory nor replace `uninstall.exe` | `TigerSetup-Design.md` §5.11: an unelevated user must not be able to tamper with what an elevated uninstall later trusts |

**`TigerWinLab-Win10-Clean` — compatibility, `en-US` only, WebView2 present
(22H2 at its current servicing)**

| Row | Scope · account | Scale | Network | Dependency state | What the row runs | Requirements covered |
|---|---|---|---|---|---|---|
| W1 | machine · administrator | — | online | clean (WebView2 only) | silent lifecycle as M1: .NET acquired; reinstall and upgrade; both runtimes present after uninstall | §6 WebView2 only, silent lifecycle, reinstall, upgrade and shared dependency preserved on Windows 10; the engine and loader on Windows 10 |
| W2 | user · `LabUser` | — | online | prepared .NET (both) | silent lifecycle in user scope with nothing to acquire; `HKCU` registration and PATH | §6 both present; per-user on Windows 10 |
| W3 | machine · administrator | — | offline | clean (.NET absent) | silent install fails cleanly with the dependency-unacquirable code; no install root, registration or state remains | §7 Scenario C on Windows 10 |
| W4 | machine · administrator | 100 % | online | clean (.NET absent) | interactive install with .NET dependency progress → verify → interactive uninstall | §8 compatibility UI and dependency progress; §7 Scenario D on Windows 10 |
| W5 | machine · administrator | — | offline | prepared .NET (both) | silent install → verify → uninstall with no connectivity | §7 Scenario B on Windows 10 |

**`TigerWinLab-Server2019-Clean` — compatibility, `en-US` only, the one
baseline without WebView2**

| Row | Scope · account | Scale | Network | Dependency state | What the row runs | Requirements covered |
|---|---|---|---|---|---|---|
| S1 | machine · administrator | — | online | clean (neither) | silent lifecycle as M1 from neither: both runtimes acquired and both preserved after uninstall | §7 Scenario A; §6 neither installed; engine on Server |
| S2 | machine · administrator | — | offline | clean (neither) | silent install fails cleanly | §7 Scenario C with nothing present |
| S3 | machine · administrator | 100 % | online | clean (neither) | interactive install with both dependencies acquired → verify → interactive uninstall | §8 compatibility UI with dependency progress for both runtimes; §7 Scenario D; `TigerSetup-Design.md` §11.3 |
| S4 | machine · administrator | — | offline | prepared .NET (only) | silent install fails cleanly: WebView2 unacquirable; nothing remains | §7 Scenario C with WebView2 the missing dependency |
| S5 | user · `LabUser` | — | online | prepared .NET (only) | silent lifecycle in user scope; WebView2 acquired per the unattended dependency policy | §6 .NET only; per-user acquisition of WebView2 |
| S6 | user · `LabUser` | 100 % | online | prepared .NET (only) | the wizard driven page by page as the standard user acquires WebView2 without any elevation prompt, with a capture of every page including dependency progress; `inspect` afterwards reports WebView2 present | `TigerSetup-Design.md` §7.8 acquisition without elevation in an interactive user-scope run; §8 dependency progress |

Coverage: every §6 scenario, every §7 scenario on at least one baseline where
it is observable, the four §8 language × scale combinations on Windows 11
plus the 125 % scale `TigerSetup-Design.md` §11.4 requires, `en-US` @ 100 %
on both compatibility baselines, the scope page and the PATH option in both
languages, all four dependency states — each on a baseline that provides it
(the table above) — both identities, both scopes, both network states, PATH
at both scopes, the migration from the installer being replaced, and every
lifecycle step. The identity × scope pairing is
deliberate: user scope always runs as the standard user, machine scope as the
administrator, because that is who runs each in practice.

**The elevation rows** (`lab/Invoke-ElevationRows.ps1`) are the acceptance
for the privilege transition itself (§2), and they run against the
self-hosted TigerSetup installer — the artifact under release — rather than
the synthetic package, because the transition is a property of the shipped
bytes. `complete-uac` and `complete-uac-admin` are one wizard end to end:
started unelevated, all users chosen, the shield on Next, the genuine prompt
answered on the secure desktop through the lab — the credential prompt a
standard user gets, and the Yes/No consent prompt an administrator gets — the
elevated child verified as this installer with the expected authority, its
wizard driven to completion, the parent stepping aside and staying
responsive, the child's outcome coming back through the parent's exit code
and document, and the machine-scope state verified. `shield-refuse` refuses
the same genuine prompt and asserts the parent is usable and nothing was
installed; `complete-user` proves per-user needs no prompt;
`complete-elevated` proves an already-elevated wizard shows no shield and
raises no prompt. No half of this is accepted as evidence for the whole, and
no security setting is changed to make it answerable: the lab types on the
VM's console as a person at the console would
(`TigerWinLab-Requirements.md` §4).

**The launch rows** (`lab/Invoke-LaunchRows.ps1`) are the acceptance for
launch after install (`TigerSetup-Design.md` §11.7), run against the
synthetic `TigerSetupTestLaunch` package (`packages/test-launch/README.md`),
whose program writes down how it was started. They ride on the elevation
rows' paths, each from the baseline:

| Row | Path | Expected |
|---|---|---|
| `launch-user` | unelevated wizard, for me only | started with the wizard's own token (parent: the wizard) |
| `launch-unchecked` | the same, the offer cleared by the person | nothing started; the log says `launch_declined` |
| `launch-uac` | standard user, for all users, the genuine credential prompt | the elevated child — another account — has the shell start it (parent: `explorer.exe`); the handed-back outcome carries the launch |
| `launch-uac-admin` | an administrator's desktop, the Yes/No consent prompt | the same, from the same account elevated |
| `launch-elevated` | an installer started elevated over the standard user's desktop | started by the shell as the standard user |
| `launch-no-shell` | the same, with the desktop's shell ended before Finish | nothing started, the person told, the installation standing — no fallback to the elevated token |

Every started program is checked for an unelevated token (not elevated, no
enabled Administrators group, integrity below high), the desktop's own
account, exactly the declared arguments, the declared working directory under
the real install root, the parent that says how it was started, its window
reaching the foreground as the program itself observed it, and the run's log
line. A real consumer closes the loop: TigerKeyring's installer acceptance
runs an interactive install whose Finish starts its tray agent, unelevated,
from the wizard, and then verifies and uninstalls it.

**M6 is where the identity × scope pairing has to be literal.** The Restart
Manager lists a holder from its open file handles, which crosses Windows
sessions, but it closes a GUI application by messaging its windows, which
does not: an application started from a job's session 0 has no desktop to be
messaged on, so it is asked, never answers, and the run stops with
`package_in_use` having changed nothing. The row therefore runs the
application as the signed-in user and the elevated upgrade beside it on the
same desktop — where a person runs both — and asserts that is where each of
them ran, that the application had a window at all, and what the Restart
Manager decided it could ask of it. A session-0 `package_in_use` outcome is
not a pass for this row.

**Checkpoint subset.** `-Rows checkpoint` runs M1, M2, M5a and W1 — about
four rows plus restores — as the smoke run between changes. The whole matrix
runs before a release. The subset is a smoke test, never a
substitute for the matrix.

**The rows are TigerSetup's, whichever lab entry point carries them.** A
packaged lab scenario is used where its shape already fits — the installer,
recovery and WinGet scenarios do — and everything else composes the lab's
generic job with a payload of TigerSetup's own. Which of the two a row uses is
an implementation detail of the driver; what the row asserts is TigerSetup's,
always. A row is never reshaped to fit a scenario's schema, and a scenario's
schema is never a reason to leave a row's stated intent unexercised.

Two policies the rows encode: M1, M7, M10, M11, W1, W4, S1, S3, S5 and S6
rely on the installer acquiring missing dependencies unattended when online,
which is the default (`TigerSetup-Design.md` §7.8); and M12 passes when the modified
owned file is still present after uninstall, the uninstall outcome reports it
as `file_modified_preserved` — `verify`, which only observes, calls the same
file `file_modified` — its directory remains, and every other owned resource
is gone (`TigerSetup-Design.md` §5.6).

### 5.3 The consolidated feature rows

The optional-resource model — boolean and choice options, option-gated
component files, environment variables, file associations, URL protocols,
`App Paths`, classic context-menu verbs, the Startup, Send To and URL
shortcuts with working directory and AppUserModelID, firewall rules,
embedded prerequisites and custom lifecycle actions (`TigerSetup-Design.md`
§4, §5.5, §5.6, §5.14, §7.7) — is accepted together, on the synthetic
`TigerSetupTestApp` package (`packages/test-app`), whose manifest declares
one of each gated by the option the acceptance names: a PATH mode of `none`,
`command` or `tools`, and the check boxes for the desktop, Startup and Send
To shortcuts, the association, the protocol, the context menu, the
environment variable, the firewall rule and the `extras` component, with an
embedded prerequisite that is a controlled fixture
(`TigerSetupTestPrereq.exe`) rather than anything from the Internet, and
five custom actions run by a second controlled fixture
(`TigerSetupTestAction.exe`), a PowerShell script and a batch script: an
option-gated pre-install marker, a post-install cache (on repair too), an
option-gated post-install failure, a pre-uninstall script that clears the
cache, and a post-uninstall marker written outside the removed install root.
`lab/Invoke-FeatureRows.ps1` drives the rows.

**Acceptance validates real Windows behaviour, not database rows.** After
every step a row reads the machine as Windows reads it — shortcuts through the
shell (the stored target, working directory, AppUserModelID; the URL of an
Internet shortcut), firewall rules through the firewall service, `App Paths` through a
real `ShellExecute` of the bare name, the environment and PATH from the
environment block, the association, scheme, capability and verb keys from the
registry, the extension's default and `UserChoice` untouched — beside the
engine's own `verify --json` and `inspect --json`, whose recorded option values
must come back with their types. The `ShellExecute` probe runs in the session
whose registry the installer wrote, and only where Windows consults it: an
elevated process ignores `HKCU\…\App Paths` — a per-user hive must not
redirect an administrator — so the per-user registration is proved from the
unelevated `standard-user` row and the machine one from the `machine` row; the
elevated per-user rows prove the key alone.

| Row | Baseline · account · scope | What the row runs | Requirements covered |
|---|---|---|---|
| `lifecycle-user` | Win11 · `LabAdmin` (elevated, session 0) · user | a pre-existing environment variable is seeded; fresh install with the preflight on (the pre-install marker and record, the post-install cache for 1.0.0, the stored uninstall programs) → upgrade with no explicit choices (every choice remembered, the prerequisite detected and not extracted again, the preflight run again, the cache rebuilt for 1.1.0) → reinstall changing several choices (the component and the tools PATH mode appear, the Send To link appears, the Startup link, the association and the firewall rule disappear, everything else stays) → an upgrade that fails before its commit (the previous choices and resources remain) → a reinstall whose post-install action fails on purpose (`action_failed`, rolled back, the resources and choices of the committed run intact, the failing program's own marker left where it wrote it, `action_not_reverted`) → the same change committed → a reinstall that converges (`already_installed`, no action run) → repair after the environment variable, the scheme command, the documentation shortcut, the firewall rule and the cache were destroyed (`verify` names each owned loss, repair restores each and rebuilds the cache because that action opted into repair) → uninstall (every owned resource gone, the pre-existing variable back, the prerequisite kept, the pre-uninstall script run first and the cache cleared, the post-uninstall program run last from the state directory with the install root already gone) | the lifecycle of §5.3's introduction; option precedence and rollback; conservative removal; repair; the embedded prerequisite run once and detected after; every custom-action phase, `run_on` and the failure semantics of §5.14 |
| `standard-user` | Win11 · `LabUser` (standard, desktop) · user | install with the defaults as the signed-in standard user → upgrade → uninstall; the firewall rule is reported skipped (`firewall_rule_skipped_unelevated`) and never created, everything else lives in that user's hive and folders; the post-install and uninstall actions run as that user | a per-user install without an administrator |
| `machine` | Win11 · `LabAdmin` · machine | install with the defaults and Send To selected → upgrade → uninstall; `shortcut_location_unavailable` for Send To, the Startup link in the shared Startup folder, the integrations in `HKLM`, the environment variable and PATH in the machine block, the firewall rule created and removed; the actions run elevated, the uninstall programs stored under `%ProgramData%` | machine scope for every new resource kind |
| `win10` | Win10 · `LabAdmin` · user | install → upgrade → uninstall with the same reads, the actions included | the minimum functional coverage on the compatibility platform |

The Windows 11 light and dark captures of §8 include the options pages as
they are for this package — a choice option's radio buttons and two pages of
check boxes — in both themes.

**The explicit registry location row** (`lab/Invoke-ExplicitRegistryRow.ps1`)
proves, on the real machine hive of the Windows 11 baseline, what the
process-level tests prove against relocated roots: a `[[registry]]` value
outside `Software` (`TigerSetup-Design.md` §5.6). It runs on its own
machine-only fixture, `packages/test-explicit-registry`, which pairs
`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled` — a
value every clean baseline holds as `0` under a key Windows owns — with a
marker under a key chain that does not exist, and reads the registry as
Windows holds it after every step: the baseline, a quiet machine-scope
install (`verify` clean, `inspect` owning the values at their explicit
paths), a reconciling reinstall that converges, an administrator's change
preserved and reported by the next reinstall and put back by a repair, and
an uninstall that leaves the pre-existing value as the baseline held it, the
created value and keys gone, and the Windows keys above them in place.

### 5.4 TigerSetup's own installer: presence, help and the WinGet path

TigerSetup's own installer (`TigerSetup-Design.md` §8.4) is accepted on its
exact release bytes by `lab/Invoke-SelfInstallerRows.ps1`. The quiet install,
the installed `tiger-setup --version`, `verify` and a clean uninstall are
checked in each scope. So is what a person finds afterwards, because a
command-line tool that installs silently is easy to install and then lose.

**Moderator usability is an acceptance requirement.** Someone installing
TigerSetup on a clean machine — a WinGet moderator is the model — must be able,
within a minute or two and without browsing Program Files or reading source,
to see what TigerSetup is, where it went, how to start it, how to get help,
and what to run first. The obvious path is **Start → TigerSetup → TigerSetup
Shell**, which shows the brief landing help (`tiger-setup --help-brief`)
immediately, and **TigerSetup Help**, which opens the PDF guide directly. The
rows prove it on
the interactive desktop as the signed-in standard user, reading the terminal's
text rather than trusting a screenshot:

| Row | What it proves |
|---|---|
| `user-nopath`, `machine-nopath` | Install with the PATH option off: the Start Menu folder holds exactly TigerSetup Shell, TigerSetup Help (the PDF) and TigerSetup Help (Markdown), each opening what it declares, and only the shell shows TigerSetup's icon — each help shows its document's. TigerSetup Shell is `%ComSpec%` started by the installed `tiger-setup.exe`; it shows the brief help (`--help-brief`) by itself, not the complete reference, fitting the window it opens in, with the PATH status coloured. It answers `tiger-setup --version`, resolves `tiger-setup` to this installation, answers `--help` with the complete reference naming no installed location and `build --help` with that command's, refuses `tiger-setup help`, starts in the user's profile, closes on `exit`, and changes neither persistent PATH. TigerSetup Help opens the PDF directly; the Markdown opens too (through Windows' app picker on a clean machine, recorded as a WARN and accepted for the secondary form). The installed help is the shipped bytes; the shipped Markdown is CRLF-only and exactly `docs/TigerSetup-Help.md` at the source commit as Git checks it out; and the shipped PDF names it as its source. The uninstall leaves no install root, state, registration, Start Menu folder or shortcut. |
| `upgrade` | A previous release is upgraded to this one: from a release without the folder the upgrade adds it and the help; from one with another shortcut layout it renames, retargets and retires links until the folder holds exactly this release's. A same-version rerun keeps them, and the PATH choice recorded at the first install holds throughout. The uninstall removes everything. |
| `winget-user`, `winget-machine` | TigerWinLab's WinGet scenario on each installer entry of the finished manifest set: `winget validate`, the hash-mismatch probe, `winget install --manifest`, the installation, the declared command, the uninstall and the cleanup. |
| `moderator` | As the signed-in standard user, whose WinGet sources open: `winget install --manifest`, `winget list` correlating the package and version, Start → type "TigerSetup Shell" → Enter with the shell checks above, both help forms, `winget uninstall`, and nothing left. |

`winget validate` accepting the manifest set is necessary but not sufficient.
The manifest must also follow the current `microsoft/winget-pkgs` authoring
guidance, which the generator encodes (`TigerSetup-Design.md` §8.2), and the
installer URL must be public, stable and version-specific. The lab cannot prove
that; the release can.

---

## 6. Required lifecycle scenarios

The matrix runs `clean VM → install → validate → upgrade → validate →
uninstall → validate absence` across at least:

- per-user installation;
- machine-wide installation;
- PATH behaviour;
- silent install, silent upgrade, silent uninstall;
- application launch / smoke test;
- ARP registration;
- neither prerequisite installed;
- only .NET Desktop Runtime installed;
- only WebView2 installed;
- both prerequisites already installed;
- acceptable already-installed dependency versions;
- dependency installation failure;
- application installation failure after a prerequisite was installed;
- preservation of shared dependencies on rollback and uninstall;
- reinstall;
- modified owned files;
- interrupted or crashed install, and recovery;
- interrupted or crashed upgrade, whose outcome is exactly the old version or
  exactly the new one and never a hybrid of the two;
- upgrade while the application is running, with shutdown and restart handled
  through Restart Manager (`TigerSetup-Design.md` §5.10);
- reboot/restart scenarios required by the transaction model.

---

## 7. Required offline and self-containment scenarios

### Scenario A — online, no dependencies

```text
fresh Windows / Internet available / .NET absent / WebView2 absent
        ↓
TigerSetup installs required dependencies
        ↓
TigerMarkView installs → launches → PASS
```

### Scenario B — offline, dependencies present

```text
fresh Windows / NO Internet / .NET present / WebView2 present
        ↓
TigerMarkView installs → launches → PASS
```

### Scenario C — offline, dependency missing

```text
fresh Windows / NO Internet / required dependency missing
        ↓
TigerSetup detects the missing requirement
        ↓
dependency cannot be acquired
        ↓
clean deterministic failure, no broken or partial TigerMarkView installation
        ↓
PASS
```

### Scenario D — plain Windows, no TigerSetup prerequisites

```text
fresh supported Windows / no TigerSetup runtime or tooling installed
        ↓
Setup.exe launches → interactive UI works → unattended mode works → PASS
```

---

## 8. Required UI validation

A strong unattended path does not remove the requirement to validate the
interactive installer: the UI is a supported product surface.

The representative matrix, run on the primary Windows 11 platform (§5.1) by
`lab/Invoke-UiCaptureRows.ps1`:

```text
en-US @ 100%   light      en-US @ 100%   dark
en-US @ 125%   light      en-US @ 150%   dark
en-US @ 200%   dark
pl-PL @ 100%   light      pl-PL @ 150%   dark
```

Three dimensions, not two: language, scale and **Windows theme**. Light and
dark are both a supported presentation of the product
(`TigerSetup-Design.md` §11.5), and an installer that follows the theme has to
be seen doing it — a wizard that quietly stayed light would pass a matrix that
never asked. Every combination covers both themes at least once at a
non-default scale, so a theme defect and a scaling defect cannot hide each
other.

Coverage includes the initial installer page, install progress, dependency
progress and status, errors, completion, the uninstall UI, the upgrade UI,
the scope page, the PATH option, and — on the synthetic package — the options
pages with a choice option's radio buttons and a second page of check boxes.

The language, scale and theme a capture was taken in are the lab's own
measurement of the interactive session, never what the run asked for, so a
capture taken in the wrong theme cannot be presented as evidence for the one
requested. That is the same rule §5.2's rows apply to language and scale.

On the Windows 10 22H2 and Windows Server 2019 compatibility platforms, UI
validation runs in `en-US` only.

### 8.1 DPI checks

Clipped text, overlapping controls, incorrectly scaled icons, layout breakage,
unreadable text, badly sized windows, incorrect scaling behaviour, keyboard
usability, and accessibility regressions where practical.

The wizard captures run against the **synthetic package**, not TigerMarkView.
The wizard is one implementation whichever product it installs, and the
synthetic package has no dependency to acquire, so a combination costs seconds
and reaches every page including the completion one — where the outcome glyph
is. TigerMarkView's own interactive pages, including dependency progress and
the elevation prompt, are what the §5.2 interactive rows cover.

### 8.1.1 Theme checks

Every surface that carries meaning has to survive the switch: page background
and text, the title bar, control borders and ticks, disabled states, the
outcome glyph on the completion page, and the secondary branding. What is
being looked for is a piece of the window that did not move with the rest —
white text on a white background, a light caption over a dark page, an icon
with its own colours baked in.

**A capture with something else on top of it is not evidence about the
wizard.** The lab photographs the screen as it is composited, which is what
makes a menu or a tooltip part of the picture rather than something a redrawn
window would omit — and it also means anything left open over the wizard
obscures exactly the surfaces this section is about. A row's mechanical checks
cannot see that: page count, scale and theme all still pass. So a capture that
is obscured is read as *no* evidence for the pages it covers, and the row is
re-run rather than reported on.

That guard is correct and it is expensive: by the time it fires the row has
already been paid for. So the desktop a row is given is established before its
wizard starts: the lab inspects its own interactive desktop and clears the
transient shell state it finds there, and a row it cannot hand a clear
workspace to stops at that precondition rather than photographing a wizard
through something the lab may not touch — `capture.desktop.precondition` is
that record. An obstruction that still appears once a row is running ends the
run at the page it was found on, because every page after it would be
photographed through the same thing. **This check is the last guard, not the
first one.**

### 8.2 Localization checks

Localization is not implemented merely because translated strings exist. Both
`en-US` and `pl-PL` are part of UI validation, covering correct language
selection, correct fallback behaviour, text clipping, control resizing, dialog
and window layout, error messages, progress messages, dependency dialogs and
messages, and both install and uninstall UI. The captures are reviewed by eye;
a visual-regression strategy beyond them is an open question
(`TigerSetup-Design.md` §17).

---

## 9. Known-clean Windows fixture

Validation requires trusted, deliberately minimal Windows environments that do
not accidentally contain dependencies which would invalidate tests. Potential
accidental contaminants:

```text
.NET Desktop Runtime versions      WebView2
Visual C++ redistributables        PowerShell 7
WinGet updates/packages            Rust tooling
Visual Studio / build tooling      Tiger development tools
previous TigerSetup state
```

Without this, a broken self-contained installer can appear correct — a
validation environment that lies is worse than no validation.

TigerWinLab's three `*-Clean` baselines are that fixture, and the contaminant
list above is not assumed: every scenario result reports each runtime as
present or absent with its version, so a row that depended on absence says so
for itself (`TigerWinLab-Requirements.md` §4). The one contaminant that is
not removed is WebView2 where Windows itself delivers it — inbox on Windows
11, through servicing on Windows 10 22H2 — because a fixture it was removed
from would no longer represent the machines the product installs onto; §5
and §5.2 record what that costs the dependency matrix and which baseline
provides each state instead. The final run confirms the
environment assumptions it relies on — Windows version, privilege, network
state, dependency state, UI language, scale, theme — from the lab's
measurement, never from the request.

---

## 10. TigerWinLab is an external project

> **TigerWinLab must not be modified as part of the TigerSetup project.**

TigerSetup consumes TigerWinLab as a generic provider: it resolves the lab
through the TigerAiCore configuration, composes the lab's public entry points
with payloads and assertions of its own, and keeps nothing consumer-specific
in the lab (`TigerWinLab-Requirements.md` §2). It may inspect the lab's
capabilities, document what its acceptance needs as required observable
behaviour rather than prescribed implementation, and treat a missing lab
capability as an external blocker; it must not cross the boundary and
implement lab features here, and a provider defect is reported to its own
repository and waited on, never worked around here to turn a row green. Every
agent working here must know which project owns a required change.

`TigerWinLab-Requirements.md` records what the acceptance depends on: the
consumer commitments, the observable lab behaviour each row relies on, and
the constraints that shape the matrix.

---

## 11. Open questions

- a visual-regression strategy beyond the per-page captures the lab collects,
  which are reviewed by eye.
- the wizard's dependency-error page — an interactive install that cannot
  acquire a missing runtime — is not captured by any automated row: the
  §5.2 offline rows are silent, and the §8 captures run the synthetic
  package, which has no dependency to fail on. A capture of that page,
  probably a §8 row with the synthetic package's fault injection, is open.
