# TigerSetup / TigerWinLab / TigerHyperLab — Session and VM Lease Policy

**Status:** Agreed design direction; not yet implemented  
**Primary project:** `TigerSetup`  
**Cross-repository scope:** `TigerSetup`, `TigerWinLab`, `TigerHyperLab`

## Purpose

This document captures the agreed session/lease model for Lab VM use.

The implementation should be carried out as a **closed-loop cross-repository task** with `TigerSetup` as the primary consumer. `TigerWinLab` and `TigerHyperLab` may be changed where required, but the task is not complete until the resulting behavior is proven through the normal TigerSetup → TigerWinLab → TigerHyperLab path.

The objective is to make VM isolation, reuse, cleanup and interleaving deterministic by contract rather than by caller convention.

## Core concepts

### Session

A **session** represents one agent task/run — normally the work performed for one prompt.

A session is the logical identity of the work using the Lab.

A session may acquire one or more VM leases during its lifetime.

### Lease

A **lease** is exclusive active access to one VM.

While a lease is active, no other session may use or mutate that VM.

A lease belongs to a `SessionId`.

### Hard invariant

A Lab VM must not be used without both:

- a valid `SessionId`;
- an active lease owned by that `SessionId`.

There must not be a public or internal execution path that mutates or actively uses a registered Lab VM while bypassing session/lease ownership.

## Lease policies

Each lease has two independent policies:

```text
EntryPolicy:
    Baseline
    DontCare

ExitPolicy:
    PreserveUntilSessionEndOrNextLease
    DontCare
```

If either policy is omitted, the defaults are:

```text
EntryPolicy = Baseline
ExitPolicy  = DontCare
```

Therefore a caller that does not care about advanced lifecycle behavior gets the safe default automatically:

```text
clean known start
→ exclusive work
→ discard resulting VM state
→ normalize VM for the next user
```

## Entry policy

### `Baseline`

Before the lease is granted for use, the Lab must establish the VM's configured baseline state.

The caller therefore receives a known starting state and is isolated from residual changes left by previous unrelated sessions.

The baseline operation is part of lease acquisition semantics; callers should not need to remember a separate reset step merely to obtain ordinary isolation.

### `DontCare`

The caller accepts the VM state that is validly available at lease acquisition time.

No baseline restoration is required solely for this lease.

This policy is useful when:

- continuing state already preserved by the same session;
- deliberately consuming whatever current state exists;
- explicitly releasing a preserved-state claim without doing more work.

## Exit policy

### `PreserveUntilSessionEndOrNextLease`

When the active lease ends, the resulting VM state remains reserved for the same session.

No other session may interleave on that VM while this preservation claim exists.

The preservation lasts until either:

1. the session ends; or
2. the same session acquires the next lease on that VM.

The next lease becomes the active owner of that preserved state. Its own `ExitPolicy` determines what happens after that lease.

This policy is for multi-step tasks that need VM state to survive between separate lease scopes.

### `DontCare`

When the lease ends, the session explicitly declares that it no longer cares about the resulting VM state.

The Lab then owns normalization of the VM.

The desired postcondition is:

```text
lease work ends
→ VM is shut down
→ configured baseline is restored
→ VM is left Off
→ VM becomes reusable by another session
```

The VM must not become available to another session in the middle of this cleanup/reset sequence.

Cleanup is part of lease completion, not an optional courtesy performed later by the caller.

## Session end

If a session ends while it still owns a preserved-state claim, that state is no longer useful to the vanished/finished session.

The preserved claim should therefore converge to the same safe reusable state as `ExitPolicy = DontCare`:

```text
session ends
→ remaining preserved VM claim expires
→ shut down
→ restore configured baseline
→ leave Off
→ make reusable
```

The exact recovery mechanism must be bounded and durable enough that an abandoned session does not leave a VM permanently reserved or running.

## Releasing a preserved VM before the session ends

A session may finish with a VM while continuing to perform unrelated work.

If the VM's previous lease ended with:

```text
ExitPolicy = PreserveUntilSessionEndOrNextLease
```

