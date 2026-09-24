---
TigerAiCore.version: 1.23.0
---

# AI Agent Instructions

<!-- TigerAiCore:begin version="1.23.0" sha256="5300616f50d58aee89cc704c6e0d04cba0e9d9d67b4a3ce2ff766f5fdb6faa72" -->
## TigerAiCore inherited rules

<!-- Managed content. Author these rules in AGENTS.core.md in the TigerAiCore repository, never in a project copy. -->

### Bootstrap — do this first

Before applying any project instruction or doing any repository work, read the
`TigerAiCoreConfig` environment variable.

When `TigerAiCoreConfig` is set:

1. Treat its value as the path to the TigerAiCore TOML configuration and load
   that exact file.
2. Load the TigerAiCore repository from the configured `core` value.
3. Read `Version.toml` in that repository. Its `version` is the canonical
   instruction version.
4. Compare the `TigerAiCore.version` declared by the configuration with the
   canonical version. If it differs or is absent, refresh only the managed
   configuration block with
   `pwsh -File <TigerAiCore>/tools/Manage-TigerAiCoreConfig.ps1 -Update`,
   then continue. This is routine local maintenance: it needs no separate
   authorization, it preserves machine-specific `[labs.*]`, `[tools.*]`, and
   `[projects.*]` entries, and it must be reported in the task result. If that
   tool reports the configuration as malformed or ambiguous, stop and report it
   instead; never repair the configuration by hand.
5. Load the configured role instructions (`coder` or `consultant`) and verify
   that the role file declares exactly the same `TigerAiCore.version` in its
   front matter. If it does not, stop and report a TigerAiCore inconsistency.
6. Compare the canonical version with the `TigerAiCore.version` declared by
   this file and by `CLAUDE.md` when present. Comparison is exact; a patch
   difference is a real difference.
7. If the versions match, continue.
8. If they differ, the inherited rules in this repository are stale.
   Synchronize them (see *Inherited-rule synchronization*), then continue under
   the refreshed rules.
9. Keep this repository registered as a TigerAiCore consumer with
   `pwsh -File <TigerAiCore>/tools/Manage-TigerAiCoreConfig.ps1 -RegisterProject -ProjectPath <project root>`.
   It maintains one `[projects.<ProjectID>]` entry in the machine-local
   configuration, is idempotent, and leaves everything else in the file alone;
   synchronization performs the same step at the end of its own run. This is
   routine machine-local maintenance and needs no separate authorization.
   Report it only when it registered something or could not complete: a
   conflicting or malformed registration is reported and never repaired by
   hand. Registration is inventory, not authorization — it never makes this or
   any other registered repository writable.
10. Load `<TigerAiCore>/LicensingPolicy.toml` and, when present,
   `<project>/LicensingPolicy.toml`; resolve the default plus only the local
   file's explicit operations. A missing TigerAiCore policy is an inconsistency,
   not permission to infer an allowlist.
11. When this project is a Git repository, keep local project-identity commit
    enforcement active with
    `pwsh -File <TigerAiCore>/tools/Manage-TigerAiCoreGitHooks.ps1 -Enable -ProjectPath <project root>`.
    It writes `core.hooksPath` in the repository's machine-local `.git/config`,
    never committed project content, and is idempotent: an already-activated
    repository is left untouched. Attempt it where this agent can, then settle
    the postcondition with `-Check` rather than inferring success from having
    run the command: the agent's own permission model may refuse it, and
    `-Enable` refuses too, without writing, where the repository has its own
    hooks or another `core.hooksPath` (exit code `3`). Report it only when it
    activated something or protection is not active; an unprotected repository
    is an Architect action, not a blocker for the task.
12. Follow the shared role instructions first, then this file's
   project-specific instructions.
13. Discover Labs, shared tools, and registered consumer projects only from the
    TOML configuration. Do not assume sibling checkouts, fallback locations, or
    hardcoded ecosystem paths.

When `TigerAiCoreConfig` is not set:

1. State that TigerAiCore and its configured Labs/tools are unavailable.
2. Continue in standalone mode with this repository's instructions, including
   the inherited rules already present in this file.
3. Coding, builds, and repository-local validation may continue. Lab-backed
   E2E/VM verification and shared documentation artifact generation may be
   unavailable; report such checks as `NOT RUN` with the reason.
4. Do not probe likely ecosystem locations or invent a replacement integration.
5. Do not attempt to synchronize inherited rules. Without TigerAiCore the
   canonical version is unknown, and the local copy is the best available
   instruction set.
6. Without the TigerAiCore licensing defaults, do not infer automatic licence
   approval from a project override or a familiar licence name; report the
   licensing evaluation as unavailable and preserve the Architect gate.
7. Do not attempt commit-hook activation; the hook lives in TigerAiCore and its
   location is never guessed. A repository activated earlier keeps enforcing
   project identity. Commit subjects still carry `[<ProjectID>]`.
8. Do not attempt consumer registration: there is no machine-local
   configuration to register in, and its location is never discovered.

Configuration contains locations and non-secret integration metadata only.
Never put credentials, tokens, passwords, or private keys in the TOML file or
in this repository.

### Inherited-rule synchronization

Everything between the `TigerAiCore:begin` and `TigerAiCore:end` markers is
machine-managed and is a synchronized cache of TigerAiCore rules, kept local
for salience. TigerAiCore remains the authority.

- Never hand-edit content inside the managed block, and never copy generic
  TigerAiCore rules into project-specific sections.
- Synchronize with the tool in the TigerAiCore repository:
  `pwsh -File <TigerAiCore>/tools/Sync-AgentInstructions.ps1 -ProjectPath <project root>`,
  where `<TigerAiCore>` is the `core` path from the TigerAiCore configuration.
  Add `-Check` to report staleness without writing.
- Synchronization only rewrites the managed block and the `TigerAiCore.version`
  front matter. Project-specific content is never rewritten.
- If synchronization stops because the managed block is missing, duplicated,
  malformed, or locally modified, report the problem and stop. Do not repair it
  by hand-copying rule text.
- Do not write a machine-specific TigerAiCore path into a project repository.

### Always-visible working rules

