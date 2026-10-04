# Third-party notices

TigerSetup includes material by others. This document reproduces the notices
their licences require, for everything TigerSetup distributes:

- **TigerSetup itself** — `tiger-setup.exe` (the builder),
  `tigersetup-setup.exe` (the engine), `tigersetup-loader.exe` (the loader)
  and the installed help, as the TigerSetup installer places them;
- **every `Setup.exe` TigerSetup builds** — which begins with the loader and
  carries the engine, and installs a copy of itself as the product's
  uninstaller.

The TigerSetup installer installs this file beside `tiger-setup.exe`, with
`LICENSE.txt`. The engine carries the same text inside it, so a generated
`Setup.exe` and the uninstaller it installs print it with `Setup.exe notices`;
`tiger-setup notices` prints it from the builder. Whoever distributes a
generated `Setup.exe` distributes TigerSetup's loader and engine with it, and
this text — TigerSetup's own licence included — travels inside it.

Nothing here is a dependency the target machine needs: every item is compiled
into, or packaged with, the binaries above. The crate list is generated from
`Cargo.lock` by `eng/Update-ThirdPartyNotices.ps1`, and a workspace test fails
when it no longer matches what the release binaries link. Every licence was
evaluated against the effective licensing policy — the TigerAiCore defaults,
which TigerSetup does not override — when the component was introduced or
updated.

---

## TigerSetup

**Material:** the builder, the engine and the loader, and through them the
loader and engine inside every generated `Setup.exe`. TigerSetup's icon and
artwork are IT Tiger's own.

**Source:** <https://github.com/rkozlowski/TigerSetup>
**Licence:** MIT

```text
MIT License

Copyright (c) 2026 IT Tiger

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

---

## Zstandard

**Material:** the Zstandard compression library (libzstd 1.5.7), compiled into
`tigersetup-setup.exe` and `tiger-setup.exe` through the `zstd-sys` crate
(2.1.0+zstd.1.5.7) and its Rust bindings `zstd-safe` (8.0.0) and `zstd`
(0.14.0), and — its decoder alone, from the same crate's copy of the library
sources — into the C loader `tigersetup-loader.exe`. The loader and the engine
inside every generated `Setup.exe` carry the decoder; the builder carries the
encoder as well.

**Source:** <https://github.com/facebook/zstd> (the library);
<https://github.com/gyscos/zstd-rs> (the bindings)
**Domain:** `linked-code`
**Licence:** BSD-3-Clause (the library is offered under BSD-3-Clause or
GPL-2.0; BSD-3-Clause is the alternative selected and recorded here, and the
three crates declare BSD-3-Clause)
**Effective rule:** `permissive-bsd-3-clause`, `automatically-approved-with-conditions`

**Obligation:** a binary redistribution reproduces the copyright notice, the
conditions and the disclaimer in the documentation or other materials
provided with it, which is what this file is. Neither the Facebook nor the Meta
name is used to endorse or promote TigerSetup. The library's notice follows;
the three crates' own BSD notices are in *Rust crates* below.

```text
BSD License

For Zstandard software

Copyright (c) Meta Platforms, Inc. and affiliates. All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

 * Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

 * Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

 * Neither the name Facebook, nor Meta, nor the names of its contributors may
   be used to endorse or promote products derived from this software without
   specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

---

## Rust standard library

**Material:** the Rust standard library (`std`, `core`, `alloc` and their
runtime support), compiled into `tiger-setup.exe` and `tigersetup-setup.exe`
by the Rust compiler.

**Source:** <https://github.com/rust-lang/rust>
**Licence:** MIT (of `MIT OR Apache-2.0`)

**Obligation:** the copyright notice and the MIT permission notice, which the
generated section below carries once for every MIT component.

- Rust standard library: Copyright (c) The Rust Project Contributors

---

## SQLite

**Material:** the SQLite library, compiled into `tiger-setup.exe` and
`tigersetup-setup.exe` through the `libsqlite3-sys` crate's bundled copy.

