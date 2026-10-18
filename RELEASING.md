# Releasing TigerSetup

TigerSetup is released through the Tiger release model (TigerAiCore
`docs/release-model.md`): one release commit, one build of it, a record of the
exact bytes, validation of those bytes, and publication of those bytes. This
document is TigerSetup's side of that model: what a release consists of, where
each kind of artifact lives, and how the stages run here. The tooling is
`eng/release/`; the version rules are in `AGENTS.md` (*Version and release
discipline*).

TigerSetup uses the permanent development branch workflow (`AGENTS.md`, *Git
workflow*; the model's *Projects with a permanent development branch*): a
release is prepared on `dev`, the Architect promotes `dev` to `main`, and
everything from the release commit on — the release action, the tag, the
release — happens on `main` only. Nothing is ever released or tagged from `dev`.
Ordinary work on `dev` starts none of this; a release begins when the
Architect asks for one to be prepared.

## What a release is

The release of version `<v>` is the **release set** the `Release TigerSetup`
workflow (`.github/workflows/release.yml`) builds from the release commit and
attaches to the GitHub Release `TigerSetup <v>` at the annotated tag `v<v>`:

| File | Kind | What it is |
|---|---|---|
| `TigerSetup-<v>-Setup.exe` | `WindowsInstaller` | the self-hosted installer (`packages/tigersetup/`) |
| `TigerSetup-<v>-WinGet.zip` | `WinGetManifests` | the WinGet manifest set for `ItTiger.TigerSetup`, generated from that installer's bytes and its public URL |
| `LICENSE.txt` | `License` | the licence that applies to this release, frozen with it |
| `PRIVACY.md` | `PrivacyStatement` | the privacy statement that applies to this release, frozen with it |
| `release-artifacts.json` | record | the version, the commit, and the kind, length and SHA-256 of the four files above, in this order (schema 1) |
| `SHA256SUMS.txt` | record | the same hashes in `sha256sum` format, in the same order |

The set is closed: a release directory, workflow artifact or draft with a file
the record does not name, or without one it does — the licence and the privacy
statement included — is refused at every stage.

The record cannot vouch for itself; the tag does. `v<v>` is an annotated tag
that only the release workflow creates, as `github-actions[bot]`, at the release
commit, and its message names the SHA-256 of that release's
`release-artifacts.json` and the run that built it. Every later stage proves a
retrieved set against it, and proves from GitHub's own record of that run that
it was the release workflow, started on `main`, for that commit.

The installer is published at
`https://github.com/rkozlowski/TigerSetup/releases/download/v<v>/TigerSetup-<v>-Setup.exe`,
the URL its WinGet manifests name. The notes come from
`.github/release-notes/<v>.md`.

### Release-bound terms

TigerSetup is an application, so every release carries both release-bound
terms of the release model: the licence and the privacy statement that apply to
exactly that release, frozen with it.

- **What is frozen.** `LICENSE.txt` and `PRIVACY.md` are the exact bytes the
  release commit holds for those files in Git (`git cat-file blob
  <commit>:LICENSE.txt`), not a checkout's copy, whose line endings depend on
  its configuration — so anyone can reproduce them from the commit. The
  installed `LICENSE.txt` is the same text as a Windows checkout writes it, as
  every shipped text file is.
- **Where it is published.** Each is an asset of the release, at
  `https://github.com/rkozlowski/TigerSetup/releases/download/v<v>/LICENSE.txt`
  and `.../v<v>/PRIVACY.md`. The URL is fixed by the tag before the release
  exists and never serves another version's file.
- **What names it.** The release's WinGet manifests give those URLs as
  `LicenseUrl` and `PrivacyUrl`. `packages/tigersetup/TigerSetup.toml` declares
  them with `{version}`, and the build refuses a manifest set whose terms URLs
  are anything else — a branch, a repository view, another version.
- **When they are settled.** In preparation (stage 1), before the release
  commit: the licence, and that `PRIVACY.md` describes what this release
  records, sends and removes. After the build they are only proved unchanged,
  never reviewed or fixed; a finding against their content ships with the next
  version. `PRIVACY.md` on `dev` is the statement being prepared for the next
  release.

Nothing else is the release. In particular:

- **A local candidate** is any set built on a developer machine —
  `packages\tigersetup\Build-Package.ps1` into `artifacts\tigersetup`, or
  `eng\release\Build-ReleaseArtifacts.ps1 -Rehearsal` into
  `artifacts\release-candidate\<v>`. It is evidence for the preparation, never
  a release, even when built from the release commit.
- **The workflow artifact** carries the set between the workflow's jobs and
  expires after 30 days. The draft or published release is the durable copy.
- **`artifacts\release\<v>`** is the release set as retrieved from GitHub by
  `Get-ReleaseArtifacts.ps1`, proved, and unpacked for validation;
  **`artifacts\release-rehearsal\<v>`** is the same script's working copy of a
  set on disk.

Rebuilding is never a substitute: the installer carries a help PDF rendered on
every build, so two builds of one commit differ even where the engine and loader
are byte-identical, and only the bytes that were validated may be published.

## Where each check runs

The release commit freezes the release, and the release set is never rebuilt,
so a check runs before the freeze whenever its failure could change the product
or the release inputs, and after the build only when it proves something the
build alone produced. Preparation is the release's verification; release
validation confirms that the workflow built, recorded and packaged the
accepted commit faithfully and that its binaries work.

| Check | Stage | Why there |
|---|---|---|
| the verification gate, every line (`AGENTS.md`) | 1 | the release builds all of it |
| release acceptance of the self-installer: every row of `lab/Invoke-SelfInstallerRows.ps1` and the elevation rows, on the candidate | 1 | each asserts the version, so every release reaches them |
| the §5.2 matrix and the other product rows | 1 | each stands as evidence for the candidate, or runs again where a change since can reach it (`TigerSetup-Validation.md` §5.2) |
| the release-bound terms, reviewed against the release's behaviour | 1 | a finding changes the commit |
| the release action's own prerequisites, the build script and every check release validation will run, on a candidate | 1 | a tool's first real run must not be on the release |
| the runner and the repository's settings | 1 | a missing tool or setting must stop preparation, not the build |
| ref, version, notes, terms, commit on `main`, tag free | 3, before the build | the only facts the commit could get wrong after preparation |
| the runner's tools, then `tiger-mark` rendering | 3, before the release build | seconds, against the minutes the build costs |
| the installer against the engine and loader just built, the manifest set's terms URLs, the record | 3, in the build | they describe the bytes being made |
| the set received is the set recorded; the run is on `main`; the terms are the commit's | 3, before the tag | the tag is the first thing written |
| integrity, provenance, run, terms, manifests, installer, help, `winget validate` | 4 | only the release's own bytes can show them |
| one Windows row on the release's bytes (`moderator`) | 4 | the runner's binaries, help and manifest set, used as a person and WinGet use them |
| the public URLs serve the exact bytes | 6 | only after publication |

## The stages

| | Who | What |
|---|---|---|
| 1 | coder | prepare the release on `dev`: inputs, gates, acceptance of a candidate, the rehearsal of release validation, the runner preflight, readiness |
| 2 | Architect | review `dev` and promote it to `main`: the release commit |
| 3 | Architect | start `Release TigerSetup` with the version |
| 4 | coder | retrieve the draft's release set, prove it, and run the one lab row on it |
| 5 | Architect | review and publish the draft |
| 6 | coder, then Architect | prepare the WinGet submission; open the pull request |

### 1. Prepare

The coder, on the Architect's "prepare release `<v>`", leaves `dev` holding a
release that has already passed everything that can fail before the freeze,
and the evidence for it:

1. **Inputs.** Set `[workspace.package] version` in `Cargo.toml` (the build
   updates `Cargo.lock`), README's "TigerSetup is at version" line and its
   installer examples, and the `default` of the `version` input in
   `release.yml`, so the release form opens prefilled with the version to
   confirm. Write `.github/release-notes/<v>.md`: a `# TigerSetup <v>` heading
   and the user-facing changes, install and verification sections. Settle the
   release-bound terms: `LICENSE.txt`, and a `PRIVACY.md` that describes this
   release's behaviour — reviewed again whenever the release changes what
   TigerSetup records, sends or removes. Update the documentation the release
   changes.
2. **Cheap gates.** Every line of the verification gate (`AGENTS.md`): at the
   release boundary they all apply, because the release builds all of it.
   Then `pwsh -File eng\release\Assert-ReleaseCommitReady.ps1 -Version <v>`,
   the release workflow's own first gate: before the promotion its
   `commit/on-main` is BLOCKED and its `ref` NOT RUN, and everything else
   passes.
3. **The candidate**, as the workflow builds it. Build `tiger-mark` from the
   pin into an empty directory — `pwsh -File eng\release\Build-TigerMark.ps1
   -Directory <empty directory>`, which proves it renders — then
   `pwsh -File eng\release\Build-ReleaseArtifacts.ps1 -Version <v> -Rehearsal
   -TigerMarkPath <the tiger-mark.exe it built>`: the release binaries, the
   release-quality self-installer, its checks and the closed set in
   `artifacts\release-candidate\<v>`. Report its path, its version and its
   engine's identity and size against `benchmark/README.md`.
4. **The rehearsal of release validation** on that set, with the script stage
   4 runs: `pwsh -File eng\release\Get-ReleaseArtifacts.ps1 -Version <v>
   -ArtifactDirectory artifacts\release-candidate\<v> -Rehearsal`. Every check
   but those that need the GitHub Release and the tag runs — the record,
   terms, manifest set, installer, help and `winget validate` — and they are
   NOT RUN, named. The retrieval half is rehearsed on the previous release,
   whose set the workflow built: `pwsh -File eng\release\Get-ReleaseArtifacts.ps1
   -Version <previous>` must pass — the API download a draft uses, the
   digests, the tag, the run, the terms and the public URLs — with the
   tooling being released.
5. **Release acceptance** on the candidate's bytes, in TigerWinLab:
   - every row of `lab/Invoke-SelfInstallerRows.ps1` — `user`, `machine`,
     `user-nopath`, `machine-nopath`, `upgrade` from the previous release's
     published installer, `winget-user`, `winget-machine` and `moderator`,
     with `-SourceWorkingTree` and the candidate's unpacked manifest set — and
     the elevation rows (`lab/Invoke-ElevationRows.ps1`). `moderator` is the
     row stage 4 runs, so this is also its rehearsal, with the command the
     script above printed;
   - the §5.2 matrix and the other product rows (`TigerSetup-Validation.md`):
     each row's last pass stands for a source whose differences from the
     candidate cannot reach what the row asserts, and runs again otherwise.
     The report names, row by row, the run each rests on or the rerun.

   A finding is fixed on `dev` and reopens what the fix can reach, the
   candidate included.
6. **Commit and push** the prepared release to `dev`, like any finished work
   (`AGENTS.md`). That commit freezes nothing.
7. **The runner preflight.** Start *Runner diagnostics* with
   `release-readiness` on `dev` — `gh workflow run ci.yml --repo
   rkozlowski/TigerSetup --ref dev`, or Actions → Runner diagnostics → Run
   workflow on `dev` — and wait for it. On a hosted runner it runs the
   release build job's own first steps and more: the toolchain, the SDKs and
   runtimes, `tiger-mark` built from the pin and rendering, `cargo fetch
   --locked`, `Test-Release.ps1` on the runner's PowerShell, git and paths,
   the release workflow's actions including the artifact handoff between jobs,
   and read access with the job's own token. It builds no release and writes
   nothing. A failure is fixed on `dev`, and the fix reopens what it reaches.
8. **Readiness.** `pwsh -File eng\release\Test-ReleaseReadiness.ps1` must
   pass: the repository is public with `main` as its default branch, Actions
   allows the workflow's actions, the `release` environment deploys only from
   `main`, no ruleset can refuse the tag, `main` holds the release workflow, a
   preflight succeeded for this tree's runner-facing files
   (`.github/workflows`, `eng/release`, `.cargo`, `Cargo.lock`), and this
   machine's `gh` can see a draft.

Preparation then reports the release ready to freeze, with this evidence. The
Architect never meets a step whose first real run is in stages 3 to 6.

### 2. Promote `dev` to `main`

The Architect reviews `dev` and promotes it to `main` with a merge that keeps
`dev`'s commits — never a squash merge, a rebase merge or any rewrite of `dev`'s
history. The commit on `main` that the promotion produced is the release
commit, and it freezes the release. It must be the accepted candidate: if
`main` carried commits `dev` lacked, the promoted tree differs from what stage 1
accepted, and the acceptance that difference can affect runs again first.

Nothing runs on the push: the gates ran in stage 1, and no hosted workflow
repeats them. The release commit is what `main` names when the release is
started, so nothing else is pushed to `main` in between; the Architect can
start the release action as soon as the promotion is pushed.

### 3. The release action

The Architect starts **Actions → Release TigerSetup → Run workflow** on `main`
and confirms the version the form is prefilled with. The workflow:

1. **prerequisites** — `Assert-ReleaseCommitReady.ps1`: the run was started on
   `refs/heads/main`; the version is what `Cargo.toml`, README and the
   workflow's prefilled default state; the notes are there; the commit holds
   `LICENSE.txt` and `PRIVACY.md` and the package names their release-asset
   URLs; the commit is on `main`; and `v<v>` does not exist. Only git is
   needed; nothing is built or tested, and a failure stops the run before
   anything is built, tagged or drafted;
2. **build** — first `Test-ReleaseRunner.ps1`, the toolchain, the SDKs and the
   runtimes; then `tiger-mark`, which renders the installed help's PDF, built
   from the TigerMarkView source commit `release.yml` pins as
   `TIGERMARKVIEW_COMMIT` and proved to render (`Build-TigerMark.ps1`: the full
   SHA, checked out exactly, containing TigerMarkView's active-content
   security fix; only the CLI project is published; no TigerMarkView release
   or installer is used); then `Build-ReleaseArtifacts.ps1`: the release
   binaries, the self-installer on the release-quality path, the installer
   checked against the engine and loader just built, the WinGet manifest set
   with its terms URLs checked, the licence and privacy statement frozen from
   the commit, and the record;
3. **publish** — `Publish-DraftRelease.ps1`, in the `release` environment: the
   run must be on `refs/heads/main`, the set it received must hash to what the
   build recorded, and its licence and privacy statement must be the commit's;
   it creates the annotated tag naming that record and the run, and the draft
   release with the set. It refuses to do so anywhere but in this workflow's
   run for the commit.

It publishes nothing. Two safeguards keep a release on `main`, and neither
relies on the other: the workflow's own ref check, which fails a run started on
`dev`, another branch or a tag in its first job, before anything is built; and
the `release` environment, which the repository's settings restrict to `main`
(Environments → release → deployment branches), so the job that can tag and
create releases never runs for another branch. Required reviewers there are
optional. `Test-ReleaseReadiness.ps1` checks the setting; the ref check is
tested by `Test-Release.ps1` and by its first job on every run.

The workflow token's write access is the one prerequisite nothing proves
without writing. The publish job reads everything it depends on first — the
received set, the commit, the existing tag, the existing draft — and its first
write is the tag push, which fails before anything exists if the token cannot
write; a rerun of the failed job then completes the release (*Recovery*).

### 4. Release validation

Release validation proves the artifacts, not the product: preparation accepted
the product, its terms and the tools used here before the release commit, and
nothing here repeats that. It answers one question — did the workflow build,
record and package the accepted commit faithfully, and do its binaries work —
in two steps.

**The files**, in seconds: `pwsh -File eng\release\Get-ReleaseArtifacts.ps1
-Version <v>`, with `gh` authenticated for the repository, since drafts are not
public. It downloads the draft's files through the API into
`artifacts\release\<v>\assets` and proves, cheapest first:

| Check | The risk it closes |
|---|---|
| `release`, `bytes` | the draft carries exactly the six files, each the bytes GitHub recorded |
| `record` | the set is closed and is what `release-artifacts.json` and `SHA256SUMS.txt` name |
| `provenance` | `v<v>` is the release workflow's annotated tag at the record's commit, naming this record: the set is the one that run built, not one uploaded |
| `run` | the run the tag names is `Release TigerSetup`, started by hand on `main`, for that commit, and succeeded: the branch safety held, as GitHub records it |
| `terms` | `LICENSE.txt` and `PRIVACY.md` are the commit's bytes, recorded as `License` and `PrivacyStatement` |
| `winget` | the manifests name this version, the installer's public URL and hash, and this release's terms |
| `installer` | it verifies and carries the engine and loader it installs, and its builder reports this version: the runner's toolchain built the right binaries into the right installer |
| `help` | the help the runner rendered from its own checkout is the commit's `docs/TigerSetup-Help.md`, CRLF as Git checks it out, and its PDF was rendered from it — the one content the release build makes differently from a candidate |
| `validate` | `winget validate` accepts the manifest set; required here, because no row repeats it |

**The binaries on Windows**, one row: the script prints the command for the
`moderator` row of `lab/Invoke-SelfInstallerRows.ps1`, with the installer's
own `tiger-setup.exe` as the builder, so the lab's engine check compares the
installer with itself rather than with a local build, and the unpacked
manifest set. In one desktop job, as the signed-in standard user, it installs
the release's bytes through WinGet from a copy served in the guest (no public
URL is needed yet), starts TigerSetup Shell from the Start Menu — where the
installed `tiger-setup --version` must answer this version — opens both help
forms, and uninstalls through WinGet, leaving nothing behind. It is the
smallest Windows evidence that the runner's binaries, help and manifest set
are usable, and it was rehearsed on the candidate in stage 1.

