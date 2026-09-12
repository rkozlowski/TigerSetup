//! Which installation a run is about when the product may be installed per
//! user and per machine at once: an ordinary rerun continues with the
//! installation the machine holds, an explicit scope is held to the
//! package's policy rather than silently redirected, two installations are
//! never chosen between, and no run ever moves an installation from one
//! scope to the other.
//!
//! These run unelevated. The fixture redirects every known folder into a
//! tree the test process owns, so the same isolated machine holds a user
//! scope and a machine scope side by side, the way a real one does.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;

/// The synthetic package with the existing-scope policy it is asked for.
fn package_with_policy(dir: &Path, policy: &str, version: &str) -> PathBuf {
    let manifest = format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{version}"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]
existing_scope = "{policy}"

[[files]]
source = "payload/**"
"#
    );
    build_small_package(&dir.join(policy).join(version), &manifest)
}

/// The scope of every installation the document lists.
fn scopes_of(installations: &Value) -> Vec<String> {
    installations
        .as_array()
        .map(|list| {
            list.iter()
                .map(|i| i["scope"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Where each scope keeps its state on this machine, whichever scope the
/// fixture was created to serve.
fn user_state_db(machine: &Machine) -> PathBuf {
    machine
        .localappdata
        .join("TigerSetup")
        .join(PRODUCT_ID)
        .join("state.db")
}

fn machine_state_db(machine: &Machine) -> PathBuf {
    machine
        .programdata
        .join("TigerSetup")
        .join(PRODUCT_ID)
        .join("state.db")
}

/// A first install without `--scope` takes the package's default scope, and
/// every rerun without one continues with the installation that exists —
/// an upgrade, then an uninstall — while the other scope stays untouched.
#[test]
fn a_rerun_without_a_scope_continues_with_the_one_installation_the_machine_holds() {
    let (a, b) = (&fixture().a, &fixture().b);
    let mut machine = Machine::new("scope-sticky-user");

    let first = machine.run(&a.installer, &["install", "--quiet"]);
    assert_eq!(first.exit_code, Some(0), "{}", first.stdout);
    assert_eq!(
        first.json()["installation"]["scope"],
        "user",
        "the package declares user scope first, so a first install takes it"
    );

    let upgrade = machine.run(&b.installer, &["install", "--quiet"]);
    assert_eq!(upgrade.exit_code, Some(0), "{}", upgrade.stdout);
    let document = upgrade.json();
    assert_eq!(document["installation"]["scope"], "user");
    assert_eq!(document["transaction"]["kind"], "upgrade");
    assert!(
        !machine_state_db(&machine).exists(),
        "no machine-scope installation was created beside the user one"
    );
    machine.assert_verified(b);

    let inspect = machine.run(&b.installer, &["inspect"]);
    assert_eq!(inspect.exit_code, Some(0), "{}", inspect.stdout);
    let report = inspect.json();
    assert_eq!(report["installation"]["scope"], "user");
    assert_eq!(scopes_of(&report["installations"]), ["user"]);

    let uninstall = machine.run(&b.installer, &["uninstall", "--quiet"]);
    assert_eq!(uninstall.exit_code, Some(0), "{}", uninstall.stdout);
    assert_eq!(uninstall.json()["outcome"], "uninstalled");
    machine.assert_absent(b);
}

/// The same rule with the installation in the scope that is *not* the
/// package's default: the rerun follows the machine-scope installation
/// instead of creating a per-user one because the default says so.
#[test]
fn a_rerun_without_a_scope_follows_a_machine_scope_installation() {
    let (a, b) = (&fixture().a, &fixture().b);
    let mut machine = Machine::in_scope(
        "scope-sticky-machine",
        tigersetup_engine::format::identity::Scope::Machine,
    );

    let first = machine.install(a);
    assert_eq!(first.exit_code, Some(0), "{}", first.stdout);

    let upgrade = machine.run(&b.installer, &["install", "--quiet"]);
    assert_eq!(upgrade.exit_code, Some(0), "{}", upgrade.stdout);
    let document = upgrade.json();
    assert_eq!(document["installation"]["scope"], "machine");
    assert_eq!(document["transaction"]["kind"], "upgrade");
    assert!(
        !user_state_db(&machine).exists(),
        "no per-user installation was created because the default scope is user"
    );
    machine.assert_verified(b);

    let repair = machine.run(&b.installer, &["repair", "--quiet"]);
    assert_eq!(repair.exit_code, Some(0), "{}", repair.stdout);
    assert_eq!(repair.json()["installation"]["scope"], "machine");

    let verify = machine.run(&b.installer, &["verify"]);
    assert_eq!(verify.json()["installation"]["scope"], "machine");

    let uninstall = machine.run(&b.installer, &["uninstall", "--quiet"]);
    assert_eq!(uninstall.exit_code, Some(0), "{}", uninstall.stdout);
    machine.assert_absent(b);
    assert!(!user_state_db(&machine).exists());
}

/// An explicit scope is never rewritten. Under the default policy a request
/// for the empty scope beside an existing installation is a structured
/// refusal that names what exists, and leaves the machine as it was.
#[test]
fn an_explicit_conflicting_scope_is_refused_under_the_default_policy() {
    let a = &fixture().a;
    let mut machine = Machine::in_scope(
        "scope-conflict",
        tigersetup_engine::format::identity::Scope::Machine,
    );
    let first = machine.install(a);
    assert_eq!(first.exit_code, Some(0), "{}", first.stdout);

    let refused = machine.run(&a.installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(refused.exit_code, Some(2), "{}", refused.stdout);
    let document = refused.json();
    assert_eq!(document["outcome"], "failed");
    assert_eq!(document["code"], "scope_conflict");
    assert_eq!(scopes_of(&document["existing_installations"]), ["machine"]);
    assert_eq!(document["existing_installations"][0]["version"], VERSION_A);
    assert!(
        !user_state_db(&machine).exists(),
        "the refusal created nothing in user scope"
    );
    assert!(!machine.localappdata.join("Programs").exists());
    machine.assert_verified(a);

    // Reading and removing the empty scope are not refusals: they report
    // what that scope holds, which is nothing.
    let verify = machine.run(&a.installer, &["verify", "--scope", "user"]);
    assert_eq!(
        verify.json()["status"],
        "not_installed",
        "{}",
        verify.stdout
    );
    let uninstall = machine.run(&a.installer, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(
        uninstall.json()["outcome"],
        "not_installed",
        "{}",
        uninstall.stdout
    );
    machine.assert_verified(a);
}

/// A package that allows parallel installations lets an explicit request
/// create the second one; a rerun without a scope is then ambiguous, and
/// each installation is removed on its own.
#[test]
fn allow_parallel_lets_an_explicit_request_create_a_second_installation() {
    let dir = scratch("scope-parallel");
    let installer = package_with_policy(&dir, "allow-parallel", VERSION_A);
    let mut machine = Machine::new("scope-parallel");

    let user = machine.run(&installer, &["install", "--quiet"]);
    assert_eq!(user.exit_code, Some(0), "{}", user.stdout);
    assert_eq!(user.json()["installation"]["scope"], "user");

    let second = machine.run(&installer, &["install", "--quiet", "--scope", "machine"]);
    assert_eq!(second.exit_code, Some(0), "{}", second.stdout);
    let document = second.json();
    assert_eq!(document["installation"]["scope"], "machine");
    assert_eq!(document["transaction"]["kind"], "install");
    assert!(user_state_db(&machine).exists() && machine_state_db(&machine).exists());

    // Neither run touched the other installation.
    for scope in ["user", "machine"] {
        let verify = machine.run(&installer, &["verify", "--scope", scope]);
        assert_eq!(verify.json()["status"], "ok", "{scope}: {}", verify.stdout);
    }

    let ambiguous = machine.run(&installer, &["install", "--quiet"]);
    assert_eq!(ambiguous.exit_code, Some(2), "{}", ambiguous.stdout);
    let document = ambiguous.json();
    assert_eq!(document["code"], "scope_ambiguous");
    assert_eq!(
        scopes_of(&document["existing_installations"]),
        ["user", "machine"]
    );
    for command in [["uninstall", "--quiet"], ["repair", "--quiet"]] {
        let run = machine.run(&installer, &command);
        assert_eq!(run.json()["code"], "scope_ambiguous", "{}", run.stdout);
    }
    let read = machine.run(&installer, &["inspect"]);
    assert_eq!(read.exit_code, Some(2), "{}", read.stdout);
    assert_eq!(read.json()["code"], "scope_ambiguous");

    // A named scope is unambiguous, and removing one leaves the other.
    let inspect = machine.run(&installer, &["inspect", "--scope", "machine"]);
    assert_eq!(
        scopes_of(&inspect.json()["installations"]),
        ["user", "machine"]
    );
    let removed = machine.run(&installer, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(
        removed.json()["outcome"],
        "uninstalled",
        "{}",
        removed.stdout
    );
    assert!(!user_state_db(&machine).exists());
    let remaining = machine.run(&installer, &["verify"]);
    assert_eq!(remaining.json()["status"], "ok", "{}", remaining.stdout);
    assert_eq!(remaining.json()["installation"]["scope"], "machine");
}

/// Under the `error` policy a run whose scope, named or defaulted, is not
/// the installed one is refused; naming the installed scope still works.
#[test]
fn the_error_policy_refuses_a_scope_that_is_not_the_installed_one() {
    let dir = scratch("scope-error");
    let installer = package_with_policy(&dir, "error", VERSION_A);
    let mut machine = Machine::new("scope-error");

    let first = machine.run(&installer, &["install", "--quiet", "--scope", "machine"]);
    assert_eq!(first.exit_code, Some(0), "{}", first.stdout);

    let implicit = machine.run(&installer, &["install", "--quiet"]);
    assert_eq!(implicit.exit_code, Some(2), "{}", implicit.stdout);
    assert_eq!(implicit.json()["code"], "scope_conflict");
    let explicit = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(
        explicit.json()["code"],
        "scope_conflict",
        "{}",
        explicit.stdout
    );
    assert!(!user_state_db(&machine).exists());

    // Reading never encounters anything: it follows the installation.
    let verify = machine.run(&installer, &["verify"]);
    assert_eq!(verify.json()["status"], "ok", "{}", verify.stdout);
    assert_eq!(verify.json()["installation"]["scope"], "machine");

    let named = machine.run(&installer, &["install", "--quiet", "--scope", "machine"]);
    assert_eq!(named.exit_code, Some(0), "{}", named.stdout);
    assert_eq!(named.json()["code"], "already_installed");
}

/// The policy holds inside the engine, not only in the client that resolved
/// the scope: a client that hands the engine a conflicting scope directly
/// is refused the same way. The interactive client relaunches itself with
/// an explicit `--scope` for an elevated run, so this is the path a
/// conflicting choice would otherwise slip through.
#[test]
fn the_engine_refuses_a_conflicting_scope_whatever_client_asked() {
    use tigersetup_engine::report::NullSink;
    use tigersetup_engine::{Package, RunOptions};

    let a = &fixture().a;
    let mut machine = Machine::in_scope(
        "scope-engine",
        tigersetup_engine::format::identity::Scope::Machine,
    );
    let first = machine.install(a);
    assert_eq!(first.exit_code, Some(0), "{}", first.stdout);

    // The engine in this process, pointed at the same isolated machine
    // through the folder seams the fixture sets for its children.
    let seams = [
        (
            "TIGERSETUP_TEST_REGISTRY_ROOT",
            machine.registry_prefix.clone(),
        ),
        (
            "TIGERSETUP_TEST_FOLDER_LOCALAPPDATA",
            machine.localappdata.display().to_string(),
        ),
        (
            "TIGERSETUP_TEST_FOLDER_PROGRAMDATA",
            machine.programdata.display().to_string(),
        ),
        (
            "TIGERSETUP_TEST_FOLDER_PROGRAMFILES",
            machine.programfiles.display().to_string(),
        ),
    ];
    for (name, value) in &seams {
        // SAFETY: the test binary sets its own environment before any other
        // thread of this test reads it; the seams are read at call time.
        unsafe { std::env::set_var(name, value) };
    }
    let package = Package::open(&a.installer).unwrap();
    let options = RunOptions {
        scope: tigersetup_engine::format::identity::Scope::User,
        log_path: Some(machine.scratch_path("engine-conflict.log")),
        ..RunOptions::default()
    };
    let outcome = tigersetup_engine::install(&package, &options, &mut NullSink);
    for (name, _) in &seams {
        unsafe { std::env::remove_var(name) };
    }
    assert_eq!(outcome.code, "scope_conflict", "{}", outcome.message);
    assert_eq!(outcome.exit_code, 2);
    assert_eq!(outcome.existing_installations.len(), 1);
    assert_eq!(outcome.existing_installations[0].scope, "machine");
    assert!(!user_state_db(&machine).exists());
}
