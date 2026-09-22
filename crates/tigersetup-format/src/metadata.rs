//! Runtime metadata: the Protocol Buffers message tree compiled from
//! `proto/tigersetup.proto`, plus encoding, decoding and validation.

use prost::Message;

use crate::FormatError;
use crate::identity::Scope;

#[allow(clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/tigersetup.metadata.v1.rs"));
}

pub use generated::{
    Acquisition, AcquisitionSource, Action, ActionFailurePolicy, ActionKind, ActionOperation,
    ActionPhase, AppPath, ContextMenuTarget, ContextMenuVerb, Dependency, DependencyInstall,
    Detector, DetectorKind, Directory, Engine, EnvironmentVariable, ExistingScopePolicy, File,
    FileAssociation, FileBatch, FirewallAction, FirewallDirection, FirewallProtocol, FirewallRule,
    Install, InstallOption, Launch, Legacy, Metadata, OptionChoice, OptionKind, Package, PathEntry,
    PayloadEntry, Predicate, Quiescence, Registration, RegistryKind, RegistryRoot, RegistryValue,
    Role, Scope as ScopeTag, Shortcut, ShortcutLocation, UrlProtocol,
};

/// The value of a declared option: a boolean, or one of a choice option's
/// declared values. The canonical text — `"true"`, `"false"` or the choice
/// value — is what the command line, the predicates and the state database
/// spell; the type is what a report preserves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptionValue {
    Bool(bool),
    Choice(String),
}

impl OptionValue {
    /// The canonical text of the value.
    pub fn as_text(&self) -> String {
        match self {
            OptionValue::Bool(true) => "true".into(),
            OptionValue::Bool(false) => "false".into(),
            OptionValue::Choice(value) => value.clone(),
        }
    }

    /// The value the canonical text names, for an option of `kind`.
    pub fn from_text(option: &InstallOption, text: &str) -> Option<OptionValue> {
        if option.is_choice() {
            option
                .choices
                .iter()
                .find(|c| c.value.eq_ignore_ascii_case(text))
                .map(|c| OptionValue::Choice(c.value.clone()))
        } else {
            parse_bool(text).map(OptionValue::Bool)
        }
    }

    /// Whether this is the value a boolean predicate means by "on".
    pub fn is_on(&self) -> bool {
        matches!(self, OptionValue::Bool(true))
    }
}

/// The spellings of a boolean option value a person may type; the canonical
/// ones are `true` and `false`.
pub fn parse_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

impl InstallOption {
    pub fn is_choice(&self) -> bool {
        self.kind == OptionKind::Choice as i32
    }

    /// The declared default.
    pub fn default_value(&self) -> OptionValue {
        if self.is_choice() {
            OptionValue::Choice(self.default_choice.clone())
        } else {
            OptionValue::Bool(self.default)
        }
    }

    /// The canonical texts this option may take, for a message.
    pub fn accepted_values(&self) -> Vec<String> {
        if self.is_choice() {
            self.choices.iter().map(|c| c.value.clone()).collect()
        } else {
            vec!["on".into(), "off".into()]
        }
    }
}

impl Predicate {
    /// The predicate a resource carries: its `when`, or the older `option`
    /// spelling, which means "while that boolean option is on".
    pub fn of(when: Option<&Predicate>, option: &str) -> Option<Predicate> {
        match when {
            Some(when) if !when.option.is_empty() => Some(when.clone()),
            _ if !option.is_empty() => Some(Predicate {
                option: option.to_ascii_lowercase(),
                equals: "true".into(),
            }),
            _ => None,
        }
    }

    /// A predicate in one line, for a report: `option = value`.
    pub fn describe(&self) -> String {
        format!("{} = {}", self.option, self.equals)
    }
}

impl Quiescence {
    /// Every operation a quiescence entry may run on.
    pub const ALLOWED_OPERATIONS: &'static [ActionOperation] = &[
        ActionOperation::Install,
        ActionOperation::Upgrade,
        ActionOperation::Reinstall,
        ActionOperation::Repair,
        ActionOperation::Uninstall,
    ];

    /// The operations an entry runs on when it names none: those that find
    /// an installation whose application may be running.
    pub const DEFAULT_OPERATIONS: &'static [ActionOperation] = &[
        ActionOperation::Upgrade,
        ActionOperation::Reinstall,
        ActionOperation::Repair,
        ActionOperation::Uninstall,
    ];

    /// The message encoded on its own, for a journal or ownership row.
    pub fn encode_to_vec(&self) -> Vec<u8> {
        Message::encode_to_vec(self)
    }

    pub fn decode_bytes(bytes: &[u8]) -> Result<Quiescence, FormatError> {
        Quiescence::decode(bytes)
            .map_err(|err| FormatError::new("metadata_invalid", format!("quiescence: {err}")))
    }

    /// The operations the entry runs on: the declared ones, or the default.
    pub fn operations(&self) -> Vec<ActionOperation> {
        let declared: Vec<ActionOperation> = self
            .run_on
            .iter()
            .filter_map(|tag| ActionOperation::try_from(*tag).ok())
            .filter(|op| *op != ActionOperation::Unspecified)
            .collect();
        if declared.is_empty() {
            Self::DEFAULT_OPERATIONS.to_vec()
        } else {
            declared
        }
    }

    pub fn runs_on(&self, operation: ActionOperation) -> bool {
        self.operations().contains(&operation)
    }

    /// The stop program; validation guarantees it is there.
    pub fn stop(&self) -> &Action {
        self.stop
            .as_ref()
            .expect("a validated quiescence entry has a stop program")
    }

    /// Whether `code` is one the stop program reports for "not running".
    pub fn means_not_running(&self, code: i32) -> bool {
        self.not_running_codes.contains(&code)
    }

    /// The packaged programs the entry carries.
    pub fn packaged(&self) -> Vec<&Action> {
        [self.stop.as_ref(), self.resume.as_ref()]
            .into_iter()
            .flatten()
            .filter(|action| action.is_packaged())
            .collect()
    }
}

/// The Windows build the engine itself requires (Windows 10 1809 / Server
/// 2019); a package may raise it, never lower it.
pub const ENGINE_MINIMUM_BUILD: u32 = 17763;

/// The metadata major version this crate understands.
pub const SCHEMA: u32 = 2;

/// The most files one file batch holds.
pub const BATCH_MAX_FILES: usize = 256;
/// The most bytes one file batch holds, unless a single file is larger.
pub const BATCH_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The file batches of a file list, by the one rule the builder applies
/// and the engine never reproduces: files are taken in order; before a
/// file is added, the batch is closed if adding it would exceed
/// [`BATCH_MAX_FILES`] files or [`BATCH_MAX_BYTES`] bytes; a file larger
/// than the byte limit is therefore a batch of its own. Deterministic in
/// the list alone.
pub fn file_batches(files: &[File]) -> Vec<FileBatch> {
    let mut batches: Vec<FileBatch> = Vec::new();
    let mut open: Option<FileBatch> = None;
    for (index, file) in files.iter().enumerate() {
        if let Some(batch) = &open
            && (batch.file_count as usize >= BATCH_MAX_FILES
                || batch.bytes.saturating_add(file.size) > BATCH_MAX_BYTES)
        {
            batches.push(open.take().unwrap());
        }
        match &mut open {
            Some(batch) => {
                batch.file_count += 1;
                batch.bytes = batch.bytes.saturating_add(file.size);
            }
            None => {
                open = Some(FileBatch {
                    first_file: index as u32,
                    file_count: 1,
                    bytes: file.size,
                });
            }
        }
    }
    batches.extend(open);
    batches
}

/// The most values a choice option may declare. The wizard shows a choice
/// as a heading and one radio button per value on a page of nine rows, so
/// a choice always fits on one page.
pub const MAX_CHOICES: usize = 8;

/// The payload entries an embedded dependency installer travels as:
/// `.tigersetup/dependencies/<file name>`. The component cannot be an
/// install-relative product path, because a product file may not start with
/// this directory (the builder refuses one), so the two never collide.
pub const DEPENDENCY_ENTRY_PREFIX: &str = ".tigersetup/dependencies/";

/// The payload entries a packaged action program or script travels as:
/// `.tigersetup/actions/<file name>`. Reserved like the dependency
/// directory, and for the same reason.
pub const ACTION_ENTRY_PREFIX: &str = ".tigersetup/actions/";

/// How long an action may run when its declaration names no timeout.
pub const DEFAULT_ACTION_TIMEOUT_SECONDS: u32 = 300;

impl ActionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionPhase::PreInstall => "pre-install",
            ActionPhase::PostInstall => "post-install",
            ActionPhase::PreUninstall => "pre-uninstall",
            ActionPhase::PostUninstall => "post-uninstall",
            ActionPhase::Quiesce => "quiesce",
            ActionPhase::Resume => "resume",
            ActionPhase::Unspecified => "unspecified",
        }
    }

    /// The phases a standalone `[[actions]]` entry may declare; `quiesce`
    /// and `resume` belong to a `[[quiescence]]` entry alone.
    pub fn parse(text: &str) -> Option<ActionPhase> {
        Some(match text {
            "pre-install" => ActionPhase::PreInstall,
            "post-install" => ActionPhase::PostInstall,
            "pre-uninstall" => ActionPhase::PreUninstall,
            "post-uninstall" => ActionPhase::PostUninstall,
            _ => return None,
        })
    }

    /// The phases of a quiescence entry's programs, by name.
    pub fn parse_quiescence(text: &str) -> Option<ActionPhase> {
        Some(match text {
            "quiesce" => ActionPhase::Quiesce,
            "resume" => ActionPhase::Resume,
            _ => return None,
        })
    }

    /// Whether the phase is one of a quiescence entry's programs.
    pub fn is_quiescence(self) -> bool {
        matches!(self, ActionPhase::Quiesce | ActionPhase::Resume)
    }

    /// Whether the phase belongs to an installing transaction (install,
    /// upgrade, reinstall, repair) rather than to an uninstall.
    pub fn is_install(self) -> bool {
        matches!(self, ActionPhase::PreInstall | ActionPhase::PostInstall)
    }

    /// Whether the phase runs before the transaction's resource operations
    /// (`pre-*`) or after them (`post-*`).
    pub fn is_before(self) -> bool {
        matches!(self, ActionPhase::PreInstall | ActionPhase::PreUninstall)
    }

    /// The operations an action of this phase may run on. The default
    /// `run_on` of an install phase is the same set without `repair`, which
    /// is opt-in because an arbitrary program is not necessarily idempotent.
    pub fn allowed_operations(self) -> &'static [ActionOperation] {
        if self.is_quiescence() {
            return Quiescence::ALLOWED_OPERATIONS;
        }
        if self.is_install() {
            &[
                ActionOperation::Install,
                ActionOperation::Upgrade,
                ActionOperation::Reinstall,
                ActionOperation::Repair,
            ]
        } else {
            &[ActionOperation::Uninstall]
        }
    }

    pub fn default_operations(self) -> &'static [ActionOperation] {
        if self.is_install() {
            &[
                ActionOperation::Install,
                ActionOperation::Upgrade,
                ActionOperation::Reinstall,
            ]
        } else {
            &[ActionOperation::Uninstall]
        }
    }
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::Exe => "exe",
            ActionKind::Powershell => "powershell",
            ActionKind::Cmd => "cmd",
            ActionKind::Unspecified => "unspecified",
        }
    }

    pub fn parse(text: &str) -> Option<ActionKind> {
        Some(match text {
            "exe" => ActionKind::Exe,
            "powershell" => ActionKind::Powershell,
            "cmd" => ActionKind::Cmd,
            _ => return None,
        })
    }

    /// Whether a file name is one this kind runs: `.exe` for a native
    /// executable, `.ps1` for PowerShell, `.cmd` or `.bat` for cmd.
    pub fn accepts_file(self, file_name: &str) -> bool {
        let lower = file_name.to_ascii_lowercase();
        match self {
            ActionKind::Exe => lower.ends_with(".exe"),
            ActionKind::Powershell => lower.ends_with(".ps1"),
            ActionKind::Cmd => lower.ends_with(".cmd") || lower.ends_with(".bat"),
            ActionKind::Unspecified => false,
        }
    }
}

