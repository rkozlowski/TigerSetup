# TigerSetup — Design

This document is the authoritative description of what TigerSetup is, what it
must do, and the architecture that constrains every implementation decision.
`README.md` is the practical entry point for building an installer; how
TigerSetup is proven is owned by `TigerSetup-Validation.md`.

---

## 1. Positioning

TigerSetup is:

> **A small, modern, native Windows setup builder for ordinary desktop
> applications.**

Its value is not a longer installer feature list. It is **removing duplicated
project-specific packaging glue and making installation state explicit and
verifiable**.

The useful combination is:

- a declarative installer definition;
- metadata extraction from MSBuild projects or built executables;
- consistency validation between source metadata, binaries, installer metadata
  and distribution metadata;
- SQLite-backed installation ownership and a transaction journal;
- upgrade, rollback, uninstall, verify and repair semantics;
- WinGet-ready package metadata from the same source of truth.

### Non-goals

TigerSetup does not try to compete with:

- MSI / Windows Installer;
- WiX / Burn;
- InstallShield;
- Advanced Installer;
- enterprise deployment frameworks;
- arbitrary installer scripting languages;
- every obscure Windows installation scenario.

**If TigerSetup does not understand an operation, it does not perform it.**

---

## 2. What TigerSetup replaces

TigerSetup replaces per-project packaging glue — PowerShell around Inno Setup,
each project re-implementing metadata extraction, version checks, PATH
handling, scope and elevation logic, dependency detection and release
integration in its own way — with one engine and a declarative package.

**TigerMarkView is the reference application**: a per-user or machine-wide
desktop application with optional PATH integration, a
`Microsoft.DotNet.DesktopRuntime.10` and a `Microsoft.EdgeWebView2Runtime`
dependency, unattended use by release automation and package managers, and
upgrade and uninstall behaviour that package managers rely on. Its package is
`packages/TigerMarkView/`, and its replacement validation is TigerSetup's
acceptance standard (`TigerSetup-Validation.md` §4).

Real applications define scope. The behaviour of the Tiger installers
TigerSetup replaces is a **requirement source, not an architecture to
translate**:

> **A requirement repeated across real applications is evidence for a TigerSetup
> primitive. A one-off installer trick is not.**

---

## 3. Developer workflow

```text
project / binary
      ↓
TigerSetup.toml
      ↓
tiger-setup build
      ↓
validated Setup.exe
      ↓
WinGet-ready package metadata
```

The builder command line, in the same command-app grammar as the generated
installer (§6.2):

```text
tiger-setup build TigerSetup.toml [--output <dir|file.exe>] [--engine <path>]
                                  [--property <Name=Value>]... [--offline] [--fast]
tiger-setup metadata TigerSetup.toml [--property <Name=Value>]... [--json]
tiger-setup inspect Setup.exe [--json] [--output-zip <file>] [--output-meta <file>]
                              [--output-meta-json <file>]
tiger-setup verify Setup.exe
tiger-setup winget prepare TigerSetup.toml --installer Setup.exe --output <dir>
tiger-setup winget finalize <manifest dir> --url <url> --installer Setup.exe
```

Naming conventions: `TigerSetup.toml`, `tiger-setup.exe`, and
`<Name>-<version>-Setup.exe` unless `--output` names the file.

---

## 4. Declarative package definition

TOML is the package-definition format. The sections are `[package]`,
`[metadata]` (§9), `[install]`, `[installer]`, `[[files]]`, `[[options]]`,
`[[shortcuts]]`, `[[path]]`, `[[environment]]`, `[[registry]]`,
`[[file_associations]]`, `[[url_protocols]]`, `[[app_paths]]`,
`[[context_menu]]`, `[[firewall]]`, `[[actions]]` (§5.14), `[registration]`,
`[legacy]` (§5.12), `[[dependencies]]` (§7) and `[winget]` (§8.2);
`README.md` shows every key, and the builder's manifest module is the
schema.

```toml
[package]
id = "ItTiger.TigerMarkView"
name = "TigerMarkView"
version = "0.8.0"
publisher = "IT Tiger"
license = "MIT"
icon = "assets/TMV.ico"                # the product's branding icon

[install]
architecture = "x64"
scopes = ["user", "machine"]           # first entry is the default scope
existing_scope = "preserve"            # what a rerun does when the product is installed in the other scope (§5.13)

[installer]
icon = "branding"                      # the generated Setup.exe's own icon (§11.6)

[[files]]
source = "publish/**"
exclude = ["*.pdb"]
```

