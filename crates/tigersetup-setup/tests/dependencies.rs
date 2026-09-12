//! Dependencies end to end, through the built `Setup.exe`: detection first,
//! acquisition only when something is missing, verification of every
//! download, unattended installation, and re-detection before the product
//! transaction opens.
//!
//! A dependency is a requirement, not an owned resource: it survives a
//! rolled-back product install and an uninstall. Nothing here knows what any
//! real runtime is — the fixture declares a generic vendor requirement whose
//! "installer" is a copy of `cmd.exe` that creates the version directory the
//! `directory-version` detector then sees.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use common::{ENGINE, Machine, PRODUCT_ID, PRODUCT_NAME, TMP, VERSION_A, sha256_hex};
use serde_json::Value;
use tigersetup_build::{BuildRequest, build};

/// A local HTTP server that answers every GET with the same body and records
/// the paths it was asked for, so a test can prove that no request was made.
struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    fn start(body: Vec<u8>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            let body = Arc::new(body);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            answer(stream, &requests, &body);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Server {
            base,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn answer(stream: TcpStream, requests: &Mutex<Vec<String>>, body: &[u8]) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) if header == "\r\n" => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }
    requests.lock().unwrap().push(
        request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string(),
    );
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// A port nobody listens on.
fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn cmd_exe() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe")
}

/// One `[[dependencies]]` entry of a fixture package.
struct Declared {
    id: &'static str,
    /// Directory under `%LOCALAPPDATA%` whose version subdirectories the
    /// `directory-version` detector reads.
    marker: &'static str,
    url: String,
    sha256: String,
    arguments: Vec<String>,
    reboot_codes: Vec<i32>,
    elevation_required: bool,
}

impl Declared {
    /// A dependency whose installer creates the version directory the
    /// detector is looking for.
    fn satisfying(
        id: &'static str,
        marker: &'static str,
        server: &Server,
        sha256: &str,
    ) -> Declared {
        Declared {
            id,
            marker,
            url: server.url(&format!("/dep/{marker}-setup.exe")),
            sha256: sha256.to_string(),
            arguments: vec![
                "/c".into(),
                "mkdir".into(),
                format!("%LOCALAPPDATA%\\{marker}\\1.0"),
            ],
            reboot_codes: Vec::new(),
            elevation_required: false,
        }
    }

    fn with_arguments(mut self, arguments: &[&str]) -> Declared {
        self.arguments = arguments.iter().map(|a| (*a).to_string()).collect();
        self
    }

    fn to_toml(&self) -> String {
        let arguments = self
            .arguments
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let reboot = self
            .reboot_codes
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "\n[[dependencies]]\nid = {:?}\nname = {:?}\nminimum = \"1.0\"\n\
             elevation = {}\n\
             detect = {{ kind = \"directory-version\", path = {:?} }}\n\
             acquire = {{ url = {:?}, sha256 = {:?} }}\n\
             install = {{ arguments = [{arguments}], reboot_codes = [{reboot}] }}\n",
            self.id,
            format!("{} Runtime", self.id),
            self.elevation_required,
            format!("%LOCALAPPDATA%\\{}", self.marker),
            self.url,
            self.sha256,
        )
    }
}

