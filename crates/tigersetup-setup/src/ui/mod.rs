//! The interactive client: a wizard drawn with plain Win32 common controls
//! over the same engine the unattended client drives.
//!
//! The split is deliberate and is the design's, not this module's: the
//! wizard decides *what the person asked for* — scope, destination, options,
//! accept, cancel — and shows *what the engine reports* — events, progress,
//! the outcome. It contains no installation logic of any kind, and reaches
//! the engine only through `tigersetup_engine`'s public API.

mod dialog;
mod glyph;
mod icon;
mod layout;
mod session;
mod text;
mod theme;
mod win;
mod window;
mod worker;

use std::path::PathBuf;

use tigersetup_engine::{Package, RunOptions};

pub use window::Completed;

use crate::Operation;

/// What the command-line client hands the wizard.
pub struct Request<'a> {
    /// The package file — the `Setup.exe` this engine was extracted from —
    /// which the worker thread opens for itself and an elevated or plain
    /// relaunch starts again; never this engine's own temporary file.
    pub exe: PathBuf,
    pub package: &'a Package,
    pub operation: Operation,
    pub options: RunOptions,
    /// The command line named a scope, so the wizard does not ask.
    pub scope_given: bool,
    /// The arguments an elevated relaunch should repeat.
    pub relaunch_arguments: Vec<String>,
}

/// Shows the wizard and returns once it has closed.
pub fn run(request: Request<'_>) -> Completed {
    let session = session::Session::build(
        request.exe,
        request.package,
        request.operation,
        request.options,
        request.scope_given,
        request.relaunch_arguments,
    );
    window::show(session)
}
