# Third-party notices

Material TigerSetup redistributes, with the notices its licences require. Rust
crate dependencies are not listed individually here: they are declared in
`Cargo.toml`, pinned in `Cargo.lock`, and evaluated against the effective
licensing policy — the TigerAiCore defaults, which TigerSetup does not override
— when they are introduced or updated.

What is listed here is material that ships **inside the generated
`Setup.exe`**, where the obligation travels with the redistributed bytes rather
than with a package manifest.

---

## Fluent UI System Icons

**Material:** the outline path data of two icons, embedded in
`crates/tigersetup-setup/src/ui/glyph.rs`:

| Icon | Source asset |
|---|---|
| `Glyph::Succeeded` | `assets/Checkmark Circle/SVG/ic_fluent_checkmark_circle_24_regular.svg` |
| `Glyph::Failed` | `assets/Error Circle/SVG/ic_fluent_error_circle_24_regular.svg` |

**Source:** <https://github.com/microsoft/fluentui-system-icons>
**Domain:** `icons-graphics`
**Licence:** MIT
**Effective rule:** `permissive-mit`, `automatically-approved-with-conditions`

**Obligation:** the MIT notice below is preserved with the redistributed
material. MIT creates no product-facing attribution obligation, so the wizard
shows no credit and the project attribution decision is not engaged; the notice
travels with the bytes, which is what this file is.

Only generic concept glyphs are used — a checkmark in a circle and an
exclamation mark in a circle. No Microsoft name, logo, product mark or
brand-styled glyph is redistributed.

```text
MIT License

Copyright (c) 2020 Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

**Re-evaluate on update.** Replacing or adding a glyph means reading the
upstream licence again and comparing it with the state recorded here. A changed
licence or material term is an Architect decision even if the new state would
otherwise match an automatic rule.