`[installer].icon` selects the icon the generated `Setup.exe` carries in its
own Windows resources, separately from the product's branding icon; the
resolution rules are §11.6. `[install].existing_scope` is the cross-scope
policy of §5.13. Both are optional and default to the least-surprising
behaviour — the branding icon (or TigerSetup's when there is none), and
`preserve`. Every declaration is typed: a file set is a glob, a shortcut names
its install-relative target, a registry value its kind, an option its default
and its label per language. A value the builder can derive — the version, the
description, the copyright — is derived once through `[metadata]` and
validated, never typed twice.

**Options and the one predicate.** An option is boolean, or a *choice* of
exactly one declared value; both are labelled per language and both are
remembered by the installation (§5.3). Every optional resource is gated by
the same predicate, `when = { option, equals }`, comparing the option's
canonical value text (`true`/`false`, or the choice value) — and by nothing
else: no expressions, no negation, no combination, no machine-state
conditions. A resource wanted under two values is declared twice. This is
deliberately the whole conditional language, because every richer one
observed in real installers grew into scripting. A *component* is therefore
not a concept of its own: it is an option that gates `[[files]]`, and its
files come and go with the option through the ordinary reconciliation,
conservative removal included. The older `option = "<name>"` spelling on
shortcuts and PATH entries remains valid and means `equals = true`.

The foundational distinction:

> **Manifest = intent. Database = reality.**

The manifest expresses desired installation state. The installed database
records actual installation state and ownership.

`TigerSetup.toml` is the **developer-facing source format** and is not shipped
inside the generated installer. `tiger-setup build` validates it, resolves
metadata sources, dependency identities and the file set, and emits **runtime
metadata** in the compact form the installer engine reads on the target machine
(§10.4). Both express the same intent: one is written by a developer, the other
is read by the engine.

---

## 5. Core architecture

### 5.1 Manifest is intent; database is reality

A per-installation SQLite database records what the installation actually owns.
Uninstall and upgrade plan from that database, **never** by inverting the
current manifest.

Concretely: if 0.8 installed `foo.dll` and 0.9 replaces it with `bar.dll`, the
0.9 package needs no hardcoded historical knowledge. The database already knows
TigerSetup owns `foo.dll`, so the upgrade plans `remove owned foo.dll` /
`install bar.dll`.

### 5.2 State location

One database per installed product/installation — not one global TigerSetup
database:

```text
Machine scope:  %ProgramData%\TigerSetup\<ProductId>\state.db
User scope:     %LOCALAPPDATA%\TigerSetup\<ProductId>\state.db
```

Per-product state isolates corruption, avoids turning TigerSetup into a
system-wide package manager, avoids requiring a background service, and keeps
ownership boundaries simple. The product ID must be stable across versions.

### 5.3 Installation state vs transaction journal

Two concepts that must stay separate.

**Installation state** — the currently committed state of the installed product:
product identity, installed version, scope, installation ID, install root,
registration key, the recorded option values (each as its canonical text, so
a boolean and a choice are one column and reports give each back with its
type), the licence text a person explicitly accepted (as the SHA-256 of its
exact bytes, or nothing), the owned resources — files with their hashes,
directories, registry keys and values, PATH entries, shortcuts, environment
variables (with the value that was there before, for the restore), firewall
rules (as written) — and the uninstall-phase custom actions the installation
keeps for its own uninstall, each with its definition and the identity of
the packaged program the state directory holds for it (§5.14). Option values
follow one precedence everywhere — an
explicit value for this run, else the last committed value, else the manifest
default — and are committed with the transaction, so a failed, cancelled or
rolled-back run leaves the recorded values exactly as they were; there is no
separate wizard preference store.

**Transaction journal** — the current install/upgrade/uninstall/repair attempt
and its rollback information: transaction ID and kind, the versions it moves
between, the package identity and metadata hash it was planned from, what the
commit will record on the installation (registration key, accepted licence),
and one row per operation with its sequence, kind, target, state, the
previous state it needs to undo (existence, hash, backup path, previous
registry data), what it wrote, and its result.

The tables are typed, one per concept, rather than generic JSON blobs:

```text
installation   transaction   operation   installation_option   transaction_option
file   directory   registry_key   registry_value   path_entry   shortcut
environment_variable   firewall_rule   action
dependency_event     what the dependency phase observed — history, never ownership
action_run           every custom action execution — evidence, never ownership
```

The schema version lives in `PRAGMA user_version`, with forward-only
migrations in place: a mutating run migrates the database it opens; a
read-only reader (`inspect`, `verify`, the wizard working out its flow)
reads every schema back to the oldest one it understands, so an installation
made by an earlier engine is described, not refused, until a mutating run
migrates it. The current schema is 6: version 4 (0.6.0) added the
environment-variable and firewall tables and made option values text,
version 5 (0.7.0) added the `action` and `action_run` tables and nothing
else, so a reader of a 0.6.0 database sees an installation with no actions
and the first mutating run adds the two tables without touching a row;
version 6 (0.7.1) added a registry value's pre-installation state to
`registry_value` (`pre_existed`, `previous_kind`, `previous_data`), so a
reader of an older database sees values that did not pre-exist — exactly
what the engine that wrote them knew — and takes them away by deletion, as
it always did; version 7 (0.8.0) added the fingerprint of an owned file as
installed (`file.modified`, `operation.applied_modified`, §5.7), so a reader
of an older database has no fingerprint to trust and hashes every file, as
that engine did; version 8 (0.9.0) added the journal batch an operation
transitions with (`operation.batch`, §5.4), so an operation an older engine
journaled has none and is walked on its own, as that engine walked it.

### 5.4 Crash consistency

SQLite cannot make the Windows filesystem, registry, services and process state
transactional. TigerSetup therefore uses a **persistent transaction journal
written in many short durable commits** — not one giant SQLite transaction held
open for the whole installation.

```text
start logical transaction
↓
record operation and durable undo state
↓
mutate Windows
↓
mark operation applied
↓
repeat
↓
commit installed state
↓
mark transaction committed
```

The hard invariant:

> **Durable undo state must be written before the Windows mutation**, so a crash
> between the two is recoverable.

A naive `planned → mutate → completed` state machine is ambiguous if the process
dies after the mutation but before `completed`. The operation states are
therefore:

```text
planned → applying → applied
```

where `applying` is written together with the operation's undo record, and on
restart resource inspection reconciles the one ambiguous state, `applying`.
(A journal an older engine left may still carry `prepared` — undo durable,
mutation not started — and it is read as such.)

**What the transaction must guarantee.** The objective is transactional
consistency: an installation reaches **success** or **full rollback**. A third
condition is legitimate but never terminal — **recoverable failure**, where the
attempt stopped part-way and durable state still describes enough to finish
converging. Durable state must always allow TigerSetup to reach success or
rollback eventually; an unknown or hybrid installation is exactly the outcome
this architecture exists to prevent.

```text
absent ──── install ────→ installed
   ↑                          │
   └──────── rollback ────────┘

version A ── upgrade ────→ version B
    ↑                          │
    └──────── rollback ────────┘        never a mixture of A and B
```

First installation is the simple case: `absent → installed`, or rollback to
`absent`. **Upgrade is the case that matters**: `A → B`, or rollback to a valid,
complete, working `A`. A failed or interrupted upgrade must never be accepted as
a hybrid A/B installation — the files of B with the registration of A is a
failure, not a partial success.

Dependencies sit outside the product transaction. From its perspective they are
external, shared prerequisites that succeed or fail independently; if TigerSetup
caused a shared dependency to be installed and the product transaction later
rolls back, that dependency normally remains installed (§7.1).

**The journal model.**

- An operation moves `planned → applying → applied`; a transaction
  `running → committed`, or `rolling_back → rolled_back`, with
  `rollback_failed` as the state that needs a later run. There is no separate
  `committing` state: the commit is one SQL transaction that rewrites the
  installation and ownership rows from the journal and marks the transaction
  committed, so there is nothing between "not committed" and "committed" to
  reconcile.
- Transitions are journaled under a rollback journal (`journal_mode = DELETE`)
  with `synchronous = FULL` and an exclusive lock for the run, and **the unit
  of a durable transition is the commit group, not the batch or the file**.
  The invariant constrains what must be durable *before* a mutation and
  *after* it, not how many operations share a commit, so the journal records
  recovery state rather than a narrative of every file. The forward walk
  takes up to **eight consecutive journal batches** as one commit group:
  the operations of the group move to `applying` together, each with its
  undo record, in one durable commit; the mutations are performed in
  sequence order with nothing written to the journal between them; and the
  group moves to `applied` together, each operation with the inventory of
  what it wrote — hash, size, last-write time — in one more commit. A crash
  anywhere inside a group leaves it `applying`, and recovery reconciles the
  *group* by inspecting each of its targets; it never needs to know which
  file's syscall was the last to succeed. The group bound is the bound on
  that work: at most eight of the builder's batches, so at most 2,048 files
  or 256 MiB of them, plus the files larger than a batch. The per-file row
  is kept throughout — it is the undo record the rollback reads and the
  inventory the ownership row is made from at the commit — but it carries
  no transition of its own inside a group.
- **The batch is the plan's unit, and the file batches are the builder's.**
  The metadata carries them (`file_batches`, §10.4): consecutive runs of
  the file list, in payload order, closed before the file that would take a
  batch past **256 files or 32 MiB** of uncompressed bytes, whichever comes
  first, so that a file larger than 32 MiB is a batch of its own. The engine
  reads the boundaries and never reproduces the rule; an uninstall removes
  files by the batches the uninstaller's own metadata carries, and an owned
  file the package no longer knows goes in a batch of the residue after
  them. Every other typed resource — a directory, a registry key or value,
  a PATH entry, an environment variable, a shortcut, a firewall rule, a
  stored action program — shares a batch with its neighbours of the same
  kind. A resource whose exact previous state cannot be reconstructed after
  its mutation — a registry value's prior type and data, the whole previous
  `Path` text — is as safe there as a file is, because the walk takes every
  undo record of a group before it mutates any of it. The rollback walks
  batch by batch, in reverse.
- **A custom action is a commit group of its own**, because its `action_run`
  row must be durable before its process exists and its effects are not
  TigerSetup's to reason about; so is an operation an injected fault names,
  so that a fault's boundary is exactly that operation's — everything
  before it durably applied, nothing after it started. The guiding
  invariant is unchanged on every path: durable recovery information before
  the mutation; completion acknowledgements batched only where recovery can
  reconcile the actual state safely. A thousand files therefore cost two
  commits per group rather than three per file, and the transaction's time
  is the mutation, not the journal.
- For a file whose target already holds a file: record the previous hash and
  where the previous file will be kept; write `<target>.tigersetup-new`,
  `FlushFileBuffers`, and replace the target in one `ReplaceFileW`, which
  moves the previous file into the transaction's staging area as the undo and
  puts the new one in its place — the target is at every instant the old file
  or the new one, and no bytes are copied. A removal moves the file into the
  staging area the same way; the staging area is deleted after the commit, in
  one directory removal, and a rollback moves the files back. A backup on
  another volume, where a rename cannot reach, falls back to a flushed copy.
  Recovery classifies an `applying` operation by inspecting the target:
  absent, equal to the payload, equal to the previous content, or different —
  and re-applies or completes accordingly, logging what it found
  (`file_missing`, `file_content_mismatch`, `operation_reapplied`).
- **The hash a file is owned by comes from the package.** The payload index
  records every entry's SHA-256 (§10.5), the engine checks the bytes it wrote
  against it before the rename, and the ownership row records that hash with
  the file's size and last-write time once it is in place. A file whose size
  and last-write time are still what the row records is the file TigerSetup
  wrote and has that hash without being read; a plan hashes only a file whose
  fingerprint has changed, and a walk does the same when it records the undo.
  An uninstall of a thousand files therefore plans in milliseconds, and an
  upgrade decides what to keep by comparing the installation's hashes with
  the package's index without decoding the stream. `verify` still hashes
  every owned file: it is the explicit request for a thorough check.
- **Direction is decided once per recovery**: forward when the running engine
  carries the same package identity, version and metadata hash and the
  transaction is still `running`, so every payload byte is at hand; rollback
  otherwise, and always for a transaction already rolling back. Rollback
  inspects before it acts and is idempotent.
- Recovery begins with a sweep of `*.tigersetup-new` temporaries and backups no
  journal row references, so a crash between a filesystem action and its
  journal row leaves nothing behind.

**The flush before the rename is not optional.** A file renamed into place
without `FlushFileBuffers` can survive a power cut at its full length with
different content — Windows' lazy writer has not yet written the data — and
only the recorded hash would detect it. Write-through on the rename covers the
directory entry; the flush covers the data; both are needed, and the recorded
hash is the check that catches what neither covered.

### 5.5 Typed operations, not scripting

Every system mutation is a typed, known, journaled, reversible operation. Each
resource kind — file, directory, registry key, registry value, PATH entry,
environment variable, shortcut, firewall rule — has an install, a remove and
a keep operation, and the Add/Remove Programs registration is registry values
like any other:

```text
InstallFile        CreateDirectory     CreateRegistryKey    SetRegistryValue     AddPathEntry     CreateShortcut
RemoveFile         RemoveDirectory     RemoveRegistryKey    RemoveRegistryValue  RemovePathEntry  RemoveShortcut
KeepFile           KeepDirectory       KeepRegistryKey      KeepRegistryValue    KeepPathEntry    KeepShortcut
SetEnvironmentVariable       RestoreEnvironmentVariable      KeepEnvironmentVariable
CreateFirewallRule           RemoveFirewallRule              KeepFirewallRule
RunAction                    StoreAction
```

**The typed Windows integrations are registry values.** A file association,
a URL protocol, an `App Paths` entry and a classic context-menu verb are each
compiled by the engine into the registry keys and values Windows documents
for them — a ProgID class with its `DefaultIcon` and `shell\open\command`,
an `OpenWithProgids` entry per extension, a capability registration under
the publisher's key and `RegisteredApplications`, a scheme class carrying
`URL Protocol`, an `App Paths\<exe>` key, a verb under `Classes\*\shell`,
`Directory\shell` or `Directory\Background\shell` — and then planned,
journaled, rolled back, verified, repaired and removed as ordinary registry
values with value-level ownership. There is no second registry engine, only
the knowledge of which values each integration is; the shell is told once,
after the walk, that associations changed. A key chain is created downward
from the deepest key Windows itself owns (`Software`, `Software\Classes`,
`App Paths`, the Add/Remove Programs root), so TigerSetup never owns a key of
Windows's own. Two rules keep the integrations from taking anything over: an
association registers the product as a *handler* — `OpenWithProgids` and the
capability, never the extension's default and never a `UserChoice` — and a
URL scheme's own class key is written only where nothing else owns it; a
scheme another application registered is left exactly as it is and reported
(`url_protocol_scheme_in_use_preserved`), while the handler ProgID and the
capability are still registered.

A `Keep` operation is how an upgrade or a repair records that an owned
resource is carried over unchanged: it is journaled `applied` at once, so
the transaction's ownership rows are complete without walking the disk.

TigerSetup resists arbitrary script execution unless a concrete requirement
proves typed mechanisms insufficient. Where a real application needs work
no typed resource can express, a **custom action** (§5.14) is the one
sanctioned form: a declared, packaged, verified and recorded program with a
bounded envelope — journaled as `RunAction`, with `StoreAction` keeping the
programs an uninstall will need — and with no pretence that TigerSetup can
undo what it did.

### 5.6 Ownership is conservative

- **Files** — record installed hashes. A file owned by TigerSetup but modified
  after installation is **preserved on uninstall and reported**: the uninstall
  outcome lists it under the stable code `file_modified_preserved`, and the
  directory that contains it is not removed. `verify --json`, which only
  observes, reports the same file as `file_modified`: an observation code
  names what was found (`<resource>_missing`, `<resource>_modified`, and for
  a container that still holds something, `<resource>_not_empty`), and a
  mutating run that left something alone adds the action (`_preserved`), so
  a reader always knows whether anything was done.
- **PATH** — if an equivalent entry existed before installation, TigerSetup does
  not claim ownership of it. The database records scope, exact/normalised entry,
  whether it pre-existed, and whether TigerSetup added it.
- **Resources whose location Windows may move** — a stored path outside the
  roots the scope resolves *now* is not automatically evidence that the
  database was tampered with. A registry hive cannot move, so a key in the
  other hive stops the run — a stored registry key or value is confined to
  the scope's hive, because a product value may live at an explicit location
  outside `Software` (below), while a PATH, environment or registration row
  is confined to the scope's own environment key and Add/Remove Programs
  root; the machine-scope database is writable by administrators alone
  (§5.11), so a row there names nothing its writer could not already reach.
  A shortcut folder can: OneDrive's Known Folder Move
  relocates the desktop and policy can redirect the Start Menu, so a link
  recorded before such a move is left untouched, reported as
  `shortcut_outside_scope_preserved`, and the rest of the uninstall proceeds.
  Both answers refuse to act outside the scope, which is the point; refusing
  the whole run would leave the product impossible to uninstall, and that is a
  worse outcome than a resource left behind.
- **Directories** — do not recursively delete unknown content merely because
  TigerSetup created the directory.
- **Registry** — prefer ownership at value level; deleting a whole key must be
  conservative when unrelated values may exist. A `[[registry]]` value lives
  under the scope's `Software` root by default, or — `root = "HKLM"` or
  `root = "HKCU"` — at an explicit location in the scope's hive, such as
  `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled`; the
  hive must be the one the package's only scope writes, and the builder
  refuses a dual-scope package that declares one. Either way the ownership
  row records what TigerSetup wrote *and* what the value held before, and
  the value follows the environment-variable model below: taking it away
  (uninstall, or the option turned off) restores the previous data where
  there was some and deletes the value where there was none, only while the
  value still holds what TigerSetup wrote; a value the user or another
  program changed since is preserved and reported
  (`registry_value_modified_preserved`) by an upgrade, a reinstall and an
  uninstall alike, and only a repair rewrites it; a value already holding
  the wanted data is kept with itself as the data to restore, so it is never
  claimed. The Add/Remove Programs registration is the one exception: it is
  TigerSetup's own bookkeeping, and the desired values always win. Keys are
  created down from the deepest key Windows owns — `Software`, its
  `Classes` and `App Paths`, the Add/Remove Programs root, or the hive's
  top-level key for an explicit location — and only the keys TigerSetup
  created are owned and removed, when empty.
- **Environment variables** — the ownership row records what TigerSetup wrote
  *and* what the variable held before. Removal (uninstall, or the option
  turned off) restores the previous value where there was one and deletes
  the variable where there was not — but only while the variable still holds
  what TigerSetup wrote; a value the user or another program changed since is
  preserved and reported (`environment_variable_modified_preserved`), and a
  variable already holding the wanted value is kept with itself as the value
  to restore, so it is never claimed. Only a repair, which is asked for,
  rewrites a changed value. The environment is broadcast once per run.
- **Firewall rules** — rules are machine-wide, identified by a name Windows
  does not keep unique, and need an administrator. TigerSetup files its rule
  under a `Grouping` naming the product, which is how it tells its own rule
  from a stranger's of the same name: a same-named rule without the grouping
  is never claimed, rewritten or removed (`firewall_rule_name_in_use_preserved`),
  and a same-named rule of TigerSetup's that reads differently belongs to
  another installation of the product — the other scope's — and is preserved
  the same way. A rule the user changed (disabled, retargeted) is reported by
  `verify`, kept by an upgrade or uninstall (`firewall_rule_modified_preserved`)
  and rewritten only by a repair. A run without an administrator — a per-user
  install by a standard user — creates and removes no rule and reports
  `firewall_rule_skipped_unelevated` for each declared one; owned rules stay
  owned for a later elevated run. Rules are written through the Windows
  Firewall API (`INetFwPolicy2`), never through a command-line tool.
- **Shortcuts, extended** — a Startup link runs at sign-in; a Send To link
  exists per user only, so a machine-scope run reports
  `shortcut_location_unavailable` rather than inventing a shared one; a URL
  shortcut is an Internet shortcut file (`.url`) removed only while it still
  opens the recorded URL; the working directory and the AppUserModelID are
  written into the link and compared on verify. The completion page's launch
  offer is the product's own unconditional Start Menu link to an installed
  file, never a URL, Startup or Send To link.

### 5.7 File backup strategy

Backups for replace/delete operations live in a transaction staging directory
rather than in SQLite blobs. The database stores metadata: original path,
installed hash, previous hash, backup path, ownership information. The undo
record — the previous hash and the backup path — is durable before the
destructive mutation; the mutation itself *moves* the previous file to the
backup path rather than copying it, so the bytes are exactly where the record
says they are and nothing was copied to get them there (§5.4). A rollback moves
them back. Obsolete backup data is cleaned up after commit, in one removal of
the staging directory — which for an uninstall is where the deletion of every
removed file actually happens.

### 5.8 Uninstall model

The uninstaller does not need the original installer manifest.

```text
Setup.exe → install → state.db → uninstall.exe + state.db
```

Uninstall queries actual installed state and removes what TigerSetup owns.

**A committed uninstall leaves nothing of TigerSetup's own behind** — no
database, no uninstaller copy, no logs — whichever executable ran it. The
uninstaller copy is the only complicated case, because it must remove the
directory it lives in: it moves itself aside into `%TEMP%\TigerSetup` first
and schedules its own deletion afterwards. An installer that uninstalls runs
from somewhere else entirely, so nothing in the directory is held open and it
is simply removed. A rolled-back install is different: it never became an
installation, and the database that recorded the attempt stays for diagnosis.

**The uninstaller lives in the state directory**, beside the database:
`%ProgramData%\TigerSetup\<ProductId>\uninstall.exe` for machine scope and
the `%LOCALAPPDATA%` twin for user scope, with the Add/Remove Programs
`UninstallString` and `QuietUninstallString` pointing there. The install root
is what an upgrade rewrites, so an uninstaller living there would be replaced
in the middle of the transaction that might need it; in the state directory
the engine that can read the current journal is always the one that wrote it,
an upgrade replaces the uninstaller as bootstrap rather than as an owned
resource, and the uninstaller survives a user deleting the install root by
hand. The uninstaller is the engine block of the installer, copied durably
before any transaction is opened.

### 5.9 Reconciliation model

```text
desired state from manifest
+ owned state from database
+ actual Windows state
→ plan
→ journal
→ apply
```

The same model supports install, upgrade, uninstall, verify and eventually
repair. Repair is not a second installer engine; it is reconciliation using the
same resource model.

### 5.10 Running applications and files in use

TigerSetup follows normal Windows installer conventions here rather than
inventing a TigerSetup-specific cooperative shutdown and restart protocol.
**Windows Restart Manager** is the mechanism for detecting which processes hold
files that are about to be replaced, asking them to shut down, and restarting
them afterwards; an application that wants to come back cleanly registers with
`RegisterApplicationRestart`.

The division of responsibility:

- the **application** saves and restores its own user and session state;
- **TigerSetup** coordinates shutdown and quiescence before it mutates
  anything, and restarts what it stopped after a successful upgrade or after a
  rollback, as appropriate.

Two properties are the design's rather than the implementation's:

- **What decides whether the run may go on is who still holds the files**, not
  what the shutdown call returned. Windows reports success once it has stopped
  what it could, and an application with no message loop is asked and simply
  never answers. **That question is asked of the machine, not of the Restart
  Manager's own list**: `RmGetList` answers with the applications the session
  was told about when the resources were registered and keeps naming one that
  has already exited, so a run that waited for *that* list to empty would wait
  for something that never happens and refuse every upgrade over a running
  application, however promptly it closed. Whether a holder is still there is a
  question about its process, asked by the identity the Restart Manager itself
  uses — the process id together with the moment it started, so a reused id is
  not mistaken for it.
- **A graceful shutdown is a request, and a request takes time to honour.** An
  application asked to close has a window to answer, work to save and a process
  to end, so the holders are re-listed over a bounded grace period before
  the run concludes that anyone refused. Re-listing immediately reports an
  application that is doing exactly what it was asked to do as one that would
  not, and fails an upgrade against a co-operating application.

  **How long the grace period is depends on what Windows can ask of the
  holder.** A windowed application is closed by messaging its windows, which is
  the request that works, so it is given real time to save and exit; a holder
  the Restart Manager found no window for is asked in a way it may never
  answer, and waiting the same time for it only delays the refusal. The
  Restart Manager's own classification of each holder is what decides which,
  and it is recorded beside the holder in the log and in the in-use message,
  because "still running after the grace period" otherwise reads identically
  for an application that refused and one that was never asked.

Quiescence is what makes replacing a file in use *pleasant*, and the journal is
what makes the transaction *safe*. A holder still there at the end is
`package_in_use` with nothing mutated, which is one of the two acceptable ends.
Forcing a running application to die is never the other one.

**Only a file something holds is put to the Restart Manager.** Before the
session is opened, every file the plan replaces or removes is probed with an
open for `DELETE` and write access that grants every sharing mode, and a file
that refuses neither has no holder, so it is not registered. Held means in
use the way Windows means it, and the two kinds of holder refuse different
halves of that open: a data file an application keeps open without delete
sharing refuses the delete, which is what would make the replacement or the
removal fail; the image of a running program — its executable and the DLLs
it has loaded — is mapped with delete sharing, so Windows lets it be renamed
from under the process and refuses only the write. A probe for delete access
alone therefore calls a running application's own files free, and an upgrade
would replace them under the live process without the Restart Manager ever
being asked — which is the case the Restart Manager exists for. The probe
asks for no data and writes none, which matters: the Restart Manager opens
every file it is given to find its holders, and on files an installation has
just written that open is what a real-time scanner reads each of them for —
seconds per thousand files, spent to learn that nobody holds them. A file
whose write is refused for a reason that is not a holder — a read-only
attribute, an access control list — is probed for delete access alone.

**Package-declared quiescence.** The Restart Manager closes an application by
messaging its windows, so a process with no window to message — a tray helper,
a service-like background process, a detached worker — is listed as a holder
and never closed, and every upgrade of such a product would end
`package_in_use`. A `[[quiescence]]` entry is the package's own answer, and it
is a structured lifecycle rather than a pre-install action moved earlier:

```toml
[[quiescence]]
name = "viewer"
run_on = ["upgrade", "reinstall", "repair", "uninstall"]   # the default
not_running_codes = [3]

[quiescence.stop]                     # the custom action's envelope (§5.14)
kind = "exe"
command = "%INSTALLROOT%\TigerMarkView.exe"
arguments = ["--quit"]
timeout_seconds = 30

[quiescence.resume]                   # optional
kind = "exe"
command = "%INSTALLROOT%\TigerMarkView.exe"
arguments = ["--background"]
```

- **When.** The stop program runs before the Restart Manager is asked and
  before the transaction opens, on the operations the entry names — by
  default upgrade, reinstall, repair and uninstall, the operations that find
  an installation whose application may be running; `install` may be named
  for a package whose stop program does not need the product's files. An
  installing run uses the package's entries; an uninstall uses the entries
  the installation recorded when it was installed, with their packaged
  programs kept in the state directory beside the uninstall actions, because
  the installer that brought the product is usually gone by then.
- **What the stop reports.** Its `success_codes` (`[0]` by default) mean the
  application was running and is now stopped; `not_running_codes` mean
  nothing was running; any other exit code, a timeout or a launch failure is
  a failed quiescence, which `on_failure` decides — `fail` (the default) ends
  the run with `quiescence_failed` before anything is mutated, `continue`
  records the failure (`quiescence_failed_continued`) and lets the Restart
  Manager have its turn. The program runs with the custom action's envelope:
  `exe`, `powershell` or `cmd`, a command on the target or a packaged file,
  arguments passed separately, hidden, captured, bounded by a job object, told
  about the run through `TIGERSETUP_*` (with `TIGERSETUP_PHASE=quiesce`).
- **Only what was stopped is resumed.** TigerSetup records which entries
  reported the application running; an application that was not running
  stays not running. The resume program is started **detached** — not waited
  for, not bounded, not captured, with the run's token, shown as it starts —
  and its start is recorded; it is the one program TigerSetup starts and does
  not wait for, because its purpose is to outlive the installer.
- **Every failure past the stop resumes.** Whether the Restart Manager then
  finds another holder that will not close, a dependency cannot be acquired,
  or the transaction rolls back, the application is started again before the
  failure is reported: a refusal never leaves it stopped for nothing. After a
  successful install, upgrade, reinstall or repair it is resumed as designed.
- **Uninstall stops and never resumes;** a rolled-back uninstall resumes,
  because the product is still there. A run that leaves its transaction open
  — a rollback that itself failed — does not resume (`quiescence_not_resumed`):
  the product's files are in no state to run from, and the recovery that
  settles them is the run that should decide.
- **Nothing is claimed about side effects.** What the stop program did to the
  machine is the package author's, exactly as a custom action's is; TigerSetup
  records that it ran, what it reported, and what it started again — in the
  log, in `action_run` rows (phases `quiesce` and `resume`) and in the outcome
  document's `actions` and findings (`quiescence_stopped`,
  `quiescence_not_running`, `quiescence_resumed`, `quiescence_resume_failed`).
  A crash while the application is stopped is not a normal failure path: the
  next run's recovery settles the product, and the application is started by
  whoever starts it next.