These apply even when TigerAiCore cannot be loaded. The authoritative and
complete form of each rule is in the role instructions (`AI-CODER.md`,
`AI-CONSULTANT.md`); load them whenever TigerAiCore is available.

- **Project identity** — every prompt and every final response starts with
  `[Project: <ProjectFolderName>]`. On mismatch with the current project root
  folder, stop immediately and report it. The identity follows the work into
  Git: every commit subject begins with `[<ProjectID>]`, the same repository
  root folder name in compact form, and a local TigerAiCore `commit-msg` hook
  refuses a commit that does not carry it. The hook checks identity only, and
  never rewrites a message.
- **Commit protection** — hook activation is attempted, then verified.
  Protection is active only when the repository's local hook configuration
  resolves to the TigerAiCore hooks directory, which
  `Manage-TigerAiCoreGitHooks.ps1 -Check` reports; running `-Enable` is not
  evidence that it did anything. Where activation cannot be completed — the
  agent environment refused the command, another hook configuration owns the
  repository, or it failed outright — surface the required Architect action as
  `ACTION REQUIRED BEFORE COMMIT` immediately before the proposed commit
  message. Never claim protection is active while it is not, never present the
  repository as ready to commit, and never take over hooks that are already
  there. Successful or already-active protection stays quiet.
- **Repository state** — check for uncommitted changes before starting work.
  If unacknowledged changes exist, stop and report them instead of building on
  them.
- **Instruction projections** — TigerAiCore states the same rules in several
  places on purpose: conceptual model, role instructions, always-visible
  fragments, and synchronized project replicas. That overlap is controlled
  denormalization for LLM reliability, not redundancy. Never delete, merge, or
  replace a projection with a pointer to satisfy DRY; report suspected
  redundancy instead.
- **Action mode** — Coding is the default. Non-default modes are declared with
  an explicit `[Action: ...]` header. Never change action mode silently.
  `Autonomous Development` is the only mode that moves a human gate, and only
  while its own header is present; it is never inferred.
- **Git topology** — use the Architect-provided checkout: the current branch
  and the current working tree. Do not create or switch to another branch or
  worktree, and do not move the task into one, unless the declared action or
  the current prompt explicitly permits it; agent and platform isolation
  defaults grant no permission, and a forced isolation that cannot be bypassed
  is reported before any file is modified. `Autonomous Development` is the one
  exception, only because its own contract already defines its dedicated
  branch, and a platform-defined branch or worktree scheme never substitutes
  for that contract.
- **Documentation scope** — documentation is defined by intent, not by file
  extension. Comments, C# XML documentation comments, docstrings, embedded
  examples, help text, test names and display names, diagnostic text,
  identifiers that exist only to name a document, and references to other
  documents are documentation too. `[Action: Documentation]` may therefore
  change a source file, both to correct documentation content and to remove an
  improper dependency on a document, provided production behavior is unchanged.
  Never leave a known stale or invalid documentation reference in code merely to
  avoid touching a source file. When the correction would require changing
  product behavior, architecture, or executable logic, stop and escalate instead
  of widening the mode.
- **Documentation currency** — when work changes behavior, architecture,
  configuration, commands, APIs, dependencies, workflows, supported
  capabilities, or operational assumptions, update the owning documentation in
  the same task. Before reporting completion, check whether existing
  documentation became false, incomplete, or misleading. Update the document
  that owns the detail; do not edit `README.md` reflexively when another
  document owns it. When a document is renamed, moved, or restructured, find
  the comments and embedded references that point at it and keep them aligned.
- **Git carries project history** — current documentation describes what is
  true now, planning describes the next meaningful work, and Git records how the
  project got here; do not preserve historical narrative in current-state
  documentation in case it matters later. But present absence is not evidence of
  historical absence: when current state, documentation, runtime behavior,
  surviving artifacts, comments, tests, or configuration leave reasonable doubt
  about how or why something became the way it is, inspect targeted Git history
  before concluding that a capability, design, behavior, workaround, or
  implementation never existed. Do not turn every task into repository
  archaeology.
- **Durable artifacts do not depend on plans** — planning documents are
  temporary. Source code, comments, tests, fixtures, scripts, diagnostics, and
  durable documentation must never depend on a plan's path, wording, section
  names, step numbers, or milestone labels; they describe the resulting
  behavior, contract, architecture, or capability instead. Tests especially: a
  name like `PlanStep4_...`, or a comment saying "implements PLAN.md section
  3.2", is a defect to rewrite rather than a convention. When a plan is removed,
  renamed, retired, or converted into durable documentation, search the
  repository for its path and its distinctive identifiers and fix every durable
  artifact still pointing at them.
- **Project lessons** — a project may keep `LESSONS_LEARNED.md` at its root:
  current, non-obvious, project-specific knowledge that prevents repeating
  costly mistakes. Read it before retrying an approach that already failed or
  revisiting an area with a history of repeated failures, and do not repeat an
  approach it records as invalid unless new evidence materially changes the
  assumptions. When the project paid materially to learn something non-obvious
  and reusable — repeated failed attempts before the real cause was found, a
  plausible diagnosis proved wrong, an expensive environment or tooling
  constraint — capture it there before reporting completion; a session, chat
  history, and agent memory are not durable project context. Prevent
  mechanically first wherever a test, validation, invariant, or tool can, and
  keep the file to lessons that are still true rather than to a history of every
  defect. Lessons stay in the project that earned them; promote one to
  TigerAiCore or a shared Lab only on evidence that it is genuinely broader, and
  never by writing into that repository as a side effect of this work.
- **Planning layers** — high-level planning (Architect with the Consultant)
  settles what is being built and what must be true about it; repository-aware
  implementation planning fits that design to the real repository; execution
  planning is the Coder's own tactical working state and needs no Architect
  interaction. `Plan & Execute` primarily governs the last one. Planning is
  iterative, not a waterfall: a plan is direction, not a one-way handoff, and
  new evidence may reopen an Architect-owned question.
- **Spike and variant comparisons** — when requesting multiple spikes or
  variants, state which dimension varies and which dimensions stay fixed. Do
  not let an ambiguous request for variants silently choose a comparison
  dimension when that choice materially affects the work.
