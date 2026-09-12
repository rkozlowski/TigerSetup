//! Which installation a run addresses.
//!
//! A product can be installed per user and per machine at the same time, and
//! each installation is its own: its own state database, its own install
//! root, its own registration. A run therefore has to say which one it is
//! about before it looks at anything else, and a client that names no scope
//! must not quietly pick the package's default and, finding nothing there,
//! create a second installation beside the one that exists.
//!
//! The rule the engine applies, for any client:
//!
//! - **One existing installation is sticky.** A run that names no scope
//!   continues with that installation, whichever scope it is in. Under the
//!   package's `error` policy a run whose defaulted scope is the other one is
//!   refused instead.
//! - **An explicit scope stays explicit.** A run that names a scope with no
//!   installation while the other scope has one is refused with
//!   `scope_conflict` — never silently redirected — unless the package's
//!   policy is `allow-parallel`, in which case the request creates a second,
//!   independent installation. Only an install can create one, so only an
//!   install is refused this way; an uninstall, repair or read of a scope that
//!   holds nothing reports that as it always did.
//! - **Two existing installations are never chosen between.** A run that
//!   names no scope is refused with `scope_ambiguous`; the client names one.
//! - **Scope never migrates.** Nothing here moves an installation from one
//!   scope to the other; that would be a separate, explicit capability.
//!
//! Both clients resolve through [`resolve`] before they decide anything about
//! elevation, and [`crate::install`] resolves again with the scope it was
//! finally given, so the policy holds whichever client asked.

use serde::Serialize;
use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::ExistingScopePolicy;

use crate::elevation::Intent;
use crate::report::{Outcome, exit};
use crate::state::{Db, installation};
use crate::{Error, Package, Result, RunOptions};

/// An installation of the product this machine holds, as the documents
/// report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExistingInstallation {
    pub scope: &'static str,
    pub version: String,
    pub install_root: String,
}

impl ExistingInstallation {
    pub fn scope(&self) -> Scope {
        Scope::parse(self.scope).unwrap_or(Scope::User)
    }
}

/// What the resolution decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The scope the run operates on.
    Scope(Scope),
    /// The run's scope — named by the client, or defaulted under the
    /// `error` policy — holds no installation while the other scope does,
    /// and the package does not allow a second one.
    Conflict {
        requested: Scope,
        existing_scope: Scope,
    },
    /// Both scopes hold an installation and the client named neither.
    Ambiguous,
}

/// The installations found and the decision made from them.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub existing: Vec<ExistingInstallation>,
    pub requested: Option<Scope>,
    pub decision: Decision,
}

impl Resolution {
    /// The scope to run in, when the resolution allowed one.
    pub fn scope(&self) -> Option<Scope> {
        match self.decision {
            Decision::Scope(scope) => Some(scope),
            _ => None,
        }
    }

    /// The installation the run continues with, when the resolved scope
    /// holds one.
    pub fn installed(&self) -> Option<&ExistingInstallation> {
        let scope = self.scope()?;
        self.existing.iter().find(|e| e.scope() == scope)
    }

    /// The stable code of a refusal, or `None` when the run may proceed.
    pub fn refusal_code(&self) -> Option<&'static str> {
        match self.decision {
            Decision::Scope(_) => None,
            Decision::Conflict { .. } => Some("scope_conflict"),
            Decision::Ambiguous => Some("scope_ambiguous"),
        }
    }

    /// The refusal as an engine error, for a caller that cannot carry the
    /// structured form.
    pub fn error(&self, package: &Package) -> Option<Error> {
        let code = self.refusal_code()?;
        Some(Error::new(code, self.describe(package)))
    }

    /// The refusal as an outcome document: the code, the message in the
    /// run's language and every installation the decision was made from,
    /// so an automated caller can name the scope it meant.
    pub fn outcome(&self, package: &Package, options: &RunOptions) -> Option<Outcome> {
        let code = self.refusal_code()?;
        let message = self.message(package, &options.lang);
        let mut outcome = Outcome::new("failed", code, exit::INVALID, message);
        outcome.existing_installations = self.existing.clone();
        Some(outcome)
    }

    /// The English wording of a refusal, for logs and errors.
    fn describe(&self, package: &Package) -> String {
        match self.decision {
            Decision::Conflict {
                requested,
                existing_scope,
            } => format!(
                "{} is installed in {} scope; a {} scope installation beside it is not allowed by the package (run with --scope {} to continue with the existing installation)",
                package.name(),
                existing_scope.as_str(),
                requested.as_str(),
                existing_scope.as_str()
            ),
            Decision::Ambiguous => format!(
                "{} is installed in both user and machine scope; name the one to continue with (--scope user or --scope machine)",
                package.name()
            ),
            Decision::Scope(_) => String::new(),
        }
    }

    fn message(&self, package: &Package, lang: &str) -> String {
        let human = |key: &str, values: &[(&str, &str)]| {
            crate::i18n::fill(crate::i18n::text(lang, key), values)
        };
        match self.decision {
            Decision::Conflict { existing_scope, .. } => human(
                match existing_scope {
                    Scope::Machine => "outcome.scope_conflict.machine",
                    Scope::User => "outcome.scope_conflict.user",
                },
                &[("name", package.name())],
            ),
            Decision::Ambiguous => human("outcome.scope_ambiguous", &[("name", package.name())]),
            Decision::Scope(_) => String::new(),
        }
    }
}