impl ActionOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionOperation::Install => "install",
            ActionOperation::Upgrade => "upgrade",
            ActionOperation::Reinstall => "reinstall",
            ActionOperation::Repair => "repair",
            ActionOperation::Uninstall => "uninstall",
            ActionOperation::Unspecified => "unspecified",
        }
    }

    pub fn parse(text: &str) -> Option<ActionOperation> {
        Some(match text {
            "install" => ActionOperation::Install,
            "upgrade" => ActionOperation::Upgrade,
            "reinstall" => ActionOperation::Reinstall,
            "repair" => ActionOperation::Repair,
            "uninstall" => ActionOperation::Uninstall,
            _ => return None,
        })
    }
}

impl ActionFailurePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionFailurePolicy::Continue => "continue",
            ActionFailurePolicy::Fail | ActionFailurePolicy::Unspecified => "fail",
        }
    }

    pub fn parse(text: &str) -> Option<ActionFailurePolicy> {
        Some(match text {
            "fail" => ActionFailurePolicy::Fail,
            "continue" => ActionFailurePolicy::Continue,
            _ => return None,
        })
    }
}

/// `phase()`, `kind()` and `on_failure()` are the generated accessors,
/// which answer `Unspecified` for a tag this crate does not know.
impl Action {
    /// The message encoded on its own, for a journal or ownership row.
    pub fn encode_to_vec(&self) -> Vec<u8> {
        Message::encode_to_vec(self)
    }

    pub fn decode_bytes(bytes: &[u8]) -> Result<Action, FormatError> {
        Action::decode(bytes)
            .map_err(|err| FormatError::new("metadata_invalid", format!("action: {err}")))
    }

    /// The failure policy; unspecified means `fail`.
    pub fn failure_policy(&self) -> ActionFailurePolicy {
        match self.on_failure() {
            ActionFailurePolicy::Continue => ActionFailurePolicy::Continue,
            _ => ActionFailurePolicy::Fail,
        }
    }

    /// The declared operations, in declaration order, unknown tags dropped.
    pub fn operations(&self) -> Vec<ActionOperation> {
        self.run_on
            .iter()
            .filter_map(|tag| ActionOperation::try_from(*tag).ok())
            .filter(|op| *op != ActionOperation::Unspecified)
            .collect()
    }

    /// Whether the action runs on `operation`.
    pub fn runs_on(&self, operation: ActionOperation) -> bool {
        self.operations().contains(&operation)
    }

    /// Whether the program travels in the package rather than being found
    /// on the target machine.
    pub fn is_packaged(&self) -> bool {
        !self.entry.is_empty()
    }

    /// The timeout in effect.
    pub fn timeout_seconds(&self) -> u32 {
        if self.timeout_seconds == 0 {
            DEFAULT_ACTION_TIMEOUT_SECONDS
        } else {
            self.timeout_seconds
        }
    }

    /// The exit codes that mean success: the declared ones, or 0 alone.
    pub fn success_codes(&self) -> Vec<i32> {
        if self.success_codes.is_empty() {
            vec![0]
        } else {
            self.success_codes.clone()
        }
    }

    /// What the action runs, for a report: the command template, or the
    /// packaged file name.
    pub fn program(&self) -> &str {
        if self.is_packaged() {
            &self.file_name
        } else {
            &self.command
        }
    }
}

impl RegistryValue {
    /// The scope whose hive an explicit root names: `Some(Ok(scope))` for
    /// `HKLM` (machine) or `HKCU` (user), `Some(Err(tag))` for a root this
    /// engine does not know, `None` for the scope's software root.
    pub fn explicit_root(&self) -> Option<Result<Scope, i32>> {
        match RegistryRoot::try_from(self.root) {
            Ok(RegistryRoot::Software) => None,
            Ok(RegistryRoot::LocalMachine) => Some(Ok(Scope::Machine)),
            Ok(RegistryRoot::CurrentUser) => Some(Ok(Scope::User)),
            Err(_) => Some(Err(self.root)),
        }
    }

    /// How the manifest spells the root: `HKLM`, `HKCU`, or `software` for
    /// the scope's software root.
    pub fn root_name(&self) -> &'static str {
        match RegistryRoot::try_from(self.root) {
            Ok(RegistryRoot::LocalMachine) => "HKLM",
            Ok(RegistryRoot::CurrentUser) => "HKCU",
            _ => "software",
        }
    }

    /// The root a manifest spelling names: `HKLM` or `HKCU` in either case,
    /// or their long `HKEY_` forms; `None` for anything else.
    pub fn root_of(name: &str) -> Option<RegistryRoot> {
        match name.to_ascii_uppercase().as_str() {
            "HKLM" | "HKEY_LOCAL_MACHINE" => Some(RegistryRoot::LocalMachine),
            "HKCU" | "HKEY_CURRENT_USER" => Some(RegistryRoot::CurrentUser),
            _ => None,
        }
    }
}

impl Metadata {
    pub fn encode_to_vec(&self) -> Vec<u8> {
        Message::encode_to_vec(self)
    }

