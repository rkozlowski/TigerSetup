//! Fault injection at the journal/mutation boundaries. Compiled into every
//! build; a fault affects only the run that asks for it, through
//! `--fault <point>[@<sequence>]:<action>[:<seconds>][:skip_flush][:batched]`.
//!
//! A fault that names an operation takes that operation out of its journal
//! batch, so that the fault's boundary is exactly the operation's own:
//! everything before it durably applied, nothing after it started. With
//! the `batched` modifier the operation stays inside its batch, and the
//! fault then lands where a real interruption would — with the batch
//! `applying`, some of its files written and none of them acknowledged —
//! which is how a batch's recovery is tested.
//!
//! A fault can also announce that it has reached its boundary, by creating
//! the file `--fault-signal` names before it acts. That turns an outside
//! interruption from a guess about timing into an answer to an observation:
//! a harness watching for the file cuts power while the run is provably
//! sitting at the boundary, instead of some number of seconds after it
//! started and hoping. The file is created and closed without being flushed,
//! deliberately — a signal that forced the volume to disk would undo the very
//! unflushed-write condition some of these faults exist to produce.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::report::Reporter;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// Before a dependency's installer is started, with its verified file
    /// already on disk.
    BeforeDependencyInstall,
    /// After a dependency's installer returned, before its detector is run
    /// again.
    AfterDependencyInstall,
    /// After a custom action's start is journaled, before its process is
    /// started: a crash here is what a crash while it runs looks like to
    /// the next run.
    AfterActionStarted,
    AfterPrepare,
    AfterApplying,
    AfterWriteBeforeFlush,
    AfterFlushBeforeRename,
    AfterRename,
    AfterApplied,
    BeforeCommit,
    AfterCommitBeforeCleanup,
    /// After an operation's undo action, before the journal records it.
    AfterRollbackUndo,
}