**What this asks of the application.** Respond promptly to the normal Windows
close or session-ending request; a service stops cleanly through the Service
Control Manager rather than being signalled directly. Release file handles as
soon as shutdown begins — a lock held past that point is what turns a
cooperating close into `package_in_use`. Save whatever state the next launch
needs before exiting, since nothing asks again. Do not run a watchdog or
background helper that relaunches the application while Restart Manager is
closing it for servicing; that reads as a refusal, not a race TigerSetup will
retry. An application that wants to come back automatically once servicing is
done registers with `RegisterApplicationRestart` before the close request
arrives, so Restart Manager knows to relaunch it.

The exact Restart Manager API flow is otherwise an implementation decision.

### 5.11 Security

Security is part of the installation-state architecture, especially for machine
scope:

- the state database and backup files need strict ACLs;
- an unelevated user must not be able to tamper with data that an elevated
  uninstall later trusts;
- stored paths must be validated before privileged operations;
- resource roots must be constrained;
- arbitrary scripts are avoided; operations are typed and validated, and
  the one exception — a custom action (§5.14) — runs only bytes the
  package carries and hashes, or a program the package names, never
  anything acquired at run time; a stored program is verified against its
  recorded hash before an elevated uninstall runs it.

Otherwise a privileged uninstall becomes a confused-deputy mechanism.

### 5.12 Migrating a legacy installation

A product moving to TigerSetup from another installer technology declares its
legacy footprint in the package definition: the legacy installer type and the
legacy registration key it wrote. The model is generic — Inno Setup is the
first type, and NSIS or another installer is added as another type, never as
brand-specific architecture.

```toml
[legacy]
installer_type   = "inno"
registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
```

Migration is **uninstall-first**. When the declared legacy registration is
present, the installer runs the quiet uninstall command that registration
records, once, outside the product transaction, verifies that the registration
is gone, and only then installs into a fresh TigerSetup-owned installation. The
old uninstaller is the proven tool for removing what it installed; adopting a
foreign footprint in place would be engine work whose only beneficiary is the
migration. A failure after the legacy uninstall is a clean `absent`, never a
hybrid.

TigerSetup **prefers a new registration identity** derived from the package
id. A package may explicitly preserve its legacy registration key name
(`[registration] key_name`) when an external consumer requires it; TigerMarkView
does not, and takes the new identity. The WinGet `PackageIdentifier` is not
part of the registration identity and stays unchanged across the transition.

WinGet permits that transition. Nothing in the manifest schema, the
`winget-pkgs` validation pipeline or its published policy gates a change of
`InstallerType` between versions of one package, and the client's only
technology gate is a compatibility set in which `inno`, `nullsoft`, `exe` and
`burn` are interchangeable, so an installed Inno version upgrades to an `exe`
version. WinGet correlates the installed legacy version with the package through
the community index, which aggregates the ProductCodes of every retained
version, and through the display name and publisher — so the legacy version's
manifest **stays in `winget-pkgs`**, the new manifest declares only the
registration TigerSetup actually writes, and `UpgradeBehavior` stays `install`,
because the migration lives in `Setup.exe` and must run identically whether
WinGet or a person starts the installer.

### 5.13 Cross-scope installation policy

A product can be installed per user and per machine at the same time, and each
installation is its own: its own state database (§5.2), its own install root,
its own Add/Remove Programs registration. So before a run touches anything it
decides **which installation it is about**, and it never silently creates a
second one beside an existing one. The decision is the engine's, one place both
clients reach, so the command line and the wizard answer it identically.

The rules, over the installations the machine actually holds:

- **One existing installation is sticky.** A run that names no scope continues
  with that installation, whichever scope it is in — an ordinary rerun of
  `Setup.exe` upgrades or repairs what is there rather than treating the
  package's default scope as a fresh-install opportunity. A machine install
  plus an ordinary rerun upgrades the machine installation; likewise for a
  per-user one.
- **An explicit scope stays explicit.** A run that names a scope which holds
  nothing, while the *other* scope holds the product, is a **scope conflict**,
  not a silent redirection: automation gets a structured `scope_conflict`
  result naming what exists, and nothing is installed. It is never rewritten to
  the existing scope.
- **Two existing installations are never chosen between.** With both scopes
  installed, a run that names no scope is a **scope ambiguity**: the interactive
  wizard shows the two and asks which, and automation gets a structured
  `scope_ambiguous` result. Each installation stays independently owned until
  one is named.
- **No implicit migration.** Changing an installation's scope is materially
  different from an upgrade — it can mean uninstall/reinstall, and different
  ownership, state and elevation — so nothing here turns user→machine or
  machine→user into an automatic migration. Scope migration, if it is ever
  added, is a separate explicit capability.

`first install` — no installation of the product anywhere — uses the package's
configured default scope (the first `[install].scopes` entry), offers only the
scopes the package allows, and lets the interactive wizard choose among them.

The consumer states three independent things: the **default scope** for a first
install (`scopes` order), the **allowed scopes** (`scopes`), and the
**cross-scope policy** — `[install].existing_scope`:

```text
preserve         # the default: continue with the existing installation;
                 # refuse an explicit request for the other scope
allow-parallel   # an explicit request for the other scope creates a second,
                 # independent installation
error            # refuse every run whose scope, named or defaulted, is not the
                 # one the product is installed in
```

Under `allow-parallel` the explicit second-scope request is honoured and the
two installations coexist; a later scope-less run of a two-installation machine
is then ambiguous as above. Reading (`verify`, `inspect`) and removing or
repairing an empty scope are never conflicts — they report what that scope
holds, which may be nothing; only an install can create a second installation,
so only an install is refused by policy.

### 5.14 Custom lifecycle actions

TigerSetup strongly prefers a typed resource wherever it understands the
operation — a file, a registry value, a shortcut, a firewall rule — because
a typed resource is owned, journaled, rolled back, verified, repaired and
removed by the engine. Real applications also need product-specific work at
install or uninstall time that no typed resource can express: building a
cache, registering with a service the product ships, migrating a settings
store, cleaning up what the product generated while it ran. A **custom
action** is the deliberate, first-class capability for that work, and it is
not a scripting escape hatch: it is a program the package declares, TigerSetup
packages and verifies, starts under a controlled envelope at a defined point
of the run, and records. The line between the two is the whole design:

