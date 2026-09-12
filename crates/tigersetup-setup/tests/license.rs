//! The licence acceptance an unattended run does not give. A package
//! manager upgrades through `install --quiet`, whatever the licence text of
//! the release; the run neither stops for the text nor records that anyone
//! agreed to it. The interactive half of the contract — the page that asks
//! once per text — is driven in `wizard.rs`.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;

struct Releases {
    /// 1.0.0 with the 2026 text.
    a: PathBuf,
    /// 1.1.0 with the same text.
    b: PathBuf,
    /// 1.2.0 with the 2027 text.
    c: PathBuf,
}

fn releases(what: &str) -> Releases {
    let dir = scratch(what);
    Releases {
        a: build_licensed_package(&dir.join("1.0.0"), "1.0.0", LICENSE_2026),
        b: build_licensed_package(&dir.join("1.1.0"), "1.1.0", LICENSE_2026),
        c: build_licensed_package(&dir.join("1.2.0"), "1.2.0", LICENSE_2027),
    }
}

fn quiet_install(machine: &mut Machine, installer: &Path) -> Value {
    let run = machine.run(installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    run.json()
}

fn installation(machine: &mut Machine, installer: &Path) -> Value {
    let run = machine.run(installer, &["inspect", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    run.json()["installation"].clone()
}

#[test]
fn a_quiet_run_never_stops_for_a_licence_and_never_accepts_one() {
    let releases = releases("quiet-licence");
    let mut machine = Machine::new("quiet-licence");

    // A first install: the package carries a licence, nobody read it, and
    // the installation says so.
    let outcome = quiet_install(&mut machine, &releases.a);
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    let installed = installation(&mut machine, &releases.a);
    assert_eq!(installed["version"], "1.0.0", "{installed}");
    assert_eq!(
        installed["accepted_license_sha256"],
        Value::Null,
        "a quiet install manufactures no acceptance: {installed}"
    );

    // The same text again, and then a changed one: both upgrades run
    // unattended to completion, and neither pretends a person agreed.
    let outcome = quiet_install(&mut machine, &releases.b);
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(outcome["transaction"]["kind"], "upgrade", "{outcome}");
    let installed = installation(&mut machine, &releases.b);
    assert_eq!(installed["version"], "1.1.0", "{installed}");
    assert_eq!(installed["accepted_license_sha256"], Value::Null);

    let outcome = quiet_install(&mut machine, &releases.c);
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(outcome["transaction"]["kind"], "upgrade", "{outcome}");
    let installed = installation(&mut machine, &releases.c);
    assert_eq!(installed["version"], "1.2.0", "{installed}");
    assert_eq!(
        installed["accepted_license_sha256"],
        Value::Null,
        "a quiet upgrade with a changed text records no acceptance: {installed}"
    );

    // A same-version quiet rerun has nothing to change, acceptance included.
    let outcome = quiet_install(&mut machine, &releases.c);
    assert_eq!(outcome["code"], "already_installed", "{outcome}");

    let run = machine.run(&releases.c, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let run = machine.run(&releases.c, &["inspect", "--scope", "user"]);
    assert_eq!(run.json()["installation"], Value::Null, "{}", run.stdout);
}
