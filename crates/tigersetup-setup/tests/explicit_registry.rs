//! A `[[registry]]` value at an explicit location outside the scope's
//! `Software` root — `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\
//! LongPathsEnabled`, the setting a real installer wants — through its whole
//! lifecycle: created where nothing was, replaced where a value was, kept
//! and repaired, rolled back with the run that wrote it, restored or deleted
//! when the product goes, preserved where somebody changed it since, and
//! left exactly as found where it already held what the package wants. The
//! scope-relative syntax keeps its meaning beside it.
//!
//! These run unelevated against relocated registry roots (the fixture's
//! `TIGERSETUP_TEST_REGISTRY_ROOT`), so `HKLM` here is the engine's HKLM:
//! the same code path the real hive takes, without touching it. The lab row
//! of `lab\Invoke-ExplicitRegistryRow.ps1` proves the real hive on a clean
//! Windows 11.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::win::registry::{Data, KeyPath, write_value};

const FILE_SYSTEM: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem";
const FRESH: &str = "HKLM\\SYSTEM\\TigerSetupTest\\Explicit";
const PRODUCT: &str = "HKLM\\Software\\IT Tiger\\TigerSetupTestApp";

/// The machine-only package: the explicit DWORD behind an option that is on
/// by default, an explicit string under a key that does not exist yet, and
/// one scope-relative value spelled the way every manifest before 0.7.1
/// spelled it.
fn manifest(version: &str, marker: &str) -> String {
    format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{version}"
publisher = "IT Tiger"

[install]
scopes = ["machine"]

[[files]]
source = "payload/**"

[[options]]
name = "long-paths"
default = true
label = {{ "en-US" = "Disable the Windows path length limit" }}

[[registry]]
root = "HKLM"
key = "SYSTEM\\CurrentControlSet\\Control\\FileSystem"
name = "LongPathsEnabled"
kind = "dword"
data = "1"
when = {{ option = "long-paths", equals = true }}

[[registry]]
root = "HKLM"
key = "SYSTEM\\TigerSetupTest\\Explicit"
name = "Marker"
kind = "string"
data = "{marker}"

[[registry]]
key = "IT Tiger\\{PRODUCT_NAME}"
name = "InstallRoot"
kind = "expand-string"
data = "%INSTALLROOT%"
"#
    )
}

struct Releases {
    a: PathBuf,
    b: PathBuf,
}

fn releases(what: &str) -> Releases {
    let dir = scratch(what);
    Releases {
        a: build_small_package(&dir.join("1.0.0"), &manifest("1.0.0", "one")),
        b: build_small_package(&dir.join("1.1.0"), &manifest("1.1.0", "two")),
    }
}

fn run_ok(machine: &mut Machine, installer: &Path, args: &[&str]) -> Value {
    let run = machine.run(installer, args);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    run.json()
}

