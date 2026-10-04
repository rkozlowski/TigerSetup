# Releasing TigerSetup

TigerSetup is released through the Tiger release model (TigerAiCore
`docs/release-model.md`): one release commit, one build of it, a record of the
exact bytes, validation of those bytes, and publication of those bytes. This
document is TigerSetup's side of that model: what a release consists of, where
each kind of artifact lives, and how the stages run here. The tooling is
`eng/release/`; the version rules are in `AGENTS.md` (*Version and release
discipline*).

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
`release-artifacts.json`. Every later stage proves a retrieved set against it.

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
  version. `PRIVACY.md` on `main` is the statement being prepared for the next
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
  `Get-ReleaseArtifacts.ps1`, proved, and unpacked for validation.

Rebuilding is never a substitute: the installer carries a help PDF rendered on
every build, so two builds of one commit differ even where the engine and loader
are byte-identical, and only the bytes that were validated may be published.

## The stages

| | Who | What |
|---|---|---|
| 1 | coder | prepare the release: version, notes, documentation, gates, local candidate |
| 2 | Architect | review, commit, push to `main` |
| 3 | Architect | start `Release TigerSetup` with the version |
| 4 | coder | retrieve the draft's release set, prove it, validate it in the lab |
| 5 | Architect | review and publish the draft |
| 6 | coder, then Architect | prepare the WinGet submission; open the pull request |

### 1. Prepare

The coder, on the Architect's "prepare release `<v>`":

- sets `[workspace.package] version` in `Cargo.toml` (the build updates
  `Cargo.lock`), README's "TigerSetup is at version" line and its installer
  examples, and the `default` of the `version` input in `release.yml`, so the
  release form opens prefilled with the version to confirm;
- writes `.github/release-notes/<v>.md`: a `# TigerSetup <v>` heading and the
  user-facing changes, install and verification sections;
- settles the release-bound terms: `LICENSE.txt`, and a `PRIVACY.md` that
  describes this release's behaviour — reviewed again whenever the release
  changes what TigerSetup records, sends or removes;
- runs the verification gate (`AGENTS.md`) and the release turn's local
  candidate and lab rows where the change needs them;
- when the TigerMarkView pin or the release build changed, proves the
  workflow's path locally: `Build-TigerMark.ps1 -Commit <pin>` into an empty
  directory, then `Build-ReleaseArtifacts.ps1 -Version <v> -Rehearsal
  -TigerMarkPath <the tiger-mark.exe it built>`;
- runs `pwsh -File eng\release\Assert-ReleaseCommitReady.ps1 -Version <v>`,
  the release workflow's own first gate. Before the push its commit check is
  BLOCKED; everything else must pass.

Nothing is committed, pushed or tagged.

### 2. Commit and push

The Architect reviews and pushes to `main`. Nothing runs on the push: the
verification gate ran in stage 1, and no hosted workflow repeats it. The release
commit is what `main` names when the release is started, so nothing else is
pushed to `main` in between; the Architect can start the release action as soon
as the push is done.

### 3. The release action

The Architect starts **Actions → Release TigerSetup → Run workflow** on `main`
and confirms the version the form is prefilled with. The workflow:

1. **prerequisites** — `Assert-ReleaseCommitReady.ps1`: the version is what
   `Cargo.toml`, README and the workflow's prefilled default state, the notes
   are there, the commit holds `LICENSE.txt` and `PRIVACY.md` and the package
   names their release-asset URLs, the commit is on `main`,
   and `v<v>` does not exist. Only git is needed; nothing is built or tested,
   and a failure here stops the run before anything is built, tagged or drafted;
2. **build** — builds `tiger-mark`, which renders the installed help's PDF,
   from the TigerMarkView source commit `release.yml` pins as
   `TIGERMARKVIEW_COMMIT` (`Build-TigerMark.ps1`: the full SHA, checked out
   exactly, containing TigerMarkView's active-content security fix; only the
   CLI project is published; no TigerMarkView release or installer is used),
   and runs
   `Build-ReleaseArtifacts.ps1`: the release binaries, the self-installer on the
   release-quality path, the installer checked against the engine and loader
   just built, the WinGet manifest set with its terms URLs checked, the
   licence and privacy statement frozen from the commit, and the record;
3. **publish** — `Publish-DraftRelease.ps1`, in the `release` environment: the
   set it received must hash to what the build recorded, and its licence and
   privacy statement must be the commit's; it creates the annotated tag naming
   that record and the draft release with the set. It refuses to do so anywhere
   but in this workflow's run for the commit.