```text
typed resource   → TigerSetup knows what changed: ownership, rollback,
                   repair, verification
custom action    → TigerSetup knows what it started, when, with what
                   result; what the program changed on the machine is the
                   package author's responsibility
```

> **TigerSetup can roll back the resources it owns and understands. It
> cannot guarantee rollback of arbitrary side effects produced by a custom
> action.**

**Phases and operations.** An action runs at one of four phases, and on the
lifecycle operations it names:

```text
pre-install      dependencies satisfied; the first operations of an installing
                 transaction, before any product resource is mutated
post-install     every product resource applied and every removal done; the
                 last operations before the commit
pre-uninstall    the first operations of an uninstall, before any owned
                 resource is removed
post-uninstall   every owned resource removed, the install root included; the
                 last operations before the uninstall commit
```

`run_on` names the operations — `install`, `upgrade`, `reinstall`, `repair` for
an install phase, `uninstall` for an uninstall phase; a phase never runs on an
operation of the other kind, and the builder refuses the combination rather
than ignoring it. The default is the phase's operations without `repair`:
repair is opt-in, because an arbitrary program is not necessarily idempotent,
and a repair that runs every action merely because it exists would repeat
work the package author never meant to repeat. An upgrade runs the *new*
package's install-phase actions; it never runs the previous installation's
uninstall actions, which belong to an uninstall alone. A same-version rerun
with nothing to reconcile (`already_installed`) opens no transaction and runs
nothing.

**One predicate.** An action carries the same `when = { option, equals }`
predicate as every optional resource (§4), evaluated against the same
effective option set — explicit value, else the last committed value, else
the manifest default. There is no action-specific condition language and no
action-specific option state; an uninstall action's predicate is evaluated
when the uninstall runs, against the options the installation recorded.

**Kinds and programs.** An action is `exe` (a native executable, run
directly), `powershell` (a script run by Windows PowerShell:
`powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File`) or
`cmd` (a batch script run by `cmd.exe /d /s /c`). The interpreters are
invoked explicitly and non-interactively, with no profile and no dependence
on the user's execution policy — the package is the trust boundary, and a
script it carries is run as the package's. The program is either a
**command** on the target machine — a template such as
`%INSTALLROOT%\tools\cache-builder.exe`, expanded with `%INSTALLROOT%`,
`%VERSION%` and the known folders, the same placeholders every other template
in the manifest uses — or a **packaged** file (`source`), which the builder
reads, hashes and carries inside the installer under
`.tigersetup/actions/<file name>` (§10.4), beside the embedded dependency
installers and with the same integrity: `verify` checks its bytes against
the recorded SHA-256, `inspect` shows the identity, and the engine verifies
the bytes again after extracting them and refuses to run bytes that do not
match (`action_program_mismatch`). A pre-install action cannot assume the
product's files exist yet, and a post-uninstall action cannot use the install
root at all — it is gone by then — so the builder refuses a post-uninstall
command or working directory under `%INSTALLROOT%`; the packaged form is what
both phases use. Nothing is ever downloaded and run.

**The envelope.** Arguments are declared as a list, expanded as templates and
passed as separate arguments, never joined by a shell; the working directory
defaults to the program's own directory; the process is started hidden, with
no standard input and both output streams captured; and it is told about the
run through its environment — `TIGERSETUP_INSTALL_ROOT`, `TIGERSETUP_VERSION`,
`TIGERSETUP_PRODUCT_ID`, `TIGERSETUP_SCOPE`, `TIGERSETUP_OPERATION`,
`TIGERSETUP_PHASE`, `TIGERSETUP_ACTION`, `TIGERSETUP_QUIET`. The action and
everything it starts live in a job object: `timeout_seconds` (300 by default)
ends the whole tree, and so does the action's own exit, so nothing an action
started outlives it — an action is bounded by definition, and a program that
must keep running belongs to the product, not to its installer. The exit code
is judged by the declaration: `success_codes` (`[0]` by default) is success,
`reboot_codes` is success with a reboot pending — carried by the same
`reboot_required` and exit code 3010 as a dependency's — and anything else,
a timeout, or a program that cannot be started is a failure. `on_failure`
decides what a failure means: `fail` (the default) fails the transaction,
which rolls back what TigerSetup owns; `continue` records the failure as a
finding (`action_failed_continued`), reports it in the outcome, and goes on.
`continue` never turns a failure into a silent success.

**Execution context.** An action runs with the token of the run it is part
of: a machine-scope install or uninstall runs it elevated, a per-user one
runs it as the user, and TigerSetup never elevates an individual action
behind the caller's back. There is no per-action elevation or impersonation.
A quiet run and an interactive run start an action identically —
non-interactive, hidden — and TigerSetup manufactures no prompt on the
program's behalf; whether the program itself can run unattended is the
package author's responsibility, which is why `TIGERSETUP_QUIET` is passed.
Because an action may run arbitrary code, and in an elevated installer, the
package must be trusted as a whole: actions are not sandboxed, and everything
that makes the package inspectable (§10.5) is what makes them auditable.

**In the transaction.** An action is a journaled operation (`run_action`)
with the same `planned → prepared → applying → applied` states as every
resource, placed by its phase — pre-actions first, post-actions last, with
the uninstall actions an installing transaction records for the installation
(below) just before the post-install ones. What differs is the undo: a run
has none. When a failing action rolls the transaction back, the typed
resources are put back and the action's own record stays exactly as it is —
`action_run` says it ran and failed, the outcome names it as the cause with
its exit code and output, and the rollback records `action_not_reverted` for
every action that ran, so nothing reads as if the program's effects were
undone.

**Evidence.** Every execution writes an `action_run` row (§5.3) — the
action's name, phase, operation, kind, resolved program and policy —
`started` **before** the process exists and finished with its status
(`completed`, `failed`, `timed_out`, `launch_failed`), exit code and reboot
flag after it exits. The log carries the launch (`action_started`, with the
command line and the envelope), the captured output line by line
(`action_output`, bounded) and the verdict; the outcome document carries
every action with its status, exit code, duration, policy, the tail of both
streams and the packaged program's hash. Failures name the action by its
stable name. Arguments are logged as they were passed, so an author who
must pass a secret should pass it through a file the action reads rather
than on the command line.

**Crash and recovery.** A crash while an action runs leaves its `action_run`
row `started`, which is how the next run tells it from an action that never
ran. The existing recovery (§5.4) decides the direction: a **forward**
recovery — the same package running again — marks the row `interrupted`,
reports `action_interrupted`, and runs the action again, then completes the
transaction; a **rollback** recovery marks it `interrupted`, never runs it,
and records `action_not_reverted`. An interrupted action has unknown external
side effects, and TigerSetup claims nothing about them; what it guarantees is
that its own state converges and that the evidence is preserved. This is the
one rule package authors must design for: **an action must be idempotent,
safe to retry, bounded, non-interactive, and able to detect work it already
did**, because TigerSetup will run it again after a crash, and may run it on
every operation it names.

**Uninstall actions belong to the installation.** The installer that brought
a product is usually gone by the time it is uninstalled, and the uninstaller
copy in the state directory carries no payload. The committed installation
therefore keeps what its own uninstall will need: the definitions of its
pre-uninstall and post-uninstall actions in the `action` ownership table
(§5.3), and the bytes of every packaged program under
`<state directory>\actions\<sha256>\<file name>`. An installing transaction
records them with `store_action` operations, content-addressed so that a new
version's program never overwrites an old one; the commit switches the
`action` table with the rest of the ownership tables, so **a successful
upgrade makes the new definitions and programs current, and a failed or
rolled-back upgrade leaves the previous ones exactly as they were**; a
program nothing owns any more is swept after the commit, and a stored
program's directory a rollback finds it created is removed. Uninstall plans
its actions from the database, never from the metadata of whichever
executable runs the uninstall, and verifies each stored program against the
recorded hash before running it; `verify` reports a stored program that is
missing or modified (`action_program_missing`, `action_program_modified`) and
`repair` restores it from the package. An install-phase program is not kept:
it is extracted into the transaction's staging area for the run and removed
with it.

**Inspectable.** A package that runs arbitrary programs is obvious wherever
the package is described: `tiger-setup inspect` lists every action with its
name, phase, operations, kind, program, packaged identity and hash,
arguments, timeout, exit codes, policy and predicate; `Setup.exe inspect`
lists the same under `package.actions`, and for an installed product the
stored uninstall actions under `owned.actions`.

**Deliberately not here.** No compensating or rollback action per action, no
per-action elevation, no network acquisition of action programs, no
action-specific condition language, and no way for an action to keep a
process running after it ends. Each is added only on a concrete requirement,
and "do not become MSI by accident" (§16) is the standing objection.

---

## 6. Execution model

### 6.1 One engine, two clients

```text
                    Inputs
                      |
          +-----------+-----------+
          |                       |
    unattended CLI            interactive UI
          |                       |
          +-----------+-----------+
                      |
                desired state
                      |
                  plan engine
                      |
             transaction engine
```

There must never be separate interactive and silent implementations that can
drift apart, and the UI must never become a second home for installation logic.

### 6.2 Automation-first, interactive-capable

> **Installation is an automatable state transition. Interaction is optional.**

Every install, upgrade and uninstall path must have deterministic unattended
semantics:

- deterministic defaults and no required prompts;
- stable command-line behaviour and stable exit codes;
- unattended install, upgrade and uninstall;
- machine-readable validation/status;
- useful logging;
- dependency handling without desktop interaction;
- deterministic recovery/failure semantics;
- repeatable behaviour in TigerWinLab.

The unattended contract — a public contract the WinGet manifest, the lab
specifications and the release gate all encode:

```text
Product-Setup.exe                                   the root operation, interactive
Product-Setup.exe install   [--quiet] [--scope user|machine] [--install-root <path>]
                            [--option <name> <on|off>]... [--no-dependency-install]
                            [--lang <tag>] [--log <path>] [--json]
Product-Setup.exe uninstall [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Product-Setup.exe repair    [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
Product-Setup.exe verify    [--scope user|machine] [--json]
Product-Setup.exe inspect   [--scope user|machine] [--json]
Product-Setup.exe install --quiet --fault <point>[@<sequence>]:<action>[:<seconds>]   (fault injection, every build)
```

`verify` and `inspect` take no `--log` because they change nothing at all: a
default log would live in the state directory and so would create it for a
package the machine does not have. A read-only command writes only to its
standard output.

The grammar is the command-app grammar shared with the other Tiger tools:
`app <command> <positional arguments> [options]` — commands express
operations, positional arguments identify their subjects, and options modify
behaviour. An option takes its value as a separate argument (`--log <path>`,
never `--log=<path>` in documentation or specifications). Without `--quiet`
an install, uninstall or repair is interactive; running `Setup.exe` without
arguments is the root operation — `install` for an installer, `uninstall`
for the uninstaller copy in the state directory (§5.8) — so a double-click
and a package manager both do the expected thing. `--option` sets a declared
installer option such as the PATH entry or the desktop shortcut; an option
not named keeps the value the installation recorded, or the declared default
on a first install. `--scope` names the installation to act on; omitted, the
run follows the installation the machine holds (§5.13). Exit codes, aligned
with WinGet's return-code types; machine-readable output carries the same
identifiers as `code` strings:

| Exit | Meaning |
|---|---|
| `0` | success |
| `1` | failed and rolled back |
| `2` | invalid arguments or package, including a `scope_conflict` or `scope_ambiguous` cross-scope refusal (§5.13) |
| `3` | dependency missing or unacquirable |
| `4` | elevation required or refused |
| `5` | cancelled |
| `6` | package in use — an application would not close |
| `7` | an earlier transaction needs recovery and could not be completed |
| `8` | unsupported platform |
| `3010` | success, reboot required |

**UI displayed is not the same as interaction required.** Double-clicking
`Setup.exe` for an ordinary application such as TigerMarkView may show progress
and offer optional choices, but installation has sensible deterministic defaults
and requires no human decision.

A future application may genuinely need installation parameters. TigerSetup
should be able to model such parameters independently of the UI and obtain
values from CLI arguments, configuration files, the environment, existing
installation state, or optional interactive input.

### 6.3 Machine-readable output is language-independent

Human-readable output may be localized. Machine-readable output must not be.

```json
{
  "code": "dependency_missing",
  "dependency": "Microsoft.DotNet.DesktopRuntime.10"
}
```

Stable identifiers, never localized text — so AI agents, TigerWinLab, CI,
release automation, support tooling and package-manager integration never parse
translated strings to determine an outcome.

---

## 7. Dependencies

### 7.1 Dependencies are requirements, not owned resources

```text
Resource
    owned by this Installation
    lifecycle managed by TigerSetup
    removed during uninstall when safe

Dependency
    requirement that must be satisfied
    may already exist
    may be installed by TigerSetup
    normally remains externally/shared owned
    normally NOT removed with the application
```

Installing WebView2 because TigerMarkView requires it must **not** mean that
uninstalling TigerMarkView removes WebView2 — another application may now depend
on it. TigerSetup may record that a dependency was installed during a
transaction, but:

> **Installed by TigerSetup does not imply owned by TigerSetup.**

