//! The dependency phase (`TigerSetup-Design.md` §7): for every declared
//! dependency, detect → present → continue; absent → acquire, install
//! unattended, re-detect. It runs before the product transaction and
//! outside it: a failure here leaves no product change, and a dependency
//! installed here is a requirement satisfied, never a resource owned
//! (§7.1) — it stays when the product rolls back or is uninstalled.
//!
//! Detection comes first, so a machine whose dependencies are present never
//! opens a network connection — nor extracts an embedded installer.
//!
//! A dependency may carry a predicate like any other optional resource: one
//! the effective options disable is not a requirement of this run.

pub mod acquire;
pub mod detect;
pub mod pattern;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tigersetup_catalog::Reason;
use tigersetup_catalog::manifest::is_msi_type;
use tigersetup_format::Payload;
use tigersetup_format::metadata::{AcquisitionSource, Dependency};

use crate::report::{
    DependencyInfo, DependencyStatus, EmbeddedInstallerInfo, Outcome, Phase, Progress, Reporter,
    exit,
};
use crate::resource::predicate::{self, Options};
use crate::state::dependency::DependencyEvent;
use crate::txn::fault::{FaultInjector, FaultPoint};
use crate::win::process::{self, LaunchError};
use crate::{Package, Result, Roots, RunOptions, i18n};

use detect::Detection;

/// Name of the directory under the state directory that holds downloads
/// for the duration of the phase.
pub const DOWNLOAD_DIR: &str = "deps";

/// Why the phase stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// `dependency_missing`, `dependency_unacquirable`,
    /// `dependency_install_failed`, `dependency_unverified`,
    /// `dependency_requires_elevation` or `dependency_cancelled`.
    pub code: &'static str,
    pub dependency: String,
    pub display_name: String,
    /// The acquisition reason beside `dependency_unacquirable`.
    pub reason: Option<&'static str>,
    pub message: String,
}

/// What the phase did.
#[derive(Debug, Clone, Default)]
pub struct PhaseOutcome {
    pub records: Vec<DependencyInfo>,
    pub events: Vec<DependencyEvent>,
    pub reboot_required: bool,
    pub failure: Option<Failure>,
}

fn display_name_of(dependency: &Dependency) -> &str {
    if dependency.display_name.is_empty() {
        &dependency.id
    } else {
        &dependency.display_name
    }
}

fn record(dependency: &Dependency, status: &'static str, action: &'static str) -> DependencyInfo {
    DependencyInfo {
        id: dependency.id.clone(),
        name: display_name_of(dependency).to_string(),
        status,
        action,
        version: None,
        url: None,
        sha256: None,
        exit_code: None,
        reason: None,
        code: None,
    }
}

fn event(dependency: &Dependency, action: &'static str, version: &str) -> DependencyEvent {
    DependencyEvent {
        dependency_id: dependency.id.clone(),
        version: version.to_string(),
        action: action.to_string(),
        url: None,
        sha256: None,
        recorded_at: crate::report::now_rfc3339(),
    }
}

/// The stable name of a dependency's acquisition source.
pub fn source_name(dependency: &Dependency) -> &'static str {
    match dependency
        .acquisition
        .as_ref()
        .and_then(|a| AcquisitionSource::try_from(a.source).ok())
    {
        Some(AcquisitionSource::Winget) => "winget",
        Some(AcquisitionSource::Url) => "url",
        Some(AcquisitionSource::Embedded) => "embedded",
        _ => "none",
    }
}

/// The current detection result of every declared dependency, for
/// `inspect`. Reads the machine only.
pub fn statuses(package: &Package) -> Result<Vec<DependencyStatus>> {
    package
        .metadata()
        .dependencies
        .iter()
        .map(|dependency| {
            let detector = dependency.detect.clone().unwrap_or_default();
            let detection = detect::detect(&detector, &dependency.minimum_version)?;
            let embedded = dependency
                .acquisition
                .as_ref()
                .filter(|a| a.source == AcquisitionSource::Embedded as i32)
                .map(|a| EmbeddedInstallerInfo {
                    entry: a.entry.clone(),
                    sha256: a.sha256.clone(),
                    size: a.size,
                });
            Ok(DependencyStatus {
                id: dependency.id.clone(),
                name: display_name_of(dependency).to_string(),
                minimum_version: dependency.minimum_version.clone(),
                status: match detection {
                    Detection::Present { .. } => "present",
                    Detection::Absent { .. } => "absent",
                },
                version: detection.version().map(str::to_string),
                source: source_name(dependency),
                embedded,
            })
        })
        .collect()
}