/// Builds a one-file package declaring `dependencies`, with the freshly
/// compiled engine. `name` keeps concurrent tests in separate directories.
fn build_installer(name: &str, dependencies: &[Declared]) -> PathBuf {
    let dir = Path::new(TMP).join(format!("dependency-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let payload = dir.join("payload").join("bin");
    std::fs::create_dir_all(&payload).unwrap();
    std::fs::write(payload.join("app.txt"), name.as_bytes()).unwrap();
    let mut manifest = format!(
        "[package]\nid = \"{PRODUCT_ID}\"\nname = \"{PRODUCT_NAME}\"\n\
         version = \"{VERSION_A}\"\npublisher = \"IT Tiger\"\n\n\
         [install]\nscopes = [\"user\"]\n\n[[files]]\nsource = \"payload/**\"\n"
    );
    for dependency in dependencies {
        manifest.push_str(&dependency.to_toml());
    }
    let manifest_path = dir.join("TigerSetup.toml");
    std::fs::write(&manifest_path, manifest).unwrap();
    let output = dir.join("Setup.exe");
    let request = BuildRequest {
        manifest_path: &manifest_path,
        output: &output,
        engine_path: Some(Path::new(ENGINE)),
        properties: &[],
        offline: true,
        // Tests build many installers; the payload's size is not what they
        // measure.
        compression: tigersetup_build::Compression::Fast,
    };
    build(&request)
        .expect("the dependency fixture builds")
        .installer_path
}

fn install(machine: &mut Machine, installer: &Path, extra: &[&str]) -> common::Run {
    let mut args = vec!["install", "--quiet", "--scope", "user"];
    args.extend_from_slice(extra);
    machine.run(installer, &args)
}

fn marker_dir(machine: &Machine, marker: &str) -> PathBuf {
    machine.localappdata.join(marker).join("1.0")
}

/// `(dependency_id, action, version)` of every recorded dependency event.
fn recorded_events(machine: &Machine) -> Vec<(String, String, String)> {
    let connection = rusqlite::Connection::open_with_flags(
        machine.state_dir().join("state.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the state database is readable after the run");
    let mut statement = connection
        .prepare("SELECT dependency_id, action, version FROM dependency_event ORDER BY id")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn dependencies_of(outcome: &Value) -> &Vec<Value> {
    outcome["dependencies"]
        .as_array()
        .unwrap_or_else(|| panic!("the outcome carries no dependencies: {outcome}"))
}

#[test]
fn a_satisfied_dependency_never_opens_a_connection() {
    let server = Server::start(std::fs::read(cmd_exe()).unwrap());
    let sha256 = sha256_hex(&std::fs::read(cmd_exe()).unwrap());
    let dependency = Declared::satisfying("Vendor.Present", "present-marker", &server, &sha256);
    let installer = build_installer("present", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-present");
    // The requirement is already met on this machine.
    std::fs::create_dir_all(marker_dir(&machine, "present-marker")).unwrap();

    let inspected = machine
        .run(&installer, &["inspect", "--scope", "user"])
        .json();
    let declared = &inspected["dependencies"][0];
    assert_eq!(declared["id"], "Vendor.Present");
    assert_eq!(declared["status"], "present");
    assert_eq!(declared["version"], "1.0");
    assert_eq!(declared["minimum_version"], "1.0");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed", "{}", run.stdout);
    assert_eq!(run.exit_code, Some(0));
    assert_eq!(outcome["reboot_required"], false);
    let recorded = &dependencies_of(&outcome)[0];
    assert_eq!(recorded["status"], "present");
    assert_eq!(recorded["action"], "detected");
    assert_eq!(recorded["version"], "1.0");
    assert!(recorded["url"].is_null(), "nothing was acquired");

    assert!(
        server.requests().is_empty(),
        "detection comes first, so a satisfied machine makes no request: {:?}",
        server.requests()
    );
    assert_eq!(
        recorded_events(&machine),
        vec![(
            "Vendor.Present".to_string(),
            "detected".to_string(),
            "1.0".to_string()
        )]
    );
    assert!(!machine.state_dir().join("deps").exists());
}

#[test]
fn a_missing_dependency_is_downloaded_verified_installed_and_recorded() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let dependency = Declared::satisfying("Vendor.Acquired", "acquired-marker", &server, &sha256);
    let installer = build_installer("acquired", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-acquired");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed", "{}", run.stdout);
    assert_eq!(run.exit_code, Some(0));
    let recorded = &dependencies_of(&outcome)[0];
    assert_eq!(recorded["status"], "installed");
    assert_eq!(recorded["action"], "installed");
    assert_eq!(recorded["version"], "1.0");
    assert_eq!(recorded["sha256"], sha256);
    assert_eq!(
        recorded["url"],
        server.url("/dep/acquired-marker-setup.exe")
    );
    assert_eq!(recorded["exit_code"], 0);

    assert_eq!(
        server.requests(),
        vec!["/dep/acquired-marker-setup.exe".to_string()],
        "exactly one download"
    );
    assert!(marker_dir(&machine, "acquired-marker").is_dir());
    assert!(run.log_has("[dependency_downloaded]"));
    assert!(run.log_has("[dependency_installing]"));
    assert!(run.log_has("[dependency_verified]"));

    let events = recorded_events(&machine);
    assert_eq!(
        events
            .iter()
            .map(|(id, action, _)| (id.as_str(), action.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Vendor.Acquired", "acquired"),
            ("Vendor.Acquired", "installed")
        ]
    );

    assert!(
        !machine.state_dir().join("deps").exists(),
        "the download staging area is removed"
    );
    let verified = machine
        .run(&installer, &["verify", "--scope", "user"])
        .json();
    assert_eq!(verified["status"], "ok");
}

#[test]
fn opting_out_of_dependency_installation_leaves_the_machine_untouched() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let dependency = Declared::satisfying("Vendor.OptOut", "optout-marker", &server, &sha256);
    let installer = build_installer("optout", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-optout");

    let run = install(&mut machine, &installer, &["--no-dependency-install"]);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "failed", "{}", run.stdout);
    assert_eq!(outcome["code"], "dependency_missing");
    assert_eq!(outcome["dependency"], "Vendor.OptOut");
    assert_eq!(run.exit_code, Some(3));
    assert_eq!(dependencies_of(&outcome)[0]["status"], "missing");

    assert!(server.requests().is_empty(), "nothing was acquired");
    assert!(
        !machine.state_dir().exists(),
        "a run stopped in the dependency phase opens no state database"
    );
    assert!(!machine.install_root().exists());
}

#[test]
fn a_download_whose_hash_differs_is_refused() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let server = Server::start(body);
    let dependency = Declared::satisfying(
        "Vendor.BadHash",
        "badhash-marker",
        &server,
        &sha256_hex(b"the bytes the package expected"),
    );
    let installer = build_installer("badhash", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-badhash");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["code"], "dependency_unacquirable", "{}", run.stdout);
    assert_eq!(outcome["reason"], "hash_mismatch");
    assert_eq!(run.exit_code, Some(3));
    assert_eq!(dependencies_of(&outcome)[0]["reason"], "hash_mismatch");

    assert_eq!(server.requests().len(), 1);
    assert!(
        !marker_dir(&machine, "badhash-marker").exists(),
        "an unverified download is never run"
    );
    assert!(!machine.install_root().exists());
    assert!(!machine.state_dir().join("deps").exists());
}

#[test]
fn a_host_that_does_not_answer_is_reported_as_no_network() {
    let dependency = Declared {
        id: "Vendor.Offline",
        marker: "offline-marker",
        url: format!("http://127.0.0.1:{}/dep/offline-setup.exe", dead_port()),
        sha256: sha256_hex(b"anything"),
        arguments: vec!["/c".into(), "exit".into(), "0".into()],
        reboot_codes: Vec::new(),
        elevation_required: false,
    };
    let installer = build_installer("offline", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-offline");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["code"], "dependency_unacquirable", "{}", run.stdout);
    assert_eq!(outcome["reason"], "network_unavailable");
    assert_eq!(run.exit_code, Some(3));
    assert!(!machine.install_root().exists());
}

#[test]
fn a_dependency_installer_that_fails_stops_the_run_with_its_exit_code() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let dependency = Declared::satisfying("Vendor.Failing", "failing-marker", &server, &sha256)
        .with_arguments(&["/c", "exit", "1"]);
    let installer = build_installer("failing", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-failing");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(
        outcome["code"], "dependency_install_failed",
        "{}",
        run.stdout
    );
    assert_eq!(run.exit_code, Some(3));
    let recorded = &dependencies_of(&outcome)[0];
    assert_eq!(recorded["status"], "install_failed");
    assert_eq!(recorded["exit_code"], 1);
    assert!(!machine.install_root().exists());
}

#[test]
fn an_installer_that_reports_success_but_stays_undetected_is_unverified() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let dependency = Declared::satisfying("Vendor.Silent", "silent-marker", &server, &sha256)
        .with_arguments(&["/c", "exit", "0"]);
    let installer = build_installer("silent", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-silent");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["code"], "dependency_unverified", "{}", run.stdout);
    assert_eq!(run.exit_code, Some(3));
    assert_eq!(dependencies_of(&outcome)[0]["status"], "unverified");
    assert!(!marker_dir(&machine, "silent-marker").exists());
    assert!(!machine.install_root().exists());
}

#[test]
fn a_dependency_that_asks_for_a_reboot_still_installs_the_product() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let mut dependency = Declared::satisfying("Vendor.Reboot", "reboot-marker", &server, &sha256);
    dependency.arguments = vec![
        "/c".into(),
        "mkdir".into(),
        "%LOCALAPPDATA%\\reboot-marker\\1.0".into(),
        "&".into(),
        "exit".into(),
        "3010".into(),
    ];
    dependency.reboot_codes = vec![3010];
    let installer = build_installer("reboot", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-reboot");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed", "{}", run.stdout);
    assert_eq!(outcome["reboot_required"], true);
    assert_eq!(run.exit_code, Some(3010));
    let recorded = &dependencies_of(&outcome)[0];
    assert_eq!(recorded["status"], "installed");
    assert_eq!(recorded["action"], "reboot_required");
    assert_eq!(recorded["exit_code"], 3010);
    assert!(marker_dir(&machine, "reboot-marker").is_dir());
    assert!(
        recorded_events(&machine)
            .iter()
            .any(|(_, action, _)| action == "reboot_required")
    );
    let verified = machine
        .run(&installer, &["verify", "--scope", "user"])
        .json();
    assert_eq!(verified["status"], "ok");
}

#[test]
fn an_unattended_run_refuses_a_dependency_that_needs_elevation() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let mut dependency =
        Declared::satisfying("Vendor.Elevated", "elevated-marker", &server, &sha256);
    dependency.elevation_required = true;
    let installer = build_installer("elevated", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-elevated");

    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    if tigersetup_engine::win::process::is_elevated() {
        // An already elevated process has nothing to ask for.
        assert_eq!(outcome["outcome"], "installed", "{}", run.stdout);
        return;
    }
    assert_eq!(
        outcome["code"], "dependency_requires_elevation",
        "{}",
        run.stdout
    );
    assert_eq!(run.exit_code, Some(4));
    assert_eq!(dependencies_of(&outcome)[0]["status"], "requires_elevation");
    assert!(
        server.requests().is_empty(),
        "an unattended run fails before acquiring anything it cannot install"
    );
    assert!(!machine.install_root().exists());
}

#[test]
fn a_dependency_outlives_a_rolled_back_install_and_an_uninstall() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let dependency = Declared::satisfying("Vendor.Kept", "kept-marker", &server, &sha256);
    let installer = build_installer("kept", std::slice::from_ref(&dependency));
    let mut machine = Machine::new("dependency-kept");
    let marker = marker_dir(&machine, "kept-marker");

    // A product transaction that rolls back leaves the requirement satisfied.
    let rolled_back = install(&mut machine, &installer, &["--fault", "before_commit:fail"]);
    assert_eq!(
        rolled_back.json()["outcome"],
        "rolled_back",
        "{}",
        rolled_back.stdout
    );
    assert!(
        marker.is_dir(),
        "the dependency is not part of the transaction"
    );
    assert!(server.requests().len() == 1);

    // The second run detects it and does not acquire it again.
    let installed = install(&mut machine, &installer, &[]);
    assert_eq!(
        installed.json()["outcome"],
        "installed",
        "{}",
        installed.stdout
    );
    assert_eq!(dependencies_of(&installed.json())[0]["status"], "present");
    assert_eq!(server.requests().len(), 1, "no second download");

    let removed = machine.run(&installer, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(
        removed.json()["outcome"],
        "uninstalled",
        "{}",
        removed.stdout
    );
    assert!(
        marker.is_dir(),
        "uninstalling the product never removes a dependency it required"
    );
}

#[test]
fn a_fault_before_a_dependency_install_names_the_dependency_by_declaration_order() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let installer = build_installer(
        "fault-before",
        &[
            Declared::satisfying("Vendor.First", "first-before-marker", &server, &sha256),
            Declared::satisfying("Vendor.Second", "second-before-marker", &server, &sha256),
        ],
    );
    let mut machine = Machine::new("dependency-fault-before");

    let run = install(
        &mut machine,
        &installer,
        &["--fault", "before_dependency_install@2:fail"],
    );
    let outcome = run.json();
    assert_eq!(
        outcome["code"], "dependency_install_failed",
        "{}",
        run.stdout
    );
    assert_eq!(outcome["dependency"], "Vendor.Second");
    assert_eq!(run.exit_code, Some(3));
    assert!(run.log_has(
        "[fault_injected] point=before_dependency_install sequence=2 action=fail target=Vendor.Second"
    ));
    assert!(
        marker_dir(&machine, "first-before-marker").is_dir(),
        "the first dependency was installed before the fault"
    );
    assert!(
        !marker_dir(&machine, "second-before-marker").exists(),
        "the second dependency's installer never ran"
    );
    assert!(!machine.install_root().exists());
}

#[test]
fn a_fault_after_a_dependency_install_leaves_the_dependency_installed() {
    let body = std::fs::read(cmd_exe()).unwrap();
    let sha256 = sha256_hex(&body);
    let server = Server::start(body);
    let installer = build_installer(
        "fault-after",
        &[
            Declared::satisfying("Vendor.First", "first-after-marker", &server, &sha256),
            Declared::satisfying("Vendor.Second", "second-after-marker", &server, &sha256),
        ],
    );
    let mut machine = Machine::new("dependency-fault-after");

    let run = install(
        &mut machine,
        &installer,
        &["--fault", "after_dependency_install@2:fail"],
    );
    let outcome = run.json();
    assert_eq!(outcome["code"], "dependency_unverified", "{}", run.stdout);
    assert_eq!(outcome["dependency"], "Vendor.Second");
    assert_eq!(run.exit_code, Some(3));
    assert!(run.log_has(
        "[fault_injected] point=after_dependency_install sequence=2 action=fail target=Vendor.Second"
    ));
    assert!(
        marker_dir(&machine, "second-after-marker").is_dir(),
        "the installer had already run when the fault fired"
    );
    assert!(!machine.install_root().exists());

    // Without the fault the same run completes and detects both.
    let run = install(&mut machine, &installer, &[]);
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed", "{}", run.stdout);
    assert_eq!(dependencies_of(&outcome).len(), 2);
    assert!(
        dependencies_of(&outcome)
            .iter()
            .all(|d| d["status"] == "present")
    );
}