The coder reports READY TO PUBLISH with the evidence, or the failure. Not
repeated here, because preparation ran it on the candidate of the same commit
and the build cannot change what it proves: the workspace tests, every other
self-installer row (the scopes, `user-nopath`, `upgrade`, the `winget-*`
scenario with its hash-mismatch probe), the elevation rows, the §5.2 matrix,
every other supported Windows, and the review of the terms. A check is added
here only where the release build can change what it proves and a candidate
could not show it, with that reason recorded here. A failure here that the
release commit already carried means the boundary was wrong: the release set is
abandoned (*Recovery*) and the check that would have found it moves into
preparation. Missing preparation evidence is a readiness failure of stage 1,
never a reason to run more here.

### 5. Publication

The Architect reviews the draft (notes, tag, the six files) and publishes it.

### 6. WinGet submission

The coder runs `Get-ReleaseArtifacts.ps1` again. For a published release it
also proves, anonymously, that the installer URL and the manifests'
`LicenseUrl` and `PrivacyUrl` serve the release's exact bytes. The coder then
runs:

```text
pwsh -File eng\release\New-WinGetSubmission.ps1 -Version <v> -WinGetPkgsRoot <clone> [-Push]
```

`<clone>` is a working copy of the publisher's `winget-pkgs` fork, whose
`upstream` remote is `https://github.com/microsoft/winget-pkgs`. The release's
manifest set is taken afresh from the proven `TigerSetup-<v>-WinGet.zip` and
committed, unchanged, on `ItTiger-TigerSetup-<v>` from `upstream/master`. `-Push` pushes that branch to the fork, and only when the
Architect's prompt authorizes the push. The Architect opens the pull request
from the compare link.

