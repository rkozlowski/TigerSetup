# TigerWinLab — What TigerSetup's acceptance depends on

TigerWinLab is the Windows lab that runs TigerSetup's acceptance
(`TigerSetup-Validation.md` §5.2 and §8). It is a separate project: this
document is TigerSetup's side of the contract — how TigerSetup consumes the
lab, the observable lab behaviour each row relies on, and the lab facts that
shape the matrix — written as **required observable behaviour, not prescribed
implementation**. Nothing here is implemented in TigerWinLab from this
repository, and a lab that meets a requirement differently from the shape
described is meeting it.

The boundary: TigerHyperLab is the generic Hyper-V and VM provider;
TigerWinLab is the generic Windows execution, interaction and evidence
capability; TigerSetup owns its orchestration, semantics and acceptance
assertions. TigerSetup consumes TigerWinLab only, never TigerHyperLab
directly. A packaged lab scenario is a convenience to use where it fits, not
a schema TigerSetup has to fit: where one does not fit, the lab's generic job
with a payload of TigerSetup's own is the supported route and needs no lab
change. The drivers themselves are documented in `lab/README.md`.

---

## 1. The question this document answers

> Can TigerWinLab support TigerSetup's automated, recovery, offline, UI and
> elevation acceptance?

Yes. Every requirement in §4 is met by the lab and exercised by the matrix,
and the constraints that remain (§3) are matrix-design inputs, not blockers.
Should a row ever need behaviour the lab does not provide, the missing
capability is documented here as observable behaviour, implemented in
TigerWinLab under its own workflow, and re-evaluated from here with
TigerSetup's own generated specifications — never worked around in this
repository to turn a row green.

---

## 2. What TigerSetup commits to as a consumer

The lab's consumer contract (its `README.md`, *Consuming TigerWinLab from
another repository*) shapes how TigerSetup uses it. TigerSetup:

- resolves the lab only by running TigerAiCore's
  `tools/Resolve-TigerAiCoreResource.ps1 -Lab TigerWinLab` against the machine
  configuration, never by parsing that configuration itself, guessing a
  sibling checkout or reading an environment variable of TigerSetup's own;
- generates its own scenario specifications inside the TigerSetup repository
  and never edits a specification kept in TigerWinLab;
- puts nothing consumer-specific into the lab: no TigerSetup folders, scripts,
  identities or configuration values;
- runs every entry point as a child process, passes `-ResultPath`, interprets
  exit codes `0` OK / `1` failed / `2` BUSY / `3` TIMEOUT, treats a missing or
  unreadable result as failure and `BUSY` as an outcome to retry, and bounds
  the child generously beyond `-TimeoutMinutes`;
- selects the run environment through `-Baseline`, `-NetworkState`,
  `-Language`, `-ScalePercent` and `-Theme`, and asserts the `environment`
  block the result returns rather than assuming the request was honoured;
- carries no copy of the lab's own guest scripts in a payload: a job that
  needs the interactive desktop asks for `-Desktop` (and `-ElevatedDesktop`,
  `-HostConsole` where the row needs them) and uses the session the lab hands
  it;
- keys its acceptance matrix on each check's stable `code`, not on its English
  `name` or `message`;
- serializes all end-to-end work: one lease covers every baseline, so the
  matrix runs in series whatever order TigerSetup picks, and Git worktree
  isolation between TigerSetup workers does not isolate the lab;
- opens one lab session per baseline and ends it in a `finally`, reads the
  per-resource reclaim report, and — because the lab never expires a session
  on its own — closes with `Close-TigerWinLabSession.ps1` any session a
  killed run left behind, as `Get-TigerWinLabSession.ps1` reports it;
- composes the existing scenarios and result shape rather than inventing a
  second result contract, flattening lab checks by prefixing their names.

---

## 3. Lab facts that shape the matrix

- **WebView2 is inbox on Windows 11**, so the *WebView2-absent* half of the
  dependency matrix — "neither installed" and ".NET Desktop Runtime only" —
  is observable only on `TigerWinLab-Win10-Clean` and
  `TigerWinLab-Server2019-Clean`. The Windows 11 rows cover "WebView2 only"
  and "both installed". Removing WebView2 from a Windows 11 image to force the
  state is not done: it would make the fixture unrepresentative of the
  machines the product actually installs onto.