impl FaultPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            FaultPoint::BeforeDependencyInstall => "before_dependency_install",
            FaultPoint::AfterDependencyInstall => "after_dependency_install",
            FaultPoint::AfterActionStarted => "after_action_started",
            FaultPoint::AfterPrepare => "after_prepare",
            FaultPoint::AfterApplying => "after_applying",
            FaultPoint::AfterWriteBeforeFlush => "after_write_before_flush",
            FaultPoint::AfterFlushBeforeRename => "after_flush_before_rename",
            FaultPoint::AfterRename => "after_rename",
            FaultPoint::AfterApplied => "after_applied",
            FaultPoint::BeforeCommit => "before_commit",
            FaultPoint::AfterCommitBeforeCleanup => "after_commit_before_cleanup",
            FaultPoint::AfterRollbackUndo => "after_rollback_undo",
        }
    }

    pub fn parse(text: &str) -> Option<FaultPoint> {
        Some(match text {
            "before_dependency_install" => FaultPoint::BeforeDependencyInstall,
            "after_dependency_install" => FaultPoint::AfterDependencyInstall,
            "after_action_started" => FaultPoint::AfterActionStarted,
            "after_prepare" => FaultPoint::AfterPrepare,
            "after_applying" => FaultPoint::AfterApplying,
            "after_write_before_flush" => FaultPoint::AfterWriteBeforeFlush,
            "after_flush_before_rename" => FaultPoint::AfterFlushBeforeRename,
            "after_rename" => FaultPoint::AfterRename,
            "after_applied" => FaultPoint::AfterApplied,
            "before_commit" => FaultPoint::BeforeCommit,
            "after_commit_before_cleanup" => FaultPoint::AfterCommitBeforeCleanup,
            "after_rollback_undo" => FaultPoint::AfterRollbackUndo,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultAction {
    /// `std::process::abort()`: no destructors, no flushes, no exit code path.
    Crash,
    /// Sleep for the given number of seconds, then continue.
    Hold(u64),
    /// Return an error so that the transaction rolls back.
    Fail,
}

impl FaultAction {
    pub fn describe(self) -> String {
        match self {
            FaultAction::Crash => "crash".into(),
            FaultAction::Hold(seconds) => format!("hold:{seconds}"),
            FaultAction::Fail => "fail".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultSpec {
    pub point: FaultPoint,
    /// The 1-based sequence the fault applies to: the operation in plan
    /// order, or the dependency in declaration order at a dependency point.
    /// Without one, the first to reach the point triggers it.
    pub sequence: Option<i64>,
    pub action: FaultAction,
    /// Rename the operation's file without `FlushFileBuffers`, deliberately
    /// breaking the protocol so that recovery's detection can be tested.
    pub skip_flush: bool,
    /// Leave the named operation inside its journal batch instead of
    /// walking it on its own.
    pub batched: bool,
}

impl FaultSpec {
    pub fn parse(text: &str) -> Result<FaultSpec> {
        let invalid = |why: &str| Error::new("fault_invalid", format!("fault {text:?}: {why}"));
        let mut parts = text.split(':');
        let location = parts.next().ok_or_else(|| invalid("missing point"))?;
        let (point, sequence) = match location.split_once('@') {
            Some((point, sequence)) => {
                let sequence: i64 = sequence
                    .parse()
                    .map_err(|_| invalid("sequence must be a positive integer"))?;
                if sequence < 1 {
                    return Err(invalid("sequence must be a positive integer"));
                }
                (point, Some(sequence))
            }
            None => (location, None),
        };
        let point = FaultPoint::parse(point).ok_or_else(|| invalid("unknown point"))?;
        let action_name = parts.next().ok_or_else(|| invalid("missing action"))?;
        let mut action = match action_name {
            "crash" => FaultAction::Crash,
            "hold" => FaultAction::Hold(60),
            "fail" => FaultAction::Fail,
            _ => return Err(invalid("action must be crash, hold or fail")),
        };
        let mut skip_flush = false;
        let mut batched = false;
        for modifier in parts {
            if modifier == "skip_flush" {
                skip_flush = true;
            } else if modifier == "batched" {
                batched = true;
            } else if let Ok(seconds) = modifier.parse::<u64>() {
                match action {
                    FaultAction::Hold(_) => action = FaultAction::Hold(seconds),
                    _ => return Err(invalid("only hold takes a duration")),
                }
            } else {
                return Err(invalid("unknown modifier"));
            }
        }
        Ok(FaultSpec {
            point,
            sequence,
            action,
            skip_flush,
            batched,
        })
    }

    pub fn describe(&self) -> String {
        let mut out = self.point.as_str().to_string();
        if let Some(sequence) = self.sequence {
            out.push_str(&format!("@{sequence}"));
        }
        out.push(':');
        out.push_str(&self.action.describe());
        if self.skip_flush {
            out.push_str(":skip_flush");
        }
        if self.batched {
            out.push_str(":batched");
        }
        out
    }
}

struct Armed {
    spec: FaultSpec,
    fired: bool,
}

/// Holds the run's faults and fires each once.
pub struct FaultInjector {
    armed: Vec<Armed>,
    /// Created when a fault reaches its boundary, for an outside observer.
    signal: Option<PathBuf>,
}

impl FaultInjector {
    pub fn new(specs: Vec<FaultSpec>) -> FaultInjector {
        FaultInjector {
            armed: specs
                .into_iter()
                .map(|spec| Armed { spec, fired: false })
                .collect(),
            signal: None,
        }
    }

    pub fn with_signal(specs: Vec<FaultSpec>, signal: Option<PathBuf>) -> FaultInjector {
        FaultInjector {
            signal,
            ..FaultInjector::new(specs)
        }
    }

    pub fn none() -> FaultInjector {
        FaultInjector::new(Vec::new())
    }

    /// Called by the executor at every boundary. Logs `fault_injected` and
    /// performs the action when an armed fault matches.
    pub fn at(
        &mut self,
        point: FaultPoint,
        sequence: Option<i64>,
        target: &str,
        reporter: &mut Reporter<'_>,
    ) -> Result<()> {
        let Some(armed) = self.armed.iter_mut().find(|armed| {
            !armed.fired
                && armed.spec.point == point
                && armed.spec.sequence.is_none_or(|s| Some(s) == sequence)
        }) else {
            return Ok(());
        };
        armed.fired = true;
        let spec = armed.spec.clone();
        reporter.event(
            "fault_injected",
            format!(
                "point={} sequence={} action={} target={target}",
                point.as_str(),
                sequence
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "-".into()),
                spec.action.describe()
            ),
        );
        self.announce(reporter);
        match spec.action {
            FaultAction::Crash => std::process::abort(),
            FaultAction::Hold(seconds) => {
                std::thread::sleep(Duration::from_secs(seconds));
                reporter.event(
                    "fault_hold_finished",
                    format!(
                        "point={} sequence={}",
                        point.as_str(),
                        sequence.unwrap_or(0)
                    ),
                );
                Ok(())
            }
            FaultAction::Fail => Err(Error::new(
                "fault_injected",
                format!(
                    "injected failure at {} for operation {}",
                    point.as_str(),
                    sequence.unwrap_or(0)
                ),
            )),
        }
    }

    /// Creates the signal file, if the run asked for one, so that whoever is
    /// watching knows the boundary has been reached. Nothing about the run
    /// depends on it: a signal that cannot be written is reported and the
    /// fault happens anyway.
    fn announce(&self, reporter: &mut Reporter<'_>) {
        let Some(path) = &self.signal else {
            return;
        };
        match create_signal(path) {
            Ok(()) => reporter.event("fault_signalled", path.display().to_string()),
            Err(err) => reporter.event("fault_signal_failed", format!("{}: {err}", path.display())),
        }
    }

    /// Whether the operation with this sequence must skip `FlushFileBuffers`.
    /// Applies to the sequence a fault names, or to every operation until an
    /// unqualified fault fires.
    /// Whether an armed fault names `sequence` — or names no sequence, and
    /// so may fire at any operation — which is where a batched forward walk
    /// ends a batch, so that the fault's boundary is the operation's own:
    /// its undo durable and nothing after it started, or it applied and
    /// nothing after it started. A `batched` fault is no boundary.
    pub fn boundary_at(&self, sequence: i64) -> bool {
        self.armed.iter().any(|armed| {
            !armed.fired && !armed.spec.batched && armed.spec.sequence.is_none_or(|s| s == sequence)
        })
    }

    pub fn skip_flush(&self, sequence: i64) -> bool {
        self.armed.iter().any(|armed| {
            armed.spec.skip_flush
                && !armed.fired
                && armed.spec.sequence.is_none_or(|s| s == sequence)
        })
    }
}

fn create_signal(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Written, not flushed: see the module comment.
    std::fs::write(
        path,
        b"boundary reached
",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse_and_describe() {
        let spec = FaultSpec::parse("after_write_before_flush@30:hold:90").unwrap();
        assert_eq!(spec.point, FaultPoint::AfterWriteBeforeFlush);
        assert_eq!(spec.sequence, Some(30));
        assert_eq!(spec.action, FaultAction::Hold(90));
        assert!(!spec.skip_flush);
        assert_eq!(spec.describe(), "after_write_before_flush@30:hold:90");

        let spec = FaultSpec::parse("after_rename@30:hold:90:skip_flush").unwrap();
        assert!(spec.skip_flush);
        assert_eq!(spec.describe(), "after_rename@30:hold:90:skip_flush");

        let spec = FaultSpec::parse("before_dependency_install@2:fail").unwrap();
        assert_eq!(spec.point, FaultPoint::BeforeDependencyInstall);
        assert_eq!(spec.sequence, Some(2));
        assert_eq!(spec.describe(), "before_dependency_install@2:fail");
        assert_eq!(
            FaultSpec::parse("after_dependency_install:crash")
                .unwrap()
                .point,
            FaultPoint::AfterDependencyInstall
        );

        let spec = FaultSpec::parse("before_commit:hold").unwrap();
        assert_eq!(spec.action, FaultAction::Hold(60));
        assert_eq!(spec.sequence, None);

        assert_eq!(
            FaultSpec::parse("after_applied@3:crash").unwrap().action,
            FaultAction::Crash
        );
        assert_eq!(
            FaultSpec::parse("after_prepare:fail").unwrap().action,
            FaultAction::Fail
        );
    }

    #[test]
    fn bad_specs_are_refused() {
        for bad in [
            "",
            "nowhere:crash",
            "after_prepare",
            "after_prepare:explode",
            "after_prepare@0:crash",
            "after_prepare:crash:5",
            "after_prepare:hold:x",
        ] {
            assert_eq!(
                FaultSpec::parse(bad).unwrap_err().code,
                "fault_invalid",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn skip_flush_applies_to_the_named_operation() {
        let injector = FaultInjector::new(vec![
            FaultSpec::parse("after_rename@7:hold:1:skip_flush").unwrap(),
        ]);
        assert!(injector.skip_flush(7));
        assert!(!injector.skip_flush(8));
        let injector = FaultInjector::new(vec![
            FaultSpec::parse("after_rename:hold:1:skip_flush").unwrap(),
        ]);
        assert!(injector.skip_flush(1));
        assert!(injector.skip_flush(9));
    }

    #[test]
    fn fail_fires_once_for_the_matching_operation() {
        let mut sink = crate::report::NullSink;
        let mut reporter = Reporter::new(&mut sink);
        let mut injector =
            FaultInjector::new(vec![FaultSpec::parse("after_applied@2:fail").unwrap()]);
        assert!(
            injector
                .at(FaultPoint::AfterApplied, Some(1), "a", &mut reporter)
                .is_ok()
        );
        assert_eq!(
            injector
                .at(FaultPoint::AfterApplied, Some(2), "b", &mut reporter)
                .unwrap_err()
                .code,
            "fault_injected"
        );
        assert!(
            injector
                .at(FaultPoint::AfterApplied, Some(2), "b", &mut reporter)
                .is_ok()
        );
    }
}
