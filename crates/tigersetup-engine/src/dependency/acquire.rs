//! Acquisition (`TigerSetup-Design.md` §7.7, §7.9): the build-time hint
//! first while it is fresh, the catalog when the hint is stale, missing or
//! fails, and never an unverified download. A URL-sourced dependency is its
//! own requirement and is never refreshed. An embedded dependency is
//! extracted from this installer's own payload and verified against the
//! hash the builder recorded, so a damaged installer fails the same way a
//! damaged download does.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use tigersetup_catalog::winget::{Requirement, resolve};
use tigersetup_catalog::{CatalogError, Reason, http};
use tigersetup_format::Payload;
use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::{Acquisition, AcquisitionSource, Dependency};

use crate::report::{Phase, Progress, Reporter};
use crate::resource::file;

/// An installer file on disk, verified, with everything needed to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acquired {
    pub path: PathBuf,
    pub url: String,
    pub sha256: String,
    pub version: String,
    pub installer_type: String,
    pub arguments: Vec<String>,
    pub success_codes: Vec<i32>,
    pub reboot_codes: Vec<i32>,
    pub elevation_required: bool,
    /// The metadata came from a catalog refresh, not from the hint.
    pub refreshed: bool,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether a WinGet hint is young enough to be used without asking the
/// catalog. `max_age_days == 0` means never refresh.
pub fn hint_is_fresh(acquisition: &Acquisition, now: i64) -> bool {
    if acquisition.max_age_days == 0 {
        return true;
    }
    if acquisition.resolved_at <= 0 {
        return false;
    }
    let age = now.saturating_sub(acquisition.resolved_at);
    age <= i64::from(acquisition.max_age_days) * 86_400
}

/// A file name for the download: the URL's last segment when it is a plain
/// name, else the dependency id.
pub fn file_name_for(url: &str, dependency_id: &str, installer_type: &str) -> String {
    let from_url = http::Url::parse(url)
        .map(|u| u.file_name().to_string())
        .unwrap_or_default();
    let safe = from_url
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && from_url.contains('.')
        && from_url.len() <= 128;
    if safe {
        from_url
    } else {
        let extension = if tigersetup_catalog::manifest::is_msi_type(installer_type) {
            "msi"
        } else {
            "exe"
        };
        let stem: String = dependency_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("{stem}.{extension}")
    }
}

/// Downloads with progress events on the reporter: one at the start, one
/// per two percent (or per 4 MiB when the length is unknown) and one when
/// the file is complete.
fn download_reporting(
    url: &str,
    to: &Path,
    expected_sha256: &str,
    display_name: &str,
    reporter: &mut Reporter<'_>,
    cancel: Option<&AtomicBool>,
) -> Result<http::Downloaded, CatalogError> {
    let mut last_mark: Option<u64> = None;
    let mut progress = |done: u64, total: u64| {
        // One mark per two percent of a known length, else per 4 MiB.
        let mark = match total {
            0 => done >> 22,
            total => done * 50 / total,
        };
        if last_mark == Some(mark) && done != 0 && done != total {
            return;
        }
        last_mark = Some(mark);
        reporter.progress(
            "dependency_downloading",
            format!("{display_name}: {done}/{total} bytes from {url}"),
            Progress {
                phase: Phase::Dependencies,
                done,
                total,
                target: display_name.to_string(),
            },
        );
    };
    let downloaded = http::download(url, to, Some(expected_sha256), &mut progress, cancel)?;
    reporter.progress(
        "dependency_downloaded",
        format!(
            "{display_name}: {} bytes, sha256 {} verified, at {}",
            downloaded.length,
            downloaded.sha256,
            to.display()
        ),
        Progress {
            phase: Phase::Dependencies,
            done: downloaded.length,
            total: downloaded.length,
            target: display_name.to_string(),
        },
    );
    Ok(downloaded)
}

