//! The wizard's text: catalog entries from the engine's `i18n` module for
//! everything the wizard says, and the metadata for everything the product
//! says. The wizard never carries an English literal of its own, with one
//! exception that is deliberate — `TigerSetup` is a name, so [`BRAND`] is a
//! constant and is never translated.

use tigersetup_engine::i18n;

/// The secondary branding shown in the footer. A name, never translated,
/// and never dressed up with a phrase.
pub const BRAND: &str = "TigerSetup";

/// Resolves catalog keys for one language and one product.
pub struct Text {
    lang: String,
    name: String,
}

impl Text {
    pub fn new(lang: &str, product_name: &str) -> Text {
        Text {
            lang: lang.to_string(),
            name: product_name.to_string(),
        }
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// The entry for `key`, with `{name}` already filled in.
    pub fn get(&self, key: &str) -> String {
        self.fill(key, &[])
    }

    /// The entry for `key` with `{name}` and `values` filled in.
    pub fn fill(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut filled = i18n::fill(i18n::text(&self.lang, key), &[("name", &self.name)]);
        if !values.is_empty() {
            filled = i18n::fill(&filled, values);
        }
        filled
    }

    /// A byte count as a person reads it, with this language's decimal mark.
    pub fn size(&self, bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * KB;
        const GB: u64 = 1024 * MB;
        let separator = self.get("ui.decimal_separator");
        let (value, unit) = match bytes {
            b if b >= GB => (format!("{:.1}", b as f64 / GB as f64), "GB"),
            b if b >= MB => (format!("{:.1}", b as f64 / MB as f64), "MB"),
            b if b >= KB => (format!("{}", b.div_ceil(KB)), "KB"),
            b => (format!("{b}"), "B"),
        };
        format!("{} {unit}", value.replace('.', &separator))
    }

    /// A literal string shown in a control, with any `&` escaped so that
    /// Windows draws it instead of reading it as a mnemonic marker. Product
    /// text — an option label, a file name — goes through this.
    pub fn literal(text: &str) -> String {
        text.replace('&', "&&")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_text_never_becomes_a_mnemonic() {
        assert_eq!(Text::literal("Tom & Jerry"), "Tom && Jerry");
    }

    #[test]
    fn sizes_use_the_language_decimal_mark() {
        let english = Text::new("en-US", "Sample");
        let polish = Text::new("pl-PL", "Sample");
        assert_eq!(english.size(37_500_000), "35.8 MB");
        assert_eq!(polish.size(37_500_000), "35,8 MB");
        assert_eq!(english.size(900), "900 B");
        assert_eq!(english.size(2048), "2 KB");
        assert_eq!(english.size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn the_product_name_reaches_every_entry_that_asks_for_it() {
        let text = Text::new("en-US", "TigerMarkView");
        assert_eq!(text.get("ui.title.install"), "TigerMarkView Setup");
        assert_eq!(
            text.fill("ui.ready.body.upgrade", &[("from", "1.0"), ("to", "1.1")]),
            "Upgrade TigerMarkView 1.0 to 1.1."
        );
        let polish = Text::new("pl-PL", "TigerMarkView");
        assert_eq!(polish.get("ui.title.install"), "Instalator — TigerMarkView");
    }
}