    /// Decodes and validates a metadata block.
    pub fn decode_block(bytes: &[u8]) -> Result<Metadata, FormatError> {
        let metadata = <Metadata as Message>::decode(bytes)?;
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn package(&self) -> &Package {
        self.package
            .as_ref()
            .expect("validated metadata has a package")
    }

    pub fn install(&self) -> &Install {
        self.install
            .as_ref()
            .expect("validated metadata has an install section")
    }

    pub fn engine(&self) -> &Engine {
        self.engine
            .as_ref()
            .expect("validated metadata has an engine section")
    }

    /// Whether this executable is the uninstaller copy rather than the
    /// installer.
    pub fn is_uninstaller(&self) -> bool {
        self.role == Role::Uninstaller as i32
    }

    /// The scope an uninstaller copy serves.
    pub fn served_scope(&self) -> Option<Scope> {
        Scope::from_tag(self.uninstaller_scope)
    }

    /// The declared option of that name, whatever its case.
    pub fn option(&self, name: &str) -> Option<&InstallOption> {
        self.options
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
    }

    /// The declared default of an option, or `None` for an undeclared name.
    pub fn option_default(&self, name: &str) -> Option<OptionValue> {
        self.option(name).map(InstallOption::default_value)
    }

    /// Whether the payload entry name belongs to an embedded dependency
    /// rather than to a product file.
    pub fn is_dependency_entry(entry: &str) -> bool {
        entry.starts_with(DEPENDENCY_ENTRY_PREFIX)
    }

    /// Whether a payload entry name lies in the directory reserved for
    /// packaged action programs.
    pub fn is_action_entry(entry: &str) -> bool {
        entry.starts_with(ACTION_ENTRY_PREFIX)
    }

    /// Whether a payload entry name lies in a directory TigerSetup reserves
    /// for its own entries, which no product file may use.
    pub fn is_reserved_entry(entry: &str) -> bool {
        Metadata::is_dependency_entry(entry) || Metadata::is_action_entry(entry)
    }

    /// The identity of the licence text the wizard shows: lower-case hex
    /// SHA-256 of the exact UTF-8 bytes of `license_text`, or `None` when
    /// there is no text to show. Any byte that differs — a changed
    /// copyright year included — is a different agreement for the purpose
    /// of recording that a person accepted it.
    pub fn license_sha256(&self) -> Option<String> {
        let text = &self.package().license_text;
        (!text.trim().is_empty()).then(|| crate::hex(&crate::sha256(text.as_bytes())))
    }

    /// The Add/Remove Programs key name: declared, or the package id.
    pub fn registration_key_name(&self) -> &str {
        match &self.registration {
            Some(r) if !r.key_name.is_empty() => &r.key_name,
            _ => &self.package().id,
        }
    }

    /// The lowest Windows build the package supports.
    pub fn minimum_build(&self) -> u32 {
        self.install().minimum_build.max(ENGINE_MINIMUM_BUILD)
    }

    /// What a run does when the product is installed in the other scope;
    /// the policy defaults to preserving the existing installation.
    pub fn existing_scope_policy(&self) -> ExistingScopePolicy {
        match ExistingScopePolicy::try_from(self.install().existing_scope) {
            Ok(ExistingScopePolicy::Unspecified) | Err(_) => ExistingScopePolicy::Preserve,
            Ok(policy) => policy,
        }
    }

    /// The scopes the package supports, in declaration order.
    pub fn scopes(&self) -> Vec<Scope> {
        self.install()
            .scopes
            .iter()
            .filter_map(|tag| Scope::from_tag(*tag))
            .collect()
    }

    /// The install-root template for a scope, or `None` when the package does
    /// not support that scope.
    pub fn install_root_template(&self, scope: Scope) -> Option<&str> {
        if !self.scopes().contains(&scope) {
            return None;
        }
        let install = self.install();
        let template = match scope {
            Scope::User => install.user_root.as_str(),
            Scope::Machine => install.machine_root.as_str(),
        };
        (!template.is_empty()).then_some(template)
    }

    /// The index record of a payload entry, by name.
    pub fn payload_region(&self, entry: &str) -> Option<&PayloadEntry> {
        self.payload.iter().find(|region| region.entry == entry)
    }

    /// The names every part of the metadata expects the payload to hold,
    /// with the length each declares: the files, the embedded dependency
    /// installers and the packaged action programs.
    pub fn expected_entries(&self) -> Vec<(&str, u64)> {
        let mut expected: Vec<(&str, u64)> = self
            .files
            .iter()
            .map(|file| (file.entry.as_str(), file.size))
            .collect();
        for dependency in &self.dependencies {
            if let Some(acquisition) = dependency
                .acquisition
                .as_ref()
                .filter(|a| a.source == AcquisitionSource::Embedded as i32)
            {
                expected.push((acquisition.entry.as_str(), acquisition.size));
            }
        }
        for action in self.actions.iter().chain(self.quiescence_programs()) {
            if action.is_packaged() {
                expected.push((action.entry.as_str(), action.size));
            }
        }
        expected
    }

    /// Every stop and resume program of every quiescence entry.
    pub fn quiescence_programs(&self) -> impl Iterator<Item = &Action> {
        self.quiescence
            .iter()
            .flat_map(|q| [q.stop.as_ref(), q.resume.as_ref()])
            .flatten()
    }

    /// The payload index describes one contiguous stream: regions in
    /// ascending order, each starting where the previous ended, the first
    /// at zero, no name twice. An installer additionally carries every
    /// entry the metadata refers to, at the length it declares; an
    /// uninstaller copy carries no payload and no index, and a builder
    /// validates its metadata before the index exists, so an empty index
    /// is not checked against the references.
    pub fn validate_payload_index(&self) -> Result<(), FormatError> {
        let invalid = |message: String| FormatError::new("metadata_invalid", message);
        let mut next = 0u64;
        let mut names = std::collections::HashSet::new();
        for region in &self.payload {
            if region.entry.is_empty() {
                return Err(invalid(format!(
                    "payload index has a nameless region at {}",
                    region.offset
                )));
            }
            if region.offset != next {
                return Err(invalid(format!(
                    "payload entry {} starts at {}, expected {next}",
                    region.entry, region.offset
                )));
            }
            next = region
                .offset
                .checked_add(region.length)
                .ok_or_else(|| invalid(format!("payload entry {} overflows", region.entry)))?;
            if !names.insert(region.entry.as_str()) {
                return Err(invalid(format!(
                    "payload entry {} is indexed twice",
                    region.entry
                )));
            }
            if !is_sha256_hex(&region.sha256) {
                return Err(invalid(format!(
                    "payload entry {} has no SHA-256",
                    region.entry
                )));
            }
        }
        if self.payload.is_empty() {
            return Ok(());
        }
        for (entry, length) in self.expected_entries() {
            match self.payload_region(entry) {
                Some(region) if region.length == length => {}
                Some(region) => {
                    return Err(invalid(format!(
                        "payload entry {entry} is {} bytes in the index and {length} where it is referenced",
                        region.length
                    )));
                }
                None => {
                    return Err(invalid(format!(
                        "payload entry {entry} is referenced but not in the payload index"
                    )));
                }
            }
        }
        Ok(())
    }

    /// The batch a file belongs to, by its index into `files`, and the
    /// batch's index into `file_batches`.
    pub fn batch_of_file(&self, file_index: usize) -> Option<usize> {
        self.file_batches.iter().position(|batch| {
            let first = batch.first_file as usize;
            (first..first + batch.file_count as usize).contains(&file_index)
        })
    }

    /// The file batches partition the file list exactly, in order, and each
    /// obeys the rule [`file_batches`] states — which is checked by
    /// recomputing nothing: a batch of more than the file limit, of more
    /// than the byte limit while holding more than one file, or one that
    /// could have taken the next file is refused.
    pub fn validate_file_batches(&self) -> Result<(), FormatError> {
        let invalid = |message: String| FormatError::new("metadata_invalid", message);
        let mut next = 0usize;
        for (index, batch) in self.file_batches.iter().enumerate() {
            let first = batch.first_file as usize;
            let count = batch.file_count as usize;
            if first != next || count == 0 {
                return Err(invalid(format!(
                    "file batch {index} starts at file {first} with {count} files, expected to start at {next}"
                )));
            }
            let files = self
                .files
                .get(first..first + count)
                .ok_or_else(|| invalid(format!("file batch {index} runs past the file list")))?;
            let bytes = files.iter().fold(0u64, |sum, f| sum.saturating_add(f.size));
            if bytes != batch.bytes {
                return Err(invalid(format!(
                    "file batch {index} declares {} bytes but its files hold {bytes}",
                    batch.bytes
                )));
            }
            if count > BATCH_MAX_FILES {
                return Err(invalid(format!(
                    "file batch {index} holds {count} files, more than {BATCH_MAX_FILES}"
                )));
            }
            if count > 1 && bytes > BATCH_MAX_BYTES {
                return Err(invalid(format!(
                    "file batch {index} holds {bytes} bytes, more than {BATCH_MAX_BYTES}"
                )));
            }
            if let Some(following) = self.files.get(first + count)
                && count < BATCH_MAX_FILES
                && bytes.saturating_add(following.size) <= BATCH_MAX_BYTES
            {
                return Err(invalid(format!(
                    "file batch {index} closes before {}, which it could have taken",
                    following.path
                )));
            }
            next = first + count;
        }
        if next != self.files.len() {
            return Err(invalid(format!(
                "the file batches cover {next} of {} files",
                self.files.len()
            )));
        }
        Ok(())
    }

    /// The launch-after-install declaration names an `.exe` the package
    /// installs, and a working directory that is the install root or a
    /// directory the package installs — so what the completion page offers
    /// exists once the run has committed, unless something removed it since.
    pub fn validate_launch(&self, launch: &Launch) -> Result<(), FormatError> {
        let invalid = |message: String| FormatError::new("metadata_invalid", message);
        validate_relative_path(&launch.executable)?;
        if !launch.executable.to_ascii_lowercase().ends_with(".exe") {
            return Err(invalid(format!(
                "launch executable {} is not an .exe",
                launch.executable
            )));
        }
        if !self
            .files
            .iter()
            .any(|file| file.path.eq_ignore_ascii_case(&launch.executable))
        {
            return Err(invalid(format!(
                "launch executable {} is not a file the package installs",
                launch.executable
            )));
        }
        if let Some(directory) = launch.working_directory.as_deref()
            && !directory.is_empty()
        {
            validate_relative_path(directory)?;
            if !self
                .directories
                .iter()
                .any(|declared| declared.path.eq_ignore_ascii_case(directory))
            {
                return Err(invalid(format!(
                    "launch working directory {directory} is not a directory the package installs"
                )));
            }
        }
        Ok(())
    }

    /// Structural validation shared by the builder (before writing) and the
    /// engine (after reading).
    pub fn validate(&self) -> Result<(), FormatError> {
        let invalid = |message: String| FormatError::new("metadata_invalid", message);
        if self.schema != SCHEMA {
            return Err(FormatError::new(
                "metadata_unsupported",
                format!(
                    "metadata schema {} is not supported by schema {SCHEMA}",
                    self.schema
                ),
            ));
        }
        let package = self
            .package
            .as_ref()
            .ok_or_else(|| invalid("package section is missing".into()))?;
        crate::identity::validate_product_id(&package.id)?;
        crate::identity::validate_name(&package.name)?;
        crate::identity::validate_version(&package.version)?;
        let install = self
            .install
            .as_ref()
            .ok_or_else(|| invalid("install section is missing".into()))?;
        if install.scopes.is_empty() {
            return Err(invalid("install.scopes is empty".into()));
        }
        for tag in &install.scopes {
            let scope =
                Scope::from_tag(*tag).ok_or_else(|| invalid(format!("unknown scope tag {tag}")))?;
            let template = match scope {
                Scope::User => &install.user_root,
                Scope::Machine => &install.machine_root,
            };
            if template.is_empty() {
                return Err(invalid(format!(
                    "install root for scope {} is missing",
                    scope.as_str()
                )));
            }
        }
        if self.engine.is_none() {
            return Err(invalid("engine section is missing".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            if file.entry.is_empty() {
                return Err(invalid(format!("file {} has no payload entry", file.path)));
            }
            if Metadata::is_reserved_entry(&file.path) || Metadata::is_reserved_entry(&file.entry) {
                return Err(invalid(format!(
                    "file {} lies in a payload directory reserved for TigerSetup's own entries",
                    file.path
                )));
            }
            if !seen.insert(file.path.to_ascii_lowercase()) {
                return Err(invalid(format!("file {} is declared twice", file.path)));
            }
        }
        let mut declared_directories = std::collections::HashSet::new();
        for directory in &self.directories {
            validate_relative_path(&directory.path)?;
            declared_directories.insert(directory.path.to_ascii_lowercase());
        }
        // The directory list is the whole truth about the directories a
        // package needs: it carries every parent of every file, and may carry
        // more (a directory the package wants even when empty). An engine
        // therefore installs what this list says instead of deriving the same
        // rule a second time from the file paths.
        for file in &self.files {
            let parts: Vec<&str> = file.path.split('/').collect();
            for depth in 1..parts.len() {
                let parent = parts[..depth].join("/");
                if !declared_directories.contains(&parent.to_ascii_lowercase()) {
                    return Err(invalid(format!(
                        "file {} needs directory {parent}, which is not declared",
                        file.path
                    )));
                }
            }
        }
        self.validate_file_batches()?;
        if Role::try_from(self.role).is_err() {
            return Err(invalid(format!("unknown role {}", self.role)));
        }
        if self.is_uninstaller() && self.served_scope().is_none() {
            return Err(invalid("an uninstaller names no scope".into()));
        }
        self.validate_payload_index()?;
        for option in &self.options {
            validate_install_option(option)?;
        }
        let mut option_names = std::collections::HashSet::new();
        for option in &self.options {
            if !option_names.insert(option.name.to_ascii_lowercase()) {
                return Err(invalid(format!("option {} is declared twice", option.name)));
            }
        }
        let predicate_known =
            |what: &str, when: Option<&Predicate>, option: &str| -> Result<(), FormatError> {
                let Some(predicate) = Predicate::of(when, option) else {
                    return Ok(());
                };
                let declared = self.option(&predicate.option).ok_or_else(|| {
                    invalid(format!(
                        "{what} depends on option {:?}, which is not declared",
                        predicate.option
                    ))
                })?;
                if OptionValue::from_text(declared, &predicate.equals).is_none() {
                    return Err(invalid(format!(
                        "{what} wants option {} to equal {:?}, which is not one of its values",
                        predicate.option, predicate.equals
                    )));
                }
                Ok(())
            };
        for file in &self.files {
            predicate_known(&format!("file {}", file.path), file.when.as_ref(), "")?;
        }
        for shortcut in &self.shortcuts {
            validate_shortcut(shortcut)?;
            predicate_known(
                &format!("shortcut {:?}", shortcut.name),
                shortcut.when.as_ref(),
                &shortcut.option,
            )?;
        }
        for entry in &self.path_entries {
            if !entry.path.is_empty() {
                validate_relative_path(&entry.path)?;
            }
            predicate_known(
                &format!("PATH entry {:?}", entry.path),
                entry.when.as_ref(),
                &entry.option,
            )?;
        }
        for variable in &self.environment_variables {
            validate_environment_name(&variable.name)?;
            predicate_known(
                &format!("environment variable {}", variable.name),
                variable.when.as_ref(),
                "",
            )?;
        }
        let mut prog_ids = std::collections::HashSet::new();
        for association in &self.file_associations {
            validate_prog_id(&association.prog_id)?;
            if !prog_ids.insert(association.prog_id.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "ProgID {} is declared twice",
                    association.prog_id
                )));
            }
            if association.extensions.is_empty() {
                return Err(invalid(format!(
                    "file association {} names no extension",
                    association.prog_id
                )));
            }
            for extension in &association.extensions {
                validate_extension(extension)?;
            }
            validate_relative_path(&association.executable)?;
            if !association.icon.is_empty() {
                validate_relative_path(&association.icon)?;
            }
            predicate_known(
                &format!("file association {}", association.prog_id),
                association.when.as_ref(),
                "",
            )?;
        }
        let mut schemes = std::collections::HashSet::new();
        for protocol in &self.url_protocols {
            validate_scheme(&protocol.scheme)?;
            if !schemes.insert(protocol.scheme.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "URL protocol {} is declared twice",
                    protocol.scheme
                )));
            }
            if !protocol.prog_id.is_empty() {
                validate_prog_id(&protocol.prog_id)?;
                if !prog_ids.insert(protocol.prog_id.to_ascii_lowercase()) {
                    return Err(invalid(format!(
                        "ProgID {} is declared twice",
                        protocol.prog_id
                    )));
                }
            }
            validate_relative_path(&protocol.executable)?;
            if !protocol.icon.is_empty() {
                validate_relative_path(&protocol.icon)?;
            }
            predicate_known(
                &format!("URL protocol {}", protocol.scheme),
                protocol.when.as_ref(),
                "",
            )?;
        }
        let mut app_path_names = std::collections::HashSet::new();
        for app_path in &self.app_paths {
            validate_relative_path(&app_path.executable)?;
            let file_name = app_path
                .executable
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !file_name.ends_with(".exe") {
                return Err(invalid(format!(
                    "App Paths entry {} is not an .exe",
                    app_path.executable
                )));
            }
            if !app_path_names.insert(file_name) {
                return Err(invalid(format!(
                    "App Paths entry {} is declared twice",
                    app_path.executable
                )));
            }
            predicate_known(
                &format!("App Paths entry {}", app_path.executable),
                app_path.when.as_ref(),
                "",
            )?;
        }
        let mut verbs = std::collections::HashSet::new();
        for verb in &self.context_menu_verbs {
            let target = ContextMenuTarget::try_from(verb.target)
                .ok()
                .filter(|t| *t != ContextMenuTarget::Unspecified)
                .ok_or_else(|| {
                    invalid(format!("context menu verb {:?} has no target", verb.verb))
                })?;
            validate_verb(&verb.verb)?;
            if verb.label.trim().is_empty() {
                return Err(invalid(format!(
                    "context menu verb {} has no label",
                    verb.verb
                )));
            }
            validate_relative_path(&verb.executable)?;
            if !verb.icon.is_empty() {
                validate_relative_path(&verb.icon)?;
            }
            if !verb.extensions.is_empty() && target != ContextMenuTarget::Files {
                return Err(invalid(format!(
                    "context menu verb {} limits itself to extensions but does not target files",
                    verb.verb
                )));
            }
            for extension in &verb.extensions {
                validate_extension(extension)?;
            }
            let identity = format!(
                "{}|{}|{}",
                verb.target,
                verb.verb.to_ascii_lowercase(),
                verb.extensions
                    .iter()
                    .map(|e| e.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            if !verbs.insert(identity) {
                return Err(invalid(format!(
                    "context menu verb {} is declared twice for the same target",
                    verb.verb
                )));
            }
            predicate_known(
                &format!("context menu verb {}", verb.verb),
                verb.when.as_ref(),
                "",
            )?;
        }
        if let Some(launch) = &self.launch {
            self.validate_launch(launch)?;
        }
        let mut rule_names = std::collections::HashSet::new();
        for rule in &self.firewall_rules {
            validate_firewall_rule(rule)?;
            if !rule_names.insert(rule.name.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "firewall rule {} is declared twice",
                    rule.name
                )));
            }
            predicate_known(
                &format!("firewall rule {}", rule.name),
                rule.when.as_ref(),
                "",
            )?;
        }
        for value in &self.registry_values {
            predicate_known(
                &format!("registry value {}\\{}", value.key, value.name),
                value.when.as_ref(),
                "",
            )?;
            validate_registry_key(&value.key)?;
            // An explicit root is a hive, and a hive is written by exactly
            // one scope: a package allowing the other scope would carry a
            // value its run could not write.
            match value.explicit_root() {
                Some(Err(root)) => {
                    return Err(invalid(format!(
                        "registry value {}\\{} has an unknown root {root}",
                        value.key, value.name
                    )));
                }
                Some(Ok(hive_scope)) if self.scopes().iter().any(|scope| *scope != hive_scope) => {
                    return Err(invalid(format!(
                        "registry value {}\\{} has root {} but the package allows {} scope: an explicit root needs a package whose only scope writes that hive",
                        value.key,
                        value.name,
                        value.root_name(),
                        match hive_scope {
                            Scope::Machine => "user",
                            Scope::User => "machine",
                        }
                    )));
                }
                _ => {}
            }
            if value.name.contains(|c: char| c.is_control()) {
                return Err(invalid(format!(
                    "registry value name {:?} is not valid",
                    value.name
                )));
            }
            if RegistryKind::try_from(value.kind).is_err()
                || value.kind == RegistryKind::Unspecified as i32
            {
                return Err(invalid(format!(
                    "registry value {}\\{} has no kind",
                    value.key, value.name
                )));
            }
            if value.kind == RegistryKind::Dword as i32 && value.data.parse::<u32>().is_err() {
                return Err(invalid(format!(
                    "registry value {}\\{} is a DWORD but {:?} is not a number",
                    value.key, value.name, value.data
                )));
            }
        }
        if let Some(registration) = &self.registration {
            if !registration.key_name.is_empty() {
                validate_registration_key_name(&registration.key_name)?;
            }
            if !registration.display_icon.is_empty() {
                validate_relative_path(&registration.display_icon)?;
            }
        }
        if let Some(legacy) = &self.legacy {
            if legacy.installer_type != "inno" {
                return Err(invalid(format!(
                    "legacy installer type {:?} is not supported",
                    legacy.installer_type
                )));
            }
            validate_registration_key_name(&legacy.registration_key)?;
        }
        let mut dependency_ids = std::collections::HashSet::new();
        for dependency in &self.dependencies {
            validate_dependency(dependency, &mut dependency_ids)?;
            predicate_known(
                &format!("dependency {}", dependency.id),
                dependency.when.as_ref(),
                "",
            )?;
        }
        let mut action_names = std::collections::HashSet::new();
        for action in &self.actions {
            validate_action(action)?;
            if action.phase().is_quiescence() {
                return Err(invalid(format!(
                    "action {} declares phase {}, which belongs to a quiescence entry",
                    action.name,
                    action.phase().as_str()
                )));
            }
            if !action_names.insert(action.name.to_ascii_lowercase()) {
                return Err(invalid(format!("action {} is declared twice", action.name)));
            }
            predicate_known(&format!("action {}", action.name), action.when.as_ref(), "")?;
        }
        for entry in &self.quiescence {
            validate_quiescence(entry)?;
            if !action_names.insert(entry.name.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "quiescence {} shares its name with an action or another quiescence entry",
                    entry.name
                )));
            }
            predicate_known(
                &format!("quiescence {}", entry.name),
                entry.when.as_ref(),
                "",
            )?;
        }
        Ok(())
    }
}

