//! Local durability tests: a synthetic package is built with the freshly
//! compiled engine, then `Setup.exe` is spawned under a redirected
//! `%LOCALAPPDATA%` with crashes injected at every journal/mutation boundary.
//! Every interrupted run must converge under recovery to a state that
//! `verify --json` confirms; rollback must be idempotent; a zero-filled or
//! missing target found mid-transaction must be detected and re-applied.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::time::Instant;

use common::*;

#[test]
fn install_verify_uninstall_lifecycle() {
    let a = &fixture().a;
    let mut machine = Machine::new("lifecycle");
    let install = machine.install(a);
    let outcome = install.json();
    assert_eq!(outcome["outcome"], "installed", "{}", install.stdout);
    assert_eq!(outcome["code"], "ok");
    assert_eq!(outcome["exit_code"], 0);
    assert_eq!(install.exit_code, Some(0));
    assert_eq!(outcome["installation"]["file_count"], a.files.len() as u64);
    assert_eq!(outcome["transaction"]["kind"], "install");
    assert_eq!(outcome["transaction"]["state"], "committed");
    assert!(outcome["recovery"].is_null());
    assert!(install.log_has("[transaction_started]"));
    assert!(install.log_has("[transaction_committed]"));
    machine.assert_verified(a);

    let uninstall = machine.uninstall(a);
    let outcome = uninstall.json();
    assert_eq!(outcome["outcome"], "uninstalled", "{}", uninstall.stdout);
    assert_eq!(uninstall.exit_code, Some(0));
    assert_eq!(outcome["transaction"]["kind"], "uninstall");
    // An uninstall leaves nothing of TigerSetup's own behind, whichever
    // executable ran it. Keeping a database "recording the absence" only left
    // an orphan uninstaller and a dead database on every machine that ever
    // uninstalled the product.
    machine.assert_uninstalled(a);

    let inspect = machine.inspect(a).json();
    assert!(inspect["installation"].is_null());
    assert!(inspect["transaction"].is_null());
}

#[test]
fn reinstall_of_same_version_is_a_no_op() {
    let a = &fixture().a;
    let mut machine = Machine::new("reinstall");
    assert_eq!(machine.install(a).json()["outcome"], "installed");
    let again = machine.install(a);
    let outcome = again.json();
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["code"], "already_installed");
    assert_eq!(again.exit_code, Some(0));
    assert!(outcome["transaction"].is_null());
    machine.assert_verified(a);
}

#[test]
fn uninstall_of_absent_installation_reports_not_installed() {
    let a = &fixture().a;
    let mut machine = Machine::new("absent");
    let run = machine.uninstall(a);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "not_installed", "{}", run.stdout);
    assert_eq!(outcome["code"], "not_installed");
    assert_eq!(run.exit_code, Some(0));
    assert!(run.log_has("[not_installed]"));
    assert!(!machine.install_root().exists());
}

