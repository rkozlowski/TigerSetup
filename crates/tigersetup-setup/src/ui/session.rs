//! What the wizard is about to do, worked out once before the window opens:
//! which flow this run is, which pages that flow shows, and every product
//! string and default the pages need.
//!
//! Everything here comes from the metadata and from the engine's own
//! `inspect` report and scope resolution. The wizard never inspects the
//! machine itself.

use std::path::PathBuf;

use tigersetup_engine::elevation::Intent;
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::format::metadata::{Launch, OptionKind, OptionValue};
use tigersetup_engine::report::Outcome;
use tigersetup_engine::resource::predicate::Options;
use tigersetup_engine::target::{self, Decision, ExistingInstallation};
use tigersetup_engine::{Package, RunOptions};

use super::layout;
use super::text::Text;
use crate::Operation;

/// The pages a flow can show, in the order a flow shows them. The options
/// are shown on as many pages as their rows need (`layout::OPTION_ROWS`
/// per page), numbered from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Scope,
    License,
    Destination,
    Options(usize),
    Ready,
    Confirm,
    Progress,
    Finish,
}

/// What this run is, which decides the wording of the Ready page and which
/// pages exist at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Install,
    Upgrade,
    Reinstall,
    Repair,
    Uninstall,
}

/// What the scope page is for in this run. It is the page whose Next raises
/// the elevation prompt, so every way a run's scope gets decided by the
/// person passes through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopePage {
    /// A first install of a package that offers both scopes: the choice.
    Choose,
    /// The one installation this run continues with; the page says which,
    /// instead of offering a choice that would create a second one.
    Existing(ExistingInstallation),
    /// Both scopes hold an installation, and each is its own. The person
    /// names the one this run is about, and the run continues in a new
    /// process for that scope — elevated where that scope needs it.
    Select(Vec<ExistingInstallation>),
}

/// How one declared option is shown: a check box, or a heading with one
/// radio button per choice.
pub enum OptionControl {
    Check {
        checked: bool,
    },
    Choice {
        /// `(value, label)` in declaration order.
        choices: Vec<(String, String)>,
        selected: usize,
    },
}

/// One declared installer option as the options page shows it.
pub struct OptionRow {
    /// The declared name, lower-case, as the engine and the command line
    /// know it.
    pub name: String,
    pub label: String,
    pub control: OptionControl,
    /// The options page the row is on, and its first row on that page.
    pub page: usize,
    pub row: usize,
}

impl OptionRow {
    /// The rows the option takes on its page: one for a check box, a
    /// heading plus one per choice for a choice.
    pub fn rows(&self) -> usize {
        match &self.control {
            OptionControl::Check { .. } => 1,
            OptionControl::Choice { choices, .. } => 1 + choices.len(),
        }
    }

    /// The value the row starts with.
    pub fn initial_value(&self) -> OptionValue {
        match &self.control {
            OptionControl::Check { checked } => OptionValue::Bool(*checked),
            OptionControl::Choice { choices, selected } => OptionValue::Choice(
                choices
                    .get(*selected)
                    .map(|(value, _)| value.clone())
                    .unwrap_or_default(),
            ),
        }
    }
}

/// Places the rows on pages, in order, filling each page to
/// `layout::OPTION_ROWS` rows; an option never straddles two pages.
/// Returns how many pages were used.
pub fn paginate(rows: &mut [OptionRow]) -> usize {
    let mut page = 0;
    let mut used = 0;
    for row in rows.iter_mut() {
        let needed = row.rows().min(layout::OPTION_ROWS);
        if used + needed > layout::OPTION_ROWS && used > 0 {
            page += 1;
            used = 0;
        }
        row.page = page;
        row.row = used;
        used += needed;
    }
    if rows.is_empty() { 0 } else { page + 1 }
}