/// A quiescence entry: a name of its own, a stop program in the quiesce
/// phase, an optional resume program in the resume phase — both valid as
/// actions, named like the entry — and operations the entry may run on.
fn validate_quiescence(entry: &Quiescence) -> Result<(), FormatError> {
    let invalid = |message: String| {
        FormatError::new(
            "metadata_invalid",
            format!("quiescence {:?}: {message}", entry.name),
        )
    };
    validate_action_name(&entry.name)?;
    let stop = entry
        .stop
        .as_ref()
        .ok_or_else(|| invalid("no stop program".into()))?;
    for (program, phase) in [
        (Some(stop), ActionPhase::Quiesce),
        (entry.resume.as_ref(), ActionPhase::Resume),
    ] {
        let Some(program) = program else {
            continue;
        };
        if program.name != entry.name {
            return Err(invalid(format!(
                "its {} program is named {:?}",
                phase.as_str(),
                program.name
            )));
        }
        if program.phase() != phase {
            return Err(invalid(format!(
                "its {} program declares phase {}",
                phase.as_str(),
                program.phase().as_str()
            )));
        }
        validate_action(program)?;
        if program.when.is_some() {
            return Err(invalid(format!(
                "its {} program carries a predicate; the entry's is the one",
                phase.as_str()
            )));
        }
    }
    let operations = entry.operations();
    let mut seen = std::collections::HashSet::new();
    for operation in &operations {
        if !seen.insert(*operation) {
            return Err(invalid(format!(
                "run_on names {} twice",
                operation.as_str()
            )));
        }
    }
    if entry.run_on.len()
        != entry
            .run_on
            .iter()
            .filter(|t| {
                ActionOperation::try_from(**t).is_ok_and(|op| op != ActionOperation::Unspecified)
            })
            .count()
    {
        return Err(invalid("run_on names an unknown operation".into()));
    }
    for code in &entry.not_running_codes {
        if stop.success_codes().contains(code) {
            return Err(invalid(format!(
                "exit code {code} is both a success code and a not-running code"
            )));
        }
    }
    Ok(())
}