#[test]
fn inspect_describes_package_and_installation() {
    let a = &fixture().a;
    let mut machine = Machine::new("inspect");
    let before = machine.inspect(a);
    let report = before.json();
    assert_eq!(before.exit_code, Some(0));
    assert_eq!(report["schema"], 1);
    assert_eq!(report["package"]["id"], PRODUCT_ID);
    assert_eq!(report["package"]["name"], PRODUCT_NAME);
    assert_eq!(report["package"]["version"], VERSION_A);
    assert_eq!(report["package"]["publisher"], "IT Tiger");
    assert_eq!(
        report["package"]["scopes"],
        serde_json::json!(["user", "machine"])
    );
    assert_eq!(
        report["package"]["metadata_sha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        report["package"]["engine"]["engine_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        report["package"]["engine"]["engine_block_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_ne!(
        report["package"]["engine"]["engine_sha256"],
        report["package"]["engine"]["engine_block_sha256"],
        "the block carries the product's identity, so it is not the engine executable"
    );
    assert!(report["installation"].is_null());
    assert!(report["transaction"].is_null());

    // What the file presents to Explorer is the product, not TigerSetup,
    // and the engine block the builder inspects is the one the engine hashed.
    let built = tigersetup_build::inspect::inspect(&a.installer)
        .unwrap()
        .to_json();
    assert_eq!(built["windows"]["product_name"], PRODUCT_NAME);
    assert_eq!(
        built["windows"]["file_description"],
        "TigerSetupTestApp Setup"
    );
    assert_eq!(built["windows"]["product_version"], VERSION_A);
    assert_eq!(
        built["package"]["engine"]["engine_block_sha256"],
        report["package"]["engine"]["engine_block_sha256"]
    );
    assert_eq!(built["verification"]["status"], "ok");

    machine.install(a);
    let after = machine.inspect(a).json();
    assert_eq!(after["installation"]["product_id"], PRODUCT_ID);
    assert_eq!(after["installation"]["scope"], "user");
    assert_eq!(
        after["installation"]["install_root"],
        machine.install_root().display().to_string()
    );
    assert_eq!(after["installation"]["file_count"], a.files.len() as u64);
}

#[test]
fn crash_at_each_boundary_then_rerun_converges_forward() {
    let a = &fixture().a;
    let points = [
        "after_prepare@30",
        "after_applying@30",
        "after_write_before_flush@30",
        "after_flush_before_rename@30",
        "after_rename@30",
        "after_applied@30",
        "before_commit",
        "after_commit_before_cleanup",
    ];
    for point in points {
        let mut machine = Machine::new("crash");
        let crashed = machine.install_with_faults(a, &[&format!("{point}:crash")]);
        assert!(!crashed.success, "{point}: the run must abort");
        assert!(
            crashed.log_has("[fault_injected]"),
            "{point}: fault logged\n{}",
            crashed.log_text()
        );
        assert!(
            !crashed.log_has("[run_finished]"),
            "{point}: the run never finished"
        );

        let committed_already = point == "after_commit_before_cleanup";
        let verify = machine.verify(a);
        let report = verify.json();
        if committed_already {
            assert_eq!(report["status"], "ok", "{point}: {}", verify.stdout);
        } else {
            assert_eq!(
                report["status"], "transaction_open",
                "{point}: {}",
                verify.stdout
            );
            assert_eq!(report["transaction"]["kind"], "install");
            assert_eq!(report["transaction"]["recovery_direction"], "forward");
            assert_eq!(verify.exit_code, Some(1));
            let inspect = machine.inspect(a).json();
            assert_eq!(
                inspect["transaction"]["recovery_direction"], "forward",
                "{point}"
            );
            assert!(
                inspect["installation"].is_null(),
                "{point}: nothing is installed before the commit"
            );
        }

        let rerun = machine.install(a);
        let outcome = rerun.json();
        assert_eq!(
            rerun.exit_code,
            Some(0),
            "{point}: {}\n{}",
            rerun.stdout,
            rerun.log_text()
        );
        assert_eq!(outcome["outcome"], "installed", "{point}");
        assert_eq!(
            outcome["code"], "already_installed",
            "{point}: recovery installed it, the run's own work found it installed"
        );
        if committed_already {
            assert!(outcome["recovery"].is_null(), "{point}");
            assert!(
                rerun.log_has("[staging_swept]"),
                "{point}: the interrupted cleanup is finished\n{}",
                rerun.log_text()
            );
        } else {
            assert_eq!(outcome["recovery"]["direction"], "forward", "{point}");
            assert!(
                rerun.log_has("[recovery_started] direction=forward"),
                "{point}"
            );
            assert!(rerun.log_has("[recovery_completed]"), "{point}");
        }
        machine.assert_verified(a);
    }
}

/// The operation sequence of every file a clean install writes, read from
/// its log: `operation_applied sequence=N kind=install_file target=<path>`.
fn install_sequences(run: &Run) -> BTreeMap<String, u64> {
    let mut sequences = BTreeMap::new();
    for line in run.log_text().lines() {
        let Some(rest) = line.split("[operation_applied] ").nth(1) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix("sequence=") else {
            continue;
        };
        let (sequence, rest) = rest.split_once(' ').unwrap();
        if let Some(target) = rest.strip_prefix("kind=install_file target=") {
            sequences.insert(
                target.trim().to_ascii_lowercase(),
                sequence.parse().unwrap(),
            );
        }
    }
    sequences
}

/// The operation sequences of the first file batch the builder declared,
/// in order: the metadata's batch, mapped through the sequences a clean
/// install gave its files.
fn first_batch_sequences(a: &VersionFixture) -> Vec<u64> {
    let mut machine = Machine::new("batch-map");
    let clean = machine.install(a);
    assert!(clean.success, "{}", clean.stdout);
    let sequences = install_sequences(&clean);
    let built = tigersetup_build::inspect::inspect(&a.installer)
        .unwrap()
        .to_json();
    let batch = &built["file_batches"].as_array().unwrap()[0];
    let first = batch["first_file"].as_u64().unwrap() as usize;
    let count = batch["file_count"].as_u64().unwrap() as usize;
    let files = built["files"].as_array().unwrap();
    let mut ops: Vec<u64> = files[first..first + count]
        .iter()
        .filter_map(|file| {
            let path = file["path"].as_str().unwrap().replace('/', "\\");
            sequences.get(&path.to_ascii_lowercase()).copied()
        })
        .collect();
    ops.sort_unstable();
    assert!(
        ops.len() >= 8,
        "the first batch holds enough files to interrupt inside: {ops:?}"
    );
    assert_eq!(
        ops.last().unwrap() - ops.first().unwrap() + 1,
        ops.len() as u64,
        "a first install walks the batch's files consecutively: {ops:?}"
    );
    ops
}

/// A file batch is the unit a restart recovers. A crash inside one leaves
/// the whole batch `applying` with nothing acknowledged — before its first
/// mutation, part-way through its files, or after every file is in place
/// but before the batch's completion is journaled — and recovery inspects
/// every target of that batch: the files the interrupted run wrote are
/// completed without being rewritten, the missing ones are written, and
/// the installation converges to the verified state.
#[test]
fn a_crash_inside_a_file_batch_is_recovered_by_reconciling_the_batch() {
    let a = &fixture().a;
    let ops = first_batch_sequences(a);
    let first = *ops.first().unwrap();
    let last = *ops.last().unwrap();
    let middle = ops[ops.len() / 2];
    let count = ops.len() as u64;
    // (fault, files the crashed run left in place, files recovery writes)
    let cases = [
        (format!("after_prepare@{first}:crash:batched"), 0, count),
        (
            format!("after_rename@{middle}:crash:batched"),
            middle - first + 1,
            last - middle,
        ),
        (format!("after_rename@{last}:crash:batched"), count, 0),
    ];
    for (fault, written, rewritten) in cases {
        let mut machine = Machine::new("batch-crash");
        let crashed = machine.install_with_faults(a, &[&fault]);
        assert!(!crashed.success, "{fault}: the run must abort");
        assert!(crashed.log_has("[fault_injected]"), "{fault}");
        // The batch, not the file, is what the journal acknowledges: the
        // whole batch is `applying` with one batch id, and no file of it is
        // `applied`, however many the crashed run wrote.
        {
            let db = rusqlite::Connection::open(machine.state_dir().join("state.db")).unwrap();
            let (applying, applied, batches): (i64, i64, i64) = db
                .query_row(
                    "SELECT sum(state = 'applying'), sum(state = 'applied'), count(DISTINCT batch)                      FROM operation WHERE kind = 'install_file' AND sequence BETWEEN ?1 AND ?2",
                    [first as i64, last as i64],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(
                (applying, applied, batches),
                (count as i64, 0, 1),
                "{fault}: the batch is one unit, applying as a whole"
            );
        }

        let verify = machine.verify(a);
        let report = verify.json();
        assert_eq!(
            report["status"], "transaction_open",
            "{fault}: {}",
            verify.stdout
        );
        assert_eq!(report["transaction"]["recovery_direction"], "forward");

        let rerun = machine.install(a);
        let outcome = rerun.json();
        assert_eq!(
            rerun.exit_code,
            Some(0),
            "{fault}: {}\n{}",
            rerun.stdout,
            rerun.log_text()
        );
        assert_eq!(outcome["outcome"], "installed", "{fault}");
        assert_eq!(outcome["recovery"]["direction"], "forward", "{fault}");
        assert_eq!(
            outcome["recovery"]["operations_reapplied"],
            rewritten,
            "{fault}: only the files the crash did not reach are written again\n{}",
            rerun.log_text()
        );
        let completed = rerun
            .log_text()
            .lines()
            .filter(|line| {
                line.contains("[operation_completed] sequence=")
                    && line.contains("kind=install_file")
            })
            .count() as u64;
        assert_eq!(
            completed,
            written,
            "{fault}: the files already in place are completed, not rewritten\n{}",
            rerun.log_text()
        );
        assert!(rerun.log_has("[recovery_completed] direction=forward state=committed"));
        machine.assert_verified(a);
    }
}

/// A crash inside a batch of an upgrade is recovered the same way, and
/// the files the previous version owned are the previous version's again
/// after a rollback of an interrupted batch: the batch's undo records —
/// each replaced file kept in the staging area — are durable before the
/// batch's first mutation.
#[test]
fn a_crash_inside_an_upgrade_batch_rolls_back_to_the_previous_version_when_the_package_is_gone() {
    let (a, b) = {
        let f = fixture();
        (&f.a, &f.b)
    };
    let mut machine = Machine::new("batch-upgrade");
    machine.install(a);
    // The upgrade's own batches: the files it replaces or adds, walked
    // after the kept ones. Interrupt part-way through the first one.
    let sequences: Vec<u64> = {
        let mut probe = Machine::new("batch-upgrade-map");
        probe.install(a);
        let upgraded = probe.install(b);
        assert!(upgraded.success, "{}", upgraded.stdout);
        let mut ops: Vec<u64> = install_sequences(&upgraded).into_values().collect();
        ops.sort_unstable();
        ops
    };
    assert!(sequences.len() >= 4, "{sequences:?}");
    let middle = sequences[sequences.len() / 2];
    let crashed =
        machine.install_with_faults(b, &[&format!("after_rename@{middle}:crash:batched")]);
    assert!(!crashed.success);

    // Without the new package, the open upgrade can only roll back — to a
    // complete, verified 1.0.0.
    let resumed = machine.uninstall(a);
    let outcome = resumed.json();
    assert_eq!(
        outcome["recovery"]["direction"],
        "rollback",
        "{}\n{}",
        resumed.stdout,
        resumed.log_text()
    );
    assert!(resumed.log_has("[recovery_completed] direction=rollback state=rolled_back"));
    // The uninstall that followed removed 1.0.0 cleanly, which it can only
    // do from a complete rollback.
    assert_eq!(outcome["outcome"], "uninstalled");
    machine.assert_absent(a);
}

#[test]
fn crash_then_uninstall_completes_the_install_first() {
    let a = &fixture().a;
    let mut machine = Machine::new("crash-uninstall");
    let crashed = machine.install_with_faults(a, &["after_applied@30:crash"]);
    assert!(!crashed.success);
    let uninstall = machine.uninstall(a);
    let outcome = uninstall.json();
    assert_eq!(
        uninstall.exit_code,
        Some(0),
        "{}\n{}",
        uninstall.stdout,
        uninstall.log_text()
    );
    assert_eq!(outcome["outcome"], "uninstalled");
    assert_eq!(outcome["recovery"]["direction"], "forward");
    assert!(uninstall.log_has("[recovery_completed] direction=forward state=committed"));
    assert!(
        uninstall.log_has("[transaction_committed] transaction=")
            && uninstall.log_has("kind=uninstall")
    );
    machine.assert_absent(a);
}

#[test]
fn injected_failure_rolls_back_to_absence() {
    let a = &fixture().a;
    for point in ["after_prepare@1", "after_applied@30", "before_commit"] {
        let mut machine = Machine::new("fail");
        let run = machine.install_with_faults(a, &[&format!("{point}:fail")]);
        let outcome = run.json();
        assert_eq!(run.exit_code, Some(1), "{point}: {}", run.stdout);
        assert_eq!(outcome["outcome"], "rolled_back", "{point}");
        assert_eq!(outcome["code"], "fault_injected", "{point}");
        assert_eq!(outcome["transaction"]["state"], "rolled_back", "{point}");
        assert!(run.log_has("[transaction_rolled_back]"), "{point}");
        machine.assert_absent(a);
        let inspect = machine.inspect(a).json();
        assert!(
            inspect["transaction"].is_null(),
            "{point}: nothing left open"
        );
    }
}

#[test]
fn interrupted_rollback_resumes_and_repeats_idempotently() {
    let a = &fixture().a;
    let mut machine = Machine::new("rollback-crash");
    let crashed = machine.install_with_faults(
        a,
        &["after_applied@30:fail", "after_rollback_undo@20:crash"],
    );
    assert!(!crashed.success, "the rollback must abort mid-way");
    assert!(crashed.log_has("[transaction_rolling_back]"));
    assert!(crashed.log_has("[operation_rolled_back] sequence=21"));
    assert!(
        !crashed.log_has("[operation_rolled_back] sequence=20"),
        "operation 20 was undone but not journaled"
    );

    let verify = machine.verify(a);
    let report = verify.json();
    assert_eq!(report["status"], "transaction_open", "{}", verify.stdout);
    assert_eq!(report["transaction"]["state"], "rolling_back");
    assert_eq!(report["transaction"]["recovery_direction"], "rollback");

    let resume = machine.uninstall(a);
    let outcome = resume.json();
    assert_eq!(
        resume.exit_code,
        Some(0),
        "{}\n{}",
        resume.stdout,
        resume.log_text()
    );
    assert_eq!(outcome["outcome"], "not_installed");
    assert_eq!(outcome["recovery"]["direction"], "rollback");
    // The batch is the unit of the rollback as of the forward walk: the
    // crash left the file batch operation 20 belongs to `rolling_back` as
    // a whole, so the resume undoes that whole batch again — operation 21,
    // undone before the crash, included, which every undo tolerates — and
    // then everything before it, down to the install root.
    let rolled_back = outcome["recovery"]["operations_rolled_back"]
        .as_u64()
        .unwrap();
    assert!(
        rolled_back > 20,
        "{rolled_back}: the batch of operation 20, then 19..1"
    );
    assert!(resume.log_has("[recovery_started] direction=rollback"));
    assert!(resume.log_has("[operation_rolled_back] sequence=21"));
    assert!(resume.log_has("[operation_rolled_back] sequence=20"));
    assert!(resume.log_has("[operation_rolled_back] sequence=1 "));
    assert!(resume.log_has("[recovery_completed] direction=rollback state=rolled_back"));
    machine.assert_absent(a);

    let again = machine.uninstall(a);
    let outcome = again.json();
    assert_eq!(outcome["outcome"], "not_installed");
    assert!(
        outcome["recovery"].is_null(),
        "nothing is left to roll back"
    );
    machine.assert_absent(a);
}

#[test]
fn zero_filled_target_is_detected_and_reapplied() {
    let a = &fixture().a;
    // A multi-megabyte file, found by what the plan calls it rather than by
    // a sequence number: the walk is in payload order, and that order is
    // the builder's.
    let big = {
        let mut machine = Machine::new("zero-fill-plan");
        let run = machine.install(a);
        assert_eq!(run.exit_code, Some(0), "{}", run.log_text());
        sequence_in_log(&run.log_text(), "install_file", "data\\big-2.bin")
    };
    let mut machine = Machine::new("zero-fill");
    let crashed = machine.install_with_faults(a, &[&format!("after_rename@{big}:crash")]);
    assert!(!crashed.success);
    let relative = faulted_target(&crashed);
    let target = machine.install_root().join(&relative);
    let size = fs::metadata(&target)
        .expect("the renamed target exists")
        .len();
    assert!(
        size > 1024 * 1024,
        "operation {big} writes a multi-megabyte file, got {size} bytes for {relative}"
    );
    fs::write(&target, vec![0u8; size as usize]).unwrap();

    let rerun = machine.install(a);
    assert_eq!(
        rerun.exit_code,
        Some(0),
        "{}\n{}",
        rerun.stdout,
        rerun.log_text()
    );
    let log = rerun.log_text();
    assert!(
        log.lines()
            .any(|l| l.contains("[file_content_mismatch]") && l.contains(&relative)),
        "{log}"
    );
    assert!(
        log.lines()
            .any(|l| l.contains(&format!("[operation_reapplied] sequence={big} "))),
        "{log}"
    );
    assert_eq!(rerun.json()["recovery"]["operations_reapplied"], 1);
    machine.assert_verified(a);
}

#[test]
fn missing_target_in_applying_state_is_reapplied() {
    let a = &fixture().a;
    let mut machine = Machine::new("missing");
    let crashed = machine.install_with_faults(a, &["after_write_before_flush@30:crash"]);
    assert!(!crashed.success);
    let relative = faulted_target(&crashed);
    let target = machine.install_root().join(&relative);
    assert!(!target.exists(), "the target was never renamed into place");
    assert_eq!(
        temp_files_under(&machine.install_root()).len(),
        1,
        "the unflushed temporary is still there"
    );

    let rerun = machine.install(a);
    assert_eq!(
        rerun.exit_code,
        Some(0),
        "{}\n{}",
        rerun.stdout,
        rerun.log_text()
    );
    let log = rerun.log_text();
    assert!(log.contains("[temp_file_swept]"), "{log}");
    assert!(
        log.lines()
            .any(|l| l.contains("[file_missing]") && l.contains(&relative)),
        "{log}"
    );
    assert!(log.contains("[operation_reapplied] sequence=30"), "{log}");
    machine.assert_verified(a);
}

#[test]
fn skipped_flush_still_recovers_clean() {
    let a = &fixture().a;
    let mut machine = Machine::new("skip-flush");
    let crashed = machine.install_with_faults(a, &["after_rename@30:crash:skip_flush"]);
    assert!(!crashed.success);
    assert!(
        crashed.log_has("[flush_skipped] sequence=30"),
        "{}",
        crashed.log_text()
    );
    let rerun = machine.install(a);
    assert_eq!(
        rerun.exit_code,
        Some(0),
        "{}\n{}",
        rerun.stdout,
        rerun.log_text()
    );
    assert!(
        rerun.log_has("[operation_completed] sequence=30"),
        "the renamed file was intact locally\n{}",
        rerun.log_text()
    );
    machine.assert_verified(a);
}

#[test]
fn modified_owned_file_is_preserved_at_uninstall() {
    let a = &fixture().a;
    let mut machine = Machine::new("modified");
    machine.install(a);
    let modified = machine
        .install_root()
        .join("doc")
        .join("manual")
        .join("ch-03.md");
    let mut bytes = fs::read(&modified).unwrap();
    bytes.extend_from_slice(b"\nlocal edit\n");
    fs::write(&modified, &bytes).unwrap();

    let verify = machine.verify(a);
    let report = verify.json();
    assert_eq!(report["status"], "failed");
    assert_eq!(
        report["findings"][0]["code"], "file_modified",
        "a read-only observation reports what it sees, not what a run would do"
    );
    assert_eq!(
        report["findings"][0]["path"],
        modified.display().to_string()
    );

    let uninstall = machine.uninstall(a);
    let outcome = uninstall.json();
    assert_eq!(uninstall.exit_code, Some(0), "{}", uninstall.stdout);
    assert_eq!(outcome["outcome"], "uninstalled");
    assert_eq!(outcome["findings"][0]["code"], "file_modified_preserved");
    assert_eq!(
        outcome["findings"][0]["path"],
        modified.display().to_string()
    );
    assert!(uninstall.log_has("[file_modified_preserved]"));
    assert_eq!(
        fs::read(&modified).unwrap(),
        bytes,
        "the modified file is preserved"
    );
    assert!(
        machine.install_root().join("doc").join("manual").is_dir(),
        "its directory is preserved"
    );
    assert!(
        !machine.install_root().join("bin").exists(),
        "everything else is removed"
    );
    assert!(
        !machine
            .install_root()
            .join("doc")
            .join("manual")
            .join("ch-02.md")
            .exists()
    );

    let verify = machine.verify(a);
    assert_eq!(verify.json()["status"], "not_installed");
    assert_eq!(machine.uninstall(a).json()["outcome"], "not_installed");
}

#[test]
fn verify_reports_missing_and_modified_files() {
    let a = &fixture().a;
    let mut machine = Machine::new("verify");
    machine.install(a);
    fs::remove_file(machine.install_root().join("LICENSE.txt")).unwrap();
    fs::write(machine.install_root().join("CHANGELOG.md"), b"rewritten").unwrap();
    let verify = machine.verify(a);
    let report = verify.json();
    assert_eq!(verify.exit_code, Some(1));
    assert_eq!(report["status"], "failed");
    let codes: Vec<&str> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["file_modified", "file_missing"]);
    assert_eq!(report["counts"]["files_checked"], a.files.len() as u64);
    assert_eq!(report["counts"]["files_ok"], a.files.len() as u64 - 2);
}

#[test]
fn invalid_invocations_exit_2_and_touch_nothing() {
    let a = &fixture().a;
    let mut machine = Machine::new("invalid");
    // Only invocations that are wrong however the engine is built. A
    // non-quiet run is not one of them: it opens the wizard. Neither is
    // `--scope machine`, which this package declares and the engine serves.
    let cases: [&[&str]; 4] = [
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--install-root",
            "relative\\path",
        ],
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--fault",
            "nowhere:crash",
        ],
        &[
            "install", "--quiet", "--scope", "user", "--option", "nope", "on",
        ],
        &["install", "--quiet", "--scope", "sideways"],
    ];
    for args in cases {
        let run = machine.run(&a.installer, args);
        assert_eq!(run.exit_code, Some(2), "{args:?}: {}", run.stdout);
        let outcome = run.json();
        assert_eq!(outcome["outcome"], "failed", "{args:?}");
        assert_eq!(outcome["exit_code"], 2, "{args:?}");
    }
    assert!(!machine.state_dir().exists(), "no state was created");
    assert!(!machine.install_root().exists());
}

