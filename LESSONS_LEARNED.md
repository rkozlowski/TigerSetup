# Lessons Learned

Non-obvious knowledge this project paid for, kept because it prevents a costly
mistake from being repeated. Current-state facts belong in the design
documents and `lab/README.md`; how the project got here belongs in Git.

## Windows writes an unflushed file out on its own within seconds

**Area:** lab recovery rows, fault injection
**Status:** Active

**Symptom:** The first power-off rows for a deliberately unflushed rename
(`skip_flush`) found the file intact after the cut, and recovery correctly
reported nothing to repair — so the rows proved nothing about the zero-fill
detector they exist for.

**Cause:** Windows' lazy writer flushes the dirty pages by itself within a few
seconds. The damaged-file symptom exists only when the cut lands about a second
after the unflushed write; with a 20 s gap the data had already reached the
disk. It reproduced for a 4 MB and a 1 KB file alike, so the file's size is not
what decides it.

**Do not:** read an intact file after a `skip_flush` power-off as proof that
unflushed writes are safe, and do not lengthen the hold at the fault point to
"make sure the cut lands inside the window" — the hold is what gives the data
time to reach the disk.

**Use instead:** interrupt on the boundary rather than on a clock. The engine's
`--fault-signal` creates a file when a fault reaches its boundary and then
holds; the lab's `signal` interruption trigger polls for that file every 100 ms
and cuts the power then, so the cut lands inside the window by construction
rather than by aim. A row whose file is nonetheless intact still reports WARN
("proved nothing") rather than FAIL, because an interruption that damaged
nothing is absence of evidence.

**Prevented by:** `durability.rs::a_fault_signals_the_boundary_before_it_acts`
asserts the announcement precedes the action; `lab/Invoke-RecoveryRows.ps1`
builds every recovery row's interruption from that signal; the local zero-fill
test in `crates/tigersetup-setup/tests/durability.rs` covers the detector
itself.

**Generalization candidate:** none any more — the generic capability this
lesson used to ask for, interrupting at a marker the product writes, exists in
TigerWinLab and is what the rows now use.

## Halves of a privilege transition do not prove the handoff

**Area:** elevation acceptance, lab rows, verification design
**Status:** Active

**Symptom:** the 0.5.1 self-installer hung after a real UAC consent — wizard
responsive, "elevating", never finishing — while every elevation check was
green: the unelevated wizard raised the prompt and stayed responsive, a
refusal recovered, the already-elevated wizard installed machine scope, and a
process test handed an outcome document back across the boundary.

**Cause:** `run_elevated` had been written for hidden `/quiet` dependency
installers and passed `SW_HIDE` to `ShellExecuteEx`; a process's first window
follows the show state it was started with, so the elevated wizard child was
created invisible and waited for a click nobody could give, and the parent
waited for the child. Every half of the acceptance ran on one side of the
consent or the other — the lab could not answer a prompt, and the substitutes
were chosen to be the same *authority* — so no check ever started the child
the way the product starts it.

**Do not:** accept evidence for a privilege transition assembled from a
pre-consent half and a post-elevation half, however faithful each is. A
substitute that shares the authority does not share the launch. And do not
weaken Windows to make the transition testable — no `PromptOnSecureDesktop=0`,
no UAC policy change, no auto-approval.

**Use instead:** one wizard end to end on the artifact under release, with the
genuine prompt answered on the real secure desktop: the lab's host console
types on the VM's console as a person would (`Approve-ElevationPrompt`), and
the row asserts the transition rather than assuming it — the prompt this press
raised, consent.exe gone and the input desktop back, exactly one elevated
process and it is this installer with `--elevated-result`, a visible child
wizard, the parent stepped aside and responsive, and the child's outcome read
from the parent's exit code and printed document. A high-integrity window is
driven through the lab's elevated agent; the standard session cannot click
into it (UIPI). The prompt kind follows the desktop — credential prompt for a
standard user, Yes/No consent prompt with *No* as its default for an
administrator — so both are rows.

**Prevented by:** `lab/Invoke-ElevationRows.ps1` rows `complete-uac` and
`complete-uac-admin` (`TigerSetup-Validation.md` §5.2); the unit test
`elevation.rs::a_wizard_child_is_started_shown_and_a_quiet_one_hidden` pins
the show state to the child's command line.

**Generalization candidate:** TigerWinLab has the mechanism (R17) and its
README carries the Windows half of the lesson; the method — a transition is
proven on one operation, never from its halves — is TigerAiCore material and
must be raised there as its own change.

## A capability recorded as missing may never have been missing

**Area:** consuming a Lab, external blockers
**Status:** Active

**Symptom:** Every machine-scope interactive row failed identically on all
three platforms and in both languages: the run advanced past the wizard's scope
page, installed per-user, and then failed on the install root a machine-scope
run must create. It was recorded as a TigerWinLab requirement — "a wizard
driver must be able to choose a radio button" — and five rows were left failing
as blocked on another project.

**Cause:** the lab had clicked the radio button, and the wizard had taken the
choice. The lab's own `interactive-install.json` for the failing row records
`"prepared": "Install for all users"` on the scope page and, four pages later,
a summary reading `Install mode: For all users` beside a destination still
inside one user's profile. The defect was TigerSetup's: the destination page
was filled once from the scope the run started in and never followed the scope
the person then chose. Two pages of one wizard disagreed, and the row reported
that disagreement where it became visible rather than where it was caused.

**Do not:** conclude that a provider lacks a capability from the shape of a
failure. A whole class of failures looking identical is evidence that one thing
is wrong, not evidence of where it is.