/// Every installation of the package this machine holds, in both scopes —
/// the scopes the package declares and the other one too, because the
/// database is reality and a package may stop declaring a scope it was
/// installed in.
///
/// A state database this process cannot open is skipped rather than made an
/// error: reading a scope's state is a run's own concern (`verify` and
/// `inspect` report an unreadable database as a finding, and a mutating run
/// opens it read-write and reports it there), and the scope decision must
/// not turn a locked or shut-out database into a failure of a command that
/// would otherwise say what it found. The parallel-installation rule
/// therefore holds over the installations this process can actually read,
/// which is every case but a database concurrently locked against it.
pub fn existing(package: &Package) -> Result<Vec<ExistingInstallation>> {
    let mut found = Vec::new();
    for scope in [Scope::User, Scope::Machine] {
        let state_db = crate::state_db_path(package, scope)?;
        let db = match Db::open_ro(&state_db) {
            Ok(Some(db)) => db,
            Ok(None) => continue,
            Err(err) if matches!(err.code, "database_busy" | "state_unreadable") => continue,
            Err(err) => return Err(err),
        };
        if let Some(row) = installation::read(&db)? {
            found.push(ExistingInstallation {
                scope: scope.as_str(),
                version: row.version,
                install_root: row.install_root,
            });
        }
    }
    Ok(found)
}

/// Resolves the scope a run operates on from what the client asked for,
/// what the machine holds and what the package allows. `intent` is `None`
/// for a read-only run, which no policy refuses.
pub fn resolve(
    package: &Package,
    requested: Option<Scope>,
    intent: Option<Intent>,
) -> Result<Resolution> {
    let existing = existing(package)?;
    let scopes: Vec<Scope> = existing.iter().map(ExistingInstallation::scope).collect();
    let decision = decide(
        package.metadata().existing_scope_policy(),
        package.default_scope(),
        requested,
        &scopes,
        intent,
    );
    Ok(Resolution {
        existing,
        requested,
        decision,
    })
}

