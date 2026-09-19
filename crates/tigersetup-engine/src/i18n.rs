//! Lookup of the engine's own human-readable text by stable key, with `en-US`
//! as the guaranteed fallback. Product strings come from the metadata, not
//! from here. Machine-readable output never goes through this module.
//!
//! `outcome.*` keys are the messages a run's outcome document carries.
//! `ui.*` keys are everything the interactive wizard shows: page titles and
//! bodies, control labels, progress lines and completion text. A `&` in a
//! label marks the Alt mnemonic of that control; no two controls that share
//! a page may carry the same mnemonic letter.
//!
//! `TigerSetup` is a name, not a word: it is never translated and never
//! appears in a catalog entry.

const EN_US: &[(&str, &str)] = &[
    ("outcome.installed", "Installed {name} {version} to {root}."),
    (
        "outcome.upgraded",
        "Upgraded {name} from {from} to {version} at {root}.",
    ),
    (
        "outcome.already_installed",
        "{name} {version} is already installed at {root}.",
    ),
    (
        "outcome.uninstalled",
        "Uninstalled {name} {version} from {root}.",
    ),
    ("outcome.repaired", "Repaired {name} {version} at {root}."),
    ("outcome.not_installed", "{name} is not installed."),
    (
        "outcome.rolled_back",
        "Installation of {name} {version} failed and was rolled back: {reason}",
    ),
    (
        "outcome.cancelled",
        "{name} {version} was cancelled; every change made so far was undone.",
    ),
    ("outcome.failed", "{name} {version} failed: {reason}"),
    (
        "outcome.scope_conflict.machine",
        "{name} is installed for all users. Setup continues with that installation when run without a scope, or with --scope machine.",
    ),
    (
        "outcome.scope_conflict.user",
        "{name} is installed for the current user. Setup continues with that installation when run without a scope, or with --scope user.",
    ),
    (
        "outcome.scope_ambiguous",
        "{name} is installed both for all users and for the current user. Name the installation to continue with: --scope machine or --scope user.",
    ),
    (
        "outcome.dependency_missing",
        "{name} {version} needs {dependency}, which is not installed.",
    ),
    (
        "outcome.dependency_unacquirable",
        "{name} {version} needs {dependency}, which could not be obtained: {reason}",
    ),
    (
        "outcome.dependency_install_failed",
        "{dependency} could not be installed, so {name} {version} was not installed: {reason}",
    ),
    (
        "outcome.dependency_unverified",
        "{dependency} reported success but is still not detected, so {name} {version} was not installed: {reason}",
    ),
    (
        "outcome.dependency_requires_elevation",
        "{dependency} must be installed by an administrator before {name} {version} can be installed.",
    ),
    (
        "outcome.dependency_cancelled",
        "Installation of {name} {version} was cancelled while preparing {dependency}.",
    ),
    // The window titles. The lab's interactive driver finds a wizard, and
    // every dialog belonging to it, by matching this text.
    ("ui.title.install", "{name} Setup"),
    ("ui.title.uninstall", "{name} Uninstall"),
    // Navigation. The visible name of a forward button is what an automated
    // run clicks, so these stay short and distinct.
    ("ui.button.back", "&Back"),
    ("ui.button.next", "&Next"),
    ("ui.button.install", "&Install"),
    ("ui.button.finish", "&Finish"),
    ("ui.button.cancel", "&Cancel"),
    ("ui.button.yes", "&Yes"),
    ("ui.button.no", "&No"),
    ("ui.button.close_applications", "&Close them"),
    ("ui.button.ok", "OK"),
    ("ui.button.browse", "B&rowse..."),
    ("ui.scope.title", "Select install mode"),
    ("ui.scope.subtitle", "How should {name} be installed?"),
    (
        "ui.scope.body",
        "{name} can be installed for you alone, or for everyone who uses this computer. Installing for all users requires administrative privileges.",
    ),
    ("ui.scope.user", "Install for &me only (recommended)"),
    ("ui.scope.machine", "Install for &all users"),
    // The same page when the product is already installed: it says which
    // installation Setup continues with instead of offering a choice that
    // would create a second one.
    ("ui.scope.existing.title", "Existing installation"),
    (
        "ui.scope.existing.subtitle",
        "{name} is already installed on this computer.",
    ),
    (
        "ui.scope.existing.machine",
        "{name} {version} is installed for all users in {root}. Setup will continue with that installation.",
    ),
    (
        "ui.scope.existing.user",
        "{name} {version} is installed for you in {root}. Setup will continue with that installation.",
    ),
    // And when it is installed both ways: each installation is its own, and
    // the person names the one this run is about.
    ("ui.scope.select.title", "Select installation"),
    (
        "ui.scope.select.subtitle",
        "Which installation of {name} should Setup continue with?",
    ),
    (
        "ui.scope.select.body",
        "{name} is installed both for all users and for you. The two installations are separate; choose the one Setup should continue with.",
    ),
    ("ui.scope.select.user", "For &me only: {version} in {root}"),
    (
        "ui.scope.select.machine",
        "For &all users: {version} in {root}",
    ),
    ("ui.license.title", "License Agreement"),
    (
        "ui.license.subtitle",
        "Please read the following important information before continuing.",
    ),
    (
        "ui.license.body",
        "Please read the following License Agreement. You must accept its terms before the installation can continue.",
    ),
    ("ui.license.accept", "I &accept the agreement"),
    ("ui.license.decline", "I &do not accept the agreement"),
    ("ui.destination.title", "Select Destination Location"),
    (
        "ui.destination.subtitle",
        "Where should {name} be installed?",
    ),
    (
        "ui.destination.body",
        "Setup will install {name} into the following folder.",
    ),
    (
        "ui.destination.hint",
        "To continue, click Next. To choose a different folder, click Browse.",
    ),
    (
        "ui.destination.browse_title",
        "Select the folder to install {name} into.",
    ),
    (
        "ui.destination.space",
        "At least {required} of free disk space is required; {free} is available on {volume}.",
    ),
    (
        "ui.destination.space_required",
        "At least {required} of free disk space is required.",
    ),
    (
        "ui.destination.not_absolute",
        "Enter a full path, including a drive letter.",
    ),
    (
        "ui.destination.not_enough_space",
        "{volume} has {free} free, but {required} is required.",
    ),
    ("ui.options.title", "Select Additional Tasks"),
    (
        "ui.options.title.paged",
        "Select Additional Tasks ({page} of {pages})",
    ),
    (
        "ui.options.subtitle",
        "Which additional tasks should be performed?",
    ),
    (
        "ui.options.body",
        "Select the additional tasks Setup should perform while installing {name}, then click Next.",
    ),
    ("ui.option.path", "Add {name} to PATH"),
    ("ui.option.desktop_shortcut", "Create a desktop shortcut"),
    ("ui.ready.title", "Ready to Install"),
    (
        "ui.ready.subtitle.install",
        "Setup is ready to install {name} on your computer.",
    ),
    (
        "ui.ready.subtitle.upgrade",
        "Setup is ready to upgrade {name} on your computer.",
    ),
    (
        "ui.ready.subtitle.repair",
        "Setup is ready to repair {name} on your computer.",
    ),
    (
        "ui.ready.body.install",
        "Click Install to continue, or click Back to review or change any setting.",
    ),
    ("ui.ready.body.upgrade", "Upgrade {name} {from} to {to}."),
    ("ui.ready.body.reinstall", "Reinstall {name} {version}."),
    ("ui.ready.body.repair", "Repair {name} {version}."),
    ("ui.summary.destination", "Destination location:"),
    ("ui.summary.scope", "Install mode:"),
    ("ui.summary.scope.user", "For me only"),
    ("ui.summary.scope.machine", "For all users"),
    ("ui.summary.options", "Additional tasks:"),
    (
        "ui.summary.dependencies",
        "Components that will be installed:",
    ),
    ("ui.summary.none", "(none)"),
    ("ui.progress.title.install", "Installing"),
    ("ui.progress.title.uninstall", "Uninstalling"),
    (
        "ui.progress.subtitle.install",
        "Please wait while Setup installs {name} on your computer.",
    ),
    (
        "ui.progress.subtitle.uninstall",
        "Please wait while Setup removes {name} from your computer.",
    ),
    ("ui.progress.preparing", "Preparing..."),
    (
        "ui.progress.dependency_download",
        "Downloading {dependency}: {done} of {total}",
    ),
    ("ui.progress.dependency_install", "Installing {dependency}"),
    ("ui.progress.applying.install", "Installing {target}"),
    ("ui.progress.applying.uninstall", "Removing {target}"),
    ("ui.progress.action", "Running {target}"),
    (
        "ui.progress.rolling_back",
        "Undoing the changes made so far...",
    ),
    (
        "ui.progress.recovering",
        "Completing an earlier operation...",
    ),
    ("ui.progress.finishing", "Finishing..."),
    ("ui.progress.cancel_confirm", "Cancel the setup of {name}?"),
    (
        "ui.quiescence.close_confirm",
        "{applications} must be closed before {name} can be updated. Close and restart them?",
    ),
    ("ui.uninstall.title", "Uninstall"),
    ("ui.uninstall.subtitle", "Remove {name} from your computer."),
    (
        "ui.uninstall.confirm",
        "Are you sure you want to completely remove {name} and all of its components?",
    ),
    ("ui.finish.title.ok", "Setup complete"),
    ("ui.finish.title.failed", "Setup did not complete"),
    (
        "ui.finish.subtitle.installed",
        "Setup has finished installing {name} on your computer.",
    ),
    (
        "ui.finish.subtitle.uninstalled",
        "Setup has finished removing {name} from your computer.",
    ),
    (
        "ui.finish.subtitle.failed",
        "Setup did not finish, and your computer was left as it was.",
    ),
    (
        "ui.finish.body.installed",
        "Setup has finished installing {name} {version} on your computer.",
    ),
    (
        "ui.finish.body.uninstalled",
        "{name} has been removed from your computer.",
    ),
    (
        "ui.finish.reboot",
        "Restart your computer to complete the installation.",
    ),
    ("ui.finish.launch", "&Launch {name}"),
    // The diagnostic affordance: a link, not a path, because a path on a
    // completion page is long and cannot be selected.
    ("ui.finish.copy_log", "&Copy log path"),
    ("ui.finish.log_copied", "Log path copied"),
    (
        "ui.error.elevation",
        "{name} could not be installed for all users: {reason}",
    ),
    ("ui.error.package", "Setup could not start: {reason}"),
    // The decimal mark of a formatted size such as "35.8 MB".
    ("ui.decimal_separator", "."),
];