If a moderator asks for a manifest change, the result is a new manifest set. It
is generated, validated in the lab and committed like the first. The installer,
its URL and its hash do not change.

## Recovery

- **A tag is never moved and a version is released once.** When validation
  fails, the Architect may delete the draft, and the fix is released as the
  next patch version.
- **A release action that failed after its build** (in `publish`) is resumed
  with *Re-run failed jobs*, which keeps the built set. `Publish-DraftRelease.ps1`
  accepts a tag and draft only when they are what it would have created: an
  existing tag at the release commit, and a compatible draft whose files are
  byte-identical, uploading only what is missing. A full rerun is refused at
  the prerequisites once the tag exists.
- **A failure before the build** — the prerequisites, the runner check or
  `tiger-mark` — is fixed in a new commit on `dev`, preflighted again where
  the fix touches the runner-facing files, and promoted to `main` like the
  first. The workflow is then started for the commit that promotion produced.
  A run started on another ref than `main` only needs starting again on
  `main`.

## Tooling

| Script | Stage | Role |
|---|---|---|
| `eng/release/TigerSetupRelease.psm1` | all | TigerSetup's release facts, the record, and the Git, tag, run and release checks |
| `eng/release/Assert-ReleaseCommitReady.ps1` | 1, 3 | the prerequisites gate, the ref check first |
| `eng/release/Test-ReleaseRunner.ps1` | 1 (preflight), 3 | what the release build needs, on the machine it runs on |
| `eng/release/Build-TigerMark.ps1` | 1, 3 | the help-PDF renderer, built from the pinned TigerMarkView commit and proved to render |
| `eng/release/Build-ReleaseArtifacts.ps1` | 3 (and 1 with `-Rehearsal`) | builds, checks and records the release set |
| `eng/release/Test-ReleaseReadiness.ps1` | 1 | the repository's settings, the runner preflight for this tree, and this machine's access |
| `eng/release/Publish-DraftRelease.ps1` | 3 | the tag and the draft; `-PlanOnly` reports and changes nothing |
| `eng/release/Get-ReleaseArtifacts.ps1` | 4, 6 (and 1 on a candidate and on the previous release) | retrieves and proves a release's set, or proves a set on disk with the same checks |
| `eng/release/New-WinGetSubmission.ps1` | 6 | the `winget-pkgs` branch |
| `eng/release/Test-Release.ps1` | every change to the tooling, and 1 | the tooling's tests: synthetic repositories and stand-ins for `gh`, `dotnet` and the download, no network; part of the verification gate, and run on the runner by the preflight |