/// Extracts an embedded installer from the payload into `deps_dir` and
/// verifies its bytes against the recorded hash.
fn extract_embedded(
    acquisition: &Acquisition,
    dependency_id: &str,
    payload: &mut Payload,
    deps_dir: &Path,
    display_name: &str,
    reporter: &mut Reporter<'_>,
) -> Result<PathBuf, CatalogError> {
    let file_name = acquisition
        .entry
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| file_name_for("", dependency_id, &acquisition.installer_type));
    let path = deps_dir.join(file_name);
    let mut staged =
        file::stage_from_payload(&path, payload, &acquisition.entry, Some(acquisition.size))
            .map_err(|err| CatalogError::new(Reason::DownloadFailed, err.message))?;
    if staged.sha256 != acquisition.sha256 {
        return Err(CatalogError::new(
            Reason::HashMismatch,
            format!(
                "embedded installer {} has SHA-256 {}, the package recorded {}",
                acquisition.entry, staged.sha256, acquisition.sha256
            ),
        ));
    }
    staged
        .flush()
        .and_then(|()| staged.commit())
        .map_err(|err| CatalogError::new(Reason::DownloadFailed, err.message))?;
    reporter.progress(
        "dependency_extracted",
        format!(
            "{display_name}: {} bytes from payload entry {}, sha256 {} verified, at {}",
            acquisition.size,
            acquisition.entry,
            acquisition.sha256,
            path.display()
        ),
        Progress {
            phase: Phase::Dependencies,
            done: acquisition.size,
            total: acquisition.size,
            target: display_name.to_string(),
        },
    );
    Ok(path)
}