const PL_PL: &[(&str, &str)] = &[
    (
        "outcome.installed",
        "Zainstalowano {name} {version} w {root}.",
    ),
    (
        "outcome.upgraded",
        "Zaktualizowano {name} z {from} do {version} w {root}.",
    ),
    (
        "outcome.already_installed",
        "{name} {version} jest już zainstalowany w {root}.",
    ),
    (
        "outcome.uninstalled",
        "Odinstalowano {name} {version} z {root}.",
    ),
    ("outcome.repaired", "Naprawiono {name} {version} w {root}."),
    ("outcome.not_installed", "{name} nie jest zainstalowany."),
    (
        "outcome.rolled_back",
        "Instalacja {name} {version} nie powiodła się i została wycofana: {reason}",
    ),
    (
        "outcome.cancelled",
        "Anulowano operację {name} {version}; wszystkie dotychczasowe zmiany zostały wycofane.",
    ),
    ("outcome.failed", "{name} {version}: błąd: {reason}"),
    (
        "outcome.scope_conflict.machine",
        "Program {name} jest zainstalowany dla wszystkich użytkowników. Instalator kontynuuje z tą instalacją, gdy zostanie uruchomiony bez zakresu albo z opcją --scope machine.",
    ),
    (
        "outcome.scope_conflict.user",
        "Program {name} jest zainstalowany dla bieżącego użytkownika. Instalator kontynuuje z tą instalacją, gdy zostanie uruchomiony bez zakresu albo z opcją --scope user.",
    ),
    (
        "outcome.scope_ambiguous",
        "Program {name} jest zainstalowany zarówno dla wszystkich użytkowników, jak i dla bieżącego użytkownika. Wskaż instalację, z którą kontynuować: --scope machine albo --scope user.",
    ),
    (
        "outcome.dependency_missing",
        "{name} {version} wymaga składnika {dependency}, który nie jest zainstalowany.",
    ),
    (
        "outcome.dependency_unacquirable",
        "{name} {version} wymaga składnika {dependency}, którego nie udało się pobrać: {reason}",
    ),
    (
        "outcome.dependency_install_failed",
        "Nie udało się zainstalować składnika {dependency}, więc {name} {version} nie został zainstalowany: {reason}",
    ),
    (
        "outcome.dependency_unverified",
        "Instalator składnika {dependency} zgłosił powodzenie, ale składnik nadal nie jest wykrywany, więc {name} {version} nie został zainstalowany: {reason}",
    ),
    (
        "outcome.dependency_requires_elevation",
        "Składnik {dependency} musi zostać zainstalowany przez administratora, zanim będzie można zainstalować {name} {version}.",
    ),
    (
        "outcome.dependency_cancelled",
        "Instalacja {name} {version} została anulowana podczas przygotowywania składnika {dependency}.",
    ),
    ("ui.title.install", "Instalator — {name}"),
    ("ui.title.uninstall", "Dezinstalator — {name}"),
    ("ui.button.back", "&Wstecz"),
    ("ui.button.next", "&Dalej"),
    ("ui.button.install", "Za&instaluj"),
    ("ui.button.finish", "&Zakończ"),
    ("ui.button.cancel", "An&uluj"),
    ("ui.button.yes", "&Tak"),
    ("ui.button.no", "&Nie"),
    ("ui.button.close_applications", "&Zamknij je"),
    ("ui.button.ok", "OK"),
    ("ui.button.browse", "P&rzeglądaj..."),
    ("ui.scope.title", "Wybierz tryb instalacji"),
    (
        "ui.scope.subtitle",
        "W jaki sposób zainstalować program {name}?",
    ),
    (
        "ui.scope.body",
        "Program {name} może zostać zainstalowany tylko dla Ciebie albo dla wszystkich osób korzystających z tego komputera. Instalacja dla wszystkich użytkowników wymaga uprawnień administratora.",
    ),
    ("ui.scope.user", "Instaluj tylko dla &mnie (zalecane)"),
    ("ui.scope.machine", "&Instaluj dla wszystkich użytkowników"),
    ("ui.scope.existing.title", "Istniejąca instalacja"),
    (
        "ui.scope.existing.subtitle",
        "Program {name} jest już zainstalowany na tym komputerze.",
    ),
    (
        "ui.scope.existing.machine",
        "Program {name} {version} jest zainstalowany dla wszystkich użytkowników w folderze {root}. Instalator będzie kontynuował z tą instalacją.",
    ),
    (
        "ui.scope.existing.user",
        "Program {name} {version} jest zainstalowany dla Ciebie w folderze {root}. Instalator będzie kontynuował z tą instalacją.",
    ),
    ("ui.scope.select.title", "Wybierz instalację"),
    (
        "ui.scope.select.subtitle",
        "Z którą instalacją programu {name} kontynuować?",
    ),
    (
        "ui.scope.select.body",
        "Program {name} jest zainstalowany zarówno dla wszystkich użytkowników, jak i dla Ciebie. Te instalacje są niezależne; wybierz tę, z którą instalator ma kontynuować.",
    ),
    (
        "ui.scope.select.user",
        "Tylko dla &mnie: {version} w {root}",
    ),
    (
        "ui.scope.select.machine",
        "Dla &wszystkich użytkowników: {version} w {root}",
    ),
    ("ui.license.title", "Umowa licencyjna"),
    (
        "ui.license.subtitle",
        "Przed kontynuowaniem przeczytaj następujące ważne informacje.",
    ),
    (
        "ui.license.body",
        "Przeczytaj poniższą Umowę licencyjną. Aby instalacja mogła być kontynuowana, musisz zaakceptować jej warunki.",
    ),
    ("ui.license.accept", "&Akceptuję warunki umowy"),
    ("ui.license.decline", "&Nie akceptuję warunków umowy"),
    ("ui.destination.title", "Wybierz lokalizację docelową"),
    (
        "ui.destination.subtitle",
        "Gdzie ma zostać zainstalowany program {name}?",
    ),
    (
        "ui.destination.body",
        "Instalator zainstaluje program {name} w następującym folderze.",
    ),
    (
        "ui.destination.hint",
        "Aby kontynuować, kliknij Dalej. Aby wybrać inny folder, kliknij Przeglądaj.",
    ),
    (
        "ui.destination.browse_title",
        "Wybierz folder, w którym zostanie zainstalowany program {name}.",
    ),
    (
        "ui.destination.space",
        "Wymagane jest co najmniej {required} wolnego miejsca na dysku; na dysku {volume} dostępne jest {free}.",
    ),
    (
        "ui.destination.space_required",
        "Wymagane jest co najmniej {required} wolnego miejsca na dysku.",
    ),
    (
        "ui.destination.not_absolute",
        "Wpisz pełną ścieżkę wraz z literą dysku.",
    ),
    (
        "ui.destination.not_enough_space",
        "Na dysku {volume} jest {free} wolnego miejsca, a wymagane jest {required}.",
    ),
    ("ui.options.title", "Wybierz dodatkowe zadania"),
    (
        "ui.options.title.paged",
        "Wybierz dodatkowe zadania ({page} z {pages})",
    ),
    (
        "ui.options.subtitle",
        "Które dodatkowe zadania mają zostać wykonane?",
    ),
    (
        "ui.options.body",
        "Zaznacz dodatkowe zadania, które instalator ma wykonać podczas instalowania programu {name}, a następnie kliknij Dalej.",
    ),
    ("ui.option.path", "Dodaj {name} do zmiennej PATH"),
    ("ui.option.desktop_shortcut", "Utwórz skrót na pulpicie"),
    ("ui.ready.title", "Gotowość do instalacji"),
    (
        "ui.ready.subtitle.install",
        "Instalator jest gotowy do zainstalowania programu {name} na tym komputerze.",
    ),
    (
        "ui.ready.subtitle.upgrade",
        "Instalator jest gotowy do zaktualizowania programu {name} na tym komputerze.",
    ),
    (
        "ui.ready.subtitle.repair",
        "Instalator jest gotowy do naprawienia programu {name} na tym komputerze.",
    ),
    (
        "ui.ready.body.install",
        "Kliknij Zainstaluj, aby kontynuować, lub kliknij Wstecz, aby przejrzeć albo zmienić ustawienia.",
    ),
    (
        "ui.ready.body.upgrade",
        "Aktualizacja programu {name} z wersji {from} do {to}.",
    ),
    (
        "ui.ready.body.reinstall",
        "Ponowna instalacja programu {name} {version}.",
    ),
    ("ui.ready.body.repair", "Naprawa programu {name} {version}."),
    ("ui.summary.destination", "Lokalizacja docelowa:"),
    ("ui.summary.scope", "Tryb instalacji:"),
    ("ui.summary.scope.user", "Tylko dla mnie"),
    ("ui.summary.scope.machine", "Dla wszystkich użytkowników"),
    ("ui.summary.options", "Dodatkowe zadania:"),
    (
        "ui.summary.dependencies",
        "Składniki, które zostaną zainstalowane:",
    ),
    ("ui.summary.none", "(brak)"),
    ("ui.progress.title.install", "Instalowanie"),
    ("ui.progress.title.uninstall", "Odinstalowywanie"),
    (
        "ui.progress.subtitle.install",
        "Poczekaj, aż instalator zainstaluje program {name} na tym komputerze.",
    ),
    (
        "ui.progress.subtitle.uninstall",
        "Poczekaj, aż instalator usunie program {name} z tego komputera.",
    ),
    ("ui.progress.preparing", "Przygotowywanie..."),
    (
        "ui.progress.dependency_download",
        "Pobieranie składnika {dependency}: {done} z {total}",
    ),
    (
        "ui.progress.dependency_install",
        "Instalowanie składnika {dependency}",
    ),
    ("ui.progress.applying.install", "Instalowanie: {target}"),
    ("ui.progress.applying.uninstall", "Usuwanie: {target}"),
    ("ui.progress.action", "Uruchamianie: {target}"),
    (
        "ui.progress.rolling_back",
        "Wycofywanie dotychczasowych zmian...",
    ),
    (
        "ui.progress.recovering",
        "Kończenie wcześniejszej operacji...",
    ),
    ("ui.progress.finishing", "Kończenie..."),
    (
        "ui.progress.cancel_confirm",
        "Anulować działanie instalatora programu {name}?",
    ),
    (
        "ui.quiescence.close_confirm",
        "Aby zaktualizować program {name}, należy zamknąć: {applications}. Zamknąć i uruchomić ponownie?",
    ),
    ("ui.uninstall.title", "Dezinstalacja"),
    (
        "ui.uninstall.subtitle",
        "Usuń program {name} z tego komputera.",
    ),
    (
        "ui.uninstall.confirm",
        "Czy na pewno chcesz całkowicie usunąć program {name} wraz ze wszystkimi jego składnikami?",
    ),
    ("ui.finish.title.ok", "Zakończono"),
    ("ui.finish.title.failed", "Nie zakończono"),
    (
        "ui.finish.subtitle.installed",
        "Instalator zakończył instalowanie programu {name} na tym komputerze.",
    ),
    (
        "ui.finish.subtitle.uninstalled",
        "Instalator zakończył usuwanie programu {name} z tego komputera.",
    ),
    (
        "ui.finish.subtitle.failed",
        "Instalator nie zakończył pracy, a komputer pozostał bez zmian.",
    ),
    (
        "ui.finish.body.installed",
        "Instalator zakończył instalowanie programu {name} {version} na tym komputerze.",
    ),
    (
        "ui.finish.body.uninstalled",
        "Program {name} został usunięty z tego komputera.",
    ),
    (
        "ui.finish.reboot",
        "Uruchom ponownie komputer, aby dokończyć instalację.",
    ),
    ("ui.finish.launch", "&Uruchom program {name}"),
    ("ui.finish.copy_log", "&Kopiuj ścieżkę dziennika"),
    ("ui.finish.log_copied", "Skopiowano ścieżkę dziennika"),
    (
        "ui.error.elevation",
        "Nie udało się zainstalować programu {name} dla wszystkich użytkowników: {reason}",
    ),
    (
        "ui.error.package",
        "Nie udało się uruchomić instalatora: {reason}",
    ),
    ("ui.decimal_separator", ","),
];