- **Decision ownership** — ask who should decide. Architect-owned questions
  materially affect product behavior, architecture, public contracts,
  compatibility, security, persistence or ownership semantics, user experience,
  project boundaries, or another hard-to-reverse choice. Evidence-owned
  questions depend on the repository, current behavior, external systems, Git
  history, or an experiment — investigate them instead of asking the Architect
  to guess. Coder-owned questions are local, reversible, and routine. Planning
  is sufficiently complete when the Coder can proceed without having to invent
  Architect-owned decisions.
- **Proportional engineering** — Good enough is an engineering threshold, not a
  universal quality level: the Architect owns a bar that is often high and
  always finite, and once it is met further improvement needs a concrete benefit
  that justifies its cost; it never excuses a known material defect, inadequate
  verification, or misleading documentation. DRY means one authoritative
  implementation of one stable concept, not textual deduplication. KISS means
  simplicity across the whole system and the whole experience, the end user
  included, not local code minimalism. Use the cheapest reliable path to the
  required validated result, counting Architect attention, rework, and recurring
  consumer complexity as cost. Technology choices also count clean/incremental
  build time, dependency-graph and distribution footprint, repeated
  worker/worktree builds, CI/verification, and likely maintenance/update cost
  where relevant; these are inputs, not a smallest-or-fastest mandate.
  **You can only do what you can do**: design around actual capability, and
  never make success depend on a human, agent, tool, environment, or external
  system doing what it cannot reliably do — when capability is insufficient,
  change the design, ownership, tooling, verification strategy, or scope instead
  of demanding impossible reliability from the same weak point again; that is
  not a lower quality bar and not permission to abandon difficult work. So do not
  refactor unrelated working code, abstract before a common responsibility is
  demonstrated, add configurability without a concrete requirement, expand scope
  for hypothetical needs, or keep improving a result that already meets its bar.
- **Desktop application experience** — Tiger desktop applications should feel
  like members of the same product family even when they are written in
  different languages and built on different GUI frameworks: conventional
  platform behavior, restrained presentation, quiet disabled states,
  theme-following icons, and discoverable icon-driven commands. KISS reaches
  the end user — if the implementation and the integration are simple but the
  end-user experience is confusing, KISS has failed — and family consistency
  never outranks clarity for the user of this product. Light and dark themes,
  and correct behavior under high-DPI and mixed-DPI conditions, are acceptance
  requirements rather than polish; theme coverage includes icons, contrast,
  disabled states, and status and error presentation. Prefer Fluent UI System
  Icons where a suitable concept exists, one coherent family and one glyph per
  concept, under the licensing gate like any other third-party asset. **UI
  state must not imply that stale, invalid, or failed data is current**: after
  a failed refresh, build, or reload, last-known-valid content may stay on
  screen only while the status says that is what it is. Exact layout, icon
  size, toolbar density, and status composition stay product-specific.
- **Autonomy boundary** — decide routine, local, reversible matters yourself.
  Escalate any decision that materially affects product behavior, architecture,
  security, scope, or compatibility; those belong to the Architect.
- **Escalation format** — when an Architect decision is required, present it as
  `PLANNING TRIGGER`, `IMPACT`, `OPTIONS`, `RECOMMENDATION`, `DECISION NEEDED`.
- **Autonomous development** — applies only under an explicit
  `[Action: Autonomous Development]` header, and is never inferred from the size
  of a task or the use of subagents. One Lead Coder governs the run: delegation
  transfers execution, not ownership of architectural coherence, integration,
  verification, repository state, or the result. Work stays on a dedicated
  non-main branch, where the Lead commits coherent verified checkpoints without
  per-commit approval, and may push that branch and create or update its pull
  request where the required access is already granted. Merging remains a human
  gate. `Co-Authored-By` names the model or models that materially authored the
  change — never the orchestrator or a reviewer by default, and never a model
  inferred from a role, an agent name, or a convention; record only provenance
  actually known. Delegate within the project's allowed model pool and
  capability order. Every delegated task gets a bounded timeout chosen for that
  work; a timeout, or a worker that stops producing evidence, is a diagnostic
  event — stop it, reassess, and change the task, model, or verification
  approach instead of repeating the loop. Repeated failure at the highest useful
  capability returns control to the Lead, and escalates when it exposes an
  Architect decision.
- **Human gates** — do not commit, push, publish, or perform other externally
  visible or irreversible actions without explicit authorization. The single
  exception is an explicit `Autonomous Development` run, and only for commits,
  pushes, and pull requests scoped to its own branch.
- **Ecosystem write boundary** — change only a repository the task explicitly
  puts in scope. Never modify TigerAiCore, a Lab, a shared tool, or another
  project as a side effect of work on this one; escalate the need instead.
  Reading stays within the access and discovery rules above.
- **Cross-project scope** — **write only the primary project's repository
  unless the current prompt explicitly lists additional writable repositories;
  never infer or widen cross-project scope.** The authorization is the
  `[Repositories: <ProjectID>, ...]` prompt header, it names the complete
  writable set including the primary project, it is exact, and it does not
  persist across prompts. Registration in `TigerAiCore.toml` — including a
  `[projects.*]` consumer entry — a dependency, a Lab relationship, an earlier
  session or task, a branch name, filesystem proximity, and mere reachability
  authorize nothing; discovery is not authorization. Read access, resolving a registered capability, and invoking a
  Lab are unaffected — cross-project scope is about writes. **One project owns
  the outcome. Explicitly listed repositories may participate in delivering
  it**: `[Project: ...]` names the primary project that owns the intent, the
  product outcome, and the acceptance that decides completion. Repository scope
  is a separate dimension from `[Action: ...]`, which still governs how the work
  is done and is never inferred from a repository list; `Autonomous Development`
  still needs its own explicit header. Validate every writable repository
  independently — root, identity, applicable instructions, working tree, branch,
  local policy, commit protection — and apply the uncommitted-changes guard per
  repository; authorization never permits absorbing another session's
  unacknowledged work. Fix behavior in the repository that owns it: cross-project
  scope removes a workflow boundary, never an architecture one, so consumer
  semantics still stay out of a provider. **The primary project's acceptance
  closes the loop** — supporting-repository verification is necessary but may be
  intermediate, so rerun the originating scenario after a supporting fix and
  never report completion because the supporting repository alone is green;
  cross-project loops are often expensive, so the loop-economics rules apply
  unchanged. If another repository turns out to need changes, stop and request an
  explicit scope change instead of adding it. **Every modified repository gets
  its own verification, hook state, Git history, and proposed `[<ProjectID>]`
  commit message**; unmodified repositories get none, and no proposal ever spans
  repositories.
