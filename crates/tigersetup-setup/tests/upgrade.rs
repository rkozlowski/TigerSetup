//! Upgrade convergence: with 1.0.0 installed, 1.1.0's installer plans from
//! the database — keep, replace, add, remove — and an interruption at any
//! boundary ends in exactly 1.0.0 or exactly 1.1.0, verified, with every
//! on-disk file carrying that version's content and nothing of the other.

mod common;

use std::fs;
use std::sync::OnceLock;

use common::*;

/// The operation the tests interrupt, by what the upgrade does to its file.
struct Probe {
    what: &'static str,
    kind: &'static str,
    target: &'static str,
}

const PROBES: [Probe; 4] = [
    Probe {
        what: "replace",
        kind: "install_file",
        target: "data\\big-4.bin",
    },
    Probe {
        what: "add",
        kind: "install_file",
        target: "data\\big-6.bin",
    },
    Probe {
        what: "remove",
        kind: "remove_file",
        target: "bin\\lib-08.dll",
    },
    Probe {
        what: "remove_directory",
        kind: "remove_directory",
        target: "doc\\legacy",
    },
];

/// Fault points an operation of this kind passes through.
fn points_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "install_file" => &[
            "after_prepare",
            "after_applying",
            "after_write_before_flush",
            "after_flush_before_rename",
            "after_rename",
            "after_applied",
        ],
        _ => &["after_prepare", "after_applying", "after_applied"],
    }
}

/// The log of one clean upgrade, learned once: the plan is deterministic, so
/// its sequence numbers hold on every machine.
fn plan_log() -> &'static str {
    static PLAN_LOG: OnceLock<String> = OnceLock::new();
    PLAN_LOG.get_or_init(|| {
        let fixture = fixture();
        let mut machine = Machine::new("upgrade-plan");
        machine.install(&fixture.a);
        let upgrade = machine.install(&fixture.b);
        assert_eq!(
            upgrade.exit_code,
            Some(0),
            "{}\n{}",
            upgrade.stdout,
            upgrade.log_text()
        );
        upgrade.log_text()
    })
}

