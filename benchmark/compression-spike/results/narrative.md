## Outliers

- **Git for Windows is the corpus's duplication test.** The same MSYS2/MinGW
  binaries sit under `bin/`, `cmd/`, `mingw64/bin/` and `usr/bin/`, tens of
  megabytes apart in path order. Every variable that reaches across that
  distance pays off there and almost nowhere else: zstd-19 goes from 24.9% at
  its 8 MiB window to 16.7% at 128 MiB; LZMA2-9 from 19.8% (64 MiB) to 15.0%
  (128 MiB); and sorting the stream by extension (`ext-path`) brings the copies
  within reach of a 64 MiB dictionary on its own, LZMA2-9 dropping 23% for that
  one application while the rest of the corpus moves by under 2%. It is the
  only payload where zstd at a large window beat LZMA2 at its default
  dictionary, and it is why the corpus-wide "does order matter" answer has to
  be read per application.
- **VS Code is the largest payload and the one with the least structure to
  exploit** beyond its size: 1.05 GB of Electron, with 17% of it in `.pak`,
  `.asar` and `.wasm` resources that no family rule knows. It is where the
  window matters most in absolute bytes (zstd-19: 257 → 236 MB from an 8 to a
  128 MiB window) and where grouping earns its largest result outside Git
  (−2.9% for zstd-19-w27, −1.8% for LZMA2-9).
- **Wireshark is the worst ratio for every codec** (30.6–33.3% at the strongest
  settings): 84% of it is native code in large Qt, FFmpeg and dissector DLLs, and
  10 MB of user-guide PNGs stays raw. It is the application that makes the case
  for an x86 branch-converter filter (see the BCJ section) rather than for a
  stronger match finder.
- **qBittorrent is one file.** 78% of its 231 MB is `qbittorrent.pdb`, which
  ships upstream and compresses to 18% with LZMA2; the solid-vs-per-file and
  layout questions are moot for it (±0.03%).
- **The .NET-based Tiger CLI applications (TigerWrap, TigerSqlCmd,
  TigerMarkView) and TigerKeyring show the solid stream at its best on small
  payloads**: 27–30% smaller than per-file compression, because a
  framework-dependent .NET publish carries the same satellite resource
  assemblies for a dozen languages and Rust executables share their runtime.
  Their ratios track the OSS corpus (18–31%), so nothing here is specific to
  IT Tiger payloads.
- **What 0.7.1 stores is almost right.** 858 files, 0.7% of the corpus, and the
  candidates would recover 0.1% of the corpus by compressing them individually.
  The individual misses are instructive but small: VS Code's 43
  accessibility-signal MP3s are 70% padding (655 KB recoverable), a handful of
  PNGs and GIFs were written with a weak deflater (1.4 MB), and Inkscape's
  `Textures.svg` (1.25 MB of base64-embedded images) is the corpus's one probe
  decision — a false positive, since zstd-19 alone takes 31% off it. Admitting
  everything to the solid stream (`all`) is 0.4% smaller than 0.7.1's rule and
  needs no classifier at all.
- **Text-first against binary-first never mattered corpus-wide** (the two
  directions agree to within 0.15% for all three settings) and has no
  consistent direction per application: the only differences above 0.5% are
  TigerWrap and TigerSqlCmd at zstd-19's 8 MiB window (2.1–2.3% in favour of
  text-first) and Inkscape at the large windows (0.8–0.9% in favour of
  binary-first). Family grouping itself buys under 1% once the window is
  large; the only ordering with a measurable corpus-wide effect is the
  semantics-free extension-then-path sort, and its effect is the Git
  duplication case plus about half a percent elsewhere.

## Recommendation

### Evidence

1. **Codec.** At equal reach, LZMA2 is about 8.5% smaller than zstd: with a
   64 MiB window LZMA2-9 produces 795.0 MB against zstd-19's 862.8 MB, with
   128 MiB 767.7 against 835.9 MB. zstd-22 (btultra2 at its widest) closes
   only 0.9% of that for 60% more build time. On the four benchmark
   applications, LZMA2-9 puts a TigerSetup payload at parity with Inno Setup's
   `lzma2/max` and NSIS's `/SOLID lzma` (ShareX +0.2%, qBittorrent +4.2%,
   VLC −7.0%, WinMerge +11.6% — the last being the 2.5 MB engine, not the
   payload), while zstd-19 at 128 MiB leaves 9–18% of the gap on three of the
   four. Either codec closes most of the observed 47–87% gap; LZMA2 closes it.