/// Returns the text for `key` in `lang` (a BCP 47 tag such as `pl-PL`),
/// falling back to `en-US` and finally to the key itself.
pub fn text(lang: &str, key: &str) -> &'static str {
    let catalog: &[(&str, &str)] = match lang.to_ascii_lowercase().as_str() {
        "pl-pl" | "pl" => PL_PL,
        _ => &[],
    };
    catalog
        .iter()
        .chain(EN_US.iter())
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| Box::leak(key.to_string().into_boxed_str()))
}

/// The languages the engine's own text is available in, as BCP 47 tags,
/// `en-US` first.
pub const LANGUAGES: &[&str] = &["en-US", "pl-PL"];

/// Maps a Windows UI language to a supported tag, `en-US` as the fallback
/// (`TigerSetup-Design.md` §12.1): explicit selection is the client's
/// business; this is the second step.
pub fn detect_ui_language() -> String {
    const LANG_POLISH: u16 = 0x15;
    let langid = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    if langid & 0x3ff == LANG_POLISH {
        "pl-PL".into()
    } else {
        "en-US".into()
    }
}

/// Substitutes `{placeholder}` tokens.
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// A label without its Alt mnemonic marker, as a summary line or an
/// automation client reads it.
pub fn strip_mnemonics(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut characters = label.chars().peekable();
    while let Some(c) = characters.next() {
        if c == '&' {
            if characters.peek() == Some(&'&') {
                characters.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The Alt mnemonic letter of a label, lower-cased, when it has one.
pub fn mnemonic_of(label: &str) -> Option<char> {
    let mut characters = label.chars().peekable();
    while let Some(c) = characters.next() {
        if c != '&' {
            continue;
        }
        match characters.peek() {
            Some('&') => {
                characters.next();
            }
            Some(next) => return next.to_lowercase().next(),
            None => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polish_covers_every_english_key_and_falls_back() {
        for (key, _) in EN_US {
            assert!(PL_PL.iter().any(|(k, _)| k == key), "pl-PL lacks {key}");
        }
        assert!(text("pl-PL", "outcome.installed").starts_with("Zainstalowano"));
        assert!(text("de-DE", "outcome.installed").starts_with("Installed"));
        assert_eq!(fill("{a}-{b}", &[("a", "1"), ("b", "2")]), "1-2");
    }

    #[test]
    fn every_catalog_entry_is_declared_once_and_keeps_its_placeholders() {
        for catalog in [EN_US, PL_PL] {
            let mut seen = std::collections::HashSet::new();
            for (key, _) in catalog {
                assert!(seen.insert(*key), "{key} is declared twice");
            }
        }
        for (key, english) in EN_US {
            let polish = text("pl-PL", key);
            for token in english
                .split('{')
                .skip(1)
                .filter_map(|t| t.split('}').next())
            {
                assert!(
                    polish.contains(&format!("{{{token}}}")),
                    "pl-PL {key} lacks the {{{token}}} placeholder"
                );
            }
        }
    }

    #[test]
    fn the_brand_name_is_never_translated() {
        for (key, polish) in PL_PL {
            assert!(
                !polish.contains("TigerSetup"),
                "{key} names the brand; the wizard writes it, the catalog never does"
            );
        }
    }

    #[test]
    fn mnemonics_are_read_off_a_label() {
        assert_eq!(mnemonic_of("&Next"), Some('n'));
        assert_eq!(mnemonic_of("Za&instaluj"), Some('i'));
        assert_eq!(mnemonic_of("A && B"), None);
        assert_eq!(strip_mnemonics("Za&instaluj"), "Zainstaluj");
        assert_eq!(strip_mnemonics("A && B"), "A & B");
    }
}