the session can explicitly release that preserved state by acquiring a new lease with:

```text
EntryPolicy = DontCare
ExitPolicy  = DontCare
```

and closing it immediately.

Conceptually:

```text
preserved for Session A
→ Session A acquires (DontCare, DontCare)
→ no work required
→ lease closes
→ Lab normalizes VM
→ VM becomes reusable
```

This avoids forcing the entire session to end merely to release one VM.

## Interleaving rules

Interleaving is governed by active ownership and preservation state.

### Another session cannot use the VM when:

- an active lease exists; or
- an outstanding `PreserveUntilSessionEndOrNextLease` claim exists.

### Another session may use the VM when:

- there is no active lease; and
- there is no outstanding preserved-state claim.

The next session's `EntryPolicy` then determines preparation:

- `Baseline` → establish baseline before granting the lease;
- `DontCare` → hand over the current valid state.

### Safe interleaving

```text
Session A:
    (Baseline, DontCare)
    work
    release
    normalize

Session B:
    (Baseline, DontCare)
    work
```

`(Baseline, DontCare)` is fully compatible with interleaving because Session A retains no state claim.

### Interleaving prohibited

```text
Session A:
    (Baseline, PreserveUntilSessionEndOrNextLease)
    work
    release

Session B:
    acquire same VM
    → BUSY / unavailable
```

Session A still owns the meaning of the preserved VM state.

## Policy matrix

| EntryPolicy | ExitPolicy | Meaning |
|---|---|---|
| `Baseline` | `DontCare` | **Default.** Start clean, discard resulting state, normalize VM for reuse |
| `Baseline` | `PreserveUntilSessionEndOrNextLease` | Start clean, preserve resulting state for later use by the same session |
| `DontCare` | `DontCare` | Accept current state, retain no claim afterward |
| `DontCare` | `PreserveUntilSessionEndOrNextLease` | Continue current state and preserve the result for the same session |

## Architectural ownership

### TigerHyperLab

TigerHyperLab remains generic and guest-OS/application agnostic.

It should own or enforce the generic provider-level mechanisms required for:

- session-aware VM lease ownership;
- exclusive active VM access;
- preservation claims;
- bounded lifecycle transitions;
- shutdown / checkpoint restore / Off normalization where these are generic VM operations;
- refusing conflicting/interleaved access;
- cleanup when a session disappears or ends;
- durable ownership state where required for correctness.

TigerHyperLab must not gain TigerSetup-, Windows Update-, installer-, application-, or other consumer-specific semantics.

### TigerWinLab

TigerWinLab owns Windows Lab meaning and its public Windows-facing contract.

It should:

- use the generic lease/session behavior correctly;
- map its configured Windows baseline into the provider lifecycle;
- make safe defaults available without requiring callers to understand Hyper-V details;
- remove ordinary caller dependence on ad-hoc reset/start/stop conventions where the lease contract now owns those semantics;
- preserve generic Windows Lab behavior rather than TigerSetup-specific behavior.

### TigerSetup

TigerSetup is the primary consumer and owns the final product outcome for this cross-repository task.

TigerSetup should:

- use TigerWinLab through its normal public interface;
- not bypass TigerWinLab to implement its own VM lifecycle;
- prove that the new default isolation/cleanup behavior works in a real consumer acceptance flow;
- close the cross-repository loop.

## Migration principle

Do not preserve old lifecycle behavior merely because callers currently depend on it.

Inspect current APIs and usage first, then simplify toward the new contract.

In particular, review any existing concepts such as:

- explicit `-Reset`;
- explicit start/stop behavior in consumer code;
- session cleanup performed manually by callers;
- lease release that leaves arbitrary VM state behind;
- VM operations callable without session/lease validation.

Where the new contract makes such caller-side behavior redundant, migrate callers to the common lifecycle rather than layering another mechanism beside the old one.

Do not invent a parallel session/lease system.

## Closed-loop implementation strategy

The implementation task should be run as one cross-repository loop:

```text
TigerSetup acceptance requirement
    ↓
inspect TigerSetup → TigerWinLab → TigerHyperLab path
    ↓
implement the smallest required provider/lab changes
    ↓
cheap/unit/contract verification in owning repositories
    ↓
run real TigerSetup acceptance through TigerWinLab
    ↓
diagnose failure across the authorized repositories
    ↓
fix the owning layer
    ↓
return to TigerSetup acceptance
    ↓
PASS or genuine blocker
```

Do not declare success because TigerHyperLab or TigerWinLab unit tests are green while the TigerSetup consumer path remains unproven.

## Required contract tests

The exact implementation is left to the repositories, but the resulting contract must prove at least these behaviors cheaply before real VM acceptance:

1. No `SessionId` → refusal.
2. No lease → VM use refused.
3. Two sessions cannot hold active leases on the same VM.
4. Default lease means `(Baseline, DontCare)`.
5. `EntryPolicy=Baseline` establishes baseline before use.
6. `ExitPolicy=DontCare` does not make the VM reusable until cleanup is complete.
7. `ExitPolicy=DontCare` converges to baseline + Off.
8. Preserve exit reserves state for the same session.
9. Another session receives BUSY while preservation is outstanding.
10. The next lease by the same session consumes/continues the preserved claim.
11. A subsequent `(DontCare, DontCare)` lease can explicitly release preserved state.
12. Session end cleans up any remaining preserved claim.
13. `(Baseline, DontCare)` sessions may interleave sequentially.
14. Cleanup/recovery is bounded; no indefinite lifecycle wait.

Tests should target the common abstraction boundaries rather than multiplying scenario-specific gates.

## Real acceptance

After cheap verification, prove the contract through a real TigerSetup consumer flow.

The real acceptance should demonstrate, at minimum:

1. TigerSetup starts a Lab session.
2. It acquires the required Windows VM using the public TigerWinLab path.
3. With policies omitted, the VM starts from the configured baseline.
4. TigerSetup performs representative Windows acceptance work.
5. The lease ends with the default `DontCare` exit behavior.
6. The VM is normalized and left Off.
7. A new independent session can acquire the same VM and receives baseline state rather than TigerSetup's previous residual state.
8. No caller-specific Hyper-V cleanup is required in TigerSetup.

If TigerSetup has a genuine multi-step scenario that needs state between leases, additionally prove:

```text
lease 1: (..., PreserveUntilSessionEndOrNextLease)
→ state remains reserved
→ another session cannot interleave
→ same session acquires next lease
→ state is preserved
→ final DontCare release normalizes the VM
```

Do not add an expensive live test merely for symmetry if the behavior has already been proven adequately by the consumer path and lower-level contract tests.

## Performance and expensive-loop requirement

This work exists to make the Labs cheaper to use, not more expensive.

Before any real VM run:

- prove the state machine and conflict rules cheaply;
- inspect the complete call path;
- avoid repeating a failed expensive run without specific new evidence that justifies it.

A green result obtained only after repeated expensive retries is not sufficient evidence that the lifecycle is healthy.

The final report should include the actual prompt-to-result elapsed time for the cross-repository acceptance task, not only successful sub-step durations.

## Non-goals

This task is **not**:

- a redesign of Windows maintenance;
- a scheduler implementation;
- a TigerSetup-specific Hyper-V API;
- a reason to move Windows/application semantics into TigerHyperLab;
- a second parallel lease/session mechanism;
- an invitation to add large numbers of scenario-specific gates.

The goal is a small, strong lifecycle contract at the existing architectural boundaries.

## Acceptance summary

The task is complete only when all of the following are true:

```text
VM use requires SessionId + lease

Default lease:
    EntryPolicy = Baseline
    ExitPolicy  = DontCare

Baseline entry:
    known isolated starting state

Preserve exit:
    state reserved to same session
    no other session may interleave

DontCare exit:
    shutdown
    restore configured baseline
    leave Off
    reusable only after cleanup completes

Session end:
    remaining preserved state is normalized

TigerSetup:
    proves the behavior end-to-end through TigerWinLab

TigerWinLab:
    exposes the Windows-facing contract

TigerHyperLab:
    provides only the generic VM/session/lease mechanics
```