- **Registered capabilities** — repository layout is not topology;
  `TigerAiCore.toml` is. Before claiming a Tiger Lab or tool is unavailable,
  guessing its location, or implementing a replacement, resolve the registered
  capability from the machine configuration named by `TigerAiCoreConfig` —
  `pwsh -File <TigerAiCore>/tools/Resolve-TigerAiCoreResource.ps1 -Lab <name>`,
  `-Tool <name>`, or `-Project <ProjectID>`. Never guess a sibling directory,
  assume a drive layout, scan the filesystem, invent a per-Lab discovery
  variable, or copy topology into a project. An explicit caller-supplied path
  remains a valid override where a public interface already accepts one; it is
  an override, not a second discovery system. Without the configuration,
  repository-local coding, builds, tests, and documentation continue; only
  capabilities that need a registered Lab or tool are unavailable, and that is
  not a project failure.
- **Consumer registry** — `[projects.<ProjectID>]` records that a local
  repository consumes TigerAiCore and where its root is, so the machine knows
  its consumers without anyone hand-editing the configuration or scanning
  disks. Bootstrap and synchronization keep the current repository registered;
  the entry is machine-local inventory, is written only by
  `Manage-TigerAiCoreConfig.ps1`, and is independent of `[labs.*]` and
  `[tools.*]` — one repository is often both a consumer and a registered
  capability, and those are different facts that are never merged. A conflict —
  a second existing path claiming one project id, a malformed entry — is
  reported, never silently resolved. **Registration is discovery, not
  authorization**: it supports read-only ecosystem questions and never makes a
  registered repository writable.
- **Lab boundaries** — Labs are generic providers. A Lab exposes parameterized
  capabilities and may document its known consumers, but consumer-specific
  folders, scripts, identities, and configuration belong in the consuming
  project. Labs provide platform capabilities; consumers provide product
  meaning — product semantics, product-specific orchestration, and acceptance
  assertions are the consumer's. Before extending a Lab, use its existing
  generic public interface, then ask whether a genuinely generic platform
  capability is missing: *would this capability still make sense if the current
  consumer did not exist?* If not, it belongs in the consumer. A missing
  generic capability is reported and implemented as a separate task in the
  provider repository, never as a side effect of consumer work.
- **Lab invocation** — a consumer invokes a Lab entry point as a child process,
  so the Lab's exit is a result rather than the end of the caller. The caller
  passes the path the Lab must write its machine-readable result to instead of
  discovering a run id or parsing shared output, defines and interprets the
  Lab's exit-code contract, and allows generous headroom beyond the Lab's own
  timeout because teardown continues after it fires. A missing or unreadable
  expected result is a failure, never a success.
- **Windows acceptance** — automated Windows GUI and system acceptance runs in
  TigerWinLab, not on the Architect's or a developer's live desktop. Resolve
  the Lab through the configuration, read its public consumer interface, and
  compose the consumer-owned payload and assertions; extend the Lab only when a
  genuinely generic Windows capability is missing. Do not drive pointer or
  keyboard input on a live desktop, require the Architect to leave their
  machine untouched, write one-off Hyper-V scripts inside a consumer, or
  rebuild DPI, theme, elevation, network, input, or evidence machinery per
  project. Manual Architect inspection remains a separate deliberate action.
- **Secrets and access** — use only explicitly granted resources; never store
  credentials, tokens, or keys in plain text anywhere in the repository.
- **Licensing gate** — load the Architect-owned TigerAiCore
  `LicensingPolicy.toml` plus any explicit project-root override. Check the
  actual licence, material terms, distribution implications, domain, and
  attribution decision before material technology or asset investment. Unknown,
  missing, ambiguous, or extra/custom terms never silently pass; genuine `OR`
  alternatives may select and record one approved option, while every `AND`
  component must pass. Re-evaluate updates against the accepted state, and
  surface any licence or material-term change as an Architect gate even if the
  new terms otherwise match an automatic rule. Agents may enforce policy but
  never add, remove, weaken, replace, or work around a licensing rule without
  explicit Architect approval.
- **Preferred technologies and formats** — use Tiger preferred technologies by
  default; deviate only for a concrete project requirement or a materially
  better engineering outcome, and never "correct" an established project
  choice unasked. Policy: TOML and JSON are the only preferred native
  structured-data formats; YAML is not a Tiger-owned file format and must not
  be chosen for new internal configuration or data — use it only where an
  external integration requires it. Preference: Fluent UI System Icons for
  Windows UI where suitable. A preference never overrides the licensing gate.
- **Verification** — verify with the strongest practical automated checks;
  aim for a clean build and green tests; distinguish a pre-existing dirty
  baseline from new failures; state clearly what could not be verified and why.
- **Open-loop work** — when verification is unavailable, become more
  conservative, not more creative. **Open loops must be kept as small as
  possible**: run it if possible; reuse mechanics a Tiger project has already
  proven; validate everything else locally — scripts, syntax, dry runs, mocked
  inputs, API shapes, the external environment's known differences reproduced;
  research what still cannot run in official documentation and proven runs
  before writing it; and leave only a minimal, isolated fragment as the next
  external run's single new uncertainty. **Close one external loop before
  opening the next.** A list of open loops is a risk register, not readiness;
  an expensive external run is the last step, not a debugging mechanism, and
  the Architect is not the probe that discovers whether automation works.
  **An external step must provide material evidence that cannot reasonably be
  obtained in the closed loop**: duplicating local tests in hosted CI is not
  additional validation by itself; the number of workflows is not a measure of
  open-loop size, and one workflow running a 20-minute suite is still a large
  open loop; hosted CI is not a substitute for the project's authoritative
  acceptance infrastructure; and the Architect must not pay a long
  hosted-validation cost after every ordinary commit for validation already
  required before handoff. Review each external step for the uncertainty it
  closes, why local validation is insufficient, and its expected external
  cost — no justification, no step.