- **The compatibility baselines are `en-US` only** and refuse a `pl-PL`
  request outright rather than substituting; the two Windows 11 baselines
  offer both languages, with an administrator and a standard account per
  language. This matches `TigerSetup-Validation.md` §5.1.
- **The matrix runs in series.** One exclusive lease covers all baselines, a
  second consumer gets `BUSY` rather than a queue, and a matrix's wall-clock
  cost is the sum of its rows plus a baseline restore each. A row that needs
  a dependency in place prepares it with a plain job and runs its scenario
  with `-SkipReset` rather than restoring again; there is no consumer-facing
  intermediate checkpoint.
- **A session holds every VM it has touched**, and the host runs a bounded
  number at a time, so a run spanning three baselines opens one session per
  baseline and ends the one it is leaving. Ending a session and reclaiming its
  VM are separate transitions, and the drivers report a VM the lab did not
  reclaim rather than assuming it.
- **A capture is of the screen as composited.** Menus, tooltips and drop
  shadows are part of the evidence, and so is anything left open over the
  window under capture. The lab establishes a clear desktop before a
  `-Desktop` job's payload starts (§4); TigerSetup's own rule — an obscured
  capture is no evidence for the pages it covers — is
  `TigerSetup-Validation.md` §8.1.1.

---

## 4. Observable behaviour the acceptance depends on

**Installer-technology-neutral scenarios.** `installer.kind` is a label the
result carries and never a gate. The install, reinstall, upgrade and
uninstall command lines, any logging switch, the Add/Remove Programs key to
verify, the uninstall command and any log patterns are supplied by the
specification; nothing is appended that the specification did not name.
Elevation and install mode are established from observable system state —
the privilege of the account, where the files landed, which registry root
holds the registration.

**Per-user scope in the installer scenario.** `"scope": "user"` selects the
pass conditions and the account together: the run executes as the signed-in
standard user, the expected install root, registration hive, `PATH` and
shortcut folders follow, that account's profile and hive are read rather than
the job account's, and no elevation prompt may be raised. The same
specification shape covers both scopes.

**Interrupted-install and restart recovery.** The recovery scenario starts an
installer, interrupts it by `process`, `reboot` or `powerOff`, reconnects to
the guest **with no baseline restore**, and runs the product's recovery in
exactly what the interruption left. The specification names the state
directories to inventory, the registration to look for, the recovery command
and what is expected afterwards. An interruption may be triggered by a
`signal` file the guest payload creates, polled every 100 ms, so the cut
lands on the boundary the engine announces through `--fault-signal` rather
than on a clock. The installer is staged outside any job workspace, so
neither workspace cleanup nor job cancellation reclaims the process the
scenario means to interrupt.

**Offline network state.** `-NetworkState offline` disconnects the guest's
adapter at the hypervisor and reconnects it afterwards, so nothing inside
Windows is reconfigured and nothing the guest does can undo it; the guest
stays fully drivable over VMBus. A requested offline state is not a health
failure; connectivity that survives a request to remove it is a failure.

**Dependency-state preparation and self-reporting.** Neither provisioning
profile installs an application runtime, and every result reports .NET
Desktop Runtime, .NET Runtime, WebView2, the Visual C++ redistributable and
PowerShell 7 as present or absent with the version where present. "Present"
is prepared by a plain job followed by a scenario with `-SkipReset`;
"absent" is a freshly restored clean baseline.

**Windows 10 22H2 and Windows Server 2019 baselines.** The same generated
specification runs against any baseline named by `-Baseline`, and the result
states the caption, build, display version and edition that answered, so a
run against the wrong image cannot pass silently.

**Interactive installation of a wizard the lab knows nothing about.** The
installer scenario carries interactive install, verify, upgrade and uninstall
phases. The window to answer, the controls that advance a page and any choice
to settle first (`preferControls`, matched by label against every control
type, radio buttons included) come from the specification. A machine-scope
wizard is answered by a credential-elevated desktop session and a user-scope
wizard by the signed-in standard user. Every page is captured before it is
answered, the offered controls are recorded, and whether an elevation prompt
appeared is reported.