/// The program and arguments that run an acquired installer unattended.
fn launch_of(acquired: &acquire::Acquired) -> (PathBuf, Vec<String>) {
    if is_msi_type(&acquired.installer_type) {
        let msiexec = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("System32")
            .join("msiexec.exe");
        let mut arguments = vec![
            "/i".to_string(),
            acquired.path.display().to_string(),
            "/qn".to_string(),
            "/norestart".to_string(),
        ];
        arguments.extend(acquired.arguments.iter().cloned());
        (msiexec, arguments)
    } else {
        (acquired.path.clone(), acquired.arguments.clone())
    }
}

/// One dependency's step in the phase; `Err` is the stable failure.
#[allow(clippy::too_many_arguments)]
fn one(
    sequence: i64,
    dependency: &Dependency,
    options: &RunOptions,
    payload: &mut Option<Payload>,
    deps_dir: &Path,
    elevated: bool,
    fault: &mut FaultInjector,
    reporter: &mut Reporter<'_>,
    outcome: &mut PhaseOutcome,
) -> std::result::Result<(), Failure> {
    let name = display_name_of(dependency);
    let detector = dependency.detect.clone().unwrap_or_default();
    let minimum = dependency.minimum_version.as_str();
    let fail = |code: &'static str, reason: Option<&'static str>, message: String| Failure {
        code,
        dependency: dependency.id.clone(),
        display_name: name.to_string(),
        reason,
        message,
    };
    let engine_error = |err: crate::Error| fail("dependency_unverified", None, err.message);

    let progress = |target: &str| Progress {
        phase: Phase::Dependencies,
        done: 0,
        total: 0,
        target: target.to_string(),
    };

    match detect::detect(&detector, minimum).map_err(engine_error)? {
        Detection::Present { version } => {
            reporter.progress(
                "dependency_detected",
                format!(
                    "{}: version {version} satisfies minimum {minimum:?}",
                    dependency.id
                ),
                progress(name),
            );
            let mut info = record(dependency, "present", "detected");
            info.version = Some(version.clone());
            outcome.records.push(info);
            outcome.events.push(event(dependency, "detected", &version));
            return Ok(());
        }
        Detection::Absent { found } => {
            reporter.progress(
                "dependency_missing",
                match &found {
                    Some(found) => format!(
                        "{}: found {found}, which does not satisfy minimum {minimum:?}",
                        dependency.id
                    ),
                    None => format!("{}: not detected", dependency.id),
                },
                progress(name),
            );
        }
    }

    if !options.install_dependencies {
        return Err(fail(
            "dependency_missing",
            None,
            "dependency installation is disabled".into(),
        ));
    }
    let cancelled = || {
        options
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    };
    let elevation_blocked = |required: bool| required && !elevated && options.quiet;
    if elevation_blocked(dependency.elevation_required) {
        return Err(fail(
            "dependency_requires_elevation",
            None,
            "the dependency's installer needs an elevated process".into(),
        ));
    }

    std::fs::create_dir_all(deps_dir).map_err(|err| {
        fail(
            "dependency_unacquirable",
            Some(Reason::DownloadFailed.code()),
            format!("cannot create {}: {err}", deps_dir.display()),
        )
    })?;
    let acquired = match acquire::acquire(
        dependency,
        options.scope,
        deps_dir,
        payload.as_mut(),
        reporter,
        options.cancel.as_deref(),
    ) {
        Ok(acquired) => acquired,
        Err(err) if err.reason == Reason::Cancelled => {
            return Err(fail("dependency_cancelled", None, err.message));
        }
        Err(err) => {
            return Err(fail(
                "dependency_unacquirable",
                Some(err.reason.code()),
                err.message,
            ));
        }
    };
    if elevation_blocked(acquired.elevation_required) {
        return Err(fail(
            "dependency_requires_elevation",
            None,
            "the dependency's installer needs an elevated process".into(),
        ));
    }
    if cancelled() {
        return Err(fail("dependency_cancelled", None, "cancelled".into()));
    }

    fault
        .at(
            FaultPoint::BeforeDependencyInstall,
            Some(sequence),
            &dependency.id,
            reporter,
        )
        .map_err(|err| fail("dependency_install_failed", None, err.message))?;

    let (program, arguments) = launch_of(&acquired);
    let elevate = acquired.elevation_required && !elevated;
    reporter.progress(
        "dependency_installing",
        format!(
            "{}: {} {}{}",
            dependency.id,
            process::command_line(&program, &arguments),
            if elevate { "(elevated) " } else { "" },
            if acquired.refreshed {
                "[refreshed]"
            } else {
                "[hint]"
            }
        ),
        progress(name),
    );
    let exit_code = match if elevate {
        process::run_elevated(&program, &arguments, process::Show::Hidden)
    } else {
        process::run_hidden(&program, &arguments)
    } {
        Ok(code) => code,
        Err(LaunchError::Refused) => {
            return Err(fail(
                "dependency_cancelled",
                None,
                "the elevation prompt was refused".into(),
            ));
        }
        Err(LaunchError::Failed(message)) => {
            return Err(fail("dependency_install_failed", None, message));
        }
    };
    let reboot = acquired.reboot_codes.contains(&exit_code);
    let success = exit_code == 0 || acquired.success_codes.contains(&exit_code) || reboot;
    reporter.progress(
        if success {
            "dependency_installed"
        } else {
            "dependency_install_failed"
        },
        format!(
            "{}: installer exit code {exit_code}{}",
            dependency.id,
            if reboot { " (reboot required)" } else { "" }
        ),
        progress(name),
    );
    // A dependency is a requirement, not an owned resource, and detection is
    // what says whether the requirement is met. A vendor's installer may
    // report an error and still have done the job — Microsoft's WebView2
    // evergreen bootstrapper does exactly that on Windows 10 — so an exit
    // code that says failure is checked against the machine before it is
    // believed.
    let installed_anyway = !success
        && matches!(
            detect::detect(&detector, minimum),
            Ok(Detection::Present { .. })
        );
    if installed_anyway {
        reporter.event(
            "dependency_installed_despite_exit_code",
            format!(
                "{}: installer exit code {exit_code}, but it is present",
                dependency.id
            ),
        );
    }
    if !success && !installed_anyway {
        let mut failure = fail(
            "dependency_install_failed",
            None,
            format!("the installer exited with code {exit_code}"),
        );
        failure.message = format!("{} (url {})", failure.message, acquired.url);
        let mut info = record(dependency, "install_failed", "none");
        info.exit_code = Some(exit_code);
        info.url = Some(acquired.url.clone());
        info.sha256 = Some(acquired.sha256.clone());
        info.code = Some("dependency_install_failed");
        outcome.records.push(info);
        return Err(failure);
    }

    fault
        .at(
            FaultPoint::AfterDependencyInstall,
            Some(sequence),
            &dependency.id,
            reporter,
        )
        .map_err(|err| fail("dependency_unverified", None, err.message))?;

    let version = match detect::detect(&detector, minimum).map_err(engine_error)? {
        Detection::Present { version } => version,
        Detection::Absent { found } => {
            let mut info = record(dependency, "unverified", "none");
            info.exit_code = Some(exit_code);
            info.url = Some(acquired.url.clone());
            info.sha256 = Some(acquired.sha256.clone());
            info.code = Some("dependency_unverified");
            outcome.records.push(info);
            return Err(fail(
                "dependency_unverified",
                None,
                match found {
                    Some(found) => format!(
                        "the installer exited with code {exit_code} but only {found} is detected"
                    ),
                    None => format!(
                        "the installer exited with code {exit_code} but the dependency is not detected"
                    ),
                },
            ));
        }
    };
    reporter.progress(
        "dependency_verified",
        format!(
            "{}: version {version} detected after installation",
            dependency.id
        ),
        progress(name),
    );
    let action = if reboot {
        "reboot_required"
    } else {
        "installed"
    };
    let mut info = record(dependency, "installed", action);
    info.version = Some(version.clone());
    info.url = Some(acquired.url.clone());
    info.sha256 = Some(acquired.sha256.clone());
    info.exit_code = Some(exit_code);
    outcome.records.push(info);
    let mut acquired_event = event(dependency, "acquired", &acquired.version);
    acquired_event.url = Some(acquired.url.clone());
    acquired_event.sha256 = Some(acquired.sha256.clone());
    outcome.events.push(acquired_event);
    outcome
        .events
        .push(event(dependency, "installed", &version));
    if reboot {
        outcome.reboot_required = true;
        outcome
            .events
            .push(event(dependency, "reboot_required", &version));
    }
    Ok(())
}