- **Loop economics** — **Open/closed describes observability. Cheap/expensive
  describes iteration economics**, and a closed loop is not automatically an
  efficient one. A loop is expensive when the next meaningful result costs
  materially in elapsed time, compute, AI usage, environment setup,
  coordination, or Architect attention. **When the loop is expensive, make every
  iteration earn its cost**: use the cheapest reliable loop that can answer the
  current question, falsify the obvious causes with cheaper reliable checks
  first, and do not repeat an expensive run unchanged without evidence that
  justifies repeating it — while the expensive acceptance gate that trustworthy
  completion requires still runs. Observation has a cost too. **Do not poll an
  expensive loop more frequently than it can reasonably produce new evidence**:
  prefer a blocking wait, lifecycle-driven completion, a completion sentinel, or
  one structured result read, because **checking again is not new evidence**.
  **For expensive loops, maximize evidence per iteration and per observation.**
  Cheap local loops need none of this ceremony.
- **Required gates** — **a pre-existing failure may show that the current change
  did not introduce a regression; it does not turn a failing required gate into
  a passing gate.** Completion requires every applicable required verification
  gate to pass deterministically, unless the project has explicitly defined that
  gate as non-required or deliberately quarantined it. **Known flaky,
  nondeterministic, or environment-dependent required tests are verification
  defects** — fix them, move the assertion to a stable contract boundary, or
  quarantine them with a documented reason and owner, rather than normalizing
  the failures as "the baseline". Verify the contract at the most stable
  available boundary — structured properties, JSON fields, exit codes, durable
  result artifacts — rather than console rendering, ANSI colour, or terminal
  width. **Do not pay for an expensive gate when a cheaper reliable gate already
  proves the candidate is not ready.** **Green means every applicable required
  gate passed with trustworthy evidence**; report anything less as what it is,
  and never under "none".
- **Background work ownership** — background work the task starts belongs to
  the task. Before reporting completion, every process, job, monitor, waiter,
  worker, or agent it started must be completed, explicitly terminated, or
  deliberately handed off and named in the response; unaccounted task-created
  background work means the task is not complete. **Background accounting is
  registry-based, not process-list-based**: account against the session's own
  record of what this task started and resolve each unit, because one whose
  process has died without reaching a terminal state leaves the process list
  while staying unaccounted for. A process sweep is supplementary evidence, not
  the accounting authority, and where it helps it is scoped by the resources the
  work touched rather than by executable names — orphans are defined by task
  ownership and touched resources, not by executable identity. A monitor ends on
  the lifecycle of what it watches — lifecycle is authoritative, output is
  descriptive — and cleans up on failure, timeout, and cancellation too, so a
  missing marker never leaves an orphan waiter. **A monitor timing out does not
  stop the pipeline it watches**; the timeout ends the observation and says
  nothing about the underlying run, whose lifecycle must be established
  separately. **Background work started by a subagent stays owned by the Lead and
  the session** until it is terminal or explicitly handed off; a worker's "done"
  is a claim about the worker. This covers what the task started, not activity it
  did not start, and a task that started no background work owes no extra
  reporting.
- **Final response** — structured for fast Architect scanning, and ending with
  a `Proposed commit message` section whenever repository contents changed.
- **Commit message** — the subject begins with `[<ProjectID>]`, and the rest is
  proportional to the change and written for `git log`. A single-line subject
  is complete when it fully describes a small, obvious change; a short body is
  for durable context the subject and diff do not give, such as rationale,
  scope, non-obvious behavior, or accepted trade-offs. Never restate the
  implementation, the changed-file list, or the verification report there; that
  detail belongs in the final response.
<!-- TigerAiCore:end -->

## Project-specific instructions

This section is TigerSetup-specific. Shared bootstrap, action-mode, autonomy,
escalation-format, human-gate, verification, and reporting rules come from the
inherited TigerAiCore rules above and from the role instructions loaded during
bootstrap; they are not repeated here.

Documentation philosophy for this project: **current product truth belongs to
the design documents; history belongs to Git.** Keep canonical documents compact
and actively maintained rather than accumulating parallel planning files.

### Repository contents and toolchain

TigerSetup is a Rust tool that builds small, self-contained Windows `Setup.exe`
installers from a declarative `TigerSetup.toml`, with SQLite-backed
transactional installation state. The repository holds the design documents
and a Cargo workspace (`crates/`): `tigersetup-format`, `tigersetup-engine`,
`tigersetup-catalog` (the WinGet catalog client shared by builder and engine),
`tigersetup-loader` (the C Win32 loader every generated `Setup.exe` begins
with, `tigersetup-loader.exe`, compiled and linked by its build script),
`tigersetup-setup` (the engine, `tigersetup-setup.exe`),
`tigersetup-build` (the builder, `tiger-setup.exe`),
`tigersetup-test-prereq` (`TigerSetupTestPrereq.exe`, the controlled
prerequisite installer the synthetic test package embeds),
`tigersetup-test-action` (`TigerSetupTestAction.exe`, the controlled program
its custom actions and quiescence entries run) and `tigersetup-test-launch`
(`TigerSetupTestLaunch.exe`, the controlled GUI program the launch-after-install
tests and lab rows offer, which reports how it was started), plus `proto/` (the
runtime-metadata schema), `packages/` (the packages it builds), `lab/` (the
TigerWinLab driver), `eng/` (developer tooling: the cleanup script and its
test, documented in `README.md`; `TigerAiCore.psm1`, the one place the
scripts resolve a registered Lab or tool; and `eng/release/`, the release
tooling `RELEASING.md` describes), `.github/` (the release workflow, the
elevated-runner test diagnostic started by hand, and the per-version release
notes), `docs/TigerSetup-Help.md` (the
installed getting-started help), `docs/assets/` (the project artwork,
including the `TigerSetup.ico` both executables compile in) and `benchmark/`
(experiments that measure the product without being part of it: the
installer-technology benchmark that generates `report.md`, and
`benchmark/compression-spike/`, the payload-compression spike with its own
Cargo workspace, corpus scripts and generated report; each has a `README.md`
and nothing under `crates/` depends on either). Disposable spike code,
when any exists, lives under `spikes/` or under `benchmark/` as an
experiment, and nothing outside it may depend on it.
Toolchain: stable Rust for
`x86_64-pc-windows-msvc` with a static CRT (`.cargo/config.toml`), the Visual
Studio C++ toolchain (`cl.exe`, `link.exe`, `rc.exe`) for the C loader,
`rusqlite` bundled, `prost` + `protox` (no `protoc`), `zstd` (libzstd, the
one compression technology, for the engine block, the payload and the
metadata block; the loader compiles its decoder from the same crate's
sources), `zip` and `flate2` (the WinGet pre-indexed source and `inspect
--output-zip` only), `yaml-rust2`, WinHTTP through `windows-sys`; PowerShell
7 for `lab/` and `packages/`.