**`pl-PL` guest UI language, DPI scale and theme.** `-Language`,
`-ScalePercent 100|125|150|200` and `-Theme light|dark` select the
interactive session's language, scale and theme, and the result reports each
as measured — the language from the installed language pack, the scale in
effect, the theme as the `apps` and `system` preferences the lab set plus its
own observation of the session — never as requested.

**Environment self-assertion in results.** Every result carries an
`environment` block: the baseline and checkpoint the run started from, the OS
caption, build, display version, edition and architecture, the account and
privilege the payload ran under, the interactive session's account, display
language, resolution, DPI, scale and theme, the network state and whether it
was the one requested, and the dependency state of the image. The same facts
are health checks with stable codes, so a run in the wrong language, at the
wrong scale, against the wrong image or with connectivity the caller asked to
remove fails rather than passes.

**Shortcut verification for a root-level Start Menu link.** A shortcut is
verified by the link — existence, target and arguments after install, absence
after uninstall — and the containing folder only where the specification opts
in, so a link sitting directly in the shared `Programs` folder is verifiable.
A per-user link is resolved by the account that owns it.

**An expectation list may be empty.** A specification whose
`expected.pathEntries` or any other expectation list is empty reports zero
assertions for it rather than failing the phase: a package installed with its
PATH option switched off expects no PATH entry, and that is the point of the
row.

**A completed scenario records its outcome.** The guest writes its outcome to
a durable record and the host reads it from there rather than from the
pipeline, so a transport failure after the phases ran still yields a result
file. TigerSetup treats a missing or unreadable result as a failing check
either way.

**A phase counts the checks its body produced**, whether the body emitted
them one at a time, returned a helper's collection, or composed the two.

**An evidence desktop established before the expensive work.** Before a
`-Desktop` job's payload starts, the lab inspects the interactive desktop's
evidence workspace, clears the transient shell state it finds there — only
ever a window owned by the process that owns the desktop, so an application's
window is reported and left alone — and, with `-RequireClearDesktop`, refuses
the job when it cannot. What it found is recorded as structured evidence.

**A genuine elevation consent answered on the secure desktop.** A job on the
standard user's desktop, or on an administrator's (`-InteractiveKind
administrator`), raises a real UAC prompt on a real secure desktop and the lab
answers it — `Approve-`, `Deny-` and `Wait-ElevationPrompt` through
`-HostConsole` — without changing UAC, its secure-desktop setting or any
other policy. The lab's keyboard is keyboard hardware to the guest, so the
answer reaches the secure desktop as a person at the console would; the lab
types the administrator's password into a credential prompt itself, so the
secret never reaches the payload; the host's rendering of the display is the
capture of the secure desktop, which the session inside the guest cannot
take. The lab establishes that the prompt it answers is the one the operation
under test raised — `consent.exe` does not name its requester, so a prompt is
attributed by time and exclusivity, and a prior or ambiguous prompt is
refused rather than answered — verifies that it left and the input desktop
returned, and reports what appeared elevated afterwards with owner, integrity
level and command line, so the consumer can assert it is the expected
process. The elevated child is a high-integrity window and is driven through
`-ElevatedDesktop`.

---

## 5. Lab limitations TigerSetup composes around

None of these blocks a row; each is recorded so the driver's shape is
understood rather than rediscovered.

- **No desktop scenario against an installed application.** "Install →
  launch the installed GUI → smoke" is composed from the installer scenario's
  `smoke` and interactive phases plus a plain job.
- **No consumer-facing intermediate checkpoint.** A prepared state is a
  preparation job plus `-SkipReset` (§3).
- **Settings preservation** is a plain job that seeds a file outside the
  install root before uninstall and asserts it survives.
- **The recovery scenario's state inventory reads a registration key name
  unconditionally**, so a specification for a product that registers nothing
  must name one anyway; the lab then reports the registration absent, which
  is the truth.
- **A consumer cannot return a baseline to rest.** A run leaves the baseline
  it used running, and the only consumer-facing entry point restores the
  checkpoint and starts it. The VM holds no lease and the next row resets it;
  it costs one of the host's running-VM slots until something else runs.