**Source:** <https://sqlite.org/>
**Licence:** public domain (<https://sqlite.org/copyright.html>). No notice is
required; it is listed so the distribution is fully described.

---

## Microsoft C runtime

**Material:** the parts of the Microsoft C runtime (the Visual C++ runtime and
the Universal CRT) the compiler links statically into `tiger-setup.exe`,
`tigersetup-setup.exe` and `tigersetup-loader.exe`, so that none of them needs
a runtime installed on the target machine.

**Licence:** the Microsoft Visual Studio and Windows SDK licence terms, which
permit distributing these libraries as part of a program built with them. No
notice or attribution is required; it is listed so the distribution is fully
described.

---
## Segoe UI (subsets in the help PDF)

**Material:** the installed `help\TigerSetup-Help.pdf` embeds subsets of the
Segoe UI and Segoe UI Bold fonts, which the PDF renderer took from Windows.

**Licence:** Microsoft's font licence, which ships with Windows. The fonts'
own embedding permission (OpenType `OS/2.fsType` `0x0008`, editable
embedding) allows them to be embedded in a document that is distributed. No
notice or attribution is required, and no font file is distributed.

---

## Rust crates

The crates linked into the engine and the builder, as `Cargo.lock` pins them.
Procedural macros and build-time tools run only on the build machine and are
not part of any binary, so they are not listed. Where a crate is offered under
alternative licences, the one selected is named and the notice it requires is
carried.

<!-- rust-crates:begin (generated by eng/Update-ThirdPartyNotices.ps1; do not edit) -->

| Crate | Version | Licence (selected) | Linked into |
|---|---|---|---|
| `anstream` | 1.0.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `anstyle` | 1.0.14 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `anstyle-parse` | 1.0.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `anstyle-query` | 1.1.5 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `anstyle-wincon` | 3.0.11 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `arraydeque` | 0.5.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `bitflags` | 2.13.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `block-buffer` | 0.10.4 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `bytes` | 1.12.1 | MIT | builder, engine |
| `cfg-if` | 1.0.4 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `clap` | 4.6.6 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `clap_builder` | 4.6.6 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `clap_lex` | 1.1.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `colorchoice` | 1.0.5 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `cpufeatures` | 0.2.17 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `crc32fast` | 1.5.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `crypto-common` | 0.1.7 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `digest` | 0.10.7 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `equivalent` | 1.0.2 | MIT (of `Apache-2.0 OR MIT`) | builder, engine |
| `fallible-iterator` | 0.3.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `fallible-streaming-iterator` | 0.1.9 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `fastrand` | 2.5.0 | MIT (of `Apache-2.0 OR MIT`) | builder |
| `flate2` | 1.1.10 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `foldhash` | 0.2.0 | Zlib | builder, engine |
| `generic-array` | 0.14.7 | MIT | builder, engine |
| `getrandom` | 0.4.3 | MIT (of `MIT OR Apache-2.0`) | builder |
| `glob` | 0.3.4 | MIT (of `MIT OR Apache-2.0`) | builder |
| `hashbrown` | 0.17.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `hashlink` | 0.12.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `indexmap` | 2.14.2 | MIT (of `Apache-2.0 OR MIT`) | builder, engine |
| `is_terminal_polyfill` | 1.70.2 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `itoa` | 1.0.18 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `libsqlite3-sys` | 0.38.2 | MIT | builder, engine |
| `memchr` | 2.8.3 | MIT (of `Unlicense OR MIT`) | builder, engine |
| `once_cell` | 1.21.4 | MIT (of `MIT OR Apache-2.0`) | builder |
| `once_cell_polyfill` | 1.70.2 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `prost` | 0.14.4 | Apache-2.0 | builder, engine |
| `rusqlite` | 0.40.2 | MIT | builder, engine |
| `serde` | 1.0.229 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `serde_core` | 1.0.229 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `serde_json` | 1.0.151 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `serde_spanned` | 1.1.1 | MIT (of `MIT OR Apache-2.0`) | builder |
| `sha2` | 0.10.9 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `smallvec` | 1.16.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `strsim` | 0.11.1 | MIT | builder, engine |
| `tempfile` | 3.27.0 | MIT (of `MIT OR Apache-2.0`) | builder |
| `toml` | 1.1.5+spec-1.1.0 | MIT (of `MIT OR Apache-2.0`) | builder |
| `toml_datetime` | 1.1.1+spec-1.1.0 | MIT (of `MIT OR Apache-2.0`) | builder |
| `toml_parser` | 1.1.3+spec-1.1.0 | MIT (of `MIT OR Apache-2.0`) | builder |
| `toml_writer` | 1.1.2+spec-1.1.0 | MIT (of `MIT OR Apache-2.0`) | builder |
| `typed-path` | 0.12.3 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `typenum` | 1.20.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `utf8parse` | 0.2.2 | MIT (of `Apache-2.0 OR MIT`) | builder, engine |
| `windows-link` | 0.2.1 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `windows-sys` | 0.61.2 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `winnow` | 1.0.4 | MIT | builder |
| `yaml-rust2` | 0.12.0 | MIT (of `MIT OR Apache-2.0`) | builder, engine |
| `zip` | 8.6.0 | MIT | builder, engine |
| `zlib-rs` | 0.6.7 | Zlib | builder, engine |
| `zmij` | 1.0.23 | MIT | builder, engine |
| `zstd` | 0.14.0 | BSD-3-Clause | builder, engine |
| `zstd-safe` | 8.0.0 | BSD-3-Clause | builder, engine |
| `zstd-sys` | 2.1.0+zstd.1.5.7 | BSD-3-Clause | builder, engine |

### Apache-2.0

- `prost` 0.14.4: Copyright (c) Dan Burkert, Lucio Franco, Casper Meijn, Tokio Contributors

```text
Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS

APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### BSD-3-Clause

`zstd` 0.14.0:

```text
BSD 3-Clause License

Copyright (c) 2026, Alexandre Bury

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

`zstd-safe` 8.0.0:

```text
BSD 3-Clause License

Copyright (c) 2026, Alexandre Bury

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

`zstd-sys` 2.1.0+zstd.1.5.7:

```text
BSD 3-Clause License

Copyright (c) 2026, Alexandre Bury

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### MIT

- `anstream` 1.0.0: Copyright (c) Individual contributors
- `anstyle` 1.0.14: Copyright (c) Individual contributors
- `anstyle-parse` 1.0.0: Copyright (c) Individual contributors
- `anstyle-query` 1.1.5: Copyright (c) Individual contributors
- `anstyle-wincon` 3.0.11: Copyright (c) Individual contributors
- `arraydeque` 0.5.1: Copyright (c) 2018 Andy Lok <andylokandy@hotmail.com>
- `bitflags` 2.13.1: Copyright (c) 2014 The Rust Project Developers
- `block-buffer` 0.10.4: Copyright (c) 2018-2019 The RustCrypto Project Developers
- `bytes` 1.12.1: Copyright (c) 2018 Carl Lerche
- `cfg-if` 1.0.4: Copyright (c) 2014 Alex Crichton
- `clap` 4.6.6: Copyright (c) Individual contributors
- `clap_builder` 4.6.6: Copyright (c) Individual contributors
- `clap_lex` 1.1.0: Copyright (c) Individual contributors
- `colorchoice` 1.0.5: Copyright (c) Individual contributors
- `cpufeatures` 0.2.17: Copyright (c) 2020-2025 The RustCrypto Project Developers
- `crc32fast` 1.5.1: Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
- `crypto-common` 0.1.7: Copyright (c) 2021 RustCrypto Developers
- `digest` 0.10.7: Copyright (c) 2017 Artyom Pavlov
- `equivalent` 1.0.2: Copyright (c) 2016--2023
- `fallible-iterator` 0.3.0: Copyright (c) 2015 The rust-openssl-verify Developers
- `fallible-streaming-iterator` 0.1.9: Copyright (c) 2016 The fallible-streaming-iterator Developers
- `fastrand` 2.5.0: Copyright (c) Stjepan Glavina
- `flate2` 1.1.10: Copyright (c) 2014-2026 Alex Crichton
- `generic-array` 0.14.7: Copyright (c) 2015 Bartłomiej Kamiński
- `getrandom` 0.4.3: Copyright (c) 2018-2026 The rust-random Project Developers; Copyright (c) 2014 The Rust Project Developers
- `glob` 0.3.4: Copyright (c) 2014 The Rust Project Developers
- `hashbrown` 0.17.1: Copyright (c) 2016 Amanieu d'Antras
- `hashlink` 0.12.1: Copyright (c) the hashlink authors
- `indexmap` 2.14.2: Copyright (c) 2016--2017
- `is_terminal_polyfill` 1.70.2: Copyright (c) Individual contributors
- `itoa` 1.0.18: Copyright (c) David Tolnay
- `libsqlite3-sys` 0.38.2: Copyright (c) 2014 The rusqlite developers
- `memchr` 2.8.3: Copyright (c) 2015 Andrew Gallant
- `once_cell` 1.21.4: Copyright (c) Aleksey Kladov
- `once_cell_polyfill` 1.70.2: Copyright (c) Individual contributors
- `rusqlite` 0.40.2: Copyright (c) 2014 The rusqlite developers
- `serde` 1.0.229: Copyright (c) Erick Tryzelaar, David Tolnay
- `serde_core` 1.0.229: Copyright (c) Erick Tryzelaar, David Tolnay
- `serde_json` 1.0.151: Copyright (c) Erick Tryzelaar, David Tolnay
- `serde_spanned` 1.1.1: Copyright (c) Individual contributors
- `sha2` 0.10.9: Copyright (c) 2006-2009 Graydon Hoare; Copyright (c) 2009-2013 Mozilla Foundation; Copyright (c) 2016 Artyom Pavlov
- `smallvec` 1.16.0: Copyright (c) 2018 The Servo Project Developers
- `strsim` 0.11.1: Copyright (c) 2015 Danny Guo; Copyright (c) 2016 Titus Wormer <tituswormer@gmail.com>; Copyright (c) 2018 Akash Kurdekar
- `tempfile` 3.27.0: Copyright (c) 2015 Steven Allen
- `toml` 1.1.5+spec-1.1.0: Copyright (c) Individual contributors
- `toml_datetime` 1.1.1+spec-1.1.0: Copyright (c) Individual contributors
- `toml_parser` 1.1.3+spec-1.1.0: Copyright (c) Individual contributors
- `toml_writer` 1.1.2+spec-1.1.0: Copyright (c) Individual contributors
- `typed-path` 0.12.3: Copyright (c) Chip Senkbeil
- `typenum` 1.20.1: Copyright (c) 2014 Paho Lurie-Gregg
- `utf8parse` 0.2.2: Copyright (c) 2016 Joe Wilm
- `windows-link` 0.2.1: Copyright (c) Microsoft Corporation.
- `windows-sys` 0.61.2: Copyright (c) Microsoft Corporation.
- `winnow` 1.0.4: Copyright (c) the winnow authors
- `yaml-rust2` 0.12.0: Copyright (c) Yuheng Chen, Ethiraric, David Aguilar
- `zip` 8.6.0: Copyright (c) 2014 Mathijs van de Nes
- `zmij` 1.0.23: Copyright (c) David Tolnay

```text
MIT License

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

### Zlib

`foldhash` 0.2.0:

```text
Copyright (c) 2024 Orson Peters

This software is provided 'as-is', without any express or implied warranty. In
no event will the authors be held liable for any damages arising from the use of
this software.

Permission is granted to anyone to use this software for any purpose, including
commercial applications, and to alter it and redistribute it freely, subject to
the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim
    that you wrote the original software. If you use this software in a product,
    an acknowledgment in the product documentation would be appreciated but is
    not required.

2. Altered source versions must be plainly marked as such, and must not be
    misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

`zlib-rs` 0.6.7:

```text
(C) 2024 Trifecta Tech Foundation 

This software is provided 'as-is', without any express or implied
warranty. In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

<!-- rust-crates:end -->