/// The rule itself, over the scopes that hold an installation.
pub fn decide(
    policy: ExistingScopePolicy,
    default: Scope,
    requested: Option<Scope>,
    existing: &[Scope],
    intent: Option<Intent>,
) -> Decision {
    match requested {
        Some(scope) => {
            if existing.contains(&scope) || existing.is_empty() {
                return Decision::Scope(scope);
            }
            // Only an install can create a second installation; a run that
            // removes, repairs or reads the named scope finds it empty and
            // says so.
            match (intent, policy) {
                (Some(Intent::Install), ExistingScopePolicy::AllowParallel) => {
                    Decision::Scope(scope)
                }
                (Some(Intent::Install), _) => Decision::Conflict {
                    requested: scope,
                    existing_scope: existing[0],
                },
                _ => Decision::Scope(scope),
            }
        }
        None => match existing {
            [] => Decision::Scope(default),
            [only] => {
                if *only == default || intent.is_none() || policy != ExistingScopePolicy::Error {
                    Decision::Scope(*only)
                } else {
                    Decision::Conflict {
                        requested: default,
                        existing_scope: *only,
                    }
                }
            }
            _ => Decision::Ambiguous,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: Scope = Scope::User;
    const MACHINE: Scope = Scope::Machine;
    const INSTALL: Option<Intent> = Some(Intent::Install);
    const UNINSTALL: Option<Intent> = Some(Intent::Uninstall);
    const READ: Option<Intent> = None;

    #[test]
    fn a_first_installation_takes_the_requested_or_default_scope() {
        for policy in [
            ExistingScopePolicy::Preserve,
            ExistingScopePolicy::AllowParallel,
            ExistingScopePolicy::Error,
        ] {
            assert_eq!(
                decide(policy, USER, None, &[], INSTALL),
                Decision::Scope(USER)
            );
            assert_eq!(
                decide(policy, USER, Some(MACHINE), &[], INSTALL),
                Decision::Scope(MACHINE)
            );
        }
    }

    #[test]
    fn one_existing_installation_is_sticky_for_a_run_that_names_no_scope() {
        for policy in [
            ExistingScopePolicy::Preserve,
            ExistingScopePolicy::AllowParallel,
        ] {
            for intent in [INSTALL, UNINSTALL, READ] {
                assert_eq!(
                    decide(policy, USER, None, &[MACHINE], intent),
                    Decision::Scope(MACHINE),
                    "{policy:?} {intent:?}"
                );
                assert_eq!(
                    decide(policy, MACHINE, None, &[USER], intent),
                    Decision::Scope(USER)
                );
            }
        }
    }

    #[test]
    fn an_explicit_conflicting_scope_is_refused_unless_parallel_installations_are_allowed() {
        assert_eq!(
            decide(
                ExistingScopePolicy::Preserve,
                USER,
                Some(USER),
                &[MACHINE],
                INSTALL
            ),
            Decision::Conflict {
                requested: USER,
                existing_scope: MACHINE
            }
        );
        assert_eq!(
            decide(
                ExistingScopePolicy::Error,
                USER,
                Some(USER),
                &[MACHINE],
                INSTALL
            ),
            Decision::Conflict {
                requested: USER,
                existing_scope: MACHINE
            }
        );
        assert_eq!(
            decide(
                ExistingScopePolicy::AllowParallel,
                USER,
                Some(USER),
                &[MACHINE],
                INSTALL
            ),
            Decision::Scope(USER)
        );
        // The scope that holds the installation is never a conflict.
        assert_eq!(
            decide(
                ExistingScopePolicy::Error,
                USER,
                Some(MACHINE),
                &[MACHINE],
                INSTALL
            ),
            Decision::Scope(MACHINE)
        );
    }

    #[test]
    fn only_an_install_can_conflict() {
        for intent in [UNINSTALL, Some(Intent::Repair), READ] {
            assert_eq!(
                decide(
                    ExistingScopePolicy::Preserve,
                    USER,
                    Some(USER),
                    &[MACHINE],
                    intent
                ),
                Decision::Scope(USER),
                "{intent:?} of an empty scope is reported by the run, not refused here"
            );
        }
    }

    #[test]
    fn the_error_policy_refuses_a_defaulted_scope_that_is_not_the_installed_one() {
        assert_eq!(
            decide(ExistingScopePolicy::Error, USER, None, &[MACHINE], INSTALL),
            Decision::Conflict {
                requested: USER,
                existing_scope: MACHINE
            }
        );
        assert_eq!(
            decide(
                ExistingScopePolicy::Error,
                USER,
                None,
                &[MACHINE],
                UNINSTALL
            ),
            Decision::Conflict {
                requested: USER,
                existing_scope: MACHINE
            }
        );
        // Reading is not an encounter, and the installed scope is the default.
        assert_eq!(
            decide(ExistingScopePolicy::Error, USER, None, &[MACHINE], READ),
            Decision::Scope(MACHINE)
        );
        assert_eq!(
            decide(
                ExistingScopePolicy::Error,
                MACHINE,
                None,
                &[MACHINE],
                INSTALL
            ),
            Decision::Scope(MACHINE)
        );
    }

    #[test]
    fn two_installations_are_never_chosen_between() {
        for policy in [
            ExistingScopePolicy::Preserve,
            ExistingScopePolicy::AllowParallel,
            ExistingScopePolicy::Error,
        ] {
            for intent in [INSTALL, UNINSTALL, READ] {
                assert_eq!(
                    decide(policy, USER, None, &[USER, MACHINE], intent),
                    Decision::Ambiguous
                );
                assert_eq!(
                    decide(policy, USER, Some(MACHINE), &[USER, MACHINE], intent),
                    Decision::Scope(MACHINE),
                    "a named scope is unambiguous"
                );
            }
        }
    }
}
