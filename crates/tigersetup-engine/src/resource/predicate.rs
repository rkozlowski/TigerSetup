//! The one predicate an optional resource carries: "while option X has value
//! Y". Every resource kind asks the same question here, so a file, a
//! shortcut, a PATH entry, a registry value, an environment variable, an
//! integration, a firewall rule and a dependency are gated identically —
//! and a component is nothing more than an option that gates some of them.

use std::collections::BTreeMap;

use tigersetup_format::metadata::{OptionValue, Predicate};

/// The effective value of every declared option, by lower-case name.
pub type Options = BTreeMap<String, OptionValue>;

/// Whether a resource is part of the desired state: it carries no predicate,
/// or the named option currently has the named value. An option the
/// predicate names but the options do not carry — which validation makes
/// impossible for a well-formed package — disables the resource, never
/// enables it.
pub fn enabled(when: Option<&Predicate>, legacy_option: &str, options: &Options) -> bool {
    match Predicate::of(when, legacy_option) {
        None => true,
        Some(predicate) => options
            .get(&predicate.option.to_ascii_lowercase())
            .is_some_and(|value| value.as_text() == predicate.equals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_predicate_compares_the_canonical_text_of_the_option_value() {
        let mut options = Options::new();
        options.insert("desktop".into(), OptionValue::Bool(true));
        options.insert("path-mode".into(), OptionValue::Choice("tools".into()));
        let when = |option: &str, equals: &str| Predicate {
            option: option.into(),
            equals: equals.into(),
        };
        assert!(enabled(None, "", &options));
        assert!(enabled(Some(&when("desktop", "true")), "", &options));
        assert!(!enabled(Some(&when("desktop", "false")), "", &options));
        assert!(enabled(Some(&when("path-mode", "tools")), "", &options));
        assert!(!enabled(Some(&when("path-mode", "command")), "", &options));
        assert!(!enabled(Some(&when("unknown", "true")), "", &options));
        // The older `option = "x"` spelling means "while x is on".
        assert!(enabled(None, "desktop", &options));
        assert!(enabled(None, "DESKTOP", &options));
        options.insert("desktop".into(), OptionValue::Bool(false));
        assert!(!enabled(None, "desktop", &options));
        // An explicit predicate wins over the legacy field.
        assert!(enabled(
            Some(&when("path-mode", "tools")),
            "desktop",
            &options
        ));
    }
}