/// An action name is an option-name-shaped word: lower-case words joined
/// by `-`. It names the action in every log line, finding and report, and
/// a directory of the transaction's staging area.
pub fn validate_action_name(name: &str) -> Result<(), FormatError> {
    let ok = !name.is_empty()
        && name.len() <= 48
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("action name {name:?} is not valid (lower-case words joined by '-')"),
        ))
    }
}

/// Whether a template names the install root, which a post-uninstall
/// action can never use: the root is removed before the action runs.
pub fn names_install_root(template: &str) -> bool {
    template.to_ascii_uppercase().contains("%INSTALLROOT%")
}

fn validate_action(action: &Action) -> Result<(), FormatError> {
    let invalid = |message: String| {
        FormatError::new(
            "metadata_invalid",
            format!("action {:?}: {message}", action.name),
        )
    };
    validate_action_name(&action.name)?;
    let phase = action.phase();
    if phase == ActionPhase::Unspecified {
        return Err(invalid("no phase".into()));
    }
    let kind = action.kind();
    if kind == ActionKind::Unspecified {
        return Err(invalid("no kind".into()));
    }
    let operations = action.operations();
    if phase.is_quiescence() {
        if !action.run_on.is_empty() {
            return Err(invalid(
                "a quiescence program names no operations of its own; the entry does".into(),
            ));
        }
    } else if operations.is_empty() || operations.len() != action.run_on.len() {
        return Err(invalid(
            "run_on is empty or names an unknown operation".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for operation in &operations {
        if !seen.insert(*operation) {
            return Err(invalid(format!(
                "run_on names {} twice",
                operation.as_str()
            )));
        }
        if !phase.allowed_operations().contains(operation) {
            return Err(invalid(format!(
                "a {} action cannot run on {}",
                phase.as_str(),
                operation.as_str()
            )));
        }
    }
    match (action.command.is_empty(), action.entry.is_empty()) {
        (true, true) => {
            return Err(invalid("neither a command nor a packaged file".into()));
        }
        (false, false) => {
            return Err(invalid(
                "both a command and a packaged file; exactly one is allowed".into(),
            ));
        }
        (false, true) => {
            if action.command.contains(|c: char| c.is_control()) {
                return Err(invalid("the command has a control character".into()));
            }
            if !action.file_name.is_empty() || action.size != 0 || !action.sha256.is_empty() {
                return Err(invalid(
                    "a command action carries packaged-file fields".into(),
                ));
            }
            if !kind.accepts_file(&action.command) {
                return Err(invalid(format!(
                    "{:?} is not a file a {} action runs",
                    action.command,
                    kind.as_str()
                )));
            }
            if phase == ActionPhase::PostUninstall && names_install_root(&action.command) {
                return Err(invalid(
                    "a post-uninstall action cannot run a program under %INSTALLROOT%, which is removed before it runs".into(),
                ));
            }
        }
        (true, false) => {
            if !action.entry.starts_with(ACTION_ENTRY_PREFIX)
                || action.entry.len() == ACTION_ENTRY_PREFIX.len()
            {
                return Err(invalid(format!(
                    "packaged entry {:?} is not under {ACTION_ENTRY_PREFIX}",
                    action.entry
                )));
            }
            if action.file_name.is_empty()
                || action.file_name.contains(['/', '\\'])
                || validate_relative_path(&action.file_name).is_err()
            {
                return Err(invalid(format!(
                    "packaged file name {:?} is not a plain file name",
                    action.file_name
                )));
            }
            if !kind.accepts_file(&action.file_name) {
                return Err(invalid(format!(
                    "{:?} is not a file a {} action runs",
                    action.file_name,
                    kind.as_str()
                )));
            }
            if action.size == 0 {
                return Err(invalid("the packaged file is empty".into()));
            }
            if !is_sha256_hex(&action.sha256) {
                return Err(invalid("the packaged file has no SHA-256".into()));
            }
        }
    }
    for argument in &action.arguments {
        if argument.contains(|c: char| c.is_control() && c != '\t') {
            return Err(invalid("an argument has a control character".into()));
        }
    }
    if !action.working_directory.is_empty() {
        if action.working_directory.contains(|c: char| c.is_control()) {
            return Err(invalid(
                "the working directory has a control character".into(),
            ));
        }
        if phase == ActionPhase::PostUninstall && names_install_root(&action.working_directory) {
            return Err(invalid(
                "a post-uninstall action cannot work under %INSTALLROOT%, which is removed before it runs".into(),
            ));
        }
    }
    if ActionFailurePolicy::try_from(action.on_failure).is_err() {
        return Err(invalid("unknown failure policy".into()));
    }
    Ok(())
}

fn validate_install_option(option: &InstallOption) -> Result<(), FormatError> {
    let invalid = |message: String| FormatError::new("metadata_invalid", message);
    validate_option_name(&option.name)?;
    let labelled = || {
        option
            .labels
            .get("en-US")
            .is_some_and(|l| !l.trim().is_empty())
    };
    match OptionKind::try_from(option.kind) {
        Ok(OptionKind::Custom) | Ok(OptionKind::Unspecified) => {
            if !labelled() {
                return Err(invalid(format!(
                    "option {} is custom and has no en-US label",
                    option.name
                )));
            }
        }
        Ok(OptionKind::Choice) => {
            if !labelled() {
                return Err(invalid(format!(
                    "option {} is a choice and has no en-US label",
                    option.name
                )));
            }
            if option.choices.len() < 2 {
                return Err(invalid(format!(
                    "option {} is a choice and needs at least two choices",
                    option.name
                )));
            }
            if option.choices.len() > MAX_CHOICES {
                return Err(invalid(format!(
                    "option {} declares more than {MAX_CHOICES} choices",
                    option.name
                )));
            }
            let mut values = std::collections::HashSet::new();
            for choice in &option.choices {
                validate_option_name(&choice.value).map_err(|err| {
                    invalid(format!(
                        "option {}: choice value {:?} is not valid ({})",
                        option.name, choice.value, err.message
                    ))
                })?;
                if parse_bool(&choice.value).is_some() {
                    return Err(invalid(format!(
                        "option {}: choice value {:?} spells a boolean",
                        option.name, choice.value
                    )));
                }
                if !values.insert(choice.value.to_ascii_lowercase()) {
                    return Err(invalid(format!(
                        "option {} declares choice {} twice",
                        option.name, choice.value
                    )));
                }
                if choice
                    .labels
                    .get("en-US")
                    .is_none_or(|l| l.trim().is_empty())
                {
                    return Err(invalid(format!(
                        "option {}: choice {} has no en-US label",
                        option.name, choice.value
                    )));
                }
            }
            if !values.contains(&option.default_choice.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "option {}: default {:?} is not one of its choices",
                    option.name, option.default_choice
                )));
            }
        }
        Ok(_) => {}
        Err(_) => {
            return Err(invalid(format!(
                "option {} has an unknown kind",
                option.name
            )));
        }
    }
    if !option.is_choice() && (!option.choices.is_empty() || !option.default_choice.is_empty()) {
        return Err(invalid(format!(
            "option {} declares choices but is not a choice option",
            option.name
        )));
    }
    Ok(())
}

fn validate_shortcut(shortcut: &Shortcut) -> Result<(), FormatError> {
    let invalid = |message: String| FormatError::new("metadata_invalid", message);
    if ShortcutLocation::try_from(shortcut.location).is_err()
        || shortcut.location == ShortcutLocation::Unspecified as i32
    {
        return Err(invalid(format!(
            "shortcut {:?} has no location",
            shortcut.name
        )));
    }
    crate::identity::validate_name(&shortcut.name)?;
    if shortcut.url.is_empty() {
        validate_relative_path(&shortcut.target)?;
        if !shortcut.working_directory.is_empty() {
            validate_relative_path(&shortcut.working_directory)?;
        }
        if shortcut.app_user_model_id.len() > 128
            || shortcut
                .app_user_model_id
                .contains(|c: char| c.is_whitespace() || c.is_control())
        {
            return Err(invalid(format!(
                "shortcut {:?} has an AppUserModelID Windows does not allow",
                shortcut.name
            )));
        }
    } else {
        validate_url(&shortcut.url)?;
        if !shortcut.target.is_empty()
            || !shortcut.arguments.is_empty()
            || !shortcut.working_directory.is_empty()
            || !shortcut.app_user_model_id.is_empty()
        {
            return Err(invalid(format!(
                "shortcut {:?} opens a URL and cannot also name a target, arguments, a working directory or an AppUserModelID",
                shortcut.name
            )));
        }
    }
    if !shortcut.icon.is_empty() {
        validate_relative_path(&shortcut.icon)?;
    }
    if !shortcut.folder.is_empty() {
        validate_relative_path(&shortcut.folder)?;
    }
    Ok(())
}

fn validate_firewall_rule(rule: &FirewallRule) -> Result<(), FormatError> {
    let invalid = |message: String| {
        FormatError::new(
            "metadata_invalid",
            format!("firewall rule {:?}: {message}", rule.name),
        )
    };
    if rule.name.trim().is_empty()
        || rule.name.len() > 255
        || rule.name.contains(|c: char| c.is_control() || c == '|')
    {
        return Err(invalid(
            "the name is empty, too long or has a character Windows does not allow".into(),
        ));
    }
    validate_relative_path(&rule.program)?;
    if FirewallDirection::try_from(rule.direction)
        .is_ok_and(|d| d == FirewallDirection::Unspecified)
        || FirewallDirection::try_from(rule.direction).is_err()
    {
        return Err(invalid("no direction".into()));
    }
    if FirewallAction::try_from(rule.action).is_ok_and(|a| a == FirewallAction::Unspecified)
        || FirewallAction::try_from(rule.action).is_err()
    {
        return Err(invalid("no action".into()));
    }
    if FirewallProtocol::try_from(rule.protocol).is_err() {
        return Err(invalid("unknown protocol".into()));
    }
    if !rule.local_ports.is_empty() {
        if rule.protocol == FirewallProtocol::Unspecified as i32 {
            return Err(invalid("local ports need a protocol".into()));
        }
        validate_ports(&rule.local_ports).map_err(|why| invalid(why.into()))?;
    }
    Ok(())
}