**Use instead:** before recording a requirement against another repository,
open the provider's own evidence artifact for the exact step claimed to have
failed, and read what it says happened. The lab writes one per phase; reading
it costs a minute and it is the difference between a defect fixed here and a
blocker filed there. Where the artifact shows the provider did its part, the
defect is the consumer's.

**Prevented by:** `wizard.rs::the_destination_follows_the_scope_until_it_is_edited`
drives the real window and asserts the destination moves with the scope, and
stops moving once the person has typed their own.

**Generalization candidate:** TigerAiCore — "read the provider's evidence
before filing a provider requirement" is method knowledge about consuming a
Lab, not anything specific to installers.

## A worker pinned to a worktree by instruction still drifts

**Area:** delegation, worktrees
**Status:** Active

**Symptom:** With the Agent tool's worktree isolation unusable on this
machine, workers were told to `cd` into their worktree in every shell command.
One worker resumed after an interruption ran `cargo fmt`, `cargo build` and
`cargo test` three times in the main checkout before noticing; only build
output landed there, but that was luck, not design.

**Cause:** the shell's working directory resets between a worker's tool calls,
and an instruction is not a boundary.

**Do not:** treat an instruction to work in a worktree as isolation, or skip
checking the main checkout after a worker finishes.

**Use instead:** `git status --porcelain` of the main checkout after every
worker report, before integrating; file edits by absolute path (which did hold);
and real isolation when the tooling provides it.

**Prevented by:** the Lead's integration step reads `git status` first;
nothing mechanical stops the worker.

**Generalization candidate:** TigerAiCore — the check belongs to the autonomous
development method, not to this project.

## Read lab evidence from the job that produced it

**Area:** lab driver, evidence handling
**Status:** Active

**Symptom:** A wizard capture read as "English under the Polish account" and
became a finding about language detection, complete with a planned change to
the detection order. The lab's own artifact for that job showed Polish.

**Cause:** the capture runner copied a row's screenshots by searching the whole
lab-artifact tree for a file name that every language × scale combination
shares, and took the first match — another combination's file.

**Do not:** derive a finding from a file copied or renamed by a driver without
checking the job artifact it claims to come from, and do not search an
artifact tree by a name that more than one job produces.