It publishes nothing. The `release` environment is restricted to `main` in the
repository's settings (Environments → release → deployment branches), so the
job that can tag and create releases never runs for another branch; required
reviewers there are optional.

### 4. Release validation

Release validation proves the artifacts, not the product: preparation accepted
the product and its terms before the release commit, and nothing here repeats
that. The coder runs `pwsh -File eng\release\Get-ReleaseArtifacts.ps1 -Version <v>`,
with `gh` authenticated for the repository, since drafts are not public. It
downloads the draft's files into `artifacts\release\<v>\assets` and proves
the chain from the commit to the bytes:

- every file is GitHub's recorded asset digest, and the bytes the record and
  `SHA256SUMS.txt` name, in a closed set;
- `v<v>` is the release workflow's annotated tag at the record's commit, naming
  that record's SHA-256 — the commit whose candidate preparation accepted;
- `LICENSE.txt` is recorded as `License` and `PRIVACY.md` as
  `PrivacyStatement`, and both are the exact bytes that commit holds for them;
- the manifests name the installer's public URL and hash, and their
  `LicenseUrl` and `PrivacyUrl` name this release's `LICENSE.txt` and
  `PRIVACY.md` assets;
- the installer verifies against the engine, loader and builder it installs.

It unpacks the manifests (`...\winget`) and the
installer's files (`...\payload`) and runs `winget validate` where winget is
installed; elsewhere it reports that check NOT RUN, and the lab's WinGet rows
run it.

The release's bytes are then proved on Windows by the rows it names, with the
installer's own `tiger-setup.exe` as the builder, so the lab's engine check
compares the installer with itself rather than with a local build, and with
the record's `sourceCommit`, so the shipped help is compared with that commit
as Git checks it out rather than with this working tree. They are the smallest
set that proves the artifacts usable (`lab/README.md`):

- `user-nopath`, the one representative path: the per-user install, TigerSetup
  Shell launched from the Start Menu, the help, the uninstall and nothing left.
  It is this row rather than a plain install row because the installed help is
  the one content the release build itself produces differently from a local
  candidate — rendered by the runner's `tiger-mark`, from the runner's
  checkout — and this row proves it is the release commit's;
- `winget-user` with the unpacked manifest set: `winget validate`, install from
  a loopback copy of the installer (no public URL is needed yet), the declared
  command and the uninstall.

Every other scope, row and scenario — the machine scope, the `moderator` row,
`upgrade`, the elevation and §5.2 rows — is acceptance that preparation ran on
the candidate of the same commit, and is not repeated on the release bytes
unless the release build changed something it proves. The coder reports READY
TO PUBLISH with the evidence, or the failure.

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
- **A failure before the build** is fixed in a new commit. The workflow is then
  started for that commit.

## Tooling

| Script | Stage | Role |
|---|---|---|
| `eng/release/TigerSetupRelease.psm1` | all | TigerSetup's release facts, the record, and the Git, tag and release checks |
| `eng/release/Assert-ReleaseCommitReady.ps1` | 1, 3 | the prerequisites gate |
| `eng/release/Build-TigerMark.ps1` | 3 (and 1, locally) | the help-PDF renderer on the runner, built from the pinned TigerMarkView commit; run locally with the pin to prove it before the push |
| `eng/release/Build-ReleaseArtifacts.ps1` | 3 (and 1 with `-Rehearsal`) | builds, checks and records the release set |
| `eng/release/Publish-DraftRelease.ps1` | 3 | the tag and the draft; `-PlanOnly` reports and changes nothing |
| `eng/release/Get-ReleaseArtifacts.ps1` | 4, 6 | retrieves and proves a release's set |
| `eng/release/New-WinGetSubmission.ps1` | 6 | the `winget-pkgs` branch |
| `eng/release/Test-Release.ps1` | 1 | the tooling's tests: synthetic repositories and a stand-in for `gh`, no network; part of the verification gate |

Every gate reports `PASS`, `BLOCKED` or `FAIL` and exits 0, 2 or 1; a check
that could not run where the gate ran is reported `NOT RUN`, with where it runs
instead, and is never counted as a pass.

The release action is the only hosted work a release needs: building the set
from exactly the release commit, recording it, tagging and drafting. The other
workflow, `ci.yml` (*Elevated runner tests*), is a diagnostic started by hand
and is no stage of a release: it runs the workspace tests under the hosted
runner's elevated administrator token, the one runner difference a developer
shell cannot reproduce (`LESSONS_LEARNED.md`), when a change touches a
token-dependent path.