fn codes(document: &Value) -> Vec<String> {
    document["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|f| f["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn seed(machine: &Machine, key: &str, name: &str, data: &Data) {
    let key = KeyPath::parse(key).unwrap();
    tigersetup_engine::win::registry::create_key(&machine.roots(), &key).unwrap();
    write_value(&machine.roots(), &key, name, data).unwrap();
}

/// The builder's `inspect --json` shows the explicit location as such, and
/// the scope-relative value as it always did.
#[test]
fn inspect_names_the_explicit_root() {
    let releases = releases("explicit-inspect");
    let document = tigersetup_build::inspect::inspect(&releases.a)
        .unwrap()
        .to_json();
    let values = document["registry_values"].as_array().unwrap();
    assert_eq!(values.len(), 3, "{document}");
    assert_eq!(values[0]["root"], "HKLM");
    assert_eq!(
        values[0]["key"],
        "SYSTEM\\CurrentControlSet\\Control\\FileSystem"
    );
    assert_eq!(values[0]["name"], "LongPathsEnabled");
    assert_eq!(values[0]["kind"], "dword");
    assert_eq!(values[0]["when"]["option"], "long-paths");
    assert_eq!(values[2]["root"], "software");
    assert_eq!(values[2]["key"], format!("IT Tiger\\{PRODUCT_NAME}"));
}

/// Absent → created; a prior value → replaced and, when the product goes,
/// restored; a created value deleted with the key chain TigerSetup made;
/// the scope-relative value untouched in its meaning.
#[test]
fn an_explicit_value_is_written_over_its_prior_state_and_the_prior_state_comes_back() {
    let releases = releases("explicit-cycle");
    let mut machine = Machine::in_scope("explicit-cycle", Scope::Machine);
    // Windows has the key, with the setting off, before any installer ran.
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));

    let outcome = run_ok(
        &mut machine,
        &releases.a,
        &["install", "--quiet", "--scope", "machine"],
    );
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1))
    );
    assert_eq!(
        machine.read_value(FRESH, "Marker"),
        Some(Data::String("one".into()))
    );
    assert_eq!(
        machine.read_value(PRODUCT, "InstallRoot"),
        Some(Data::ExpandString(
            machine.install_root().display().to_string()
        )),
        "the scope-relative value still lands under the scope's Software root"
    );

    // The installation owns the explicit values, and says where they are.
    let document = run_ok(
        &mut machine,
        &releases.a,
        &["inspect", "--scope", "machine"],
    );
    let owned: Vec<&str> = document["owned"]["registry_values"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        owned.contains(&format!("{FILE_SYSTEM}\\LongPathsEnabled").as_str()),
        "{document}"
    );
    assert!(
        owned.contains(&format!("{FRESH}\\Marker").as_str()),
        "{document}"
    );
    let verify = run_ok(&mut machine, &releases.a, &["verify", "--scope", "machine"]);
    assert_eq!(verify["counts"]["registry_values_checked"], 3, "{verify}");
    assert_eq!(verify["counts"]["registry_values_ok"], 3, "{verify}");

    // A same-version reinstall that reconciles (an explicit option makes it
    // one) converges without rewriting anything.
    let outcome = run_ok(
        &mut machine,
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--option",
            "long-paths",
            "on",
        ],
    );
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert!(codes(&outcome).is_empty(), "{outcome}");
    assert!(
        outcome["transaction"].is_object(),
        "the reinstall reconciled: {outcome}"
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1))
    );

    // Uninstall: the pre-existing setting goes back to what it was, the
    // created value and the keys TigerSetup created go, Windows's own stay.
    let outcome = run_ok(
        &mut machine,
        &releases.a,
        &["uninstall", "--quiet", "--scope", "machine"],
    );
    assert_eq!(outcome["outcome"], "uninstalled", "{outcome}");
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
    assert!(machine.key_exists(FILE_SYSTEM));
    assert!(!machine.key_exists(FRESH));
    assert!(!machine.key_exists("HKLM\\SYSTEM\\TigerSetupTest"));
    assert!(
        machine.key_exists("HKLM\\SYSTEM"),
        "the hive's top-level key is never TigerSetup's"
    );
    assert_eq!(machine.read_value(PRODUCT, "InstallRoot"), None);
}

/// Where nothing was there before, the value is created and, when the
/// product goes, deleted; where the package's exact value was already there
/// and was not TigerSetup's, it is left exactly as found.
#[test]
fn a_value_that_was_absent_is_deleted_and_one_that_was_already_right_is_left_alone() {
    let releases = releases("explicit-absent");
    let mut machine = Machine::in_scope("explicit-absent", Scope::Machine);
    seed(&machine, FILE_SYSTEM, "Unrelated", &Data::Dword(7));

    run_ok(
        &mut machine,
        &releases.a,
        &["install", "--quiet", "--scope", "machine"],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1))
    );
    run_ok(
        &mut machine,
        &releases.a,
        &["uninstall", "--quiet", "--scope", "machine"],
    );
    assert_eq!(machine.read_value(FILE_SYSTEM, "LongPathsEnabled"), None);
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "Unrelated"),
        Some(Data::Dword(7)),
        "a key TigerSetup did not create keeps everything else it holds"
    );

    // Already on: kept, with itself as what to put back.
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(1));
    let outcome = run_ok(
        &mut machine,
        &releases.a,
        &["install", "--quiet", "--scope", "machine"],
    );
    assert!(codes(&outcome).is_empty(), "{outcome}");
    run_ok(
        &mut machine,
        &releases.a,
        &["uninstall", "--quiet", "--scope", "machine"],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1)),
        "what was found is what is left"
    );
}

