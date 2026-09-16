//! The firewall-rule resource: one Windows Firewall rule for an installed
//! program. Ownership is by rule name, and because Windows lets two rules
//! share a name, the rule TigerSetup writes carries the product as its
//! `Grouping`; a same-named rule without it is a stranger's, which the
//! planner neither claims, rewrites nor removes. Removal is conservative in
//! the same way every resource's is: a rule that no longer reads as the one
//! TigerSetup wrote is preserved and reported.
//!
//! Rules are machine-wide and need an administrator. A run that has none —
//! a per-user install by a standard user — reports
//! `firewall_rule_skipped_unelevated` for each declared rule and creates
//! nothing; the requirement is not silently dropped, and nothing is left
//! half done.

use std::path::Path;

use tigersetup_format::Metadata;
use tigersetup_format::metadata::{FirewallAction, FirewallDirection, FirewallProtocol};

use crate::plan::{absolute, to_relative};
use crate::resource::predicate::{self, Options};
use crate::win::firewall::Rule;
use crate::{Error, Result};

/// The group every rule of a product is filed under, which is also how the
/// engine recognises its own rule.
pub fn grouping(metadata: &Metadata) -> String {
    format!("TigerSetup: {}", metadata.package().name)
}

/// The rules the effective options enable, resolved for this installation.
pub fn desired(metadata: &Metadata, options: &Options, install_root: &Path) -> Result<Vec<Rule>> {
    let group = grouping(metadata);
    metadata
        .firewall_rules
        .iter()
        .filter(|rule| predicate::enabled(rule.when.as_ref(), "", options))
        .map(|rule| {
            let program = absolute(install_root, &to_relative(&rule.program))?;
            Ok(Rule {
                name: rule.name.clone(),
                description: rule.description.clone(),
                grouping: group.clone(),
                program: program.display().to_string(),
                direction: match FirewallDirection::try_from(rule.direction) {
                    Ok(FirewallDirection::Out) => "out".into(),
                    _ => "in".into(),
                },
                action: match FirewallAction::try_from(rule.action) {
                    Ok(FirewallAction::Block) => "block".into(),
                    _ => "allow".into(),
                },
                protocol: match FirewallProtocol::try_from(rule.protocol) {
                    Ok(FirewallProtocol::Tcp) => "tcp".into(),
                    Ok(FirewallProtocol::Udp) => "udp".into(),
                    _ => "any".into(),
                },
                local_ports: rule.local_ports.clone(),
                enabled: true,
            })
        })
        .collect()
}

/// Whether a rule on the machine is one TigerSetup wrote for this product:
/// it carries the product's grouping.
pub fn is_ours(rule: &Rule, group: &str) -> bool {
    rule.grouping == group
}

/// Whether a rule on the machine still reads exactly as recorded, apart
/// from the case of its program path, which Windows may canonicalise.
pub fn matches(current: &Rule, recorded: &Rule) -> bool {
    current.name.eq_ignore_ascii_case(&recorded.name)
        && current.grouping == recorded.grouping
        && current.program.eq_ignore_ascii_case(&recorded.program)
        && current.direction == recorded.direction
        && current.action == recorded.action
        && current.protocol == recorded.protocol
        && current.local_ports == recorded.local_ports
        && current.enabled == recorded.enabled
        && current.description == recorded.description
}

/// The rule an operation row or an ownership row carries.
pub fn rule_of(text: Option<&str>, what: &str) -> Result<Rule> {
    let text = text.ok_or_else(|| {
        Error::new(
            "journal_inconsistent",
            format!("{what} carries no firewall rule"),
        )
    })?;
    Rule::deserialize(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::metadata::{FirewallRule, OptionValue, Package, Predicate};

    #[test]
    fn rules_resolve_their_program_and_follow_their_predicate() {
        let metadata = Metadata {
            package: Some(Package {
                name: "TestApp".into(),
                ..Default::default()
            }),
            firewall_rules: vec![
                FirewallRule {
                    name: "TestApp listener".into(),
                    description: "Lets TestApp listen".into(),
                    program: "bin/app.exe".into(),
                    direction: FirewallDirection::In as i32,
                    action: FirewallAction::Allow as i32,
                    protocol: FirewallProtocol::Tcp as i32,
                    local_ports: "47110".into(),
                    when: Some(Predicate {
                        option: "firewall".into(),
                        equals: "true".into(),
                    }),
                },
                FirewallRule {
                    name: "TestApp out".into(),
                    program: "bin/app.exe".into(),
                    direction: FirewallDirection::Out as i32,
                    action: FirewallAction::Block as i32,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut options = Options::new();
        options.insert("firewall".into(), OptionValue::Bool(true));
        let rules = desired(&metadata, &options, Path::new("C:\\P")).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].program, "C:\\P\\bin\\app.exe");
        assert_eq!(rules[0].grouping, "TigerSetup: TestApp");
        assert_eq!(rules[0].direction, "in");
        assert_eq!(rules[0].protocol, "tcp");
        assert_eq!(rules[0].local_ports, "47110");
        assert!(rules[0].enabled);
        assert_eq!(rules[1].direction, "out");
        assert_eq!(rules[1].action, "block");
        assert_eq!(rules[1].protocol, "any");
        options.insert("firewall".into(), OptionValue::Bool(false));
        assert_eq!(
            desired(&metadata, &options, Path::new("C:\\P"))
                .unwrap()
                .len(),
            1
        );

        let recorded = rules[0].clone();
        let mut current = recorded.clone();
        current.program = current.program.to_uppercase();
        assert!(
            matches(&current, &recorded),
            "the program path compares case-insensitively"
        );
        current.enabled = false;
        assert!(
            !matches(&current, &recorded),
            "a disabled rule was modified"
        );
        assert!(is_ours(&recorded, "TigerSetup: TestApp"));
        let mut stranger = recorded.clone();
        stranger.grouping = String::new();
        assert!(!is_ours(&stranger, "TigerSetup: TestApp"));
        assert!(rule_of(None, "operation 3").is_err());
        assert_eq!(rule_of(Some(&recorded.serialize()), "x").unwrap(), recorded);
    }
}