/// `80`, `8000-8010`, `80,443`: decimal ports and ranges, comma-separated.
pub fn validate_ports(text: &str) -> Result<(), &'static str> {
    for part in text.split(',') {
        let part = part.trim();
        let (low, high) = match part.split_once('-') {
            Some((low, high)) => (low.trim(), high.trim()),
            None => (part, part),
        };
        let parse = |p: &str| p.parse::<u16>().ok().filter(|n| *n > 0);
        match (parse(low), parse(high)) {
            (Some(low), Some(high)) if low <= high => {}
            _ => return Err("local ports are not a list of ports and port ranges"),
        }
    }
    Ok(())
}

/// An environment variable name: no `=`, no control characters, not `Path`
/// (which the PATH resource owns), at most 255 characters.
pub fn validate_environment_name(name: &str) -> Result<(), FormatError> {
    let ok = !name.trim().is_empty()
        && name.len() <= 255
        && !name.contains(|c: char| c == '=' || c.is_control() || c.is_whitespace());
    if !ok {
        return Err(FormatError::new(
            "metadata_invalid",
            format!("environment variable name {name:?} is not valid"),
        ));
    }
    if name.eq_ignore_ascii_case("path") {
        return Err(FormatError::new(
            "metadata_invalid",
            "the PATH variable is managed by [[path]] entries, not as an environment variable",
        ));
    }
    Ok(())
}

/// A ProgID: one registry key component of letters, digits and dots that
/// starts with a letter, e.g. `TigerMarkView.Document`.
pub fn validate_prog_id(prog_id: &str) -> Result<(), FormatError> {
    let ok = !prog_id.is_empty()
        && prog_id.len() <= 39
        && prog_id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && prog_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
        && !prog_id.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!(
                "ProgID {prog_id:?} is not valid (letters, digits, dots; at most 39 characters)"
            ),
        ))
    }
}

/// A file extension with its dot: `.md`, `.tigertest`.
pub fn validate_extension(extension: &str) -> Result<(), FormatError> {
    let ok = extension.len() >= 2
        && extension.len() <= 64
        && extension.starts_with('.')
        && extension[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("file extension {extension:?} is not valid (a dot then letters and digits)"),
        ))
    }
}

/// A URL scheme (RFC 3986): a letter, then letters, digits, `+`, `-`, `.`,
/// lower-case, without the colon; never one Windows itself owns.
pub fn validate_scheme(scheme: &str) -> Result<(), FormatError> {
    let ok = !scheme.is_empty()
        && scheme.len() <= 64
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
        && scheme
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    if !ok {
        return Err(FormatError::new(
            "metadata_invalid",
            format!(
                "URL scheme {scheme:?} is not valid (lower-case letters, digits, '+', '-', '.')"
            ),
        ));
    }
    if matches!(
        scheme,
        "http" | "https" | "file" | "ftp" | "mailto" | "ms-settings" | "ms-windows-store" | "shell"
    ) {
        return Err(FormatError::new(
            "metadata_invalid",
            format!(
                "URL scheme {scheme:?} belongs to Windows or the web; a package registers a scheme of its own"
            ),
        ));
    }
    Ok(())
}

/// A context-menu verb key name: one registry key component without spaces.
pub fn validate_verb(verb: &str) -> Result<(), FormatError> {
    let ok = !verb.is_empty()
        && verb.len() <= 64
        && verb
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("context menu verb {verb:?} is not valid (letters, digits, '-', '_', '.')"),
        ))
    }
}

/// An absolute `http`, `https` or `file` URL for an Internet shortcut.
pub fn validate_url(url: &str) -> Result<(), FormatError> {
    let lower = url.to_ascii_lowercase();
    let ok = (lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("file://"))
        && url.len() <= 2048
        && !url.contains(|c: char| c.is_control() || c.is_whitespace());
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("URL {url:?} is not an absolute http, https or file URL"),
        ))
    }
}

fn validate_dependency(
    dependency: &Dependency,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), FormatError> {
    let invalid = |message: String| {
        FormatError::new(
            "metadata_invalid",
            format!("dependency {:?}: {message}", dependency.id),
        )
    };
    if dependency.id.trim().is_empty() {
        return Err(invalid("no id".into()));
    }
    if !seen.insert(dependency.id.to_ascii_lowercase()) {
        return Err(invalid("declared twice".into()));
    }
    if !dependency.minimum_version.is_empty() {
        validate_dotted_version(&dependency.minimum_version)?;
    }
    let detector = dependency
        .detect
        .as_ref()
        .ok_or_else(|| invalid("no detector".into()))?;
    match DetectorKind::try_from(detector.kind) {
        Ok(DetectorKind::DirectoryVersion) | Ok(DetectorKind::FileVersion) => {
            if detector.path.is_empty() {
                return Err(invalid("the detector needs a path".into()));
            }
        }
        Ok(DetectorKind::RegistryVersion) => {
            if detector.keys.is_empty() || detector.value.is_empty() {
                return Err(invalid(
                    "the registry detector needs keys and a value name".into(),
                ));
            }
        }
        Ok(DetectorKind::Registration) => {
            if detector.pattern.is_empty() {
                return Err(invalid("the registration detector needs a pattern".into()));
            }
        }
        _ => return Err(invalid("unknown detector kind".into())),
    }
    if let Some(acquisition) = &dependency.acquisition {
        match AcquisitionSource::try_from(acquisition.source) {
            Ok(AcquisitionSource::Winget) => {
                if acquisition.package_identifier.is_empty() {
                    return Err(invalid(
                        "WinGet acquisition needs a package identifier".into(),
                    ));
                }
            }
            Ok(AcquisitionSource::Url) => {
                if acquisition.url.is_empty() || !is_sha256_hex(&acquisition.sha256) {
                    return Err(invalid("URL acquisition needs a URL and a SHA-256".into()));
                }
            }
            Ok(AcquisitionSource::Embedded) => {
                if !Metadata::is_dependency_entry(&acquisition.entry)
                    || !is_sha256_hex(&acquisition.sha256)
                    || acquisition.size == 0
                {
                    return Err(invalid(
                        "embedded acquisition needs a dependency payload entry, its SHA-256 and its size"
                            .into(),
                    ));
                }
                if dependency.install.is_none() {
                    return Err(invalid(
                        "an embedded installer needs its unattended switches".into(),
                    ));
                }
            }
            _ => return Err(invalid("unknown acquisition source".into())),
        }
        if !acquisition.sha256.is_empty() && !is_sha256_hex(&acquisition.sha256) {
            return Err(invalid("the acquisition hash is not a SHA-256".into()));
        }
    }
    Ok(())
}

/// Option names are lower-case ASCII words joined by `-`.
pub fn validate_option_name(name: &str) -> Result<(), FormatError> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("option name {name:?} is not valid (lower-case words joined by '-')"),
        ))
    }
}

/// A registry key path relative to a root: backslash-separated non-empty
/// components without control characters.
pub fn validate_registry_key(key: &str) -> Result<(), FormatError> {
    let invalid =
        |why: &str| FormatError::new("metadata_invalid", format!("registry key {key:?} {why}"));
    if key.is_empty() {
        return Err(invalid("is empty"));
    }
    if key.starts_with('\\') || key.ends_with('\\') {
        return Err(invalid("must not start or end with a separator"));
    }
    if key.len() > 512 {
        return Err(invalid("is too long"));
    }
    for component in key.split('\\') {
        if component.trim().is_empty() {
            return Err(invalid("has an empty component"));
        }
        if component.contains(|c: char| c.is_control()) || component.len() > 255 {
            return Err(invalid("has a component Windows does not allow"));
        }
    }
    Ok(())
}

/// An Add/Remove Programs key name: one registry key component.
pub fn validate_registration_key_name(name: &str) -> Result<(), FormatError> {
    if name.contains('\\') {
        return Err(FormatError::new(
            "metadata_invalid",
            format!("registration key name {name:?} must be one key component"),
        ));
    }
    validate_registry_key(name)
}

/// A dependency version: one to four decimal components.
pub fn validate_dotted_version(version: &str) -> Result<(), FormatError> {
    let parts: Vec<&str> = version.split('.').collect();
    let ok = (1..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.chars().all(|c| c.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("version {version:?} is not a dotted numeric version"),
        ))
    }
}

/// Lower-case hex of 32 bytes.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Install-relative paths use `/`, never start with a separator, never contain
/// `.` or `..` components, drive letters or characters Windows forbids.
pub fn validate_relative_path(path: &str) -> Result<(), FormatError> {
    let invalid = |why: &str| FormatError::new("metadata_invalid", format!("path {path:?} {why}"));
    if path.is_empty() {
        return Err(invalid("is empty"));
    }
    if path.contains('\\') {
        return Err(invalid("must use forward slashes"));
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Err(invalid("must not start or end with a separator"));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid("has an empty, current or parent component"));
        }
        if component.contains(|c: char| {
            matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
        }) {
            return Err(invalid(
                "contains a character Windows does not allow in file names",
            ));
        }
        if component.ends_with(' ') || component.ends_with('.') {
            return Err(invalid("has a component ending in a space or a dot"));
        }
        let stem = component.split('.').next().unwrap_or(component);
        if is_reserved_device_name(stem) {
            return Err(invalid("uses a reserved Windows device name"));
        }
    }
    Ok(())
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, case-insensitive:
/// Windows resolves these to devices whatever the extension.
fn is_reserved_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

impl Scope {
    pub fn from_tag(tag: i32) -> Option<Scope> {
        match ScopeTag::try_from(tag).ok()? {
            ScopeTag::User => Some(Scope::User),
            ScopeTag::Machine => Some(Scope::Machine),
            ScopeTag::Unspecified => None,
        }
    }