**Use instead:** copy and read from `<OutputRoot>\<jobId>\` — the lab's result
names the job — and record the job id beside every copied artifact.

**Prevented by:** `lab/Invoke-UiCaptureRows.ps1` copies only from the row's
job directory and reports the job id in a check.

**Generalization candidate:** none — the TigerWinLab result already names the
job; the mistake was the consumer's.

## A finished process is not a finished task, and "timed out" is not "stopped"

**Area:** delegation, monitors, completion reporting
**Status:** Active

**Symptom:** A run reported "all subagents finished, all lab and build
processes terminated ... no lab lease held" while the session still showed two
shells running and an active researcher whose status read *Confirming no
lingering probe processes*. Investigated afterwards, the machine held **seven**
orphans and the session held **three** live task records — the youngest already
an hour old, the oldest an hour and three quarters.

**Cause:** one bad verification, and three ways for background work to survive
a completion claim.

- **The check measured the wrong things, twice over.** It was a
  `Get-CimInstance Win32_Process` sweep filtered to `pwsh`, `cargo`, `rustc`,
  `dotnet`, `zopfli-probe`, `tiger-setup`, `tigersetup-setup`. Every orphan was
  a `tail`, a `grep` or a `find`, so the filter excluded all of them — and a
  process sweep of any width cannot see a task whose process has died but whose
  registry entry never reached a terminal state, which is what the two shells
  were.
- **A monitor's timeout does not stop its pipeline.** Three monitors reported
  *"[Monitor timed out — re-arm if needed]"*, and all three left their
  `tail -f … | grep --line-buffered …` running. `tail -f` has no reason to
  exit; the notification is about the watch, not about the processes.
- **A delegated worker's background work is the Lead's.** A researcher had
  started `find / -iname "zopfli-probe"` and
  `until grep -q "^264," <file>; do sleep 15; done`. Both were registered
  against the Lead's session and outlived the worker's own final report.
  Neither was bounded: a `find /` walks a filesystem, and the marker the waiter
  polled for went to a different file once the run it watched was restarted, so
  it could never appear.
- **The worker's self-report was believed.** It stated "no orphaned process
  remains" while holding both.

**Do not:** answer "is my background work finished?" with a process list, and
never with a list filtered by the executable names you happened to think of.
Do not read a monitor's timeout as a termination. Do not let a delegated
worker's account of its own cleanliness stand in for checking it.

**Use instead:** the task registry — `/tasks`, or the task tools — as the
authoritative account, and stop each live entry explicitly. Supplement it with
a sweep keyed on **what the work touches** (the repository path, the session's
scratchpad directory, the Lab, the probe name) rather than on process names, so
that a `tail` or a `find` cannot hide behind a filter. Stop a monitor's
pipeline yourself; give every waiter a bound it cannot miss; and require the
same of a delegated task in its prompt.

**Prevented by:** nothing mechanical — the registry belongs to the harness, not
to this repository, so no check inside TigerSetup can enforce it. The
compensating controls are procedural and deliberately high-salience:
`AGENTS.md` makes reading the registry and sweeping by work-touched the last
step before reporting completion, and `TigerSetup-AI-Approach.md` §3 puts the
constraint on delegated tasks where this project's delegation policy lives.

**Generalization candidate:** TigerAiCore. The rule this violated is already
there and is explicit — *"Polling indefinitely for a string … is not a
termination condition"* describes one of these orphans exactly. What is missing
is the sentence naming **what counts as evidence** that the rule was honoured.
That belongs in TigerAiCore and must be made as its own change, not from here.

## A lab row measures the engine in the installer, not the one in the workspace

**Area:** lab rows, build provenance
**Status:** Active

**Symptom:** Recovery rows that had just been rewritten to interrupt on a
boundary the engine announces reported that the interruption trigger never
fired, twice, at about six minutes a time. The engine code was right, its
local test passed, and the installers had been rebuilt after the change.

**Cause:** the installers had been rebuilt, but the **engine had not**.
`tiger-setup build` embeds `tigersetup-setup.exe` from beside itself, so the
installers carried a release engine from before the change and rejected the new
`--fault-signal` argument outright. Nothing in the result said so: the run
simply never reached the fault, so no signal was written and the trigger timed
out looking for it. `cargo build --workspace` had been run many times; it does
not touch the release profile the builder reads.

**Do not:** rebuild an installer to pick up an engine change. That is not what
it does. And do not read a lab row as evidence about the code in the working
tree — it is evidence about the bytes inside the installer, which is exactly
why the exact-artifact principle exists.

**Use instead:** `cargo build --release` first, then the installers, then the
rows. Where a row is about to run, the two can be compared: the installer
declares the SHA-256 of the engine it carries, and the builder's engine is the
file beside the builder.

**Prevented by:** `Assert-TigerSetupEngineIsCurrent` in `lab/TigerSetupLab.psm1`,
called by both row drivers before any guest work, refuses a mismatch with both
hashes and the command that fixes it. It costs a second; discovering the same
thing from a matrix costs the matrix.

**Generalization candidate:** a shared Lab or TigerAiCore — "assert the artifact
under test was built from the tree you think it was" is general to any
harness that validates a built artifact rather than a source tree.

## A lab row's own defects must be found before the guest time is spent

**Area:** lab driver, PowerShell
**Status:** Active

**Symptom:** Three separate rows died *after* their guest work succeeded, each
on a property that was not there, each costing the minutes the row had already
spent. `The property 'status' cannot be found on this object`;
`The variable '$Theme' cannot be retrieved because it has not been set`;
`The property 'Count' cannot be found on this object`.

**Cause:** three different ways a PowerShell value silently is not what the
code assumes, all fatal only under `Set-StrictMode -Version Latest`, and all
reached only after the expensive part of the row:

- **A local overwrote a parameter differing only in case.** `$prepare` and
  `[string[]] $Prepare` are one variable; the type constraint survives the
  binding, so a run object was coerced to a single-element string array.
- **A line was pasted into a function that has no such parameter.** An added
  `$Theme` read compiled fine and threw at run time.
- **A function returned an empty array.** PowerShell unrolls a returned
  collection, so `return @()` returns *nothing*, the caller's variable becomes
  `$null`, and the next `.Count` ends the row. It only ever happens on the
  path where the guest produced no logs — a `BUSY` lease, a failed job — which
  is exactly the path a row is least often exercised on.

**Do not:** rely on running a row to find out whether the row is correct. A
row is the most expensive test in the project and the worst debugger.

**Use instead:** `pwsh -File lab\Test-LabScripts.ps1` before any lab run — it
parses everything and reports any function reading a variable nothing declares.
Return `, @()` rather than `@()` from a helper whose empty result is a
legitimate outcome. Treat a lab run that is not `OK` as an outcome to report
and stop on, never as a base to keep reading evidence off: `BUSY` means
nothing ran, so every reader after it is interpreting an absence.

**Prevented by:** `lab/Test-LabScripts.ps1` — the undeclared-variable class,
and the case-collision class beside it, which cost a second run before it was
guarded: an assignment whose name matches a parameter of its own scope in a
different spelling is that parameter, and the value the caller supplied is
gone;
`Test-LabRunUsable` ends a recovery row on a lab run that produced no evidence;
`ConvertTo-TigerSetupFlattenedChecks` rejects an object that is not an
entry-point run; and both row drivers record the failing statement and the
script stack in an `ERROR` summary, so the next attempt starts from the line
rather than from the message.

**Generalization candidate:** TigerAiCore or a shared Lab — "a driver whose
each attempt costs minutes needs a static gate and an error record that keeps
its position" is general to any expensive-loop harness. The unrolling trap is
not merely ordinary language knowledge either: the same mistake, in the same
shape, is what TigerWinLab's R12 turns out to be
(`TigerWinLab-Requirements.md`), and reading the helper is what hides it —
`$list.ToArray()` is never `$null`, but a function returning it *when the list
is empty* hands the caller `$null` all the same.

**The other half of that trap, which cost a second matrix pass:** `, $list`
fixes the helper and moves the defect to its callers. A helper that emits its
collection as one object is only correct for callers that *assign* it; a caller
that enumerates — `@(& $body)`, or piping into the property form
`Where-Object status -eq 'FAIL'` — now receives one `Object[]` instead of the
checks, and reports a count of 1 however many there were. Under
`$ErrorActionPreference = 'Stop'` the property form throws
`The input name "status" cannot be resolved to a property`; under the default
preference it silently matches nothing, which is worse. That is R15, and it is
why TigerSetup's own `Get-JobLog` was checked rather than assumed: every one of
its callers assigns, so the convention is safe *there*. **Both ends of a
collection boundary have to agree, so a return convention is only verifiable at
its call sites** — and a regression test that exercises the helper alone, as
the lab's did for 0, 1 and 3 items, passes either way. TigerWinLab now normalizes at the boundary that had the
mismatch — every scenario collects its phase body through `ConvertTo-CheckArray`
— and tests the public `Invoke-Phase` with each shape a body can produce rather
than the helper alone.

## A graceful Restart Manager shutdown leaves a process without a message loop running

**Area:** engine, quiescence around a transaction
**Status:** Active

**Symptom:** `RmShutdown` returned `ERROR_SUCCESS` for a console process
holding a payload file open, yet the file was still locked and the process was
still running. Trusting that return value would have let the transaction open
and then fail halfway through replacing the file.

**Cause:** without `RmForceShutdown`, the Restart Manager asks and does not
insist. An application with no message loop never answers, so it is neither
closed nor terminated, and the call still reports success.

**Do not:** treat a successful `RmShutdown` as evidence that the files are
free, and do not reach for `RmForceShutdown` to make it so — forcing a running
application to die is exactly what the design refuses to do.

**Use instead:** decide from who still holds the files, over a bounded grace
period rather than in one look — the complement of "success does not mean they
are gone" is "still there does not mean they refused": an application with a
message loop needs a moment to answer the request, save and exit, and
concluding at once reports it as a holder that would not close. Ask that
question of the machine, not of `RmGetList`; see the entry below for why.

**Prevented by:** `restart::Quiescence::acquire` waits through
`wait_for_holders_to_go`, and `quiescence.rs` asserts that an upgrade blocked
by a console holder either succeeded outright or changed nothing.

**Generalization candidate:** none — this is Restart Manager behaviour, and
TigerSetup is where it matters.

## `RmGetList` answers with what the session was told, not with what is running

**Area:** engine, quiescence around a transaction
**Status:** Active

**Symptom:** M6 exited 6 `package_in_use` after the full grace period, twice,
against an application the Restart Manager had listed as a *main window*
application and had asked to close. Lengthening the grace from fifteen seconds
to sixty changed only how long it took to say so. Every other signal said the
arrangement was right: the application had a window, it ran on the signed-in
user's desktop, and the elevated upgrade ran beside it.

**Cause:** the wait re-listed the holders with `RmGetList`, and `RmGetList`
answers with the applications the Restart Manager session knows about — the
list taken when the resources were registered — rather than with a fresh look
at the machine. It keeps naming a process that has already exited. A wait that
ends when that list empties therefore never ends, and **every** upgrade over a
running application was going to report `package_in_use` however promptly the
application closed. The application had been closing all along; nothing in the
run could see it — asked the other way, the same application in the same row
closes 379 ms after the request, and M6's upgrade went from a sixty-three
second refusal to a three-and-a-half second success.

**Do not:** treat `RmGetList` as an observation of the machine, and do not read
a holder still listed after a shutdown as an application that refused.

**Use instead:** ask Windows about the holder's process. The Restart Manager
gives a `RM_UNIQUE_PROCESS` — a process id together with the moment it started
— which is exactly the identity needed, because Windows reuses a process id as
soon as the process that had it is gone. A handle that will not open because
this run may not look at that process is not evidence that it ended: an
unelevated run keeps waiting rather than concluding that another account's
application has closed.

**Prevented by:**
`restart.rs::a_holder_that_goes_away_by_itself_ends_the_wait` — a holder that
closes the file and exits on its own, with no shutdown to attribute it to, must
end the wait. It runs in three seconds and fails in thirty against the old
code; the whole class was invisible to the tests that existed, because the only
holder they used was one that never closes.

**Generalization candidate:** none — this is Restart Manager behaviour, and
TigerSetup is where it matters.

## A quiescence row must run the application where a person would run it

**Area:** lab rows, M6; any row that asks the Restart Manager to close a GUI
application

**Status:** Active

**Symptom:** M6 failed identically every time it was run — the upgrade exits 6
`package_in_use`, the row reports "the upgrade log records no shutdown", and
the installed version stays at the old one. Two distinct causes produced that
one verdict, and fixing the first changed the counts by two and nothing else,
which reads as "the fix did nothing" and is not what happened.

**Cause:** the row started TigerMarkView with a plain guest-job command, and
those run as the job account in the lab's non-interactive session. The Restart
Manager lists a holder from its open file handles, which crosses Windows
sessions, but it closes a GUI application by messaging its windows, which does
not. An application started that way has no interactive desktop to be messaged
on, so it was asked and never answered — the same shape as the console holder
above, reached by a different route. Underneath it, and invisible until the
arrangement was right, was the engine's own defect: it asked `RmGetList`
whether the holder had gone, and `RmGetList` never says so (see above).

**What makes this hard to read:** the engine emits `[restart_manager_shutdown]`
only once the holders have actually gone, so the check that looks for it reports
"no shutdown" for a shutdown that was requested and honoured. And "still running
after the grace period" reads identically for an application that refused, one
that was never asked, and one that had already closed.

**Do not:** read `package_in_use` in a quiescence row as evidence that the
product's Restart Manager handling is broken, and do not stop at the first
plausible cause because the counts barely moved when it was fixed.

**Use instead:** make the row say where each command ran and what Windows could
ask of the holder, then read those rather than the verdict. The Restart
Manager's own `RM_PROCESS_INFO.ApplicationType` is what separates "the request
that works was made" from "no window was ever found to ask", and it costs
nothing to record.

**Prevented by:** the engine records the classification beside every holder
(`restart.rs::classify`, in the log line and in the in-use message) and derives
the grace period from it —
`restart.rs::a_windowed_holder_is_given_longer_than_one_windows_cannot_ask`;
M6 asserts that the application ran on the signed-in user's desktop, that the
elevated upgrade ran beside it, that the application presented a window, and
that the Restart Manager listed it as one it could ask.

**Generalization candidate:** none — the Windows behaviour is general, but what
it cost was a TigerSetup row's ability to evidence its own requirement.

## A Restart Manager shutdown kills the console it is asked from, not just the holder

**Area:** tests, quiescence; any harness that spawns a console process the
Restart Manager will be asked to close

**Status:** Active

**Symptom:** `cargo test --workspace` killed the terminal it was started from.
Running it from an agent session closed that session outright — twice — with no
error, no exception, no crash dialog and no entry in any event log. The test
run simply stopped existing partway through `quiescence`.

**Cause:** the Restart Manager closes a **console** application by delivering a
console control event to it, and a console control event goes to every process
attached to that console rather than to the one process being closed.
`quiescence`'s `Holder` spawns `powershell.exe` to hold a payload file open,
and it was spawned with no creation flags — so it inherited the console of the
test runner, which is the console of whatever started the suite. When the
engine asked for the graceful shutdown the quiescence path exists to perform,
Windows delivered the event to that whole console: the holder, the test binary,
`cargo`, the shell, and the agent session hosting them all died together with
`STATUS_CONTROL_C_EXIT` (`0xC000013A`, seen as exit code `-1073741510`).

The silence is the trap. A console control event is not a fault, so there is
nothing to report: no `Application Error`, no Restart Manager event, no
Defender detection, no resource exhaustion. Every log is clean and every
plausible cause — a crash, memory, the disk, antivirus, the engine's own
`AttachConsole` — is innocent. What identifies it is the **exit code of the
run**, and the run has to be observed from outside its own console to have one.

**Do not:** spawn a process the Restart Manager will be asked to close without
giving it a console of its own, and do not diagnose a silently vanishing test
run by reading event logs — this failure writes to none of them.

**Use instead:** `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` on any such
child, which keeps it a console application with no message loop — the thing
the quiescence path is actually being tested against — while containing the
event to a console nothing else shares. More generally, when a test run
disappears without a diagnostic, re-run it detached from the session's own
console (`Start-Process` with redirected output) and read its exit code: an
observer inside the blast radius cannot report what killed it.

**Prevented by:** `quiescence.rs::HOLDER_ISOLATION`, applied where the holder
is spawned.

**Generalization candidate:** TigerAiCore — "an agent session must not share a
console with a child that will be signalled, and a run that vanishes without a
diagnostic must be re-observed from outside its own console" is method
knowledge about running expensive suites from an agent session, not a fact
about installers.

## Windows rewrites an access control list in its own spelling

**Area:** engine, machine-scope state directory
**Status:** Active

**Symptom:** A DACL written as
`D:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)` reads back as
`D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)`. Comparing the two as
text says the list drifted, so every run would rewrite it and no run could
ever detect real tampering.

**Cause:** the SDDL writer renders an access mask with the abbreviations that
add up to it (`FR|FX` is `0x1200a9`), reorders nothing but reformats
everything, and adds the header flags the object actually carries.

**Do not:** compare access control lists, or any other Windows security
descriptor, by string equality or substring search.

**Use instead:** parse both sides into entries and compare rights as numbers
(`win::acl::Dacl`).

**Generalization candidate:** none — it belongs with the code that reads DACLs.

## `%ProgramData%` is writable by a standard user, so it cannot decide elevation

**Area:** engine, elevation
**Status:** Active

**Symptom:** A machine-scope run decided it needed no administrator because it
could create files under `%ProgramData%`, and then failed creating its install
root under `%ProgramFiles%` — after the elevation prompt was no longer on
offer.

**Cause:** the default access control list of `%ProgramData%` grants
`CREATOR OWNER` and lets any user create files there. Only `%ProgramFiles%`
is closed to a standard user from the start; the state directory becomes
closed later, once TigerSetup writes its own list on it.

**Do not:** decide that a run needs elevation from the scope alone, or from a
single root.

**Use instead:** probe every root the run will write — the state directory and
the install root — and require elevation if any of them refuses
(`elevation::required`).

**Generalization candidate:** none — the roots are TigerSetup's.

## A known folder can move under a live installation

**Area:** engine, ownership and scope confinement

**Status:** Active

**Symptom:** A user-scope installation could not be uninstalled at all. The run
refused with `path_outside_root: stored shortcut ...\Desktop\App.lnk is outside
...\Start Menu\Programs and C:\Users\<user>\OneDrive\Desktop`, before opening a
transaction, so the product stayed installed with no way to remove it.

**Cause:** confining stored resource paths to the scope's own roots is right,
but it was applied to shortcuts as a reason to refuse the whole run. A hive
cannot move, so a registry key outside the scope really does mean the database
is wrong about the machine. A shortcut folder is different: OneDrive's Known
Folder Move relocates the desktop, and policy can redirect the Start Menu, so a
link recorded before the move is legitimately outside the folders the scope
resolves afterwards. The check could not tell the two apart and treated both as
tampering.

**Do not:** treat "this stored path is outside the roots I resolve now" as
proof of tampering for any resource whose location Windows may move, and do not
let a conservative refusal end in a state the user cannot get out of. An
installation that cannot be uninstalled is a worse outcome than a resource left
behind.

**Use instead:** decide per resource kind. Where the location cannot move
(registry hives, the install root), refuse the run. Where it can, do not touch
the resource, report it with its own code (`shortcut_outside_scope_preserved`)
and finish removing everything else. Both answers leave the out-of-scope
resource untouched, which is what the confinement exists to guarantee.

**Prevented by:** `plan::reconcile` takes the scope's shortcut folders as an
input and preserves a link outside them;
`resources.rs::a_shortcut_whose_folder_moved_is_preserved_and_the_rest_is_removed`
moves the folder and asserts the uninstall completes;
`machine_scope.rs::a_shortcut_row_outside_the_scope_is_preserved_and_never_deleted`
asserts a tampered row is still never acted on.

**Generalization candidate:** the principle — separate "cannot legitimately
differ" from "may legitimately differ" before treating a mismatch as an attack
— is general, but the resource kinds are TigerSetup's.

## Locking a directory down is not the same as owning it

**Area:** engine, machine-scope privilege boundary

**Status:** Active

**Symptom:** An independent review traced a working confused-deputy path
through code that looked correct: the machine-scope state directory was given
an explicit access control list granting only SYSTEM and Administrators, yet a
standard user could still replace the `uninstall.exe` inside it that Add/Remove
Programs later runs elevated.

**Cause:** two facts that are individually unremarkable and together decisive.
`%ProgramData%` grants `BUILTIN\Users` the right to create subdirectories, so a
standard user can create the product's state directory before any install ever
runs — and the creator owns what it creates. An object's **owner** keeps
`WRITE_DAC` no matter what the list says, so it can hand the rights back to
itself at any later moment. Writing a DACL onto a directory somebody else owns
protects it only until that owner objects. `create_dir_all` also adopted a
pre-existing directory without ever asking who owned it.

**Do not:** treat "I set the access control list" as "this object is mine", and
do not stage an executable you are about to run elevated in a directory the
invoking user can write — under same-account elevation `%TEMP%` is still that
user's own folder, with inheritable full control over anything created in it.

**Use instead:** set the owner with the list (`OWNER_SECURITY_INFORMATION`, an
`O:BA` prefix in the SDDL), read the owner back before trusting a directory
that already exists, and take ownership when it is someone else's. For a
binary about to be executed elevated, create a fresh directory under a
system-owned root with an unpredictable name and protect it before writing
into it.

**Prevented by:** `scope::protect_state_directory` checks owner and list and
reports `state_directory_ownership_claimed` when it takes a directory over;
`lib::staging_directory` refuses to stage an elevated relaunch in the user's
`%TEMP%`.

**Generalization candidate:** the principle is Windows-wide and belongs in
whatever shared guidance covers privileged Windows code — an installer is
simply where it bites first.

## Coordination is not a precondition: an unavailable service must not fail the run

**Area:** engine, quiescence

**Status:** Active

**Symptom:** A lab row installed nothing and reported
`restart_manager_failed: the Restart Manager could not start a session: The
system cannot write to the specified device`. The run had not reached its
transaction; the machine was left untouched but the product was not installed.
It happened on a guest that had just been power-cut and restarted — that is,
during the recovery scenario the row exists to test.

**Cause:** `RmStartSession` was treated as a step that must succeed. Restart
Manager is a service, and a service can be unavailable, most plausibly right
after an unclean boot, which is exactly when a recovery run happens. The engine
turned "I could not ask who holds these files" into "I refuse to install".

**Do not:** let an advisory mechanism become a precondition. Quiescence exists
to make replacing a file in use *pleasant*; it is not what makes the
transaction safe. The journal is.

**Use instead:** report the mechanism as unavailable
(`restart_manager_unavailable`) and continue. A file genuinely held open then
fails its own operation, and the transaction rolls back — a recoverable outcome
the engine already handles — instead of a run that will not start at all.

**Prevented by:** `restart::Quiescence::acquire` degrades to `none()` when the
session cannot be started, files cannot be registered, or holders cannot be
listed.

**Generalization candidate:** the principle is general — decide for every
external service whether it is load-bearing or advisory, and make an advisory
one fail open — but the mechanism is Windows-specific.

## Windows writes the registry back lazily, so a journal can outlive its own mutation

**Area:** engine, durability of registry operations

**Status:** Active

**Symptom:** After a power cut during an upgrade, recovery reported nothing to
do and exited in 0.3 s, but `verify` failed with `registration_modified` on
`DisplayVersion`: the state database said 0.8.2 and the machine's Add/Remove
Programs entry still said 0.8.1. The transaction had committed, so nothing
would ever reconcile the two — a same-version re-run short-circuits to
`already_installed`.

**Cause:** files are flushed as they are written (`win::fs`), but registry
values were not. `RegSetValueEx` returns once the change is in memory; Windows
writes the hive back when it chooses. So a power cut could take a value the
journal had already marked applied, and the transaction that committed
afterwards recorded state the machine did not have. The invariant the design
states — durable undo before the mutation — was upheld; its unstated other
half, *the mutation durable before the record of it*, was not.

**Do not:** assume a Windows API that returned success has written anything to
disk. `RegSetValueEx`, unlike a flushed file handle, has not.

**Use instead:** flush the hive once before the commit, not once per value —
`RegFlushKey` on a predefined key flushes that hive. A cut before the flush
leaves the transaction open and recovery reconciles it; a cut after it finds
the values already on disk. One call per transaction is affordable; one per
value is not.

**Prevented by:** `txn::Executor::commit` calls `win::registry::flush` before
`journal::commit_*`, and lab row M5b (power cut during an upgrade) verifies
the registration afterwards.

**Generalization candidate:** the question "does success mean durable?" is
worth asking of every mutation an installer makes, on any platform.

## A vendor's installer may report failure and still have done the job

**Area:** engine, dependency acquisition

**Status:** Active

**Symptom:** Two Windows 10 rows failed to install the product at all. The log
showed TigerSetup doing everything right — 258 MB downloaded from Microsoft's
own URL, SHA-256 verified, the vendor's installer run with `/silent /install`
— and then `dependency_install_failed: the installer exited with code
-2147219970`, a clean rollback and exit 3. The very next run's log records
`dependency_detected Microsoft.EdgeWebView2Runtime: version 152.0.4191.66`.
The runtime was there. The installer had succeeded and said otherwise.

**Cause:** the exit code was treated as the verdict. For a *resource* that
would be right; for a *requirement* it is not. `TigerSetup-Design.md` §7.2
separates identity, detection, acquisition, installation and verification
precisely because the question is "is the requirement met on this machine",
and only detection answers that. Microsoft's WebView2 evergreen bootstrapper
returns vendor-specific codes for conditions that are not failures.

**Do not:** let a third-party installer's exit code be the last word on
whether a dependency is present, in either direction. Success already had to
be confirmed by detection (`dependency_unverified` exists for an installer
that claims success and leaves nothing); failure deserves the same check.

**Use instead:** on a non-zero exit, detect again. If the dependency is now
present the requirement is met — report
`dependency_installed_despite_exit_code` and continue. If it is absent, the
failure stands.

**Prevented by:** `dependency::install` re-detects before believing a failing
exit code; lab rows W4 and W6 are the ones that exercise acquiring a genuinely
absent WebView2.

**Generalization candidate:** the shape is general — when something else
performs the work, verify the world rather than trusting the report — but the
detectors are TigerSetup's.

## A wizard row's keys are aimed at a page number, and pages move

**Area:** lab rows, wizard capture; any row that drives a wizard by keystroke

**Status:** Active

**Symptom:** W6 captured one page and then died with TigerWinLab's Desktop
Capture reporting `The handle is invalid`. Every other signal said the wizard
was fine: the lab's `wait-window` found the window, `hit-test` answered from
it, and the window's bounds were unchanged. Refreshing the guest support
scripts changed nothing, and a secure-desktop transition stayed a hypothesis
for two sessions.

**Cause:** the row sent Alt+A to page 1 to accept a licence. Page 1 is the
**scope** page, whose second choice is `Install for &all users` — so Alt+A
selected the machine-scope install, Enter asked for elevation, and Windows put
the UAC prompt on the secure desktop. Nothing on the secure desktop can be
photographed by a session that may not open it, so the very next capture failed
with a bare Win32 message, six steps away from the keystroke that caused it. The
row is the one row of the matrix whose whole point is acquiring a dependency
*without* elevation.

**Do not:** number a wizard's pages in a row and aim keys at the numbers
without evidence of what each page offers, and do not read a capture failure as
a capture defect — the window is still there and still enumerable while the
desktop in front of it is one the session cannot read.

**Use instead:** read the page's own UI Automation tree, which every capture
already writes beside the screenshot, and derive the keys from what the page
offers. The wizard's control identifiers are grouped by page and stable
(`ui::window`'s `ID_SCOPE_*`, `ID_LICENSE_*`, …), so `controls` on each captured
page names which page it was, in every language. TigerWinLab now names the
input desktop when a capture fails there, so the same mistake reads as an
elevation prompt rather than as a broken handle.

**Prevented by:** the page record carries its control identifiers, so a row's
key plan can be checked against what was actually in front of it; W6's plan
answers the scope page with Alt+M and the licence page that follows with Alt+A.

**Generalization candidate:** none — the Windows fact (a secure desktop cannot
be captured) is TigerWinLab's and is now documented there; aiming keys at page
numbers is a TigerSetup row's own discipline.

## A page that is working looks exactly like a page that was not answered

**Area:** lab rows, wizard capture

**Status:** Active

**Symptom:** W6 could not have completed even with the right keys. Its wizard
budget was ten pages and its loop advanced every 800 ms, so the dependency
download — minutes on one page — would have exhausted the budget and killed the
wizard part-way through installing WebView2.

**Cause:** the wizard deliberately keeps its forward control visible and
disabled while it is busy (`ui::window::update_buttons`), so a key pressed then
is lost rather than queued. A capture loop that advances on a timer therefore
photographs the same page repeatedly and calls each one a page.

**Do not:** give such a row a bigger page budget or a longer settle delay. Both
are guesses about how long a download takes, and the row still cannot tell
"still working" from "not answered".

**Use instead:** wait for the page itself to change. A wizard keeps one window
with one title and rewrites a progress page's label as it works, so the stable
identity is the set of control identifiers the page shows;
`lab/guest/Invoke-WizardCapture.ps1` waits for that set to change or for the
process to exit, bounded by the wizard's `pageTimeoutSeconds`. A page that never
changes then ends the wizard naming what it was still showing, which is a wrong
key rather than a page budget quietly running out.

**Prevented by:** the wait itself; W6 captures seven distinct pages —
scope, licence, destination, options, ready, progress, finish — and its process
ends `exited` rather than `killed`.

**Generalization candidate:** possibly TigerWinLab — its own `Invoke-WizardRun`
already waits for a *named* advance control, which is the label-driven form of
the same rule. Promote only if a second consumer needs the language-independent
form.

## Ending a lab session and reclaiming its VM are two transitions

**Area:** lab drivers, Lab sessions, background-work accounting
**Status:** Active

**Symptom:** A full matrix run was killed by the operating system for want of
memory, part-way through its first row. Nothing about that run was wrong: a VM
from the *previous* run had been left powered on, and two guests plus the host's
own work did not fit.

**Cause:** the earlier run ended its Server session correctly and the lab
reported, per resource, that ending it had **not** reclaimed the VM —
`cleanup: Failed`, `Shutting down VM … failed: … Access is denied.
(0x80070005)`. The driver discarded that report. Ending a session is
authoritative and stays ended; stopping the VM it released is a separate
transition, and a stop the guest is allowed to refuse is one it can fail.

**Do not:** treat "the session ended" as "the VM was released", and do not
discard the per-resource report because the rows all passed. A VM left running
costs host memory and one of the host's few running slots, so the run that pays
is the next one — killed, or refused a baseline it cannot start.

**Use instead:** read the report the close writes and name anything the lab did
not reclaim (`LEFT RUNNING:`). Completion accounting reads
`Get-TigerWinLabSession.ps1`, which answers from the lab's own records: no
process list shows a VM.

**Prevented by:** `Exit-TigerSetupLabSession` reads the close report and prints
every resource whose cleanup is not `Completed`, `NotRequired` or `Protected`.
The refusal that produced this entry no longer happens: TigerHyperLab's stop is
a shutdown the guest cannot veto, which is what a Windows Server guest was
doing.

**Generalization candidate:** none — the provider now owns the refusal, and
this entry keeps only the accounting rule that outlived it.

## A shortcut's target reads back with the drive letter Windows canonicalized

**Area:** process-level tests; any assertion comparing a path the engine wrote
against a path a Windows API read back

**Status:** Active

**Symptom:** the whole `durability` suite (and every other suite that calls
`assert_verified`) failed on one build and passed on the next with no source
change, on `assertion left == right` where the two paths differed only in the
drive letter: the link's target was `C:\…` and the expected install root was
`c:\…`.

**Cause:** `assert_verified` compared the Start Menu link's target
byte-for-byte against `install_root().join(...)`. Windows canonicalises the
drive letter of a link's target when it saves the `.lnk`, so the target always
reads back upper-case; the expected side is the test's temporary directory,
whose drive-letter case is whatever `CARGO_TARGET_TMPDIR` had **baked in at
compile time** — which flips with how cargo was invoked (a `c:\` path from Git
Bash, a `C:\` path from PowerShell). So the assertion passed only when the
compile happened to bake an upper-case drive. The engine's own `verify` never
had the problem: it compares shortcut targets with `eq_ignore_ascii_case`.

**Do not:** compare a Windows path the OS round-tripped against one the test
assembled with `==`, and do not chase a pass/fail that flips between runs as a
real regression before checking whether only the drive-letter case differs.

**Use instead:** compare Windows paths case-insensitively, at the same contract
the engine holds (`eq_ignore_ascii_case`). Windows paths are case-insensitive;
a case-sensitive comparison is the defect.

**Prevented by:** the helper now compares the shortcut target
case-insensitively, so the assertion no longer depends on the drive-letter case
the build directory happened to carry.

**Generalization candidate:** none — it is a test-hygiene rule specific to
paths that cross a Windows API, and the engine already applies it.

## An interrupted wizard test leaks a GUI installer that poisons a later build

**Area:** process-level wizard tests; background-work ownership

**Status:** Active

**Symptom:** every `wizard` test failed at the very first step — building the
shared fixture — with a bare `io_error` "Access is denied. (os error 5)", six
steps removed from anything the failing test does. The same fixture code built
fine for every other suite.

**Cause:** a `Setup.exe` a wizard test had started (an interactive run left
mid-flight when a `cargo test` was moved to the background and not awaited to a
terminal state) stayed running, holding a handle to its installer file under
`target\…\tmp\fixture-wizard\out\`. The next `wizard` run's fixture build does
`remove_dir_all` on that directory and could not delete the open file, so the
build failed — in a suite, and at a step, that had nothing to do with the leak.

**Do not:** move a wizard `cargo test` to the background and start other work
without driving it to completion or killing it; a wizard test spawns real GUI
`Setup.exe` processes (and, for a scope choice, a child process), and an
abandoned one keeps a window and a file handle alive. Do not read the resulting
access-denied as a build bug.

**Use instead:** account for wizard-spawned processes as task-owned background
work (`AGENTS.md`, and *A finished process is not a finished task*): before
reporting or re-running, sweep for `Setup.exe` / `<product>-Setup` / the engine
by the directory they run from, not by a fixed name list, and stop any that a
prior run left behind. `Get-Process … | Where Path -like '…\fixture-*\out\*'`
finds them.

**Prevented by:** nothing mechanical yet — a wizard run interrupted from
outside the test cannot clean up after itself. The discipline is to await or
kill a backgrounded wizard test, and to sweep before trusting a fresh run.

**Generalization candidate:** none — it is the general background-ownership
rule applied to this project's GUI tests.

## A wizard page is seen before its buttons are, and a synthetic run outruns the reader

**Area:** wizard tests, wizard automation
**Status:** Active

**Symptom:** `wizard.rs`'s install test, green for weeks, failed twice in a
row on "a busy page keeps its forward button visible but disabled": it had
waited for the progress page and then read the Next button once, and found it
enabled. The wizard, when looked at, was sitting on its finish page with the
install complete.

**Cause:** two things that are each fine alone. `enter_page` shows the new
page's controls first and updates the buttons after — a few statements later
on the same thread, so a click can never land in between, but a reader in
another process, which reads window state directly, can. And a synthetic
install finishes in well under a second, so the reader that missed the busy
state may already be looking at the finish page, where the forward button is
enabled by contract. A single sample taken the instant a page marker appears
answers neither the page's contract nor the run's state.

**Do not:** assert a page's button state from one read taken right after its
marker became visible, and do not read a fast run's transient page and expect
it to still be there.

**Use instead:** what the cancel test already does — wait for the settled
contract (`wait_until(… !enabled(ID_NEXT))`, as it waits for Cancel), and hold
the run at a fault point (`--fault after_prepare@20:hold:<s>`) when the page
would otherwise be gone before the reader arrives. A held run makes "the
button stays enabled" a real violation rather than the finish page arriving.
The same holds for any automation that drives the wizard from outside: wait
for the page *and* the button state it wants, never for the page alone.

**Prevented by:** the install test in `crates/tigersetup-setup/tests/wizard.rs`
holds its run and waits for the contract.

**Generalization candidate:** the observation half — a cross-process GUI reader
sees intermediate states the UI thread never exposes to input — may belong with
TigerWinLab's wizard driver; it is recorded here until a lab row shows it.