The same holds when a product transaction fails: rolling back an installation or
an upgrade does not uninstall a shared dependency TigerSetup caused to be
installed along the way (§5.4).

### 7.2 Dependency model

The dependency engine separates at least these concerns:

```text
Identity        What dependency/capability is required?
Detection       Is the requirement already satisfied?
Acquisition     Where can an installer that satisfies it be obtained?
Installation    How is it installed unattended?
Verification    Is the requirement satisfied after installation?
```

The separation matters because a package identifier and an application
capability are not always the same thing. TigerMarkView conceptually requires a
compatible .NET Windows Desktop Runtime 10; `Microsoft.DotNet.DesktopRuntime.10`
is the package identity that acquires it, while what proves it present is a
version directory under the shared runtime root. WebView2 has the same shape,
with a registry value as the proof.

**TigerSetup stays generic.** The engine carries no catalogue, library or
policy for any named product — nothing in it knows what .NET or WebView2 is.
A dependency is declared in the package with a **typed detector** and an
**acquisition source**, and every product-specific fact lives in that
declaration:

```toml
[[dependencies]]
id = "Microsoft.DotNet.DesktopRuntime.10"     # the requirement: a WinGet identity
minimum = "10.0"                               # same major, not lower
detect = { kind = "directory-version",
           path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App",
           pattern = "10.*" }

[[dependencies]]
id = "Microsoft.EdgeWebView2Runtime"
detect = { kind = "registry-version",
           keys = ["HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
                   "HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
                   "HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"],
           value = "pv" }
```