Every gate reports `PASS`, `BLOCKED` or `FAIL` and exits 0, 2 or 1; a check
that could not run where the gate ran is reported `NOT RUN`, with where it runs
instead, and is never counted as a pass.

The release action is the only hosted work a release builds from: building the
set from exactly the release commit, recording it, tagging and drafting. The
other workflow, `ci.yml` (*Runner diagnostics*), is started by hand and is no
stage of a release: `release-readiness` is the runner preflight of stage 1,
and `elevated-tests` runs the workspace tests under the hosted runner's elevated
administrator token, the one runner difference a developer shell cannot
reproduce (`LESSONS_LEARNED.md`), when a change touches a token-dependent path.
A workflow can be started only once `main` holds it, and a run uses the file at
the ref it was started on, so both run on `dev` before a promotion.

### What the readiness evidence proves, and where it comes from

| Requirement | Evidence | Kind |
|---|---|---|
| the release scripts' logic, success and failure | `Test-Release.ps1` on synthetic repositories and stand-ins | local, and on a runner by the preflight |
| the build path, the checks and validation tooling | the candidate's build and its rehearsal (stage 1, steps 3–5) | local |
| retrieval of a workflow-built set, the tag, the run record, the public URLs | `Get-ReleaseArtifacts.ps1` on the previous release | local, against GitHub |
| the toolchain, MSVC, the Windows SDK, .NET, WebView2, `gh` | `Test-ReleaseRunner.ps1` | on a runner (preflight), and again at the top of every release build |
| TigerMarkView reachable anonymously at the pin, `tiger-mark` built and rendering | `Build-TigerMark.ps1` | on a runner (preflight), and again before every release build |
| the crates resolvable from the lockfile | `cargo fetch --locked` | on a runner (preflight) |
| the release workflow's actions, at its versions, including the artifact handoff | the preflight's jobs | on a runner |
| the job token reads the repository and its releases; the checkout credentials read origin | `Test-ReleaseRunner.ps1 -GitHubAccess` | on a runner (preflight) |
| the repository is public, Actions allows the actions, the `release` environment is `main`-only, no tag ruleset, `main` holds the workflow | `Test-ReleaseReadiness.ps1` with the maintainer's `gh` | the repository's settings, read locally |
| the ref check refuses `dev`, other branches, tags and no ref, and passes `main` | `Test-Release.ps1`; a run started on `dev` fails in its first job | local, and in a real run |
| the workflow's jobs, steps and permissions in the order these depend on | `Test-Release.ps1`, reading the workflow files | static |
| the workflow token can push the tag and create the draft | the release action itself; nothing proves a write without writing | the real release only, recoverable by a rerun |