/// An administrator who turned the setting back off keeps it that way
/// through a reinstall and an upgrade, both of which say so; a repair, which
/// is asked for, puts the package's value back; an uninstall leaves their
/// value alone and says so.
#[test]
fn a_value_changed_after_installation_is_preserved_except_by_a_repair() {
    let releases = releases("explicit-modified");
    let mut machine = Machine::in_scope("explicit-modified", Scope::Machine);
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));
    run_ok(
        &mut machine,
        &releases.a,
        &["install", "--quiet", "--scope", "machine"],
    );

    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));
    let outcome = run_ok(
        &mut machine,
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--option",
            "long-paths",
            "on",
        ],
    );
    assert_eq!(
        codes(&outcome),
        vec!["registry_value_modified_preserved"],
        "{outcome}"
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
    // `verify` observes the change and exits non-zero, as any mismatch does.
    let run = machine.run(&releases.a, &["verify", "--scope", "machine"]);
    assert_ne!(run.exit_code, Some(0), "{}", run.stdout);
    let verify = run.json();
    assert_eq!(verify["status"], "failed", "{verify}");
    assert!(
        codes(&verify).contains(&"registry_value_modified".to_string()),
        "{verify}"
    );

    // The upgrade rewrites the marker whose desired data changed, and still
    // preserves the setting the administrator changed.
    let outcome = run_ok(
        &mut machine,
        &releases.b,
        &["install", "--quiet", "--scope", "machine"],
    );
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(
        codes(&outcome),
        vec!["registry_value_modified_preserved"],
        "{outcome}"
    );
    assert_eq!(
        machine.read_value(FRESH, "Marker"),
        Some(Data::String("two".into()))
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );

    let outcome = run_ok(
        &mut machine,
        &releases.b,
        &["repair", "--quiet", "--scope", "machine"],
    );
    assert_eq!(outcome["transaction"]["kind"], "repair", "{outcome}");
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1))
    );

    // Changed again, then uninstalled: theirs stays, and the run says so.
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));
    let outcome = run_ok(
        &mut machine,
        &releases.b,
        &["uninstall", "--quiet", "--scope", "machine"],
    );
    assert_eq!(outcome["outcome"], "uninstalled", "{outcome}");
    assert!(
        codes(&outcome).contains(&"registry_value_modified_preserved".to_string()),
        "{outcome}"
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
    assert!(!machine.key_exists(FRESH));
}

/// The option off: the value is not written, and an installation that had
/// it on gives it back when the option is turned off.
#[test]
fn the_predicate_gates_the_explicit_value() {
    let releases = releases("explicit-predicate");
    let mut machine = Machine::in_scope("explicit-predicate", Scope::Machine);
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));

    run_ok(
        &mut machine,
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--option",
            "long-paths",
            "off",
        ],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
    assert_eq!(
        machine.read_value(FRESH, "Marker"),
        Some(Data::String("one".into()))
    );

    run_ok(
        &mut machine,
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--option",
            "long-paths",
            "on",
        ],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(1))
    );
    run_ok(
        &mut machine,
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--option",
            "long-paths",
            "off",
        ],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0)),
        "turning the option off restores the pre-installation state"
    );
    run_ok(
        &mut machine,
        &releases.a,
        &["uninstall", "--quiet", "--scope", "machine"],
    );
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
}

/// A run that fails after the explicit value was written rolls it back to
/// what was there: the prior value, or nothing.
#[test]
fn a_rolled_back_install_puts_the_prior_state_back() {
    let releases = releases("explicit-rollback");
    let mut machine = Machine::in_scope("explicit-rollback", Scope::Machine);
    seed(&machine, FILE_SYSTEM, "LongPathsEnabled", &Data::Dword(0));

    let run = machine.run(
        &releases.a,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--fault",
            "before_commit:fail",
        ],
    );
    assert_ne!(run.exit_code, Some(0), "{}", run.stdout);
    assert_eq!(run.json()["outcome"], "rolled_back", "{}", run.stdout);
    assert_eq!(
        machine.read_value(FILE_SYSTEM, "LongPathsEnabled"),
        Some(Data::Dword(0))
    );
    assert_eq!(machine.read_value(FRESH, "Marker"), None);
    assert!(!machine.key_exists("HKLM\\SYSTEM\\TigerSetupTest"));
    assert!(machine.key_exists(FILE_SYSTEM));
}

/// The explicit hive must be the one the package's only scope writes: the
/// builder refuses a dual-scope package, and HKCU needs a user-only one.
#[test]
fn the_builder_refuses_an_explicit_root_the_scopes_cannot_write() {
    let dir = scratch("explicit-refused");
    let dual = manifest("1.0.0", "x")
        .replace("scopes = [\"machine\"]", "scopes = [\"user\", \"machine\"]");
    std::fs::create_dir_all(dir.join("payload")).unwrap();
    std::fs::write(dir.join("payload").join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("TigerSetup.toml"), dual).unwrap();
    let request = tigersetup_build::BuildRequest {
        manifest_path: &dir.join("TigerSetup.toml"),
        output: &dir.join("out"),
        engine_path: Some(Path::new(ENGINE)),
        loader_path: Some(Path::new(LOADER)),
        properties: &[],
        offline: true,
        compression: tigersetup_build::Compression::Fast,
    };
    let err = tigersetup_build::build(&request).unwrap_err();
    assert_eq!(err.code, "manifest_invalid");
    assert!(
        err.message.contains("HKLM") && err.message.contains("user scope"),
        "{}",
        err.message
    );
}
