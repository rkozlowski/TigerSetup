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
    Acquisition, AcquisitionSource, AppPath, ContextMenuTarget, ContextMenuVerb, Dependency,
    DependencyInstall, Detector, DetectorKind, Directory, Engine, EnvironmentVariable,
    ExistingScopePolicy, File, FileAssociation, FirewallAction, FirewallDirection,
    FirewallProtocol, FirewallRule, Install, InstallOption, Legacy, Metadata, OptionChoice,
    OptionKind, Package, PathEntry, Predicate, Registration, RegistryKind, RegistryValue, Role,
    Scope as ScopeTag, Shortcut, ShortcutLocation, UrlProtocol,
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

/// The Windows build the engine itself requires (Windows 10 1809 / Server
/// 2019); a package may raise it, never lower it.
pub const ENGINE_MINIMUM_BUILD: u32 = 17763;

/// The metadata major version this crate understands.
pub const SCHEMA: u32 = 1;

/// The most values a choice option may declare. The wizard shows a choice
/// as a heading and one radio button per value on a page of nine rows, so
/// a choice always fits on one page.
pub const MAX_CHOICES: usize = 8;

/// The payload entries an embedded dependency installer travels as:
/// `.tigersetup/dependencies/<file name>`. The component cannot be an
/// install-relative product path, because a product file may not start with
/// this directory (the builder refuses one), so the two never collide.
pub const DEPENDENCY_ENTRY_PREFIX: &str = ".tigersetup/dependencies/";

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
            if Metadata::is_dependency_entry(&file.path)
                || Metadata::is_dependency_entry(&file.entry)
            {
                return Err(invalid(format!(
                    "file {} lies in the payload directory reserved for embedded dependencies",
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
        if Role::try_from(self.role).is_err() {
            return Err(invalid(format!("unknown role {}", self.role)));
        }
        if self.is_uninstaller() && self.served_scope().is_none() {
            return Err(invalid("an uninstaller names no scope".into()));
        }
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
        Ok(())
    }
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
mod tests {
    use super::*;

    pub(crate) fn sample() -> Metadata {
        Metadata {
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
            }),
            ..Default::default()
        }
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
        metadata.schema = 2;
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