2. **Decompression.** zstd decodes at 910–950 MiB/s single-threaded, LZMA2 at
   113–126 MiB/s — 7.6× — and both are far above what the engine's file
   writes and journal commits sustain. Over the whole corpus that is 3.6 s
   against 27.4 s of decode CPU; for WinMerge, 0.07 s against 0.46 s in an
   install the benchmark measured at 10.9 s. zstd's speed advantage is real
   and large in ratio, and small in the installer's wall clock.
3. **Build cost.** LZMA2-9 and zstd-19 (128 MiB) compress at 3.1 and 3.5
   MiB/s single-threaded: 5–6 minutes for VS Code, 3 minutes for ShareX,
   half a minute for WinMerge — the same order as the 0.7.1 DEFLATE-9 search
   on the largest payloads (47 s for ShareX) times four, and the same order as
   Inno Setup and NSIS took in the benchmark. LZMA2-9e (+25% time for
   −0.17%) and zstd-22 (+60% for −0.9%) do not pay for themselves.
4. **Memory.** Decode memory is the window: 70 MB for LZMA2-9, 134 MB for
   zstd-19 at 128 MiB, 133 MB for LZMA2 at 128 MiB, and capped at the payload
   size for small packages. Encode memory is 680 MB for LZMA2-9 (64 MiB),
   1.36 GB at 128 MiB and 2.7 GB at 256 MiB; 284 MB for zstd-19 at 128 MiB.
5. **Solid compression helps materially**: −12.9% (zstd-19) and −17.5%
   (LZMA2-9) over per-file compression of the same files, and 27–38% on the
   applications with duplicated binaries or satellite assemblies.
6. **Classification.** 0.7.1's signature/extension rule keeps 0.7% of the
   corpus raw at a cost of 0.1%; the sampled probe decided one file in
   28,399 and got it wrong. Admitting every file to the solid stream costs
   nothing (−0.4%).
7. **Grouping and order.** Family grouping is worth −0.3% to −1.0%
   corpus-wide and its direction does not matter (text-first and binary-first
   agree to within 0.15% corpus-wide; per application the largest difference
   is 2.3%, with no consistent winner). Extension-then-path
   ordering is worth −0.7% (zstd-19), −0.9% (zstd-19, 128 MiB) and −2.9%
   (LZMA2-9), the last almost entirely Git for Windows.
8. **Determinism.** Every repeated compression — separate processes, hours
   apart, single-threaded, no multithreading parameter set — produced
   byte-identical output for DEFLATE, zstd (with and without long-distance
   matching) and LZMA2, in one stream and in blocks.
9. **Blocks.** Independently compressed blocks of whole files cost, over the
   ten payloads of 32 MB or more, +4.5% (32 MiB), +3.0% (64 MiB) and +1.6%
   (128 MiB) with LZMA2-9 in path order; +7.5%, +6.1% and +3.5% with zstd-19
   at its 128 MiB window, whose long-range matching is exactly what a block
   boundary cuts. Extension ordering lowers the LZMA2 penalty at 64 and 128 MiB
   (+3.8%, +1.1%) but not at 32 MiB. The cost is concentrated where the
   duplication is: Git for Windows +6–15% (+9–41% in extension order, where
   the copies it had brought together get split again), VLC +3–14%, VS Code
   +2–5%; ShareX, Wireshark, qBittorrent and every payload under 64 MB are
   within 1.5% at 64 MiB and within 0.3% at 128 MiB. Decoding a set of blocks
   costs about the same CPU as one stream (the differences in the table were
   measured under load).
10. **BCJ.** liblzma's x86 branch converter ahead of LZMA2-9 takes another
    2.2% off the corpus (795.0 → 777.8 MB, −39.7% against 0.7.1), 2–4% on the
    typical application and 7.6–10% on the two payloads that are one big
    native executable (Tiger3dForge, WinSCP); its decode rate, measured under
    load, was 112 MiB/s against LZMA2-9's 125 MiB/s unloaded, so the filter
    costs at most about a tenth of an already slow decode. It has one sharp
    edge: the converter
    rewrites relative branch targets into absolute ones using the byte's
    position in the stream, so two identical executables at different offsets
    stop being byte-identical — Git for Windows loses 3.9% of its 128 MiB-
    dictionary gain. Applied per file (position reset at each file boundary),
    the two would compose; the spike did not test that form.

### Engineering judgment