The detector kinds are `directory-version` (the child directories of a path
are versions), `registry-version` (a value read as a version from the first
key that has it), `file-version` (a file's Windows version resource) and
`registration` (an Add/Remove Programs entry matched by display name). A
version satisfies the requirement when it has the minimum's major component
and is not lower; no minimum means any version. Acquisition defaults to the
WinGet catalog entry for `id` (§7.9); a dependency with no catalog entry
declares `acquire = { url, sha256 }` and `install = { arguments,
success_codes, reboot_codes }` instead, and is never refreshed.

### 7.3 Scope: two real prerequisites, no framework

The dependency model is proven on the two prerequisites a real application
needs — `Microsoft.DotNet.DesktopRuntime.10` and
`Microsoft.EdgeWebView2Runtime` — and its scope stays deliberately narrow: a
correct generic model, exercised on real prerequisites, rather than a
universal prerequisite framework, and never by special-casing either of them.

### 7.4 Custom developer-defined dependencies

The dependency model must be broader than WinGet's package model — a developer
must be able to define a dependency with no WinGet entry, describing enough for
TigerSetup to `detect → acquire → verify installer bytes → install unattended →
verify result`.

```text
TigerSetup dependency identity
        |
        +-- WinGet mapping (when available)
        |
        +-- Chocolatey mapping (optional, when useful)
        |
        `-- custom / no external mapping
```

Typed detection mechanisms may include registry values, file/version presence,
executable/version checks and built-in dependency-specific detectors. Arbitrary
scripting is resisted unless typed mechanisms are demonstrably insufficient.

### 7.5 Offline behaviour

> **TigerSetup itself must never require Internet access merely to execute.**

Network access is only an optional acquisition mechanism for missing application
dependencies.

```text
Fresh supported Windows
.NET Desktop Runtime 10 present
WebView2 Runtime present
NO Internet connection
        ↓
TigerMarkView-Setup.exe
        ↓
installation succeeds
```

This is a hard requirement, not an aspiration.

### 7.6 Offline failure behaviour

If a required dependency is missing and acquisition needs Internet access that
is unavailable, installation fails cleanly:

```text
dependency missing → network unavailable → dependency cannot be acquired
        ↓
application installation does not commit
        ↓
clear deterministic failure
```

TigerSetup must not leave a partially committed application installation because
dependency acquisition failed. The installer reports which dependency is
unsatisfied, why it could not be acquired, whether any system changes were made,
and whether rollback or cleanup was performed.

### 7.7 Remote and embedded acquisition

The same dependency requirement can be satisfied by remote acquisition **or**
an embedded payload, enabling two package styles:

```text
Small online-capable installer        Larger fully-offline installer
  Setup.exe + app payload               Setup.exe + app payload
  → download dependency if needed       + .NET Desktop Runtime installer
                                        + WebView2 Runtime installer
```

`acquire = { file = "<manifest-relative .exe or .msi>" }` embeds the
installer: the builder carries its exact bytes as a payload entry under the
reserved `.tigersetup/dependencies/` prefix, which no product file may use,
and records the entry name, size and SHA-256 in the dependency's acquisition
metadata. Everything else is the model of §7.2 unchanged: detection first,
so a satisfied requirement extracts nothing; extraction to the state
directory only for a missing one, refusing bytes whose hash differs
(`dependency_unacquirable`, `hash_mismatch`); the declared unattended
switches, success and reboot codes, elevation and re-detection exactly as
for a download; the extracted file removed with the phase. `tiger-setup
verify` checks every embedded installer against its recorded hash and size
without running anything, and `inspect --json` reports the dependency's
source as `embedded` with what it carries. A dependency may carry the one
predicate of §4, so an optional component's prerequisite is a requirement
only while the component is selected.

### 7.8 Acquisition policy

Resolving and acquiring missing dependencies is a TigerSetup strength, not an
exception. When a required dependency is missing and the machine is online,
the installer **acquires and installs it by default**, unattended and
interactive alike. `--no-dependency-install` opts out; a run that opted out
fails with `dependency_missing` before any product change.

A dependency whose installer needs elevation — a machine-wide runtime required
by a user-scope installation run by a standard user — never becomes an
unexpected prompt: an unattended run fails deterministically with
`dependency_requires_elevation` before any product change, and an interactive
run asks for elevation for the dependency alone.

### 7.9 The requirement is pinned, the artifact is refreshed

A dependency declaration pins the **requirement** — the WinGet package
identity and the version requirement (same major, at least the declared
minimum) — and that is what is durable. The URL, version, hash, unattended
switches and return codes the builder resolves from the WinGet catalog at
build time are embedded as an **acquisition hint**: refreshable acquisition
metadata, a cache, never runtime truth. The engine uses the hint while it is
fresh, and refreshes from the catalog at install time when the hint is older
than its lifetime, has no URL (an offline build), or fails — a missing
artifact, a download that cannot be completed, bytes that do not match. A
refresh selects the current package that satisfies the requirement and
verifies the download against the refreshed hash. TigerSetup never maintains
a dependency package library of its own, never installs an unverified
download — there is no unhashed fallback of any kind — and never needs
`winget.exe` on the target (§8.1). Offline, a missing dependency with no
usable hint is a clean `dependency_unacquirable` before any product change.

The catalog the engine refreshes from is the WinGet community source as
Microsoft serves it, which needs no client: a pre-indexed SQLite database
(`cdn.winget.microsoft.com/cache/source2.msix`), a per-package compressed
version list naming every manifest with its SHA-256, and the merged manifest
itself with `InstallerUrl`, `InstallerSha256`, the silent switches and the
return codes — three plain HTTPS reads over WinHTTP. Every acquisition
source is generic: the same chain serves any package identifier, and a
dependency declared with a fixed URL and hash bypasses it.

Staleness is decided by **age or a missing artifact, not by a hash mismatch**:
a versioned artifact keeps serving the bytes its hash describes long after a
newer one exists, so a mismatch never fires. The hint lifetime is per
dependency (`max_age_days`, two weeks by default). Refreshing is on by
default: the requirement, the detector and the hash check are what is
proven, and validation proves the behaviour, not one artifact. Detection
always comes first, so a machine whose dependencies are present never
touches the network.

---

## 8. Distribution

### 8.1 WinGet-aligned, WinGet-independent at runtime

WinGet support is not bolted on later: a TigerSetup-built installer is
WinGet-ready by default, and the same metadata drives one identity throughout.

```text
TigerSetup package identity
        ↓
Add/Remove Programs identity
        ↓
WinGet package identity
```

This prevents drift between installer metadata, ARP metadata, release metadata
and WinGet manifests.

TigerSetup aligns with WinGet package identifiers, community package metadata,
dependency identities, scope/architecture concepts, silent-installer
conventions, stable exit behaviour and manifest generation. For public
dependencies the WinGet package identifier is normally the preferred external
identity where a good mapping exists.

```text
TigerSetup.toml
      ↓
tiger-setup build
      ↓
resolve dependency metadata from a WinGet-compatible catalog   ← build time
      ↓
capture the information the generated installer needs
      ↓
Setup.exe
      ↓
detect / download / verify / install dependency itself         ← run time
```

The target machine must never need `winget.exe` — nor TigerSetup, Rust, .NET or
anything else — to execute the setup engine.

> **WinGet is a catalog/integration model, not a runtime prerequisite for
> TigerSetup installers.**

### 8.2 WinGet manifest generation

The same dependency declarations feed both TigerSetup's standalone installer
behaviour and generated WinGet dependency metadata where a clean mapping exists:

```text
                    TigerSetup.toml
                           |
            +--------------+---------------+
            |                              |
     TigerSetup Setup.exe            WinGet manifests
            |                              |
 detect/install/verify               PackageDependencies
```

If WinGet installs the prerequisites first, TigerSetup verifies they are
satisfied and continues. If the user downloads `Setup.exe` directly, TigerSetup
handles the same prerequisites itself. Both paths stay correct.

The final WinGet installer manifest needs the immutable public installer URL and
SHA-256, so generation is a two-stage workflow:

```text
tiger-setup winget prepare
    → manifests with unresolved / expected publication values

tiger-setup winget finalize --url https://.../TigerMarkView-0.9.0-Setup.exe
    → hash computed from the published exact bytes, final URL filled,
      consistency validated, submission-ready manifests
```

> **Build once, validate exact bytes, publish those exact bytes. Never rebuild
> just for WinGet.**

Automatic submission to `winget-pkgs` remains a separate release/policy
decision.

### 8.3 Chocolatey compatibility

> **WinGet-aligned by design; Chocolatey-compatible by convention.**

TigerSetup does not distort its core model to make both ecosystems equally
native. WinGet shapes dependency identities, metadata alignment, installer
automation conventions and release output. Chocolatey compatibility comes from
producing a conventional, predictable Windows installer with reliable silent
install and uninstall, stable exit codes, deterministic scope behaviour,
logging, upgrade support, correct ARP registration, predictable reboot/failure
behaviour, and dependency metadata that can be mapped when appropriate.

A Chocolatey package should be a thin wrapper around a TigerSetup-generated
installer, never a second installer implementation. Both of these must remain
possible, and the installer must be correct either way:

- Chocolatey manages mapped dependencies before invoking `Setup.exe`; or
- the Chocolatey package stays thin and `Setup.exe` handles its own
  dependencies.

### 8.4 TigerSetup's own distribution

A TigerSetup release is the exact bytes of the self-hosted installer (below),
`TigerSetup-<version>-Setup.exe`, which installs `tiger-setup.exe` and
`tigersetup-setup.exe`; it is built on the release-quality path and verified
with `tiger-setup verify` before it leaves the machine that built it. There is
no public release channel yet: a release is distributed as that file with its
SHA-256, and a consuming project pins it by version and hash. When TigerSetup
is published at an immutable public URL, its WinGet package follows the same
§8.2 workflow it offers every consumer, on those same bytes.

TigerSetup's source repository is private, so it is not a destination for end
users. Product metadata that reaches a user — the Add/Remove Programs links and
the WinGet locale manifest — names only public destinations, today the
publisher's site (`https://www.ittiger.net/`), and omits an optional URL rather
than point where a user cannot go.

TigerSetup also **installs itself with itself**. `packages/tigersetup/`
(`ItTiger.TigerSetup`) is a normal TigerSetup package whose payload is the two
release binaries, built by TigerSetup's own current release engine on the
release-quality path — so the `Setup.exe` that installs TigerSetup is produced
by exactly the mechanism it produces for every other product, its product
version read from the packaged `tiger-setup.exe`'s VERSIONINFO (§9.2) rather
than typed. Every product coding session ends by rebuilding and verifying this
self-installer (`AGENTS.md`, *Version and release discipline*), which keeps the
whole build-and-package path honest against the product it most has to work
for.

---

## 9. Application metadata integration

Easy .NET integration is one of TigerSetup's strongest practical advantages: it
removes the per-project scripting that parses project metadata and feeds it to
an installer script. `[metadata].source` selects the provider — `static`
(everything typed in `[package]`, the default), `msbuild` or `exe` — and
`[metadata].executable` names a built binary that is validated against
whichever source is in use.

### 9.1 Evaluate MSBuild; do not parse XML

Values may come from `Version.props`, `Directory.Build.props`, imported
`.props`/`.targets`, conditions, SDK defaults or command-line properties.
TigerSetup uses **evaluated MSBuild properties**, not naive `.csproj` parsing:
`dotnet msbuild <project> -getProperty:…` evaluates the project without
running a target, and `--property Name=Value` on the command line (or
`[metadata].properties`) passes a global property to that evaluation.

```toml
[metadata]
source = "msbuild"
project = "../TigerMarkView/TigerMarkView.csproj"
executable = "publish/TigerMarkView.exe"     # validated against the project
```

The properties read are `Version`, `Product`, `Company`, `Description`,
`Copyright`, `FileVersion`, `InformationalVersion` and `AssemblyName`.
`dotnet` must be on `PATH` when the manifest names a project.

### 9.2 Metadata from compiled executables

```toml
[metadata]
source = "exe"
executable = "publish/MyApp.exe"
```

The Windows `VERSIONINFO` fields — ProductName, ProductVersion, FileVersion,
CompanyName, FileDescription, LegalCopyright — supply the version, description
and copyright, and validate the product name and publisher. This keeps
TigerSetup useful for C++ applications, Rust applications, third-party
binaries and anything without MSBuild metadata, and enables very small package
definitions: `[package]` with an id, a name and a publisher, `[metadata]`
naming the executable, and `[[files]]`.

### 9.3 Mixing and overriding sources

Metadata is composable: project/application metadata comes from the provider
while installer/distribution-specific values stay explicit in `[package]`,
and a value typed in `[package]` wins over the provider's.

```toml
[package]
id = "ItTiger.TigerMarkView"
name = "TigerMarkView"
publisher = "IT Tiger"
license = "MIT"

[metadata]
source = "msbuild"
project = "../TigerMarkView/TigerMarkView.csproj"
```

The source must be selectable and visible rather than relying on excessive
magic.

### 9.4 Provenance and validation

TigerSetup does not merely extract metadata; it shows where every value came
from. The providers are `static` (typed in `[package]`), `msbuild-project`
and `pe-version-info`, and `tiger-setup metadata` lists every candidate in
precedence order — `=` marks the value the build uses, `~` a value a
higher-precedence one displaced — with its source, the property or field it
was read from, and the file:

```text
version               = 0.8.2
                        msbuild-project · Version of source\src\TigerMarkView\TigerMarkView.csproj
description           = A local Markdown viewer, reviewer and PDF exporter.
                        msbuild-project · Description of source\src\TigerMarkView\TigerMarkView.csproj
validated             binary_product_version_matches · ProductVersion 0.8.2 of publish\TigerMarkView.exe matches the package version (Version)
validated             binary_company_name_matches_publisher · CompanyName IT Tiger of publish\TigerMarkView.exe matches package.publisher
```

`--json` gives the same as one document with stable identifiers (`source`,
`origin`, `file`, `effective`; each validation's `check`). Where the manifest
names an executable, the built binary is validated against the declared
source: version, product name and company name must agree. If the project
says `0.9.0` but the executable is still `0.8.0`, TigerSetup refuses to build
the installer — catching stale binaries before they become release artifacts.

Two different questions, two different answers, kept apart on purpose. The
generated installer's **Windows VERSIONINFO** (its `ProductName`,
`FileDescription`, `ProductVersion`, `CompanyName`, `LegalCopyright`,
`OriginalFilename`) answers *what product is this an installer for* — derived
from the manifest's product/publisher/copyright metadata, so a TigerMarkView
installer reads as TigerMarkView in Explorer, and TigerSetup does not put its
own name there. **Engine provenance** — *which TigerSetup engine built and runs
this installer* — stays in the embedded runtime metadata's `Engine` message:
the TigerSetup version, the SHA-256 of the release engine executable
(`engine_sha256`), and the SHA-256 of this installer's engine block after the
identity rewrite (`engine_block_sha256`). `tiger-setup inspect` reports both —
a `windows` object with the shell identity the file presents and an `icon`
array describing its executable icon, alongside the engine identity — and
`verify` checks the engine block against the recorded `engine_block_sha256`.
Engine identity is never leaked into `ProductName`/`FileDescription` to carry
provenance, and product identity is never written into the `Engine` message.

### 9.5 Version semantics

.NET distinguishes `Version`, `AssemblyVersion`, `FileVersion` and
`InformationalVersion`; Windows PE metadata distinguishes file and product
versions. TigerSetup normalizes rather than copying whichever value it finds
first:

```text
package_version       = 0.8.0
file_version          = 0.8.0.0
informational_version = 0.8.0+abc123
```

For a .NET project the evaluated MSBuild `Version` is the authority for the
installer/package version; EXE metadata is validation input.

---

## 10. Platform baseline and the generated installer

### 10.1 Supported Windows baseline

```text
Windows 10 1809 x64+
Windows 11 x64
Windows Server 2019 x64+
```

Windows Server 2016 is intentionally outside the support baseline.

Windows 10 1809 is the **API baseline**: every Windows API the engine and the
UI use must exist in 1809, checked by review. Validation runs on Windows
Server 2019 (build 17763, the 1809 code base) and on Windows 10 22H2
(`TigerSetup-Validation.md` §5); TigerSetup does not claim direct testing on a
Windows 10 1809 client.

TigerSetup distinguishes **TigerSetup platform support** (can the generated
installer engine run on this OS?) from **application/package support** (does the
packaged application and its dependencies support this OS?). A generated
installer may therefore run on a platform that a particular application does not
support.

### 10.2 Self-contained installer

> **`Setup.exe` must bring everything TigerSetup itself requires to execute.**

A generated installer must start and run on a plain supported Windows
installation without requiring TigerSetup, Rust, .NET, PowerShell 7, WinGet,
Chocolatey, Windows App SDK runtime, a separately installed UI framework, or any
development tooling.

Application dependencies are a separate concern: TigerMarkView may require .NET
Desktop Runtime and WebView2, but the TigerSetup installer engine must not.

### 10.3 One file

A generated installer is **a single Windows executable**. `Setup.exe` is the
whole deliverable: no side-by-side data files, no extraction step the user
performs, no second stub to ship or version. Multi-file or split-media packages
are out of scope for v1.

### 10.4 Installer composition

The executable is a small native **loader** followed by the compressed
**engine** and the package:

```text
+-----------------------------------------------------------------+
| loader                        (a small PE: bootstraps the engine) |
+-----------------------------------------------------------------+
| compressed engine             (the engine PE, one zstd frame)     |
+-----------------------------------------------------------------+
| payload                       (every file, one solid zstd stream) |
+-----------------------------------------------------------------+
| compressed metadata           (Protocol Buffers, one zstd frame)  |
+-----------------------------------------------------------------+
| fixed footer                  (the map, 320 bytes)                |
+-----------------------------------------------------------------+
```

- **Loader** — the executable Windows runs: a deliberately boring native C
  Win32 program of a few tens of kilobytes, with no runtime beyond the
  Windows API — no Rust, no protobuf, no SQLite, no engine library — and
  only the Zstandard *decoder* linked in. Its whole job is to get the
  engine running against the file it came from: it locates and validates
  the footer, decompresses the engine block into a fresh temporary file,
  checks the decompressed length and SHA-256 (through Windows CNG) against
  the footer *before* anything is executed, starts the engine with the
  original command line verbatim plus `--package <this file>`, waits,
  propagates the engine's exit code, and removes the temporary. It carries
  no metadata, payload, state or transaction logic and makes no decision
  about the run — the engine parses the command line and the engine asks
  for elevation, so a machine-scope run is an elevated `Setup.exe` (the
  loader again) that extracts its own engine under a system-owned root
  nobody else can write.
  Unelevated, the engine runs from `%TEMP%\TigerSetup\<pid>-<tick>\`; elevated,
  from a directory under `%SystemRoot%\Temp` created with an access control
  list granting SYSTEM and Administrators alone, atomically at creation.
  The temporary keeps the package's own file name, so Task Manager, the
  Restart Manager and a crash dialog name the installer that is running.
  The loader loads system DLLs from `System32` only, so the download folder
  it runs from is never a DLL search path. The *release loader executable*
  (`tigersetup-loader.exe`) is one file for every package; the loader
  *block* is that file after the builder gave its copy the product's own
  Windows VERSIONINFO and icon (§11.6), which is what Explorer shows for
  `Product-1.2.3-Setup.exe`.
- **Engine** — the common TigerSetup installer engine, compressed with
  Zstandard. The *release engine executable* (`tigersetup-setup.exe`) is
  identical bytes for every package built by one TigerSetup version, and its
  SHA-256 is what the embedded metadata records as the engine identity (§9)
  and what a lab compares against the engine beside the builder. The engine
  the loader runs is that executable **after the builder gave its copy the
  product's identity and icons too** — so the running process, and not only
  the file on disk, is the product's — and its SHA-256 is recorded twice, in
  the metadata (`engine_block_sha256`) and in the footer, where the loader
  reads it. The engine reads the metadata and the payload from the original
  `Setup.exe`, and everything it relaunches — an elevated run, the temporary
  uninstaller copy — is that package, never its own temporary file.
- **Payload** — **one solid Zstandard stream** holding every packaged file:
  the product's files under their install-relative names, the installers of
  embedded dependencies (§7.7) under `.tigersetup/dependencies/` and the
  packaged programs of custom actions and quiescence entries (§5.14, §5.10)
  under `.tigersetup/actions/`, directories no product file may occupy.
  Every file is in the stream; there is no per-file compression, no
  compressibility probe, no signature or extension classifier and no raw
  region for files that look compressed already. Directories are metadata
  and state, never bytes in the stream.
- **Metadata** — the runtime form of `TigerSetup.toml` (§4), encoded with
  **Protocol Buffers** and compressed whole as **one Zstandard frame** under
  the same profile as the payload: the package identity, resources,
  dependencies, localized strings and everything the engine plans from, plus
  the **payload index** — for every entry of the stream, its name, its
  offset and length in the uncompressed stream, its CRC-32 and its SHA-256 —
  and the **file batches** — consecutive runs of the file list, each with
  its first file, its file count and its bytes, closed by the builder before
  the file that would take a batch past 256 files or 32 MiB (§5.4). Both are
  data, not rules: the engine reads the stream in index order and journals
  the files by the batches it is given, and never reproduces the builder's
  ordering or its batching. The metadata follows the payload in the file
  because it carries the payload's index, which is only known once the
  payload has been written; that order lets the builder write the whole file
  in one pass. Protocol Buffers stays the logical model; the compression is
  of the serialized bytes as one block, with no separate string table or
  filename interning, and a reader decodes it within the length the footer
  declares.
- **Footer** — a fixed 320-byte trailer carrying the format identification
  and version (format 3), the absolute offsets and lengths of the engine,
  payload and metadata blocks, the uncompressed lengths of all three, the
  SHA-256 of the compressed engine block and of the engine executable it
  decompresses to, of the payload block, of the compressed metadata block
  and of the metadata it decompresses to, and its own CRC-32. It is the
  only thing the loader and the engine have to find by position — at the
  end of the file, or immediately before the PE security directory when a
  downstream signature has been appended — and everything else it
  addresses directly. Everything the loader needs is in the footer alone,
  so it decodes no metadata; everything a reader needs to reach the
  metadata safely is there too, so a damaged block can neither exhaust the
  decoder nor be accepted short. The hash of the decompressed metadata is
  the identity of its content, the same for a release-quality and a
  `--fast` build of one package.

#### How the payload is encoded

The engine block, the payload and the metadata block use one codec and, by
default, one profile: **Zstandard level 19, a 128 MiB window (`windowLog`
27), long-distance matching, single-threaded** — the `zstd-19-w27` setting
of the compression spike (`benchmark/compression-spike/report.md`); a block
smaller than the window is encoded with a window no wider than itself. The encoding is deterministic:
the same files in the same order produce the same bytes, in separate processes
and on separate days, which the spike verified for exactly these settings.
Single-threaded is a choice, not an omission: Zstandard's worker threads
split the input into jobs whose shared history is at most a fraction of the
window, so on this setting they buy a 1.3–2.5× build (VLC: 48 s → 22 s on
four workers) for a payload up to 0.9 % larger and three to five times the
builder's memory (`benchmark/tuning-2026-09-20.md`). The build is paid once;
the bytes are downloaded and decoded on every installation.

Two build modes make the build-time trade explicit:

```text
tiger-setup build TigerSetup.toml            release quality (zstd-19-w27)
tiger-setup build TigerSetup.toml --fast     the iteration loop (zstd-3)
```

- **Release** is the default and is what a published installer is built with.
  An installer is downloaded and installed far more often than it is built, so
  build CPU is the cheap side.
- **`--fast`** is for the developer and AI build-test loop: a fast level with
  the default window. The installer it produces is functionally identical and
  installs the same bytes; it is simply larger.

**Stream order.** The reserved `.tigersetup/` entries come first — the
dependency installers the engine needs before its transaction and the action
programs it needs at the transaction's start — then the product files sorted by
extension and then by path, compared as bytes. The spike measured
extension-then-path as a consistent gain over plain path order with no
classifier: files of one kind share bytes, and putting them side by side keeps
those bytes inside the window. A static type family ahead of the extension,
and same-named files side by side across directories, were measured on the
whole corpus afterwards (`benchmark/tuning-2026-09-20.md`) and gain nothing
the 128 MiB window does not already reach. The metadata's file list is
written in that order, and the engine's install walk follows it, so an
installation decodes the stream exactly once, sequentially.

**Why Zstandard, when LZMA2 is smaller.** The spike is unambiguous about ratio:
at the same window LZMA2 produces payloads about 8.5 % smaller, and on the
benchmark applications it reaches parity with Inno Setup and NSIS where
Zstandard leaves 9–18 % of the gap. It is equally unambiguous about decoding:
Zstandard decodes at 910–950 MiB/s single-threaded, LZMA2 at 113–126 MiB/s.
The product priority decides between them, and it is explicit:

> **TigerSetup optimizes for the shortest reliable installation transaction.**

TigerSetup is transactional and transaction-aware: the package is downloaded
*before* the transaction, and decompression and machine mutation happen
*inside* it — inside the window in which a crash, a power cut or a cancel has
to be recovered from, and during which the application is stopped. Bytes
saved on the download shorten nothing that has to be recoverable; decode time
lengthens exactly that window. A codec that is 7.6× faster to decode and
within a tenth of the size is the right one for a transaction-optimized
installer, and the choice is not reopened to chase a ratio. The spike's
measurements stand as recorded; its engineering judgment favoured LZMA2 on the
ratio criterion it was given, and this section is where the product's own
criterion overrides it. On the benchmark's real applications the format
change alone took 26–48 % off every 0.7.1 installer and left them 6 % below
to 14 % above the smaller of the Inno Setup and NSIS builds
(`benchmark/README.md`).

**Random access is deliberately not bought back.** Reading one entry means
decoding the stream from its start up to that entry, so a repair or a
single-file extraction pays the skip; the spike priced bounded blocks at
+3.5 % (128 MiB blocks) to +7.5 % (32 MiB) for this codec, and fast sequential
decode is exactly why Zstandard was chosen. An engine that reads in index order
never pays it.

**A stronger encoder is deliberately not used.** Level 22 (`btultra2` at its
widest) closes 0.9 % of the gap to LZMA2 for 60 % more build time; the spike
measured it and it does not pay for itself.

### 10.5 Inspectable by design

The format is deliberately easy to inspect, decompose and verify **without
executing the installer**. A reviewer, a build pipeline, an AI agent or a
support engineer can read the footer, decode the metadata, list the payload
index and compare them against what the package claims — with ordinary
tools, and with `tiger-setup inspect` for the stream itself.

`tiger-setup inspect` is the decomposition in one command. Besides its
report — which lists the file batches beside the files — `--output-payload`
writes the payload block out exactly as the footer addresses it, the same
byte range `verify` hashes, never re-packed or re-encoded, so the SHA-256 of
the exported file is the hash the footer records; `--output-meta` writes the
metadata decompressed, byte for byte, whose SHA-256 is the metadata hash the
footer and every transaction row record; `--output-zip` reconstructs the
payload as an ordinary ZIP archive of stored entries, one per payload entry in
stream order, which any archive tool opens; `--output-engine` writes the engine
executable the loader runs, decompressed, whose SHA-256 is the engine block
hash the metadata and the footer record; and `--output-meta-json` writes the
metadata decoded to JSON, every field of the message tree under its proto
name with enumerations as stable names. Nothing is exported from an installer
that fails verification, and no existing file is overwritten: an export is
evidence about the file, and evidence that could be mistaken for a good payload
is not produced.

There is no proprietary obfuscation, no container encryption, and no format
trick whose purpose is to make the contents hard to read.

The integrity model:

```text
CRC-32 of the footer
SHA-256 of the compressed engine block, and of the engine executable it
    decompresses to — checked by the loader before the engine is executed
SHA-256 of the compressed metadata block, and of the Protocol Buffers
    metadata it decompresses to, decoded within the length the footer
    declares
SHA-256 of the payload block
per-entry CRC-32 and SHA-256 in the payload index — the CRC checked as an
    entry is read, the SHA-256 checked before a written file is renamed into
    place, so an index that mis-slices the stream can never install the
    wrong bytes
```

The per-entry SHA-256 is also what an installed file is *owned by*: an
upgrade or a repair compares the hash the installation recorded with the hash
the package's index carries and decides without decoding the stream or
reading the disk (§5.9). The two kinds of entry the engine *executes* — an
embedded dependency installer and a packaged program — are recorded a second
time beside their declaration; `verify` checks both, and the engine checks the
bytes again before running them. There is no chain-of-custody machinery.

> **TigerSetup is an installer builder, not a supply-chain security framework.**

**No earlier format is read.** An installer of format 1 — the engine as the
executable stub and a ZIP payload — or of format 2 — the loader/engine split
with an uncompressed metadata block, never published — carries its own
engine and stays self-contained on every machine it was built for; the
builder, the engine and the lab inspect format 3 only, and an older file is
refused with `format_unsupported` rather than half-read.

### 10.6 Code signing is outside the core design

TigerSetup does not own a signing workflow, a signing policy, or key handling.
A downstream project may sign a generated installer if it wants to; the core
design neither requires it nor provides it.

---

## 11. Interactive UI

### 11.1 Philosophy

The UI must be clean, modern, native-feeling, fast, small, easy to understand,
DPI-aware, accessible, and visually consistent with modern Windows conventions.
The goal is **not** a visually elaborate installer.

> **Small, neat and fast — not beautiful bloatware.**

The UI is a presentation layer over the same installation engine used by
unattended installation (§6.1). It contains no separate installation
implementation and no installer-specific business logic.

**Options pages.** A boolean option is a check box; a choice option is a
heading with one radio button per value. Every row starts from the value the
engine itself would resolve — explicit, else recorded, else default — and the
options take as many pages as their rows need, nine rows to a page, an option
never straddling two; several pages are numbered in the header ("… (1 of 2)").
Pages rather than a scrolling list keep the proven static layout, its DPI
scaling and its keyboard order, at the cost of a Next per page for a package
with many options; a choice option is limited to eight values so that it
always fits one page. The Ready page summarises the selection as the person
made it: each selected check box, and each choice with the value chosen.

**The licence page asks once per licence text.** Three facts are kept apart:
the package carries licence text, a person explicitly accepted a particular
text, and the run may proceed unattended. The page is shown when the package
carries text and the installation (§5.3) records no acceptance of exactly
that text — a first install, an installation nobody accepted a licence for,
or a text that differs by any byte, an edited copyright year included — and
the run cannot continue until the person accepts. The acceptance is passed
to the engine with the run and committed with the installation it was
accepted for: a cancelled, failed or rolled-back run records nothing, and an
upgrade that rolls back keeps the previous acceptance. An interactive upgrade
or reinstall under the accepted text skips the page; uninstall and repair
never show it. A `--quiet` run is the third fact and never the second: it
neither shows nor waits for the page, proceeds whatever the text, and
records no acceptance — an unattended upgrade to a changed text leaves the
earlier acceptance as it was, for the next interactive run to ask about.

### 11.2 No Windows App SDK runtime requirement

A modern appearance must not cost a large framework/runtime requirement. The UI
direction is:

> **A clean, modern, Fluent-aligned native Windows UI, without an external
> Windows App SDK runtime requirement.**

The implementation technology is **plain Win32 common controls** (comctl32 v6
under the visual-styles manifest, GDI text, a hand-kept device-independent
layout rescaled on `WM_DPICHANGED`). It was chosen over the Win32 Aero wizard
and over a Rust UI toolkit against the real constraints — executable size
(about 0.4 MB over the engine), start-up, Windows 10 1809 / Server 2019
compatibility, no runtime requirement, DPI, accessibility, localization (every
string the wizard shows is the installer's own, in the installer's language),
visual quality and maintainability from Rust — and it is the settled
technology: a visual change varies the layout within Win32, not the
technology.

Identity in the wizard is the **application's**: the product name in the
title and headers, the product icon from the metadata. TigerSetup's own
branding is secondary and is exactly the word `TigerSetup` — never translated,
never "Powered by".

### 11.3 Engine and UI platform support are separate

```text
TigerSetup engine
    |
    +-- unattended/headless CLI
    |
    `-- optional native UI
```

The engine's platform support is the primary compatibility contract. This
matters most on Windows Server: unattended Server installation must never depend
on UI technology. Interactive UI support may be narrower than engine support
if a technical limitation forces it; the same experience on desktop and
Server is the target, and the Server 2019 interactive row proves it
(`TigerSetup-Validation.md` §5.2).

### 11.4 DPI awareness is a hard requirement

The UI must be properly DPI-aware, not merely acceptable at 100% scaling.
Validation covers 100%, 125%, 150% and 200% (see `TigerSetup-Validation.md`),
checking for clipped text, overlapping controls, incorrectly scaled icons,
layout breakage, unreadable text, badly sized windows, incorrect scaling
behaviour, keyboard usability and accessibility regressions. Moving between
monitor DPI contexts may also need validation, depending on the chosen UI
technology.

### 11.5 Light and dark are both the product

The wizard follows the person's Windows theme. That is not decoration: an
installer that opens a white window in front of somebody who set Windows to
dark is the first thing they see of the product, and it looks like a program
that was not finished.

Windows does not do this for a plain Win32 application. `GetSysColor` keeps
answering with the light palette whatever the app-theme setting says, because
the classic system colours belong to high contrast. Following the theme is
therefore the wizard's own work, and it has three parts:

- **the palette** the window paints its bands, rules and text with;
- **the title bar**, asked of the Desktop Window Manager, so a dark page does
  not sit under a white caption;
- **the common controls**, moved onto their dark visual style, because a
  control draws its own border, tick and frame and only its style can change
  those. The one exception is a radio button's label, which the dark style
  draws in its own dim colour: the wizard paints that label itself, in the
  palette's text colour, and leaves the button its behaviour and its glyph.

Two settings decide it, in this order. **High contrast wins**: a person using
it has told Windows exactly which colours they can see, so the palette is the
system's own and nothing overrides it. Otherwise the app-theme preference
decides, and its absence means light. A theme changed while the wizard is open
is picked up and repainted.

Both themes are acceptance requirements on Windows 11, alongside the scale
dimension (`TigerSetup-Validation.md` §8).

### 11.6 Icons

The wizard's icons are the ones a Windows installer is expected to show. Where
Windows owns the meaning, the system's own icon is used and not replaced: the
elevation shield is the shield Windows draws everywhere else, and a folder is
the shell's folder. The shield sits on the control that actually raises the
prompt — the wizard's **Next** button, set through `BCM_SETSHIELD` when the
selected scope needs an administrator and cleared when it does not — following
the Windows convention that the affordance marks the action, not the choice
that leads to it. Selecting "install for all users" does not itself elevate;
pressing Next while it is selected does, so the shield belongs on Next.
Because it is the themed button's own state rather than a drawn glyph, it
survives hover, focus, repaint, a theme change, a DPI change and page
navigation without the wizard painting anything.

What the shield promises is what pressing Next does. The wizard relaunches
this same executable elevated for the chosen scope, on a worker thread so the
window keeps pumping messages for the whole life of the prompt; the elevated
child is started **shown** — a process's first window follows the show state
it was started with, so a child started hidden would put up an invisible
wizard and wait forever for a click — and continues the wizard from its next
page on. Once the prompt has been answered and the child is running, the
parent steps aside for it, exactly as it does for a plain relaunch between two
installations, and reports the child's outcome and exit code as its own when
the child finishes. A refused prompt reaches no child: the parent stays where
it was, visible and usable, and says the prompt was declined. Both halves of
that — the prompt and the elevated run — are only proven together, on one
wizard, from the unelevated request to the outcome that comes back to it
(`TigerSetup-Validation.md` §5.2, the elevation rows).

Where the meaning is the product's, the glyph comes from **Fluent UI System
Icons**, which is the Tiger family's icon language. Only concept glyphs are
used, and only where they say something the words beside them do not: the
outcome mark on the completion page, a checkmark or an exclamation in a
circle.

They are embedded as their **outline path data** and drawn, not shipped as
bitmaps, which is what makes one glyph right at every scale the wizard runs at,
lets it take the theme's foreground colour instead of colours baked into an
image, and adds nothing measurable to the installer. Provenance and the
licence notice are in `THIRD-PARTY-NOTICES.md`; the notice travels with the
redistributed material, and adding or replacing a glyph means re-reading the
upstream licence and comparing it with what is recorded there.

#### The generated installer's own icon

The icon Explorer, the taskbar and the wizard's title bar show for a
`Setup.exe` is the **product's**, not TigerSetup's — a TigerMarkView installer
looks like TigerMarkView. That icon is a resource the builder writes into the
engine copy it composes, alongside the product's version resource (§10.4), and
it is resolved from the manifest, never guessed from a payload executable:

```text
[installer].icon = "<path>"   an explicit .ico, relative to the manifest → that icon
                 = "tigersetup"   always the built-in TigerSetup icon
                 = "branding"     the product's [package].icon; a validation error if there is none
                 omitted          [package].icon when one is declared, otherwise the TigerSetup icon
```

`"branding"` without a `[package].icon` fails the build deterministically
rather than falling back silently, so a package that means to carry its own
mark cannot ship TigerSetup's by accident. The engine copy keeps a second
icon group as well — TigerSetup's own — which the wizard draws as the small
secondary brand mark beside the product's; the resource-id contract
(`RT_GROUP_ICON 1` the product's, `RT_GROUP_ICON 2` TigerSetup's, `RT_VERSION 1`
the product's identity) is in the builder's `resource` module. Multi-size,
theme-following icon quality is preserved: every image of the `.ico` is
carried, so Windows picks the right size at every DPI.

TigerSetup's own application icon — the one the built-in default and the brand
mark come from — remains provisional artwork.

---

## 12. Localization

Localization is first-class, not a later framework. The installer's own text
— outcome messages and everything the wizard shows — is available in:

```text
en-US
pl-PL
```

Strings live in the engine's text catalog (`i18n`), keyed by stable
identifiers, separate from UI and engine logic: a catalog entry per language,
`en-US` complete and every other language falling back to it key by key, so a
missing or incomplete translation never makes the installer unusable. Product
strings — names, descriptions, option labels — come from the package
metadata, where a custom option carries a label per language with `en-US`
required. Adding a language is adding a catalog and a language mapping; no
installer logic changes. Machine-readable output never goes through the
catalog (§6.3). `TigerSetup` is a name, not a word: it is never translated.

### 12.1 Language selection

```text
--lang <tag>  (explicit installer language selection)
        ↓
Windows UI language
        ↓
en-US fallback
```

Polish is a deliberate layout stress case, because string lengths often
differ materially from English, and both languages are acceptance
requirements (`TigerSetup-Validation.md` §8).

---

## 13. Technology choices

**Rust** is the implementation language: a native Windows executable, no .NET
runtime dependency on the target, a good fit for a small self-contained
CLI/builder and for low-level Windows integration, and a natural fit with the
Tiger tool philosophy.

**SQLite through `rusqlite` with the bundled feature**, so SQLite compiles into
the executable and the self-contained deployment model is preserved. The
bundled build is configured in `.cargo/config.toml` for what TigerSetup
actually uses: the full-text search, R*Tree and DBSTAT modules, extension
loading, column metadata, STAT4 and `soundex()` the crate opts into by default
are declined, and SQLite's recommended options for an embedded database are
set — no double-quoted string literals, no memory statistics, no deprecated
interfaces, no progress callback, no shared cache — about 360 KB of code in
every executable that nothing reaches. SQLite stays thread-safe: `rusqlite`
refuses a single-threaded build, and one process holds two connections (the
state database and the WinGet index), each opened without its own mutex.
Durability behavior is the crate's default. The state database's schema is
at version 7; a reader tolerates every schema back to 2.

**Protocol Buffers and Zstandard** are the installer-format technologies
(§10.4). Both are read by the generated installer — Zstandard by the loader
and by the engine — so both must compile into the executable and neither may
pull in a runtime prerequisite on the target machine. Zstandard is libzstd
1.5.7 through the `zstd` crate, BSD-3-Clause, built without its legacy formats
and dictionary trainer (`THIRD-PARTY-NOTICES.md`); there is deliberately one
compression technology, and the loader needs only its decoder. Which Rust
crate provides Protocol Buffers, and how the `.proto` schema is compiled
during the build, are implementation choices (§17).

**Durability settings** are a rollback journal (`journal_mode = DELETE`, no
WAL sidecars), `synchronous = FULL`, `foreign_keys = ON`, an exclusive lock
for the duration of a mutating run, and the schema version in
`PRAGMA user_version` with forward-only migrations; `inspect` and `verify`
open the database read-only and report `database_busy` while a run holds it.
The recovery rows exercise it under process kill, reboot and power-off
(`TigerSetup-Validation.md` §3).

**Implementation structure.** A Cargo workspace of six crates whose
dependency directions the compiler enforces: `tigersetup-format` (footer,
metadata, payload, compose, inspect, verify, and the package-identity
derivations both sides must agree on; no Windows API), `tigersetup-catalog`
(the WinGet catalog client — the pre-indexed source, version data, merged
manifests — which both sides read, the builder at build time and the engine
when it refreshes an acquisition hint), `tigersetup-engine` (state, journal,
planner, transaction executor, recovery, resources, quiescence, reports,
fault injection; depends on the format and catalog crates),
`tigersetup-loader` (the C Win32 loader every generated `Setup.exe` begins
with, compiled and linked by the package's build script from `src/loader.c`
and libzstd's decoder — taken from the sources the `zstd-sys` crate carries,
so the loader decodes with the same libzstd the engine links — into the
profile directory beside the other executables; it reads the footer by its
own code, links nothing of Rust or of the engine, and its own tests run it
against synthetic packages), `tigersetup-setup` (the engine executable: the
command-line client and the interactive client, which reach the engine only
through its public API), and `tigersetup-build` (the builder library and
`tiger-setup.exe`; depends on the format and catalog crates and never on the
engine, so nothing that installs can leak into the tool that packages). One
format implementation serves builder and engine, and the loader's footer
reader is checked against it by every process-level test; one
package-identity implementation serves both sides. Static CRT,
`opt-level = "z"` with the two hot crates — the SHA-256 implementation and
the Zstandard decoder — at full optimization, fat LTO, one codegen unit,
`panic = "abort"`, symbols stripped: a profile audited against opt-level s,
2 and 3 and thin LTO by the engine's compressed bytes and its install,
uninstall and verify times, where z is the smallest compressed and, with
those two crates at 3, as fast as the whole engine at 3
(`benchmark/README.md`). The engine executable is about 2.5 MB raw and
1.15 MB as the compressed block every installer carries, and links the
Zstandard decoder alone (the uninstaller copy's metadata block is a stored
frame); the loader is 74,752 bytes — 42 KB of code, of which the Zstandard
decoder is 28 KB and what the compiler needs of the C runtime 3.5 KB, 9 KB
of read-only data and 20 KB of resources (`benchmark/README.md`). Both
import only inbox DLLs.
Fault injection (`--fault <point>[@<sequence>]:<action>[:<seconds>][:skip_flush]`)
is compiled into every build and affects only the invoking run, so the bytes
the interrupted rows validate are the bytes that ship.

---

## 14. Product scope

### 14.1 What TigerSetup does

The product is bounded by what replacing a real desktop application's
installer needs. TigerSetup provides:

- declarative package definition;
- a self-contained, natively generated `Setup.exe`;
- per-user and per-machine scope, with UAC elevation for machine scope, and
  the cross-scope policy of §5.13;
- file installation with conservative ownership;
- Start Menu, Desktop, Startup and Send To shortcuts, with working
  directory and AppUserModelID, and URL shortcuts;
- PATH integration and environment variables;
- registry values under the scope's `Software` root or at an explicit
  location in the scope's hive, with their pre-installation state restored;
- file associations, URL protocols, `App Paths` and classic context-menu
  verbs, registered as handlers rather than as defaults;
- Windows Firewall rules for installed programs;
- custom lifecycle actions — a packaged or installed program, PowerShell or
  batch script run at pre-install, post-install, pre-uninstall or
  post-uninstall on the operations it names, under a bounded, recorded
  envelope, with no claim of rollback for what the program changed (§5.14);
- boolean and choice options with the one `when` predicate, and optional
  components as option-gated files;
- Add/Remove Programs registration;
- uninstall, reinstall, upgrade and repair;
- SQLite-backed installation state;
- a persistent transaction journal with rollback and recovery;
- silent mode and logging;
- automated machine-readable verification;
- dependency detection, acquisition, installation and verification through
  typed detectors and the WinGet catalog, or from an installer embedded in
  `Setup.exe`;
- offline installation when dependencies are already satisfied;
- migration from an Inno Setup installation;
- a small native DPI-aware, theme-following interactive UI localized to
  `en-US` and `pl-PL`;
- a single-file `Setup.exe` in the documented composition — engine, Protocol
  Buffers metadata, ZIP payload, footer — inspectable without executing it
  (§10.3–10.5);
- WinGet manifest generation;
- TigerWinLab end-to-end validation.

### 14.2 Not in scope today

Deliberately outside the product; each is added only on a concrete
requirement:

- Windows services;
- ARM64;
- automatic update system;
- binary patching;
- enterprise-style repair policies;
- drivers;
- modern shell extensions — COM shell-extension registration,
  `DllRegisterServer`, sparse AppX/MSIX packages, the Windows 11 top-level
  context menu — as distinct from the classic registry-declared verbs
  TigerSetup does register; a separate design topic;
- arbitrary script execution beyond the declared custom actions of §5.14 —
  no inline scripts, no compensating or rollback actions, no per-action
  elevation, no action-specific condition language;
- elaborate or highly customised installer UI beyond the small native wizard;
- multi-file or split-media installer packages (§10.3);
- code signing, which is outside the core design entirely (§10.6);
- a downgrade guard — a downgrade is mechanically an upgrade with an older
  target and is tested as one;
- installation-state history retention;
- custom detector and installer types beyond the built-ins, and dependency
  version ranges beyond "same major, at least the declared minimum";
- a redistribution licensing policy for downloaded prerequisites;
- a Chocolatey package skeleton, and `winget validate` inside the builder;
- Authenticode interaction with the footer — the locate rule is implemented
  but has not been checked against a signed file;
- installation parameters beyond declared boolean and choice options — free
  text, paths other than the install root, numbers;
- optional application-data purge on uninstall, AutoPlay handlers, Windows
  Terminal fragments, ACL or security-descriptor declarations, registry
  values in a hive the installation's scope does not write (an `HKLM` value
  from a per-user install, or one gated on the scope of a dual-scope
  package), and registry keys other than `HKLM` and `HKCU` — `HKU`, `HKCR`
  as a root of its own, remote registries, loaded hives;
- deeper accessibility automation beyond the UI Automation ids the wizard's
  controls carry, and a deliberately narrower Server UI;
- a visual-regression strategy beyond the lab's per-page captures, which are
  reviewed by eye.

---

## 15. Vocabulary

Use these terms consistently:

- **Package** — desired installation definition.
- **Installation** — currently committed installed state.
- **Transaction** — one install / upgrade / uninstall / repair attempt.
- **Operation** — one reversible system mutation.
- **Resource** — typed system entity managed by TigerSetup.
- **Manifest** — desired state / intent.
- **State database** — actual committed ownership/state.
- **Journal** — in-progress transaction and rollback information.

---

## 16. Guiding principles

1. **Small and native.** TigerSetup is a focused Windows tool, not a deployment
   platform. Installer size, startup speed and implementation simplicity matter;
   a modern appearance never justifies framework bloat.
2. **Declarative before programmable.** Typed operations beat arbitrary setup
   scripts.
3. **Manifest is intent; database is reality.** Never reconstruct uninstall
   state from the current package definition.
4. **Every mutation is journaled and reversible.** Durability must precede
   destructive changes, and an attempt ends in success or full rollback. A
   recoverable in-progress state is legitimate; an unknown or hybrid
   installation is not.
5. **Ownership is conservative.** Do not remove resources TigerSetup cannot
   prove it owns.
6. **Dependencies are requirements, not owned resources.** TigerSetup may
   satisfy a shared dependency without claiming lifecycle ownership of it.
7. **One source of packaging truth.** Installer, ARP, release and WinGet
   metadata must not drift.
8. **Understand the application when useful.** MSBuild and PE metadata
   integration should eliminate glue, not create framework lock-in.
9. **Build once; distribute exact bytes.** WinGet preparation must respect
   exact-artifact release discipline.
10. **Self-contained and inspectable on the target.** Generated installers run
    on a plain supported Windows installation with no TigerSetup, runtime or UI
    prerequisites, and the single file can be decomposed and verified without
    being executed.
11. **Automation-first, interactive-capable.** Unattended install, upgrade and
    uninstall are the primary execution paths; UI is a client of the same
    engine.
12. **Machine-verifiable by design.** Install, upgrade, uninstall, dependency
    and recovery results must be objectively inspectable by automated tools.
13. **Offline-capable.** Network access is optional dependency acquisition, not
    an installer-engine requirement.
14. **Localizable by design.** Strings stay separate from engine and UI logic;
    `en-US` and `pl-PL` are first-class tested languages.
15. **DPI-aware by design.** The interactive UI must be tested and usable across
    common Windows DPI scaling levels.
16. **WinGet-aligned, WinGet-independent at runtime; Chocolatey-compatible by
    convention.** Use WinGet identities and conventions where useful; generated
    installers must never require WinGet, and must stay easy to wrap in
    Chocolatey.
17. **Real applications define scope.** Generalize recurring requirements from
    the applications TigerSetup packages; do not invent installer-framework
    features without evidence.
18. **Respect project ownership boundaries.** TigerSetup may specify missing
    TigerWinLab capabilities but must not implement them inside this project.
19. **Do not become MSI by accident.** If the design starts reproducing Windows
    Installer's complexity, reconsider scope.
20. **Replacement is the standard.** TigerSetup is acceptable only while a
    TigerSetup-generated installer passes real TigerMarkView replacement
    validation (`TigerSetup-Validation.md` §4).

---

## 17. Open questions

Deliberately undecided, to be resolved on concrete requirements and
implementation evidence rather than speculative framework design.

**Transaction and state**

- installation-state history retention policy;
- a downgrade policy (today a downgrade is mechanically an upgrade to an
  older version, with no guard);
- repair semantics beyond reconciliation.

**Packaging and build**

- how a downstream project's Authenticode signature interacts with the footer
  in practice (the locate rule reads the footer before the PE security
  directory when one is present; unverified against a signed file, §10.6);
- whether an encoder stronger than the ZIP container's DEFLATE ever becomes
  worth its cost (§10.4); a different container is not on the table.

**Dependencies and distribution**

- licensing/redistribution policy for downloaded or embedded prerequisites;
- whether dependency payloads may be embedded in `Setup.exe` (the acquisition
  model allows a fixed source; an embedded one is not implemented);
- whether and how Chocolatey package mappings are represented, and whether
  TigerSetup later generates a Chocolatey package skeleton.

**Execution surface**

- how optional interactive installation parameters beyond declared on/off
  options are modeled.

**UI and localization**

- the exact accessibility target and automation approach beyond the UI
  Automation ids the wizard's controls carry;
- whether interactive UI support on Server differs deliberately from desktop
  Windows.