#[test]
fn hold_pauses_then_continues() {
    let a = &fixture().a;
    let mut machine = Machine::new("hold");
    let started = Instant::now();
    let run = machine.install_with_faults(a, &["after_prepare@1:hold:2"]);
    assert!(
        started.elapsed().as_secs_f64() >= 2.0,
        "the hold lasted at least two seconds"
    );
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert!(run.log_has("[fault_injected] point=after_prepare sequence=1 action=hold:2"));
    assert!(run.log_has("[fault_hold_finished]"));
    machine.assert_verified(a);
}

/// The boundary announces itself before it acts.
///
/// A harness outside the process — the lab cutting a virtual machine's power —
/// cannot see a journal boundary, so without this it has to guess how many
/// seconds after launch the run will be sitting at one. The signal turns that
/// guess into an observation: the file exists only once the fault point has
/// been reached, and the run is still there when it is seen, because the fault
/// holds afterwards.
#[test]
fn a_fault_signals_the_boundary_before_it_acts() {
    let a = &fixture().a;
    let mut machine = Machine::new("fault-signal");
    let signal = machine.scratch_path("boundary.signal");
    assert!(!signal.exists());

    let run =
        machine.install_with_faults_signalling(a, &["after_write_before_flush@30:hold:2"], &signal);

    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert!(
        signal.exists(),
        "the boundary was announced
{}",
        run.log_text()
    );
    assert!(run.log_has("[fault_signalled]"));
    // The order is what makes the signal usable: announced, then held, so an
    // interruption arriving just after the file appears lands at the boundary
    // rather than after it.
    let log = run.log_text();
    let signalled = log.find("[fault_signalled]").expect("signalled");
    let finished = log.find("[fault_hold_finished]").expect("hold finished");
    assert!(signalled < finished, "the signal precedes the hold's end");
    machine.assert_verified(a);
}