_The judgment below is the spike's, made on the ratio criterion it was
given; the measurements above stand as recorded. The product decided
otherwise on a criterion the spike did not weigh: TigerSetup optimizes for
the shortest reliable installation transaction, and the payload is decoded
inside that transaction, so decode speed was ranked above bytes. TigerSetup
0.8.0 therefore ships one solid `zstd-19-w27` stream with every file in it,
extension-then-path order recorded in the metadata's payload index, and no
raw region — the "admit everything" alternative priced at −0.4% below. The
record of that decision is `TigerSetup-Design.md` §10.4._

- **LZMA2 for the solid region, at preset 9 (64 MiB dictionary), with an
  x86 BCJ filter.** The primary criterion is ratio, and LZMA2 wins it by a
  margin (8.5%) that zstd cannot buy back with a wider window or a higher
  level; on the benchmark applications it is the difference between "within
  a few percent of Inno Setup and NSIS" and "10–18% larger". Its decode speed
  is the price, and at 120 MiB/s it is not the bottleneck of an installer that
  fsyncs a journal per operation. The BCJ filter is the measured stream-wide
  form (−2.2%); a per-file form that would also keep duplicated executables
  identical is an implementation choice for the format work, and was not
  measured.
- **zstd would be the right answer if decode CPU mattered more than bytes** —
  a network install where the download is not the bound, or a payload decoded
  many times. Neither describes a Setup.exe. At the same window it is 8.5%
  larger; its 7.6× decode advantage is 0.4 s on WinMerge.
- **Window/dictionary: 64 MiB by default; 128 MiB as a build option** for a
  package that a build report shows would benefit (Git-like duplication). The
  jump from 64 to 128 MiB is worth 3.4% corpus-wide but doubles encode memory
  to 1.36 GB and decode memory to 133 MB; 256 MiB is not worth its 2.7 GB.
- **Order: extension-then-path, or plain path order.** Both are trivially
  deterministic; the semantic families cost a classifier and buy nothing over
  the extension sort. If the format ever records an order, it should be the
  file list itself, so the order is data and not a rule the engine must
  reproduce.
- **Partition: keep the signature/extension rule as a raw region for
  already-compressed files, drop the sampled probe.** The rule is right about
  which files are incompressible (the codecs recover 0.1%), costs nothing, and
  gives the engine a region it can copy without a codec; the probe adds a
  code path for one wrong decision in the corpus. Admitting everything is the
  simplest alternative and 0.4% smaller — a legitimate KISS choice if the raw
  region has no other use in the format.
- **One stream against blocks: 64 MiB blocks of whole files are the sensible
  bound if the format wants one; 32 MiB is too expensive; 128 MiB is nearly
  free but bounds little.** At 64 MiB the corpus pays 3% (1.1–3.8% depending
  on order) for blocks that a repair or a single-file extraction can decode
  without touching the rest, that localize corruption, and that a future
  parallel decoder could split — and a block never has to be larger than the
  decoder's dictionary, so it also bounds decode memory. The one solid stream
  is the right default for the *build*: the bytes that ship should be one
  stream unless the format's own needs (repair, random access) justify the
  3%; that is the Architect's trade, and this spike only prices it. Whatever
  is chosen, block boundaries at file boundaries and a decoder per block are
  what was measured here; a format with an index of (file → block, offset)
  needs nothing more from the codec.

### Remaining uncertainty

- **The engine's real decode path.** Every decode here ran into a CRC; the
  engine writes files, journals, and may want to verify SHA-256 per stream.
  The 120 MiB/s LZMA2 figure is the codec alone, not a prediction of install
  time; the benchmark's lab rows will measure that once a format exists.
- **A decoder for the engine.** liblzma (C, 0BSD) or a pure-Rust LZMA2
  decoder (`lzma-rs`, MIT) is a product-dependency and binary-size decision
  the spike did not make; the engine needs only the decoder, and its size and
  speed differ between those two. zstd has the same choice (libzstd or
  `ruzstd`).
- **Multi-threaded builds.** Everything here is single-threaded and
  deterministic by construction. liblzma's multi-threaded encoder changes the
  output (it splits the stream into independently coded blocks) and zstd's
  changes it too; a parallel build is a different artifact and would have to
  be deterministic on its own terms (fixed block size, fixed job size).
- **Memory on the smallest supported machines.** 70 MB (64 MiB dictionary)
  is comfortable on the Windows 10 1809 baseline; 133 MB is probably fine and
  was not tested on a constrained VM.
- **The corpus is 15 applications**, x64, with one x86 build (WinSCP) and no
  ARM64; GIMP could not be acquired. The IT Tiger payloads behave like the
  OSS ones, so the choice is not specific to either.