pub struct Session {
    /// The package file (see `ui::Request::exe`).
    pub exe: PathBuf,
    pub operation: Operation,
    pub kind: FlowKind,
    pub pages: Vec<Page>,
    pub product_name: String,
    pub product_version: String,
    pub icon_bytes: Vec<u8>,
    pub license_text: String,
    pub scope: Scope,
    /// Why the scope page is shown, when it is.
    pub scope_page: Option<ScopePage>,
    /// A run the engine has already refused — a scope conflict it will not
    /// resolve on its own — shown on the completion page without starting
    /// anything.
    pub refusal: Option<Outcome>,
    /// Whether choosing machine scope would need an administrator, as the
    /// engine's elevation module decides it — not "the scope says machine",
    /// but whether this process can write where that scope keeps its state
    /// and its files. Settled once, here, where the package is at hand.
    pub machine_needs_elevation: bool,
    pub install_root: PathBuf,
    /// Where a fresh installation of each offered scope would go by default.
    ///
    /// The scope page and the destination page describe one decision: for all
    /// users means Program Files, for me only means the user's own folder. A
    /// destination the person has not edited follows the scope they choose, so
    /// the two pages cannot disagree — and a destination they *have* edited is
    /// theirs and is left alone.
    pub default_root_for_scope: Vec<(Scope, PathBuf)>,
    pub estimated_size: u64,
    pub options: Vec<OptionRow>,
    /// How many options pages the rows take.
    pub option_pages: usize,
    /// Options the command line set explicitly; they win over the pages.
    pub explicit_options: Options,
    pub absent_dependencies: Vec<String>,
    pub installed_version: Option<String>,
    /// What the completion page offers to start once the run has ended
    /// installed (`TigerSetup-Design.md` §11.7): the package's declaration,
    /// on the flows that install — never a repair, never an uninstall.
    pub launch: Option<Launch>,
    pub base: RunOptions,
    /// The command line an elevated relaunch should run, without the scope
    /// and language the wizard appends.
    pub relaunch_arguments: Vec<String>,
}

impl Session {
    pub fn has(&self, page: Page) -> bool {
        self.pages.contains(&page)
    }

    pub fn lang(&self) -> &str {
        &self.base.lang
    }