/// Acquires the installer for `dependency` into `deps_dir`; an embedded one
/// comes out of `payload`.
pub fn acquire(
    dependency: &Dependency,
    scope: Scope,
    deps_dir: &Path,
    payload: Option<&mut Payload>,
    reporter: &mut Reporter<'_>,
    cancel: Option<&AtomicBool>,
) -> Result<Acquired, CatalogError> {
    let acquisition = dependency.acquisition.clone().unwrap_or_default();
    let source =
        AcquisitionSource::try_from(acquisition.source).unwrap_or(AcquisitionSource::Unspecified);
    let display_name = if dependency.display_name.is_empty() {
        dependency.id.as_str()
    } else {
        dependency.display_name.as_str()
    };
    let declared = dependency.install.clone().unwrap_or_default();
    let has_hint = !acquisition.url.is_empty() && !acquisition.sha256.is_empty();

    if source == AcquisitionSource::Embedded {
        let payload = payload.ok_or_else(|| {
            CatalogError::new(
                Reason::DownloadFailed,
                "this executable carries no payload for the embedded installer",
            )
        })?;
        let path = extract_embedded(
            &acquisition,
            &dependency.id,
            payload,
            deps_dir,
            display_name,
            reporter,
        )?;
        return Ok(Acquired {
            path,
            url: format!("payload:{}", acquisition.entry),
            sha256: acquisition.sha256.clone(),
            version: acquisition.version.clone(),
            installer_type: acquisition.installer_type.clone(),
            arguments: declared.arguments.clone(),
            success_codes: declared.success_codes.clone(),
            reboot_codes: declared.reboot_codes.clone(),
            elevation_required: dependency.elevation_required,
            refreshed: false,
        });
    }

    if source == AcquisitionSource::Url {
        if !has_hint {
            return Err(CatalogError::new(
                Reason::NoCompatibleVersion,
                "the dependency declares a URL acquisition without a URL and hash",
            ));
        }
        let path = deps_dir.join(file_name_for(
            &acquisition.url,
            &dependency.id,
            &acquisition.installer_type,
        ));
        let downloaded = download_reporting(
            &acquisition.url,
            &path,
            &acquisition.sha256,
            display_name,
            reporter,
            cancel,
        )?;
        return Ok(Acquired {
            path,
            url: acquisition.url.clone(),
            sha256: downloaded.sha256,
            version: acquisition.version.clone(),
            installer_type: acquisition.installer_type.clone(),
            arguments: declared.arguments.clone(),
            success_codes: declared.success_codes.clone(),
            reboot_codes: declared.reboot_codes.clone(),
            elevation_required: dependency.elevation_required,
            refreshed: false,
        });
    }

    if source != AcquisitionSource::Winget {
        return Err(CatalogError::new(
            Reason::NoCompatibleVersion,
            "the dependency declares no acquisition source",
        ));
    }

    // The hint, while fresh.
    if has_hint {
        if hint_is_fresh(&acquisition, now_unix()) {
            let path = deps_dir.join(file_name_for(
                &acquisition.url,
                &dependency.id,
                &acquisition.installer_type,
            ));
            match download_reporting(
                &acquisition.url,
                &path,
                &acquisition.sha256,
                display_name,
                reporter,
                cancel,
            ) {
                Ok(downloaded) => {
                    return Ok(Acquired {
                        path,
                        url: acquisition.url.clone(),
                        sha256: downloaded.sha256,
                        version: acquisition.version.clone(),
                        installer_type: acquisition.installer_type.clone(),
                        arguments: declared.arguments.clone(),
                        success_codes: declared.success_codes.clone(),
                        reboot_codes: declared.reboot_codes.clone(),
                        elevation_required: dependency.elevation_required,
                        refreshed: false,
                    });
                }
                Err(err) if err.reason == Reason::Cancelled => return Err(err),
                Err(err) => reporter.event(
                    "dependency_hint_failed",
                    format!("{}: {err}; refreshing from the catalog", dependency.id),
                ),
            }
        } else {
            reporter.event(
                "dependency_hint_stale",
                format!(
                    "{}: hint resolved at {} is older than {} days; refreshing from the catalog",
                    dependency.id, acquisition.resolved_at, acquisition.max_age_days
                ),
            );
        }
    } else {
        reporter.event(
            "dependency_hint_missing",
            format!(
                "{}: no acquisition hint; resolving from the catalog",
                dependency.id
            ),
        );
    }

    // The catalog: current compatible metadata.
    let package = if acquisition.package_identifier.is_empty() {
        dependency.id.as_str()
    } else {
        acquisition.package_identifier.as_str()
    };
    let architecture = if acquisition.architecture.is_empty() {
        "x64"
    } else {
        acquisition.architecture.as_str()
    };
    let requirement = Requirement {
        minimum_version: &dependency.minimum_version,
        architecture,
        scope,
    };
    let resolved = resolve(package, &requirement, deps_dir, cancel)?;
    let installer = resolved.installer;
    reporter.event(
        "dependency_refreshed",
        format!(
            "{}: catalog version {} type {} scope {:?} elevation {} url {} sha256 {}",
            dependency.id,
            resolved.version,
            installer.installer_type,
            installer.scope,
            installer.elevation_required,
            installer.url,
            installer.sha256
        ),
    );
    let path = deps_dir.join(file_name_for(
        &installer.url,
        &dependency.id,
        &installer.installer_type,
    ));
    let downloaded = download_reporting(
        &installer.url,
        &path,
        &installer.sha256,
        display_name,
        reporter,
        cancel,
    )?;
    // Switches the package manifest declared outlive a refresh; resolved
    // ones are replaced by the current manifest's.
    let (arguments, success_codes, reboot_codes) = if declared.declared {
        (
            declared.arguments.clone(),
            declared.success_codes.clone(),
            declared.reboot_codes.clone(),
        )
    } else {
        (
            installer.arguments.clone(),
            installer.success_codes.clone(),
            installer.reboot_codes.clone(),
        )
    };
    Ok(Acquired {
        path,
        url: installer.url,
        sha256: downloaded.sha256,
        version: resolved.version,
        installer_type: installer.installer_type,
        arguments,
        success_codes,
        reboot_codes,
        elevation_required: dependency.elevation_required || installer.elevation_required,
        refreshed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_freshness_follows_age_and_lifetime() {
        let mut hint = Acquisition {
            resolved_at: 1_000_000,
            max_age_days: 14,
            ..Default::default()
        };
        assert!(hint_is_fresh(&hint, 1_000_000 + 13 * 86_400));
        assert!(!hint_is_fresh(&hint, 1_000_000 + 15 * 86_400));
        hint.max_age_days = 0;
        assert!(hint_is_fresh(&hint, i64::MAX));
        hint.max_age_days = 1;
        hint.resolved_at = 0;
        assert!(!hint_is_fresh(&hint, 10), "an unknown age is stale");
    }

    #[test]
    fn download_names_come_from_the_url_or_the_id() {
        assert_eq!(
            file_name_for(
                "https://x.invalid/a/windowsdesktop-runtime-10.0.11-win-x64.exe",
                "Microsoft.DotNet.DesktopRuntime.10",
                "burn"
            ),
            "windowsdesktop-runtime-10.0.11-win-x64.exe"
        );
        assert_eq!(
            file_name_for("https://x.invalid/download?id=5", "Vendor.Thing", "msi"),
            "Vendor-Thing.msi"
        );
        assert_eq!(
            file_name_for("https://x.invalid/", "Vendor.Thing", "exe"),
            "Vendor-Thing.exe"
        );
        assert_eq!(file_name_for("not a url", "V", "exe"), "V.exe");
    }
}