The verification gate every change must pass:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # includes process-level crash-recovery tests (several minutes)
cargo test -p tigersetup-engine --release --lib win::   # the hand-declared COM code as the release engine runs it (LESSONS_LEARNED.md)
cargo build --release             # binaries under target\x86_64-pc-windows-msvc\release\
pwsh -File lab\Test-LabScripts.ps1   # the lab driver parses, reads no variable that is not there, and its help source check holds
pwsh -File eng\release\Test-Release.ps1   # the release tooling, against synthetic repositories
```

The gate is the coder's, and it runs here, before the handoff. No hosted workflow
repeats it on a push or as a release prerequisite (`RELEASING.md`).

Before reporting completion, one more check that is not a build step. **Read the
task registry** (`/tasks`, or the task tools) and stop every background shell,
waiter, monitor and subagent this task started that has not reached a terminal
state — including those a subagent started, which are registered against this
session. Then **sweep by what the work touches** — the repository path, the
session scratchpad, the Lab, a probe name — rather than by a list of executable
names, because the orphans are usually a `tail`, a `grep` or a `find` that no
such list contains. A monitor reporting "timed out" has not stopped its
pipeline. The two checks answer different questions and neither substitutes for
the other (`LESSONS_LEARNED.md`). **A Lab session is the third question**, and
no process list answers it: a run opens one TigerWinLab session and ends it in
a `finally`, and a killed run leaves that session open — its lease is
recovered by the Lab once the process is gone, and its preserved VM after the
Lab's bound, but the session record stays open until
`Close-TigerWinLabSession.ps1 -SessionId <id>` ends it, and a VM the Lab could
not normalize stays `Faulted` until `Reset-TigerWinLab.ps1` recovers it. Ask
`Get-TigerWinLabSession.ps1`, which answers from the Lab's own records.

The lab-script gate is cheap and belongs *before* a lab run rather than after:
everything under `lab/` runs with `Set-StrictMode -Version Latest`, where a
variable that is not there ends the row, and a row that dies on one has already
spent minutes of guest time to say so.

**Build order matters before a lab run:** `cargo build --release`, then the
installers, then the rows. A row measures the engine and the loader
*embedded in the installer*, and the builder takes them from
`tigersetup-setup.exe` and `tigersetup-loader.exe` beside itself — so
rebuilding an installer does not pick up an engine change, and a whole matrix
can be evidence about the wrong engine without saying so. The lab drivers
refuse a mismatch before spending guest time on it (`LESSONS_LEARNED.md`).

`tiger-setup build` has two modes. The default is release quality and is what a
published installer is built with; `--fast` skips the compression search for the
edit-build-test loop and produces a functionally identical, larger installer
(`TigerSetup-Design.md` §10.4). Use `--fast` while iterating, and the default
whenever the bytes will be measured, validated or published — the exact-artifact
principle means the bytes that pass validation are the bytes that ship.

`packages/test-app/README.md` builds the synthetic two-version package the
transactional tests and lab rows use; `lab/README.md` runs the lab rows.
Integration tests redirect `%LOCALAPPDATA%` and write only under `target\`.

### Version and release discipline

TigerSetup has one product version, in the workspace `Cargo.toml`
(`[workspace.package] version`), and every version-bearing artifact reads it
from there: the engine and builder VERSIONINFO through the build scripts, the
embedded runtime metadata, and the self-hosted installer's product metadata.
Do not write the version down a second time; README's statement of the
current version and its installer examples are the deliberate exception, and
the release gate checks the statement.

The version is strictly `<Major>.<Minor>.<Patch>`, and its parts have owners:

- a bug fix or a small backwards-compatible feature bumps **Patch**;
- a larger backwards-compatible feature bumps **Minor**;
- **Major** is an Architect decision only. **An agent never increments Major
  on its own** — surface it as an escalation instead.

Where Windows VERSIONINFO needs a four-part file version, pad the three-part
version with `.0` (`0.5.0` → `0.5.0.0`); the public version stays three parts.

**Every coding session that changes the product ends with a release turn**, in
this order, after the normal gate above:

1. bump the version by the rule above — normally near the end, once the size
   of the change is known, and always before the final artifact is built;
2. build the release-quality binaries (`cargo build --release`);
3. build a new release-quality self-installer with TigerSetup itself
   (`pwsh -File packages\tigersetup\Build-Package.ps1`; `packages/tigersetup/`),
   which also renders the installed help's PDF with the registered `tiger-mark`;
4. verify that installer (`tiger-setup inspect --json`, or `verify`), and
   report its artifact path and the version and engine identity it carries;
   a candidate validation also runs it end to end in the lab
   (`pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath <artifact>`:
   quiet install, the installed `tiger-setup --version`, `verify`, quiet
   uninstall, nothing left behind, in each scope; and the presence rows: the
   Start Menu folder, TigerSetup Shell and both help forms on the interactive
   desktop with the PATH option off, in each scope).

That installer is a **local candidate**: it proves the session's change, and it
is never published. A release is built only by the `Release TigerSetup`
workflow from the release commit the Architect pushed, and goes through the
lifecycle in `RELEASING.md`. "Prepare release <version>" is that lifecycle's
first stage: the release turn above, the release notes and the version
references, and no commit, push or tag.

`--fast` is for the engineering loop only; it never satisfies the final
artifact requirement. This applies to coding sessions that change the product;
a Review, Documentation or other non-coding mode changes the version only under
an explicit instruction to do so.

### Project documents

Each topic has exactly one owning document. There is no precedence chain and no
addenda: where a detail appears in more than one place, the owning document is
authoritative.

- `README.md` — the project entry point, written for a developer who wants to
  build an installer: what TigerSetup is, how to get `tiger-setup`, the first
  installer, the manifest, common packaging tasks, scopes, upgrades,
  dependencies, branding, automation, diagnostics, supported Windows.
- `TigerSetup-Design.md` — the product: positioning, non-goals, what it
  replaces, architecture (manifest/database, state vs journal, crash
  consistency, transactional guarantees, ownership, security, migration,
  cross-scope policy), execution model, dependencies, distribution, metadata
  integration, platform baseline, the single-file installer format, UI,
  localization, technology choices, scope, guiding principles, open questions.
- `TigerSetup-Validation.md` — the proof: validation levels and which of them
  is acceptance, fault injection, the acceptance standard, the acceptance
  matrix, the UI matrix, and the TigerWinLab boundary.
- `TigerWinLab-Requirements.md` — the lab contract: how TigerSetup consumes
  TigerWinLab, the observable lab behaviour the acceptance depends on, and the
  lab facts that shape the matrix.
- `RELEASING.md` — how TigerSetup is released: the release set and its
  record, where each kind of artifact lives, the stages and who acts at each,
  recovery, and the `eng/release` tooling. The common model is TigerAiCore's
  `docs/release-model.md`.
- `LESSONS_LEARNED.md` — the project's lessons, under the inherited rule.
- `THIRD-PARTY-NOTICES.md` — material redistributed inside a generated
  `Setup.exe`, with its licence notices.
- `docs/TigerSetup-Help.md` — the public getting-started help installed with
  TigerSetup (and its PDF, rendered from it at package build time), written for
  a person who has just installed it: what it is, how to start it, the first
  commands and where to go next. It tells the same story as the opening of
  `README.md` and the WinGet description; keep internals out of it.

Planning documents are temporary: retire each section as its work lands, and
never let source, tests, scripts or design documents depend on one.

The summaries below are a deliberate high-salience cache of the constraints that
bind every task. Where they are shorter than the owning document, read the
document.

### Core architecture (the parts that constrain every decision)

**Manifest is intent; database is reality.** `TigerSetup.toml` declares desired
state. A per-installation SQLite database
(`%ProgramData%\TigerSetup\<ProductId>\state.db` for machine scope,
`%LOCALAPPDATA%\...` for user scope) records what is actually owned. Uninstall
and upgrade plan from the database, never by inverting the current manifest.

**Two separate concepts:** *installation state* (committed ownership: files,
registry values, PATH entries, shortcuts) and *transaction journal* (the
in-progress attempt with durable undo information). The journal uses many short
durable SQLite commits — not one long-held transaction — because SQLite cannot
make the filesystem and registry transactional. The hard invariant: **durable
undo state must be written before the Windows mutation**, so a crash between the
two is recoverable. The operation state machine is
`planned → prepared → applying → applied`, with reconciliation by resource
inspection on restart.

**Success or full rollback.** A transaction ends installed or fully rolled back.
*Recoverable failure* is a legitimate intermediate state — durable state must
always allow convergence to one of the two ends — but an unknown or hybrid
installation is not. Upgrade is the case that matters: `A → B`, or back to a
valid complete `A`, never a mixture. Dependencies sit outside the product
transaction and normally stay installed when it rolls back.

**One file, inspectable, transaction-optimized.** A generated installer is a
single executable: `[small loader][zstd-compressed engine][one solid zstd
payload][Protocol Buffers metadata][fixed footer]`. The loader decompresses
and verifies the engine and runs it against the original file; the footer
identifies the format and points directly at the other blocks; the metadata
is the build-time-resolved form of `TigerSetup.toml`, which is developer-facing
source and is not shipped, plus the payload index (name, offset, length,
CRC-32, SHA-256 per entry). The format is meant to be decomposed and verified
without executing it — no obfuscation, no encryption. Integrity is SHA-256 of
each block and of the decompressed engine, and the index's per-entry CRC-32
and SHA-256. Code signing is outside the core design. **TigerSetup optimizes
for the shortest reliable installation transaction**: the payload is decoded
inside the transaction, so Zstandard was chosen over LZMA2's smaller output
(`TigerSetup-Design.md` §10.4); do not reopen that for a ratio.

**Running applications follow Windows conventions.** Restart Manager and
`RegisterApplicationRestart`, not a TigerSetup-specific shutdown protocol.
Applications save and restore their own state; TigerSetup coordinates
quiescence before mutation and restart afterwards. Whether the run may go on
is decided by who still holds the files — asked of the process, not of the
Restart Manager's list — over a bounded grace period, and only a file
something holds is put to the Restart Manager at all: one that refuses a
`DELETE`-and-write open, because the image of a running program shares
delete and refuses only the write. A package whose application the Restart
Manager cannot close declares `[[quiescence]]`: a stop program run before the
Restart Manager, a resume program started detached afterwards, and TigerSetup
resumes only what it stopped, on every path that leaves the product installed
— a refusal included — and never after an uninstall (`TigerSetup-Design.md` §5.10).

**One engine, two clients.** Unattended CLI and interactive UI both feed the
same desired-state → plan engine → transaction engine pipeline. There must never
be a second installation implementation living in the UI.

**Typed operations, not scripting.** `InstallFile`, `SetRegistryValue`,
`AddPathEntry`, `CreateShortcut`, etc. Each is journaled and reversible. Resist
arbitrary script execution unless a concrete requirement proves typed mechanisms
insufficient.

**Ownership is conservative.** Do not delete a modified owned file, do not claim
a PATH entry that pre-existed, do not recursively delete directory contents
TigerSetup did not install, prefer registry ownership at value level.

**Dependencies are requirements, not owned resources.** Installing WebView2 for
TigerMarkView does not mean uninstalling TigerMarkView removes WebView2. The
dependency model separates identity / detection / acquisition / installation /
verification.

**WinGet-aligned, WinGet-independent at runtime.** Use WinGet package identities
and metadata at *build* time; the generated `Setup.exe` must never require
`winget.exe` on the target machine — nor Rust, .NET, PowerShell 7, Chocolatey,
Windows App SDK runtime, or TigerSetup itself.

**Machine-readable output is language-independent.** Human text may be
localized; `inspect --json` / `verify --json` style output must use stable
identifiers (e.g. `"code": "dependency_missing"`) so agents and CI never parse
localized strings.

**Interactive elevated children are started shown; quiet dependency
installers are started hidden.** A process's first window follows the show
state it was started with, so an elevated wizard child started hidden would
wait invisibly for a click.

### Hard requirements to check work against

- Platform baseline: Windows 10 1809 x64 / Windows 11 x64 / Windows Server 2019
  x64+. Server 2016 is out of scope.
- Offline: the engine must never need the Internet to run. If dependencies are
  already satisfied, installation must succeed with no connectivity. If a
  dependency is missing and cannot be acquired, fail cleanly with no partially
  committed application install.
- Localization is first-class: `en-US` and `pl-PL` both tested; strings
  separated from engine and UI logic; English is the guaranteed fallback. Full
  `en-US` + `pl-PL` validation runs on the primary Windows 11 platform; the
  Windows 10 22H2 and Server 2019 compatibility rows are `en-US` only.
- DPI awareness is a hard requirement, tested at 100/125/150/200%; light and
  dark themes are both the product.
- UI must be native, small, fast, Fluent-aligned, with **no** Windows App SDK
  runtime dependency. "Small, neat and fast — not beautiful bloatware."
- Build once, validate exact bytes, publish those exact bytes. Never rebuild an
  artifact for WinGet submission.

### Acceptance standard

TigerSetup is acceptable only while a TigerSetup-generated **TigerMarkView**
installer passes the automated TigerWinLab matrix of
`TigerSetup-Validation.md` §5.2: clean VM → install → verify → upgrade →
verify → uninstall → verify absence, across user/machine scope, silent
operation, PATH, ARP registration, the four dependency-presence combinations,
offline scenarios, interrupted-install recovery, the running-application
upgrade, legacy migration, and the genuine UAC handoff. Producing an `.exe` is
not the criterion; passing replacement validation is, and reducing the matrix
is an Architect decision.

**Supporting evidence is not acceptance evidence.** Unit and local integration
tests gate a change before the expensive rows run; only the lab rows, on the
exact bytes, establish that the product is acceptable on Windows. Where a
requirement is end to end, acceptance exercises the actual transition: testing
the pre-consent and post-elevation halves of the UAC handoff separately does not
prove the handoff.

Real applications define scope: a requirement repeated across real applications
is evidence for a TigerSetup primitive; a one-off installer trick is not.

### Project boundary: TigerWinLab

TigerWinLab is a separate project and, as an instance of the inherited ecosystem
write boundary, **must not be modified from this repository**. What is specific
here: TigerSetup may assess TigerWinLab's capabilities and document what it
needs in `TigerWinLab-Requirements.md` — written as required observable
behaviour, not prescribed implementation — and must treat missing lab capability
as an external blocker. Do not quietly implement lab features here.

### Working model in this repository

Work here follows an Architect (human) → Lead Coder (main session) → bounded
subagents model. Relevant to how you should operate:

- **Delegation and model policy.** The Lead Coder is Fable 5.1; subagents run
  on Fable 5.1, Opus 5 or Sonnet 5; Haiku is not used; at most three subagents
  run concurrently. Inside those limits, model choice and delegation shape are
  the Lead Coder's own execution decisions — choose the model per task from
  complexity, uncertainty, context needs and consequences of failure, and do
  not parallelize merely because parallelism is available. Record what was
  used and how it performed. This is the project policy the inherited
  `Autonomous Development` mode reads.
- Keep the subagent set small and current — the definitions under
  `.claude/agents/` (implementer, researcher, reviewer) are the set; no unique
  agent per task, and no role kept past its usefulness. Subagents get bounded
  tasks with a timeout chosen for that task and return findings, diffs and
  evidence; a timeout or a stalled worker is a diagnostic event, never an
  automatic retry.
- Only the Lead Coder integrates results and updates canonical design documents
  — subagents do not rewrite them.
- Modifying subagents work in isolated Git worktrees. Worktrees do not isolate
  TigerWinLab: E2E work against the single mutable lab must be serialized. A
  worker pinned to a worktree by instruction can still drift, so the Lead
  checks the main checkout's `git status` before integrating any worker's
  result (`LESSONS_LEARNED.md`).
- A delegated task may not leave background work behind, and a worker's word
  about its own cleanliness is not evidence: the Lead checks the task registry
  and the Lab's session records itself (see the completion check above).
- Fault injection is a real, retained testing capability around journal/mutation
  boundaries — not throwaway debug code.
- **DRY** here means one authoritative implementation per stable concept —
  journaling, the operation model, resource ownership, configuration loading,
  localization lookup and error reporting each exist once; duplication
  introduced by parallel workers is consolidated at integration, and
  coincidental similarity is not generalized. **KISS** is measured across the
  whole system — developer experience, end-user UX, architecture,
  maintainability — with developer-facing simplicity weighted highest:
  TigerSetup willingly contains sophisticated internals when that removes
  recurring installer code, configuration or decisions from every consuming
  project, and never pushes complexity into applications to keep its own
  internals simple. Sophistication is not a licence for scope growth; "do not
  become MSI by accident" still binds.

### Project-specific escalation triggers

In addition to the inherited autonomy boundary, and using the inherited
escalation format, **escalate to the Architect** (rather than deciding) when
work would: materially change the product model; weaken a stated invariant;
expand scope beyond `TigerSetup-Design.md` §14; introduce arbitrary scripting
where typed operations were expected; change the ownership or
dependency-lifecycle model; make a generated installer require WinGet at
runtime; reduce the acceptance matrix or move the acceptance standard; add a
significant external dependency; contradict a recorded Architect decision; or
require a policy/security trade-off rather than an engineering choice. Ordinary
implementation choices inside the approved design do not need escalation.