/// Journal sequences of the probed operations.
fn sequences() -> &'static Vec<(&'static Probe, i64)> {
    static SEQUENCES: OnceLock<Vec<(&'static Probe, i64)>> = OnceLock::new();
    SEQUENCES.get_or_init(|| {
        PROBES
            .iter()
            .map(|probe| (probe, sequence_in_log(plan_log(), probe.kind, probe.target)))
            .collect()
    })
}

#[test]
fn crash_during_upgrade_rollback_resumes_to_a_complete_a() {
    let fixture = fixture();
    // A replaced file late in the walk — files are walked in payload order,
    // extension first — and an added file that is undone before it (lower
    // sequence): its undo deletes it, and the crash lands right after that
    // deletion.
    let replace = sequence_in_log(plan_log(), "install_file", "doc\\readme.md");
    let undone_first = sequence_in_log(plan_log(), "install_file", "bin\\lib-09.dll");
    assert!(undone_first < replace);

    let mut machine = Machine::new("upgrade-rollback-crash");
    install_a(&mut machine);
    let crashed = machine.install_with_faults(
        &fixture.b,
        &[
            &format!("after_applied@{replace}:fail"),
            &format!("after_rollback_undo@{undone_first}:crash"),
        ],
    );
    assert!(!crashed.success, "the rollback must abort mid-way");
    assert!(crashed.log_has("[transaction_rolling_back]"));
    assert!(crashed.log_has(&format!("[operation_rolled_back] sequence={replace} ")));
    assert!(!crashed.log_has(&format!("[operation_rolled_back] sequence={undone_first} ")));
    assert!(
        !machine
            .install_root()
            .join("bin")
            .join("lib-09.dll")
            .exists(),
        "the undo ran before the crash"
    );

    let report = machine.verify(&fixture.a).json();
    assert_eq!(report["status"], "transaction_open", "{report}");
    assert_eq!(report["transaction"]["kind"], "upgrade");
    assert_eq!(report["transaction"]["state"], "rolling_back");
    assert_eq!(report["transaction"]["recovery_direction"], "rollback");
    assert_eq!(
        machine.inspect(&fixture.b).json()["transaction"]["recovery_direction"],
        "rollback",
        "even 1.1.0's own engine finishes an open rollback as a rollback"
    );

    let resume = machine.install(&fixture.a);
    let outcome = resume.json();
    assert_eq!(
        resume.exit_code,
        Some(0),
        "{}\n{}",
        resume.stdout,
        resume.log_text()
    );
    assert_eq!(outcome["code"], "already_installed");
    assert_eq!(outcome["recovery"]["direction"], "rollback");
    assert!(resume.log_has(&format!("[operation_rolled_back] sequence={undone_first} ")));
    assert!(resume.log_has("[recovery_completed] direction=rollback state=rolled_back"));
    machine.assert_inspect_version(&fixture.a, VERSION_A);
    machine.assert_verified(&fixture.a);
}

fn install_a(machine: &mut Machine) {
    let run = machine.install(&fixture().a);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
}

#[test]
fn upgrade_keeps_replaces_adds_and_removes() {
    let fixture = fixture();
    let mut machine = Machine::new("upgrade");
    install_a(&mut machine);
    let a_hash_of_changed = fixture.a.payload_sha256("data/big-4.bin");
    let a_hash_of_kept = fixture.a.payload_sha256("data/big-5.bin");

    let upgrade = machine.install(&fixture.b);
    let outcome = upgrade.json();
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["code"], "ok");
    assert_eq!(outcome["transaction"]["kind"], "upgrade");
    assert_eq!(outcome["transaction"]["state"], "committed");
    assert_eq!(outcome["transaction"]["from_version"], VERSION_A);
    assert_eq!(outcome["transaction"]["to_version"], VERSION_B);
    assert_eq!(outcome["installation"]["version"], VERSION_B);
    assert_eq!(
        outcome["installation"]["file_count"],
        fixture.b.files.len() as u64
    );
    assert!(outcome["recovery"].is_null());
    assert!(outcome["findings"].is_null() || outcome["findings"].as_array().unwrap().is_empty());
    let log = upgrade.log_text();
    assert!(
        log.contains("kind=upgrade from=1.0.0 to=1.1.0")
            && log.contains(
                "kept=54 replaced=5 added=5 removed=5 directories_created=1 directories_removed=1"
            ),
        "{log}"
    );
    assert!(
        log.contains("[operation_applied] sequence=")
            && log.contains("kind=remove_directory target=doc\\legacy")
    );
    assert!(
        !log.contains("kind=keep_file"),
        "keeps are never walked\n{log}"
    );

    let inspect = machine.assert_inspect_version(&fixture.b, VERSION_B);
    assert_eq!(
        inspect["installation"]["file_count"],
        fixture.b.files.len() as u64
    );
    machine.assert_verified(&fixture.b);
    assert!(
        !machine.install_root().join("doc").join("legacy").exists(),
        "the directory only 1.0.0 needed is gone"
    );
    assert!(
        machine
            .install_root()
            .join("data")
            .join("extra")
            .join("notes.txt")
            .exists()
    );
    assert_ne!(
        sha256_hex(&fs::read(machine.install_root().join("data").join("big-4.bin")).unwrap()),
        a_hash_of_changed,
        "the changed file carries 1.1.0's content"
    );
    assert_eq!(
        sha256_hex(&fs::read(machine.install_root().join("data").join("big-5.bin")).unwrap()),
        a_hash_of_kept,
        "the kept file is untouched"
    );

    let uninstall = machine.uninstall(&fixture.b);
    assert_eq!(
        uninstall.json()["outcome"],
        "uninstalled",
        "{}",
        uninstall.stdout
    );
    machine.assert_absent(&fixture.b);
}