#[test]
fn install_root_override_is_honoured() {
    let a = &fixture().a;
    let mut machine = Machine::new("root");
    let custom = machine.localappdata.join("Elsewhere").join("TestApp");
    let run = machine.run(
        &a.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--install-root",
            &custom.display().to_string(),
        ],
    );
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert_eq!(
        outcome["installation"]["install_root"],
        custom.display().to_string()
    );
    assert!(custom.join("bin").join("TigerSetupTestApp.exe").exists());
    assert_eq!(machine.verify(a).json()["status"], "ok");
    machine.uninstall(a);
    assert!(!custom.exists());
}

#[test]
fn file_edited_after_a_crashed_removal_is_preserved_by_recovery() {
    let a = &fixture().a;
    // The removal sequence of bin\lib-01.dll is the same on every machine;
    // learn it from a clean uninstall.
    let mut scout = Machine::new("uninstall-plan");
    scout.install(a);
    let clean = scout.uninstall(a);
    let sequence = sequence_of(&clean, "remove_file", "bin\\lib-01.dll");

    let mut machine = Machine::new("removal-crash-edit");
    machine.install(a);
    let crashed = machine.run(
        &a.installer,
        &[
            "uninstall",
            "--quiet",
            "--scope",
            "user",
            "--fault",
            &format!("after_applying@{sequence}:crash"),
        ],
    );
    assert!(!crashed.success, "the uninstall must abort");
    let target = machine.install_root().join("bin").join("lib-01.dll");
    assert!(target.exists(), "the crash landed before the deletion");
    fs::write(&target, b"edited after the crash").unwrap();

    let resume = machine.uninstall(a);
    let outcome = resume.json();
    assert_eq!(
        resume.exit_code,
        Some(0),
        "{}\n{}",
        resume.stdout,
        resume.log_text()
    );
    assert_eq!(
        outcome["outcome"], "not_installed",
        "recovery finished the uninstall"
    );
    assert_eq!(outcome["recovery"]["direction"], "forward");
    let findings = findings_of(&outcome);
    assert!(
        findings.contains(&(
            "file_modified_preserved".to_string(),
            target.display().to_string()
        )),
        "{findings:?}"
    );
    assert!(
        findings.contains(&(
            "directory_not_empty_preserved".to_string(),
            machine.install_root().join("bin").display().to_string()
        )),
        "{findings:?}"
    );
    assert!(resume.log_has("[file_modified_preserved]"));
    assert_eq!(
        fs::read(&target).unwrap(),
        b"edited after the crash",
        "the edited file survives"
    );
    let mut expected = std::collections::BTreeSet::new();
    expected.insert("bin\\lib-01.dll".to_string());
    assert_eq!(
        files_under(&machine.install_root()),
        expected,
        "everything else is gone"
    );
    assert_eq!(machine.verify(a).json()["status"], "not_installed");
}

#[test]
fn foreign_content_keeps_its_directory_at_uninstall() {
    let a = &fixture().a;
    let mut machine = Machine::new("foreign");
    machine.install(a);
    let foreign = machine.install_root().join("bin").join("foreign.txt");
    fs::write(&foreign, b"not ours").unwrap();

    let uninstall = machine.uninstall(a);
    let outcome = uninstall.json();
    assert_eq!(uninstall.exit_code, Some(0), "{}", uninstall.stdout);
    assert_eq!(outcome["outcome"], "uninstalled");
    let findings = findings_of(&outcome);
    assert_eq!(
        findings,
        vec![
            (
                "directory_not_empty_preserved".to_string(),
                machine.install_root().join("bin").display().to_string()
            ),
            (
                "directory_not_empty_preserved".to_string(),
                machine.install_root().display().to_string()
            ),
        ]
    );
    assert!(foreign.exists(), "foreign content is never deleted");
    assert!(
        !machine
            .install_root()
            .join("bin")
            .join("lib-01.dll")
            .exists()
    );
    assert_eq!(machine.verify(a).json()["status"], "not_installed");
}