    pub fn tag(self) -> i32 {
        match self {
            Scope::User => ScopeTag::User as i32,
            Scope::Machine => ScopeTag::Machine as i32,
        }
    }
}

impl ExistingScopePolicy {
    /// The manifest spelling of a policy, which machine-readable output
    /// carries too.
    pub fn as_str(self) -> &'static str {
        match self {
            ExistingScopePolicy::AllowParallel => "allow-parallel",
            ExistingScopePolicy::Error => "error",
            ExistingScopePolicy::Preserve | ExistingScopePolicy::Unspecified => "preserve",
        }
    }

    /// The policy a manifest names.
    pub fn parse(text: &str) -> Option<ExistingScopePolicy> {
        match text {
            "preserve" => Some(ExistingScopePolicy::Preserve),
            "allow-parallel" => Some(ExistingScopePolicy::AllowParallel),
            "error" => Some(ExistingScopePolicy::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `metadata` with its file batches computed for its file list.
    pub(crate) fn batched(mut metadata: Metadata) -> Metadata {
        metadata.file_batches = file_batches(&metadata.files);
        metadata
    }

    pub(crate) fn sample() -> Metadata {
        batched(Metadata {
            schema: SCHEMA,
            package: Some(Package {
                id: "IT-Tiger.Sample".into(),
                name: "Sample".into(),
                version: "1.0.0".into(),
                publisher: "IT Tiger".into(),
                ..Default::default()
            }),
            install: Some(Install {
                scopes: vec![Scope::User.tag()],
                user_root: "%LOCALAPPDATA%\\Programs\\Sample".into(),
                ..Default::default()
            }),
            files: vec![File {
                path: "bin/app.exe".into(),
                size: 3,
                entry: "bin/app.exe".into(),
                when: None,
            }],
            directories: vec![Directory { path: "bin".into() }],
            engine: Some(Engine {
                tigersetup_version: "0.1.0".into(),
                engine_sha256: "00".into(),
                engine_block_sha256: "00".into(),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    /// The completion page may only offer a program the package installs,
    /// in a directory that exists once the run has committed.
    #[test]
    fn a_launch_names_an_installed_exe_and_an_installed_working_directory() {
        let with = |launch: Launch| {
            let mut metadata = sample();
            metadata.launch = Some(launch);
            metadata.validate()
        };
        let launch = Launch {
            executable: "bin/app.exe".into(),
            arguments: vec!["--from-setup".into(), "%INSTALLROOT%\\data".into()],
            working_directory: None,
            checked: true,
        };
        with(launch.clone()).unwrap();
        // Paths compare as Windows compares them.
        with(Launch {
            executable: "BIN/App.EXE".into(),
            working_directory: Some("Bin".into()),
            ..launch.clone()
        })
        .unwrap();
        // Empty is the install root, which always exists.
        with(Launch {
            working_directory: Some(String::new()),
            ..launch.clone()
        })
        .unwrap();

        let message = |launch: Launch| with(launch).unwrap_err().message;
        assert!(
            message(Launch {
                executable: "bin/other.exe".into(),
                ..launch.clone()
            })
            .contains("not a file the package installs")
        );
        let mut script = sample();
        script.files[0].path = "bin/app.cmd".into();
        script.files[0].entry = "bin/app.cmd".into();
        script.launch = Some(Launch {
            executable: "bin/app.cmd".into(),
            ..launch.clone()
        });
        assert!(
            script
                .validate()
                .unwrap_err()
                .message
                .contains("is not an .exe")
        );
        assert!(
            message(Launch {
                working_directory: Some("data".into()),
                ..launch.clone()
            })
            .contains("not a directory the package installs")
        );
        assert!(
            with(Launch {
                executable: "../bin/app.exe".into(),
                ..launch
            })
            .is_err()
        );
    }

    fn files_of(sizes: &[u64]) -> Vec<File> {
        sizes
            .iter()
            .enumerate()
            .map(|(i, size)| File {
                path: format!("f{i}"),
                size: *size,
                entry: format!("f{i}"),
                when: None,
            })
            .collect()
    }

    fn ranges(batches: &[FileBatch]) -> Vec<(u32, u32, u64)> {
        batches
            .iter()
            .map(|b| (b.first_file, b.file_count, b.bytes))
            .collect()
    }

    #[test]
    fn a_batch_closes_at_the_file_limit_and_the_next_file_opens_another() {
        let files = files_of(&vec![1; 256]);
        assert_eq!(ranges(&file_batches(&files)), vec![(0, 256, 256)]);
        let files = files_of(&vec![1; 257]);
        assert_eq!(
            ranges(&file_batches(&files)),
            vec![(0, 256, 256), (256, 1, 1)]
        );
        assert!(file_batches(&[]).is_empty());
    }

    #[test]
    fn a_batch_closes_before_the_file_that_would_overflow_the_byte_limit() {
        let mib = 1024 * 1024;
        // 31 MiB + 1 MiB fits exactly; the next byte does not.
        let files = files_of(&[31 * mib, mib, 1]);
        assert_eq!(
            ranges(&file_batches(&files)),
            vec![(0, 2, 32 * mib), (2, 1, 1)]
        );
        let files = files_of(&[31 * mib, mib + 1, 1]);
        assert_eq!(
            ranges(&file_batches(&files)),
            vec![(0, 1, 31 * mib), (1, 2, mib + 2)]
        );
    }

    #[test]
    fn a_file_larger_than_the_byte_limit_is_a_batch_of_its_own() {
        let mib = 1024 * 1024;
        let files = files_of(&[5, 40 * mib, 5, 33 * mib, 7]);
        assert_eq!(
            ranges(&file_batches(&files)),
            vec![
                (0, 1, 5),
                (1, 1, 40 * mib),
                (2, 1, 5),
                (3, 1, 33 * mib),
                (4, 1, 7)
            ]
        );
    }

    #[test]
    fn batches_are_deterministic_and_are_validated_against_the_rule() {
        let files = files_of(&[3, 4, 5]);
        assert_eq!(file_batches(&files), file_batches(&files.clone()));
        let mut metadata = sample();
        metadata.files = files;
        metadata.directories.clear();
        metadata.file_batches = file_batches(&metadata.files);
        metadata.validate().unwrap();

        let mut split = metadata.clone();
        split.file_batches = vec![
            FileBatch {
                first_file: 0,
                file_count: 1,
                bytes: 3,
            },
            FileBatch {
                first_file: 1,
                file_count: 2,
                bytes: 9,
            },
        ];
        assert!(
            split
                .validate()
                .unwrap_err()
                .message
                .contains("could have taken")
        );

        let mut short = metadata.clone();
        short.files[1].size = 40 * 1024 * 1024;
        short.file_batches = file_batches(&short.files);
        short.validate().unwrap();
        short.file_batches.pop();
        assert!(
            short
                .validate()
                .unwrap_err()
                .message
                .contains("cover 2 of 3")
        );

        let mut wrong_bytes = metadata.clone();
        wrong_bytes.file_batches[0].bytes += 1;
        assert!(
            wrong_bytes
                .validate()
                .unwrap_err()
                .message
                .contains("declares")
        );

        let mut none = metadata.clone();
        none.file_batches.clear();
        assert!(none.validate().is_err());

        let mut gap = metadata.clone();
        gap.file_batches = vec![FileBatch {
            first_file: 1,
            file_count: 2,
            bytes: 9,
        }];
        assert!(
            gap.validate()
                .unwrap_err()
                .message
                .contains("expected to start at 0")
        );

        assert_eq!(metadata.batch_of_file(2), Some(0));
        assert_eq!(metadata.batch_of_file(3), None);
    }

    #[test]
    fn a_payload_index_must_describe_one_contiguous_stream() {
        let mut metadata = sample();
        metadata.payload = vec![PayloadEntry {
            entry: "bin/app.exe".into(),
            offset: 0,
            length: 3,
            crc32: 0,
            sha256: "ab".repeat(32),
        }];
        metadata.validate().unwrap();

        let mut gap = metadata.clone();
        gap.payload[0].offset = 1;
        assert!(gap.validate().unwrap_err().message.contains("starts at 1"));

        let mut wrong_length = metadata.clone();
        wrong_length.payload[0].length = 4;
        assert!(
            wrong_length
                .validate()
                .unwrap_err()
                .message
                .contains("4 bytes in the index")
        );

        let mut unreferenced = metadata.clone();
        unreferenced.payload[0].entry = "other".into();
        assert!(
            unreferenced
                .validate()
                .unwrap_err()
                .message
                .contains("not in the payload index")
        );

        let mut twice = metadata.clone();
        twice.payload.push(PayloadEntry {
            entry: "bin/app.exe".into(),
            offset: 3,
            length: 0,
            crc32: 0,
            sha256: "ab".repeat(32),
        });
        assert!(twice.validate().unwrap_err().message.contains("twice"));
    }

    /// The engine installs the declared directories rather than deriving them
    /// again from the file paths, so metadata that does not carry a file's
    /// parents is invalid rather than merely incomplete.
    #[test]
    fn a_file_whose_parent_directory_is_not_declared_is_refused() {
        let mut metadata = sample();
        metadata.files.push(File {
            path: "bin/x64/native.dll".into(),
            size: 3,
            entry: "bin/x64/native.dll".into(),
            when: None,
        });
        let mut metadata = batched(metadata);
        let error = metadata.validate().unwrap_err().to_string();
        assert!(error.contains("bin/x64"), "{error}");

        metadata.directories.push(Directory {
            path: "bin/x64".into(),
        });
        metadata.validate().unwrap();
    }

    #[test]
    fn resources_and_dependencies_are_validated() {
        let mut metadata = sample();
        metadata.options.push(InstallOption {
            name: "path".into(),
            default: true,
            kind: OptionKind::Path as i32,
            ..Default::default()
        });
        metadata.path_entries.push(PathEntry {
            path: String::new(),
            option: "path".into(),
            when: None,
        });
        metadata.shortcuts.push(Shortcut {
            location: ShortcutLocation::StartMenu as i32,
            name: "Sample".into(),
            target: "bin/app.exe".into(),
            ..Default::default()
        });
        metadata.registry_values.push(RegistryValue {
            key: "IT Tiger\\Sample".into(),
            name: "InstallRoot".into(),
            kind: RegistryKind::ExpandString as i32,
            data: "%INSTALLROOT%".into(),
            when: None,
            root: 0,
        });
        metadata.dependencies.push(Dependency {
            id: "Vendor.Runtime".into(),
            minimum_version: "10.0".into(),
            detect: Some(Detector {
                kind: DetectorKind::DirectoryVersion as i32,
                path: "%PROGRAMFILES%\\runtime".into(),
                pattern: "10.*".into(),
                ..Default::default()
            }),
            acquisition: Some(Acquisition {
                source: AcquisitionSource::Winget as i32,
                package_identifier: "Vendor.Runtime".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        metadata.validate().unwrap();
        assert_eq!(
            metadata.option_default("PATH"),
            Some(OptionValue::Bool(true))
        );
        assert_eq!(metadata.registration_key_name(), "IT-Tiger.Sample");
        assert_eq!(metadata.minimum_build(), ENGINE_MINIMUM_BUILD);

        let mut undeclared = metadata.clone();
        undeclared.path_entries[0].option = "nope".into();
        assert!(undeclared.validate().is_err());
        let mut no_detector = metadata.clone();
        no_detector.dependencies[0].detect = None;
        assert!(no_detector.validate().is_err());
        let mut bad_hash = metadata.clone();
        bad_hash.dependencies[0].acquisition = Some(Acquisition {
            source: AcquisitionSource::Url as i32,
            url: "https://example.invalid/x.exe".into(),
            sha256: "zz".into(),
            ..Default::default()
        });
        assert!(bad_hash.validate().is_err());
        let mut uninstaller = metadata.clone();
        uninstaller.role = Role::Uninstaller as i32;
        assert!(uninstaller.validate().is_err());
        uninstaller.uninstaller_scope = Scope::User.tag();
        uninstaller.validate().unwrap();
        assert!(uninstaller.is_uninstaller());

        // An explicit registry root is a hive, and the package's only scope
        // must be the one that writes it; the sample allows user scope.
        let mut explicit = metadata.clone();
        explicit.registry_values[0].root = RegistryRoot::LocalMachine as i32;
        assert!(explicit.registry_values[0].explicit_root().is_some());
        assert_eq!(explicit.registry_values[0].root_name(), "HKLM");
        let err = explicit.validate().unwrap_err();
        assert!(err.message.contains("HKLM"), "{}", err.message);
        explicit.install.as_mut().unwrap().machine_root = "%PROGRAMFILES%\\Sample".into();
        explicit.install.as_mut().unwrap().scopes = vec![Scope::Machine.tag()];
        explicit.validate().unwrap();
        explicit.install.as_mut().unwrap().scopes = vec![Scope::Machine.tag(), Scope::User.tag()];
        assert!(explicit.validate().is_err());
        explicit.registry_values[0].root = RegistryRoot::CurrentUser as i32;
        assert!(explicit.validate().is_err());
        explicit.install.as_mut().unwrap().scopes = vec![Scope::User.tag()];
        explicit.validate().unwrap();
        explicit.registry_values[0].root = 7;
        assert_eq!(explicit.registry_values[0].explicit_root(), Some(Err(7)));
        assert!(
            explicit
                .validate()
                .unwrap_err()
                .message
                .contains("unknown root")
        );
        assert_eq!(
            RegistryValue::root_of("hkcu"),
            Some(RegistryRoot::CurrentUser)
        );
        assert_eq!(
            RegistryValue::root_of("HKEY_LOCAL_MACHINE"),
            Some(RegistryRoot::LocalMachine)
        );
        assert_eq!(RegistryValue::root_of("HKCR"), None);
    }

    fn packaged_action(name: &str, phase: ActionPhase, kind: ActionKind, file: &str) -> Action {
        Action {
            name: name.into(),
            phase: phase as i32,
            run_on: phase
                .default_operations()
                .iter()
                .map(|op| *op as i32)
                .collect(),
            kind: kind as i32,
            entry: format!("{ACTION_ENTRY_PREFIX}{file}"),
            file_name: file.into(),
            size: 10,
            sha256: "ab".repeat(32),
            ..Default::default()
        }
    }

    /// Every rule the action declaration is held to: identity, phase,
    /// the operations a phase allows, one program source, a file the kind
    /// runs, a packaged file's identity, and the post-uninstall rule that
    /// the install root is already gone.
    #[test]
    fn actions_are_validated() {
        let mut metadata = sample();
        metadata.options.push(InstallOption {
            name: "cache".into(),
            default: true,
            kind: OptionKind::Custom as i32,
            labels: [("en-US".to_string(), "Cache".to_string())].into(),
            ..Default::default()
        });
        metadata.actions.push(packaged_action(
            "build-cache",
            ActionPhase::PostInstall,
            ActionKind::Powershell,
            "build-cache.ps1",
        ));
        metadata.actions.push(Action {
            name: "notify".into(),
            phase: ActionPhase::PreUninstall as i32,
            run_on: vec![ActionOperation::Uninstall as i32],
            kind: ActionKind::Exe as i32,
            command: "%INSTALLROOT%\\app.exe".into(),
            arguments: vec!["--bye".into()],
            when: Some(Predicate {
                option: "cache".into(),
                equals: "true".into(),
            }),
            ..Default::default()
        });
        metadata.validate().unwrap();
        let action = &metadata.actions[0];
        assert_eq!(action.phase(), ActionPhase::PostInstall);
        assert_eq!(action.kind(), ActionKind::Powershell);
        assert_eq!(action.failure_policy(), ActionFailurePolicy::Fail);
        assert_eq!(action.timeout_seconds(), DEFAULT_ACTION_TIMEOUT_SECONDS);
        assert_eq!(action.success_codes(), vec![0]);
        assert!(action.runs_on(ActionOperation::Upgrade));
        assert!(!action.runs_on(ActionOperation::Repair), "repair is opt-in");
        assert!(action.is_packaged());
        assert_eq!(action.program(), "build-cache.ps1");

        let refused = |mutate: &dyn Fn(&mut Action), needle: &str| {
            let mut metadata = metadata.clone();
            mutate(&mut metadata.actions[0]);
            let error = metadata.validate().unwrap_err().to_string();
            assert!(error.contains(needle), "{error}");
        };
        refused(&|a| a.name = "Build Cache".into(), "not valid");
        refused(&|a| a.phase = 0, "no phase");
        refused(&|a| a.kind = 0, "no kind");
        refused(&|a| a.run_on.clear(), "run_on is empty");
        refused(&|a| a.run_on.push(ActionOperation::Upgrade as i32), "twice");
        refused(
            &|a| a.run_on = vec![ActionOperation::Uninstall as i32],
            "cannot run on uninstall",
        );
        refused(&|a| a.command = "x.ps1".into(), "both a command");
        refused(&|a| a.entry = String::new(), "neither a command");
        refused(
            &|a| a.file_name = "build-cache.cmd".into(),
            "not a file a powershell action runs",
        );
        refused(&|a| a.entry = "bin/build-cache.ps1".into(), "not under");
        refused(&|a| a.size = 0, "empty");
        refused(&|a| a.sha256 = "nope".into(), "no SHA-256");
        refused(
            &|a| {
                a.when = Some(Predicate {
                    option: "missing".into(),
                    equals: "true".into(),
                })
            },
            "not declared",
        );
        refused(&|a| a.name = "notify".into(), "declared twice");
        let mut post = metadata.clone();
        post.actions[1].phase = ActionPhase::PostUninstall as i32;
        let error = post.validate().unwrap_err().to_string();
        assert!(error.contains("%INSTALLROOT%"), "{error}");
        let mut uninstall_on_install = metadata.clone();
        uninstall_on_install.actions[1].run_on = vec![ActionOperation::Install as i32];
        let error = uninstall_on_install.validate().unwrap_err().to_string();
        assert!(error.contains("cannot run on install"), "{error}");
        // A product file may not use the reserved directory.
        let mut reserved = metadata.clone();
        reserved.files[0].path = format!("{ACTION_ENTRY_PREFIX}x.exe");
        reserved.files[0].entry = reserved.files[0].path.clone();
        assert!(reserved.validate().is_err());
    }

    #[test]
    fn metadata_round_trips() {
        let metadata = sample();
        let bytes = metadata.encode_to_vec();
        assert_eq!(Metadata::decode_block(&bytes).unwrap(), metadata);
    }

    #[test]
    fn unknown_schema_is_refused() {
        let mut metadata = sample();
        metadata.schema = SCHEMA + 1;
        assert_eq!(
            Metadata::decode_block(&metadata.encode_to_vec())
                .unwrap_err()
                .code,
            "metadata_unsupported"
        );
    }

    #[test]
    fn bad_paths_are_refused() {
        for bad in [
            "",
            "/abs",
            "a\\b",
            "a/../b",
            "con:",
            "trailing.",
            "x/",
            "nul.txt",
            "COM1",
            "dir/aux",
            "Lpt9.log",
        ] {
            assert!(
                validate_relative_path(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
        for good in [
            "a",
            "a/b",
            "a b/c.d",
            "x.y/z",
            "com10",
            "console.log",
            "nulls",
        ] {
            assert!(
                validate_relative_path(good).is_ok(),
                "{good:?} should be valid"
            );
        }
    }

    /// The recorded acceptance names the text a person read, byte for byte:
    /// an annual copyright edit is a new text, and no text is no agreement.
    #[test]
    fn the_licence_identity_is_the_hash_of_the_exact_text() {
        let mut metadata = sample();
        assert_eq!(metadata.license_sha256(), None);
        metadata.package.as_mut().unwrap().license_text = " \r\n\t".into();
        assert_eq!(metadata.license_sha256(), None, "blank text is no licence");
        let text = "MIT License\r\n\r\nCopyright (c) 2026 IT Tiger\r\n";
        metadata.package.as_mut().unwrap().license_text = text.into();
        let expected = crate::hex(&crate::sha256(text.as_bytes()));
        assert_eq!(
            metadata.license_sha256().as_deref(),
            Some(expected.as_str())
        );
        metadata.package.as_mut().unwrap().license_text = text.replace("2026", "2027");
        assert_ne!(
            metadata.license_sha256().as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn scope_lookup_respects_declared_scopes() {
        let metadata = sample();
        assert_eq!(
            metadata.install_root_template(Scope::User),
            Some("%LOCALAPPDATA%\\Programs\\Sample")
        );
        assert_eq!(metadata.install_root_template(Scope::Machine), None);
    }
}