/// Runs the phase for every declared dependency the effective options
/// require, in declaration order, stopping at the first failure. Downloads
/// and extracted installers live under `<state directory>\deps` for the
/// duration and are removed afterwards whatever the result.
pub fn run(
    package: &Package,
    options: &RunOptions,
    effective: &Options,
    roots: &Roots,
    fault: &mut FaultInjector,
    reporter: &mut Reporter<'_>,
) -> PhaseOutcome {
    let mut outcome = PhaseOutcome::default();
    let dependencies = &package.metadata().dependencies;
    if dependencies.is_empty() {
        return outcome;
    }
    let deps_dir = roots.state_dir.join(DOWNLOAD_DIR);
    let elevated = process::is_elevated();
    // The payload is opened only when an embedded installer is needed, and
    // once; a machine whose dependencies are present never reads it here.
    let mut payload: Option<Payload> = None;
    for (index, dependency) in dependencies.iter().enumerate() {
        if !predicate::enabled(dependency.when.as_ref(), "", effective) {
            reporter.event(
                "dependency_not_required",
                format!(
                    "{}: disabled by option {}",
                    dependency.id,
                    dependency
                        .when
                        .as_ref()
                        .map(|w| w.describe())
                        .unwrap_or_default()
                ),
            );
            outcome
                .records
                .push(record(dependency, "not_required", "none"));
            continue;
        }
        let needs_payload = dependency
            .acquisition
            .as_ref()
            .is_some_and(|a| a.source == AcquisitionSource::Embedded as i32);
        if needs_payload && payload.is_none() {
            match package.installer().payload() {
                Ok(archive) => payload = Some(archive),
                Err(err) => {
                    let failure = Failure {
                        code: "dependency_unacquirable",
                        dependency: dependency.id.clone(),
                        display_name: display_name_of(dependency).to_string(),
                        reason: Some(Reason::DownloadFailed.code()),
                        message: format!("this installer's payload cannot be opened: {err}"),
                    };
                    let mut info = record(dependency, "unacquirable", "none");
                    info.reason = failure.reason;
                    info.code = Some(failure.code);
                    outcome.records.push(info);
                    outcome.failure = Some(failure);
                    break;
                }
            }
        }
        let result = one(
            index as i64 + 1,
            dependency,
            options,
            &mut payload,
            &deps_dir,
            elevated,
            fault,
            reporter,
            &mut outcome,
        );
        if let Err(failure) = result {
            reporter.event(
                "dependency_failed",
                format!(
                    "{}: {}{}: {}",
                    failure.dependency,
                    failure.code,
                    failure
                        .reason
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default(),
                    failure.message
                ),
            );
            if !outcome.records.iter().any(|r| r.id == failure.dependency) {
                let status = match failure.code {
                    "dependency_missing" => "missing",
                    "dependency_unacquirable" => "unacquirable",
                    "dependency_requires_elevation" => "requires_elevation",
                    "dependency_cancelled" => "cancelled",
                    "dependency_install_failed" => "install_failed",
                    _ => "unverified",
                };
                let mut info = record(dependency, status, "none");
                info.reason = failure.reason;
                info.code = Some(failure.code);
                outcome.records.push(info);
            }
            outcome.failure = Some(failure);
            break;
        }
    }
    if deps_dir.exists() {
        match std::fs::remove_dir_all(&deps_dir) {
            Ok(()) => reporter.event(
                "dependency_downloads_removed",
                deps_dir.display().to_string(),
            ),
            Err(err) => reporter.event(
                "dependency_downloads_cleanup_failed",
                format!("{}: {err}", deps_dir.display()),
            ),
        }
    }
    outcome
}