#[test]
fn crash_during_upgrade_then_b_completes_forward() {
    let fixture = fixture();
    let mut scenarios: Vec<(String, String)> = Vec::new();
    for (probe, sequence) in sequences() {
        for point in points_for(probe.kind) {
            scenarios.push((
                format!("{point}@{sequence}:crash"),
                format!("{} {}", probe.what, probe.target),
            ));
        }
    }
    scenarios.push(("before_commit:crash".into(), "commit".into()));
    scenarios.push(("after_commit_before_cleanup:crash".into(), "cleanup".into()));

    for (fault, what) in scenarios {
        let label = format!("{fault} ({what})");
        let mut machine = Machine::new("upgrade-crash");
        install_a(&mut machine);
        let crashed = machine.install_with_faults(&fixture.b, &[&fault]);
        assert!(!crashed.success, "{label}: the upgrade must abort");
        assert!(
            crashed.log_has("[fault_injected]"),
            "{label}\n{}",
            crashed.log_text()
        );

        let committed_already = fault.starts_with("after_commit_before_cleanup");
        let verify = machine.verify(&fixture.b);
        let report = verify.json();
        if committed_already {
            assert_eq!(report["status"], "ok", "{label}: {}", verify.stdout);
            assert_eq!(report["installation"]["version"], VERSION_B, "{label}");
        } else {
            assert_eq!(
                report["status"], "transaction_open",
                "{label}: {}",
                verify.stdout
            );
            assert_eq!(report["transaction"]["kind"], "upgrade", "{label}");
            assert_eq!(report["transaction"]["from_version"], VERSION_A, "{label}");
            assert_eq!(report["transaction"]["to_version"], VERSION_B, "{label}");
            assert_eq!(
                report["transaction"]["recovery_direction"], "forward",
                "{label}"
            );
            assert_eq!(
                report["installation"]["version"], VERSION_A,
                "{label}: the database still says 1.0.0"
            );
            let via_a = machine.inspect(&fixture.a).json();
            assert_eq!(
                via_a["transaction"]["recovery_direction"], "rollback",
                "{label}: 1.0.0's engine would roll back"
            );
            assert_eq!(via_a["installation"]["version"], VERSION_A, "{label}");
        }

        let rerun = machine.install(&fixture.b);
        let outcome = rerun.json();
        assert_eq!(
            rerun.exit_code,
            Some(0),
            "{label}: {}\n{}",
            rerun.stdout,
            rerun.log_text()
        );
        assert_eq!(outcome["outcome"], "installed", "{label}");
        assert_eq!(outcome["code"], "already_installed", "{label}");
        assert_eq!(outcome["installation"]["version"], VERSION_B, "{label}");
        if committed_already {
            assert!(outcome["recovery"].is_null(), "{label}");
            assert!(
                rerun.log_has("[staging_swept]"),
                "{label}\n{}",
                rerun.log_text()
            );
        } else {
            assert_eq!(outcome["recovery"]["direction"], "forward", "{label}");
            assert!(
                rerun.log_has("[recovery_started] direction=forward"),
                "{label}"
            );
            assert!(
                rerun.log_has("[recovery_completed] direction=forward state=committed"),
                "{label}"
            );
        }
        machine.assert_inspect_version(&fixture.b, VERSION_B);
        machine.assert_verified(&fixture.b);
    }
}

#[test]
fn injected_failure_during_upgrade_rolls_back_to_a() {
    let fixture = fixture();
    let mut faults: Vec<(String, String)> = sequences()
        .iter()
        .map(|(probe, sequence)| {
            (
                format!("after_applied@{sequence}:fail"),
                probe.what.to_string(),
            )
        })
        .collect();
    faults.push(("before_commit:fail".into(), "commit".into()));

    for (fault, what) in faults {
        let label = format!("{fault} ({what})");
        let mut machine = Machine::new("upgrade-fail");
        install_a(&mut machine);
        let run = machine.install_with_faults(&fixture.b, &[&fault]);
        let outcome = run.json();
        assert_eq!(
            run.exit_code,
            Some(1),
            "{label}: {}\n{}",
            run.stdout,
            run.log_text()
        );
        assert_eq!(outcome["outcome"], "rolled_back", "{label}");
        assert_eq!(outcome["code"], "fault_injected", "{label}");
        assert_eq!(outcome["transaction"]["kind"], "upgrade", "{label}");
        assert_eq!(outcome["transaction"]["state"], "rolled_back", "{label}");
        assert!(run.log_has("[transaction_rolled_back]"), "{label}");
        machine.assert_inspect_version(&fixture.a, VERSION_A);
        machine.assert_verified(&fixture.a);
        assert!(
            !machine
                .install_root()
                .join("data")
                .join("big-6.bin")
                .exists(),
            "{label}: 1.1.0-only files are gone"
        );
        assert!(
            !machine.install_root().join("data").join("extra").exists(),
            "{label}: the 1.1.0-only directory is gone"
        );
    }
}