    pub fn title_key(&self) -> &'static str {
        match self.operation {
            Operation::Uninstall => "ui.title.uninstall",
            _ => "ui.title.install",
        }
    }

    /// Works out the flow for this package, this command and this machine.
    pub fn build(
        exe: PathBuf,
        package: &Package,
        operation: Operation,
        mut base: RunOptions,
        scope_given: bool,
        relaunch_arguments: Vec<String>,
    ) -> Session {
        let metadata = package.metadata();
        let declared = metadata.package();
        let text = Text::new(&base.lang, &declared.name);

        // Which installation this run is about is the engine's decision. A
        // scope the command line named is held to the package's policy; one
        // it did not name follows the installation the machine holds. What
        // the engine will not decide alone — two installations — the person
        // decides on the scope page; what it refuses is shown as refused.
        let intent = match operation {
            Operation::Install => Some(Intent::Install),
            Operation::Uninstall => Some(Intent::Uninstall),
            Operation::Repair => Some(Intent::Repair),
            Operation::Verify | Operation::Inspect => None,
        };
        let requested = scope_given.then_some(base.scope);
        let resolution = target::resolve(package, requested, intent).ok();
        if let Some(scope) = resolution.as_ref().and_then(target::Resolution::scope) {
            base.scope = scope;
        }
        let select = match &resolution {
            Some(resolution) if !scope_given && resolution.decision == Decision::Ambiguous => {
                Some(resolution.existing.clone())
            }
            _ => None,
        };
        let refusal = match (&resolution, &select) {
            (Some(resolution), None) => resolution.outcome(package, &base),
            _ => None,
        };
        let continuing = resolution
            .as_ref()
            .and_then(target::Resolution::installed)
            .cloned();

        let report = tigersetup_engine::inspect(package, &base).ok();
        let installation = report
            .as_ref()
            .and_then(|report| report.installation.clone());
        // The recorded values, as the report carries them: a boolean as a
        // boolean, a choice as its value.
        let recorded: Options = report
            .as_ref()
            .and_then(|report| report.owned.as_ref())
            .map(|owned| {
                owned
                    .options
                    .iter()
                    .filter_map(|(name, value)| {
                        let value = match value {
                            serde_json::Value::Bool(b) => OptionValue::Bool(*b),
                            serde_json::Value::String(s) => OptionValue::Choice(s.clone()),
                            _ => return None,
                        };
                        Some((name.clone(), value))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let absent_dependencies = report
            .as_ref()
            .map(|report| {
                report
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.status == "absent")
                    .map(|dependency| dependency.name.clone())
                    .collect()
            })
            .unwrap_or_default();

        let kind = match (operation, &installation) {
            (Operation::Uninstall, _) => FlowKind::Uninstall,
            (Operation::Repair, _) => FlowKind::Repair,
            (_, None) => FlowKind::Install,
            (_, Some(installed)) if installed.version == declared.version => FlowKind::Reinstall,
            (_, Some(_)) => FlowKind::Upgrade,
        };

        // The licence page asks for the text this package carries, once: it
        // is shown while the installation records no acceptance of exactly
        // that text — a first install, an installation nobody accepted a
        // licence for, or a text that changed by as much as a byte — and
        // skipped once a person has accepted it and a run has committed.
        let license_sha256 = metadata.license_sha256();
        let asks_license = license_sha256.is_some()
            && installation
                .as_ref()
                .and_then(|installed| installed.accepted_license_sha256.as_ref())
                != license_sha256.as_ref();

        // The page starts from the same precedence the engine applies:
        // an explicit value, then the last committed one, then the default.
        // A recorded value the option no longer takes falls back to the
        // default, as the engine's own resolution does.
        let mut options: Vec<OptionRow> = metadata
            .options
            .iter()
            .map(|option| {
                let name = option.name.to_ascii_lowercase();
                let value = base
                    .options
                    .get(&name)
                    .or_else(|| recorded.get(&name))
                    .and_then(|value| OptionValue::from_text(option, &value.as_text()))
                    .unwrap_or_else(|| option.default_value());
                let control = if option.is_choice() {
                    let choices: Vec<(String, String)> = option
                        .choices
                        .iter()
                        .map(|choice| (choice.value.clone(), choice_label(choice, &text)))
                        .collect();
                    let selected = choices
                        .iter()
                        .position(|(candidate, _)| OptionValue::Choice(candidate.clone()) == value)
                        .unwrap_or(0);
                    OptionControl::Choice { choices, selected }
                } else {
                    OptionControl::Check {
                        checked: value.is_on(),
                    }
                };
                OptionRow {
                    label: option_label(option, &text),
                    name,
                    control,
                    page: 0,
                    row: 0,
                }
            })
            .collect();
        let option_pages = paginate(&mut options);

        // What a fresh installation of each offered scope would use, asked of
        // the engine rather than assembled here, so the wizard shows the very
        // path the run will take.
        let default_root_for_scope: Vec<(Scope, PathBuf)> = metadata
            .scopes()
            .iter()
            .filter_map(|scope| {
                let mut probe = base.clone();
                probe.scope = *scope;
                probe.install_root = None;
                tigersetup_engine::resolve_roots(package, &probe)
                    .ok()
                    .map(|roots| (*scope, roots.install_root))
            })
            .collect();

        let install_root = match &installation {
            Some(installed) => PathBuf::from(&installed.install_root),
            None => base
                .install_root
                .clone()
                .or_else(|| default_root_of(&default_root_for_scope, base.scope))
                .unwrap_or_default(),
        };

        let mut pages = Vec::new();
        let mut scope_page = None;
        // A run the engine cannot even describe — an unsupported scope, an
        // unreadable state directory — has no choices to offer: it runs and
        // reports what the engine says. One it has refused already is not
        // started at all.
        let describable = report.is_some();
        // The scope page belongs to a package that offers both scopes, in a
        // run whose command line named neither: a first install chooses, a
        // rerun is told which installation it continues with.
        let scope_undecided = metadata.scopes().len() > 1 && !scope_given;
        if let Some(installations) = select {
            scope_page = Some(ScopePage::Select(installations));
            pages.push(Page::Scope);
        } else if refusal.is_some() {
            // Nothing to run: the completion page shows the refusal.
        } else {
            match kind {
                FlowKind::Uninstall => {
                    if describable && installation.is_some() {
                        pages.push(Page::Confirm);
                    }
                }
                FlowKind::Repair => {
                    if describable {
                        pages.push(Page::Ready);
                    }
                }
                FlowKind::Install if describable => {
                    if scope_undecided {
                        scope_page = Some(ScopePage::Choose);
                        pages.push(Page::Scope);
                    }
                    if asks_license {
                        pages.push(Page::License);
                    }
                    pages.push(Page::Destination);
                    pages.extend((0..option_pages).map(Page::Options));
                    pages.push(Page::Ready);
                }
                FlowKind::Upgrade | FlowKind::Reinstall if describable => {
                    // Scope and destination belong to the installation already
                    // on the machine; an upgrade never moves it. The page says
                    // so where the person might otherwise expect the choice.
                    if scope_undecided && let Some(existing) = continuing.clone() {
                        scope_page = Some(ScopePage::Existing(existing));
                        pages.push(Page::Scope);
                    }
                    if asks_license {
                        pages.push(Page::License);
                    }
                    pages.extend((0..option_pages).map(Page::Options));
                    pages.push(Page::Ready);
                }
                _ => {}
            }
            pages.push(Page::Progress);
        }
        pages.push(Page::Finish);

        let launch = match operation {
            Operation::Install => metadata.launch.clone(),
            _ => None,
        };

        Session {
            exe,
            operation,
            kind,
            pages,
            product_name: declared.name.clone(),
            product_version: declared.version.clone(),
            icon_bytes: declared.icon.clone(),
            license_text: declared.license_text.clone(),
            scope: base.scope,
            scope_page,
            refusal,
            machine_needs_elevation: {
                let mut machine = base.clone();
                machine.scope = Scope::Machine;
                tigersetup_engine::elevation::requirement(package, &machine)
                    .is_ok_and(tigersetup_engine::elevation::Requirement::needs_elevation)
            },
            install_root,
            default_root_for_scope,
            estimated_size: metadata.install().estimated_size,
            explicit_options: base.options.clone(),
            options,
            option_pages,
            absent_dependencies,
            installed_version: installation.map(|installed| installed.version),
            launch,
            base,
            relaunch_arguments,
        }
    }
}

/// The default install root recorded for `scope`, when the package offers it.
pub fn default_root_of(roots: &[(Scope, PathBuf)], scope: Scope) -> Option<PathBuf> {
    roots
        .iter()
        .find(|(offered, _)| *offered == scope)
        .map(|(_, root)| root.clone())
}

/// The label of a declared option: the wizard's own wording for the two
/// kinds it understands, and the package's wording for a custom or choice
/// one — its locale, then `en-US`, then the declared name.
fn option_label(
    option: &tigersetup_engine::format::metadata::InstallOption,
    text: &Text,
) -> String {
    match OptionKind::try_from(option.kind) {
        Ok(OptionKind::Path) => text.get("ui.option.path"),
        Ok(OptionKind::DesktopShortcut) => text.get("ui.option.desktop_shortcut"),
        _ => {
            let declared = option
                .labels
                .get(text.lang())
                .or_else(|| option.labels.get("en-US"))
                .unwrap_or(&option.name);
            // A package author's label is product text, never a mnemonic.
            Text::literal(declared)
        }
    }
}

/// The label of one choice: its locale, then `en-US`, then the value.
fn choice_label(choice: &tigersetup_engine::format::metadata::OptionChoice, text: &Text) -> String {
    let declared = choice
        .labels
        .get(text.lang())
        .or_else(|| choice.labels.get("en-US"))
        .unwrap_or(&choice.value);
    Text::literal(declared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_engine::format::metadata::InstallOption;

    fn option(kind: OptionKind, labels: &[(&str, &str)]) -> InstallOption {
        InstallOption {
            name: "extra-tools".into(),
            default: false,
            kind: kind as i32,
            labels: labels
                .iter()
                .map(|(tag, label)| (tag.to_string(), label.to_string()))
                .collect(),
            choices: Vec::new(),
            default_choice: String::new(),
        }
    }

    /// Rows fill a page up to its capacity and never straddle two pages: a
    /// choice whose rows do not fit moves whole to the next page.
    #[test]
    fn options_paginate_without_splitting_a_choice() {
        let check = |name: &str| OptionRow {
            name: name.into(),
            label: name.into(),
            control: OptionControl::Check { checked: false },
            page: 0,
            row: 0,
        };
        let choice = |name: &str, values: usize| OptionRow {
            name: name.into(),
            label: name.into(),
            control: OptionControl::Choice {
                choices: (0..values)
                    .map(|i| (format!("v{i}"), format!("Value {i}")))
                    .collect(),
                selected: 0,
            },
            page: 0,
            row: 0,
        };
        let mut none: Vec<OptionRow> = Vec::new();
        assert_eq!(paginate(&mut none), 0);

        // Nine check boxes fill one page; the tenth opens a second.
        let mut rows: Vec<OptionRow> = (0..10).map(|i| check(&format!("o{i}"))).collect();
        assert_eq!(paginate(&mut rows), 2);
        assert_eq!((rows[8].page, rows[8].row), (0, 8));
        assert_eq!((rows[9].page, rows[9].row), (1, 0));

        // The acceptance package's shape: a three-value choice (four rows)
        // and nine check boxes take two pages, the choice and five boxes
        // on the first.
        let mut rows = vec![choice("path-mode", 3)];
        rows.extend((0..9).map(|i| check(&format!("o{i}"))));
        assert_eq!(paginate(&mut rows), 2);
        assert_eq!(rows[5].page, 0, "five check boxes fit after the choice");
        assert_eq!(rows[6].page, 1);
        assert_eq!(rows[6].row, 0);

        // A choice that would straddle the page break moves whole.
        let mut rows: Vec<OptionRow> = (0..7).map(|i| check(&format!("o{i}"))).collect();
        rows.push(choice("mode", 3));
        assert_eq!(paginate(&mut rows), 2);
        assert_eq!((rows[7].page, rows[7].row), (1, 0));
        assert_eq!(rows[7].rows(), 4);
        assert_eq!(rows[7].initial_value(), OptionValue::Choice("v0".into()));
    }

    #[test]
    fn the_wizard_words_the_kinds_it_understands() {
        let english = Text::new("en-US", "TigerMarkView");
        let polish = Text::new("pl-PL", "TigerMarkView");
        assert_eq!(
            option_label(&option(OptionKind::Path, &[]), &english),
            "Add TigerMarkView to PATH"
        );
        assert_eq!(
            option_label(&option(OptionKind::Path, &[]), &polish),
            "Dodaj TigerMarkView do zmiennej PATH"
        );
        assert_eq!(
            option_label(&option(OptionKind::DesktopShortcut, &[]), &english),
            "Create a desktop shortcut"
        );
    }

    #[test]
    fn a_custom_label_falls_back_to_english_and_then_to_the_name() {
        let polish = Text::new("pl-PL", "TigerMarkView");
        let both = option(
            OptionKind::Custom,
            &[
                ("en-US", "Install extra tools"),
                ("pl-PL", "Dodatkowe narzędzia"),
            ],
        );
        assert_eq!(option_label(&both, &polish), "Dodatkowe narzędzia");
        let english_only = option(OptionKind::Custom, &[("en-US", "Install extra tools")]);
        assert_eq!(option_label(&english_only, &polish), "Install extra tools");
        let none = option(OptionKind::Custom, &[]);
        assert_eq!(option_label(&none, &polish), "extra-tools");
    }

    #[test]
    fn an_ampersand_in_a_package_label_is_shown_not_read_as_a_mnemonic() {
        let english = Text::new("en-US", "Sample");
        let label = option(OptionKind::Custom, &[("en-US", "Tom & Jerry")]);
        assert_eq!(option_label(&label, &english), "Tom && Jerry");
    }
}