/// Records what the phase observed, each event in its own commit unit, once
/// there is a state database to record it in. The records are history of a
/// requirement being satisfied; nothing plans from them.
pub fn record_events(db: &crate::state::Db, phase: &PhaseOutcome) -> Result<()> {
    for event in &phase.events {
        crate::state::dependency::record(db, event)?;
    }
    Ok(())
}

/// The outcome document of a run the phase stopped.
pub fn failure_outcome(
    package: &Package,
    options: &RunOptions,
    phase: &PhaseOutcome,
    failure: &Failure,
) -> Outcome {
    let key = match failure.code {
        "dependency_missing" => "outcome.dependency_missing",
        "dependency_unacquirable" => "outcome.dependency_unacquirable",
        "dependency_install_failed" => "outcome.dependency_install_failed",
        "dependency_unverified" => "outcome.dependency_unverified",
        "dependency_requires_elevation" => "outcome.dependency_requires_elevation",
        _ => "outcome.dependency_cancelled",
    };
    let reason = match failure.reason {
        Some(reason) => format!("{reason}: {}", failure.message),
        None => failure.message.clone(),
    };
    let message = i18n::fill(
        i18n::text(&options.lang, key),
        &[
            ("name", package.name()),
            ("version", package.version()),
            ("dependency", &failure.display_name),
            ("reason", &reason),
        ],
    );
    let exit_code = crate::Error::new(failure.code, "").exit_code();
    let mut outcome = Outcome::new("failed", failure.code, exit_code, message);
    outcome.dependency = Some(failure.dependency.clone());
    outcome.reason = failure.reason;
    outcome.dependencies = phase.records.clone();
    outcome
}

/// Attaches the phase's records to a completed run's outcome; a successful
/// run whose dependency asked for a reboot exits with 3010.
pub fn attach(outcome: &mut Outcome, phase: &PhaseOutcome) {
    outcome.dependencies = phase.records.clone();
    // A custom action may already have asked for one.
    outcome.reboot_required |= phase.reboot_required;
    if outcome.reboot_required && outcome.exit_code == exit::OK {
        outcome.exit_code = exit::REBOOT_REQUIRED;
    }
}