#[test]
fn a_uninstall_rolls_an_open_upgrade_back_then_uninstalls() {
    let fixture = fixture();
    let replace_sequence = sequences()
        .iter()
        .find(|(p, _)| p.what == "replace")
        .unwrap()
        .1;
    for fault in [
        "before_commit:crash",
        &format!("after_applied@{replace_sequence}:crash"),
    ] {
        let mut machine = Machine::new("upgrade-a-uninstall");
        install_a(&mut machine);
        let crashed = machine.install_with_faults(&fixture.b, &[fault]);
        assert!(!crashed.success, "{fault}");

        let uninstall = machine.uninstall(&fixture.a);
        let outcome = uninstall.json();
        assert_eq!(
            uninstall.exit_code,
            Some(0),
            "{fault}: {}\n{}",
            uninstall.stdout,
            uninstall.log_text()
        );
        assert_eq!(outcome["outcome"], "uninstalled", "{fault}");
        assert_eq!(outcome["recovery"]["direction"], "rollback", "{fault}");
        assert!(
            outcome["recovery"]["operations_rolled_back"]
                .as_u64()
                .unwrap()
                > 0,
            "{fault}"
        );
        let log = uninstall.log_text();
        assert!(
            log.contains("[recovery_started] direction=rollback"),
            "{fault}\n{log}"
        );
        assert!(
            log.contains("[transaction_rolled_back] transaction=") && log.contains("kind=upgrade"),
            "{fault}\n{log}"
        );
        assert!(
            log.contains("[recovery_completed] direction=rollback state=rolled_back"),
            "{fault}\n{log}"
        );
        assert!(
            log.contains("[transaction_committed] transaction=") && log.contains("kind=uninstall"),
            "{fault}\n{log}"
        );
        assert_eq!(outcome["transaction"]["kind"], "uninstall", "{fault}");
        assert_eq!(
            outcome["transaction"]["from_version"], VERSION_A,
            "{fault}: what was uninstalled was 1.0.0"
        );
        machine.assert_absent(&fixture.a);

        let again = machine.uninstall(&fixture.a);
        let outcome = again.json();
        assert_eq!(outcome["outcome"], "not_installed", "{fault}");
        assert!(outcome["recovery"].is_null(), "{fault}");
        machine.assert_absent(&fixture.a);
    }
}

#[test]
fn crash_before_commit_then_a_rollback_leaves_a_complete_a() {
    let fixture = fixture();
    let mut machine = Machine::new("upgrade-a-restore");
    install_a(&mut machine);
    let crashed = machine.install_with_faults(&fixture.b, &["before_commit:crash"]);
    assert!(!crashed.success);
    // 1.0.0's installer run as an install rolls the upgrade back, then finds
    // 1.0.0 installed.
    let run = machine.install(&fixture.a);
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(outcome["code"], "already_installed");
    assert_eq!(outcome["recovery"]["direction"], "rollback");
    machine.assert_inspect_version(&fixture.a, VERSION_A);
    machine.assert_verified(&fixture.a);
}

#[test]
fn modified_a_only_file_is_preserved_by_the_upgrade() {
    let fixture = fixture();
    let mut machine = Machine::new("upgrade-preserve");
    install_a(&mut machine);
    let edited = machine.install_root().join("bin").join("lib-08.dll");
    fs::write(&edited, b"user data in a file 1.1.0 drops").unwrap();

    let upgrade = machine.install(&fixture.b);
    let outcome = upgrade.json();
    assert_eq!(upgrade.exit_code, Some(0), "{}", upgrade.stdout);
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["findings"][0]["code"], "file_modified_preserved");
    assert_eq!(outcome["findings"][0]["path"], edited.display().to_string());
    assert!(edited.exists(), "the edited file is preserved");
    let verify = machine.verify(&fixture.b);
    assert_eq!(
        verify.json()["status"],
        "ok",
        "1.1.0 does not own the preserved file\n{}",
        verify.stdout
    );
    let mut expected = fixture.b.relative_paths();
    expected.insert("bin\\lib-08.dll".into());
    assert_eq!(files_under(&machine.install_root()), expected);
}

#[test]
fn downgrade_is_an_upgrade_to_an_older_version() {
    let fixture = fixture();
    let mut machine = Machine::new("downgrade");
    let install = machine.install(&fixture.b);
    assert_eq!(install.exit_code, Some(0), "{}", install.stdout);
    let downgrade = machine.install(&fixture.a);
    let outcome = downgrade.json();
    assert_eq!(
        downgrade.exit_code,
        Some(0),
        "{}\n{}",
        downgrade.stdout,
        downgrade.log_text()
    );
    assert_eq!(outcome["transaction"]["kind"], "upgrade");
    assert_eq!(outcome["transaction"]["from_version"], VERSION_B);
    assert_eq!(outcome["transaction"]["to_version"], VERSION_A);
    machine.assert_inspect_version(&fixture.a, VERSION_A);
    machine.assert_verified(&fixture.a);
}

#[test]
fn upgrade_refuses_to_move_the_installation() {
    let fixture = fixture();
    let mut machine = Machine::new("upgrade-move");
    install_a(&mut machine);
    let elsewhere = machine.localappdata.join("Elsewhere");
    let run = machine.run(
        &fixture.b.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--install-root",
            &elsewhere.display().to_string(),
        ],
    );
    assert_eq!(run.exit_code, Some(2), "{}", run.stdout);
    assert_eq!(run.json()["code"], "install_root_conflict");
    machine.assert_verified(&fixture.a);
}
