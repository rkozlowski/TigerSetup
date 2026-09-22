# TigerSetup 0.10.0 across the broad corpus: Inno Setup 7.1.0, NSIS 3.12, fifteen real payloads

The installer-technology benchmark ([`report.md`](../report.md), [`report-0.9.0.md`](report-0.9.0.md))
compared TigerSetup with Inno Setup and NSIS on four applications packaged
under their real upstream contracts. This campaign asks whether those
conclusions generalize: the same three technologies, **frozen TigerSetup
0.10.0**, and the **fifteen real application payloads** the payload-compression
spike pinned ([`compression-spike/report.md`](compression-spike/report.md)) —
from four files to twelve thousand, from 2 MB to 1 GB — each packaged under
one deliberately small common contract ([`packages/broad/contract.md`](packages/broad/contract.md):
install the canonical tree into the named root in user scope, register with
Add/Remove Programs, uninstall from the registered uninstaller, remove
everything), so what differs between two installers of one application is
the technology, not the script. The minimal no-payload installer of each
technology is measured beside them as the fixed-overhead anchor.

Every number below is read from `benchmark/results/0.10.0-broad/*.json` by
`benchmark/scripts/New-BroadReport.ps1`; nothing is transcribed by hand.
`build.csv`, `sizes.csv` and `runtime.csv` beside the JSON carry the same
per-row figures for external analysis. Every figure is a single measurement:
one build per definition on the build machine, one lab row per installer
on the clean baseline (see *Limitations*). MB and KB here are 2^20 and
2^10 bytes; byte columns are exact.

| Evidence | What | Where | When |
|---|---|---|---|
| Corpus | 15 payloads, 28,399 files, 3,467 MB, verified against the spike inventories | the build machine | 2026-09-21 21:44 +01:00 |
| Builds | 48 builds (16 TigerSetup, 16 Inno Setup, 16 NSIS), serial, under one process-tree meter | the build machine (below) | 2026-09-21 21:53 +01:00 → 2026-09-21 22:32 +01:00 |
| Runtime | 45 rows, 45 passed | `TigerWinLab-Win11-Clean` | 2026-09-21 22:47 +01:00 → 2026-09-22 00:23 +01:00 |

## A. Corpus

The fifteen payloads as the compression spike pinned them
(`compression-spike/results/corpus.json`), verified on disk file by file
against its inventories before anything was built (`corpus.json`,
`canonical/<App>.json`: every file's path, size and SHA-256). The family
mix is the spike's own classification of each file (native code, managed
code, text, resources, already-compressed content, other), by bytes.

| App | Version | Files | Bytes | Avg file | Largest file | native | managed | text | precompressed | other | Notes |
|---|---|---:|---:|---:|---|---:|---:|---:|---:|---:|---|
| ShareX | 21.0.0 | 1,230 | 541,619,146 | 430 KB | `ffmpeg.exe` (192.9 MB) | 55.1% | 40.5% | 3.8% | 0.1% | 0.5% |  |
| WinMerge | 2.16.58.2 | 472 | 81,636,621 | 169 KB | `TreeSitterGrammars/tree-sitter-fsharp.dll` (10.4 MB) | 81.6% | 0.0% | 9.9% | 0.4% | 8.2% |  |
| qBittorrent | 5.2.3 | 38 | 231,534,246 | 5,950 KB | `qbittorrent.pdb` (173.4 MB) | 18.7% | 2.8% | 0.0% | 0.0% | 0.0% |  |
| VLC | 3.0.23 | 583 | 191,373,841 | 321 KB | `plugins/gui/libqt_plugin.dll` (16.7 MB) | 75.8% | 22.9% | 0.3% | 0.6% | 0.2% |  |
| TigerMarkView | 0.8.1 | 56 | 33,092,743 | 577 KB | `libSkiaSharp.dll` (11.1 MB) | 59.9% | 39.9% | 0.2% | 0.0% | 0.0% | IT Tiger release payload |
| TigerWrap | 0.9.1 | 62 | 15,922,990 | 251 KB | `cli/runtimes/win/lib/net9.0/Microsoft.Data.SqlClient.dll` (1.8 MB) | 11.9% | 79.8% | 7.8% | 0.0% | 0.0% | IT Tiger release payload |
| TigerSqlCmd | 0.8.7 | 68 | 16,023,249 | 230 KB | `cli/runtimes/win/lib/net9.0/Microsoft.Data.SqlClient.dll` (1.8 MB) | 11.9% | 85.2% | 2.1% | 0.0% | 0.0% | IT Tiger release payload |
| Tiger3dForge | 0.17.0.2 | 89 | 41,560,997 | 456 KB | `bin/TKGeomAlgo.dll` (4.2 MB) | 96.4% | 0.0% | 1.4% | 2.2% | 0.0% | IT Tiger release payload |
| TigerKeyring | 0.2.0 | 4 | 1,980,965 | 484 KB | `tiger-keyring.exe` (1.3 MB) | 99.8% | 0.0% | 0.2% | 0.0% | 0.0% | 2 reserved `.tigersetup/` entries excluded from every package (1,977,856 B, 2 files installed); IT Tiger release payload |
| GitForWindows | 2.55.0.5 | 9,584 | 403,630,328 | 41 KB | `mingw64/bin/git-lfs.exe` (12.1 MB) | 66.6% | 7.8% | 20.4% | 0.1% | 4.4% |  |
| VSCode | 1.138.0 | 2,484 | 1,046,165,352 | 411 KB | `Code.exe` (224.2 MB) | 55.9% | 0.0% | 25.1% | 0.5% | 17.9% |  |
| Wireshark | 4.6.8 | 1,354 | 319,059,572 | 230 KB | `libwireshark.dll` (89.7 MB) | 84.8% | 3.9% | 7.5% | 3.2% | 0.0% |  |
| WinSCP | 6.5.7 | 4 | 24,389,189 | 5,954 KB | `WinSCP.exe` (23.0 MB) | 99.8% | 0.0% | 0.2% | 0.0% | 0.0% |  |
| Inkscape | 1.4.4 | 12,152 | 660,170,071 | 53 KB | `bin/libopenblas.dll` (41.7 MB) | 50.3% | 9.5% | 26.8% | 1.1% | 11.7% |  |
| NotepadPlusPlus | 8.9.8 | 219 | 27,100,036 | 121 KB | `notepad++.exe` (8.1 MB) | 39.4% | 0.0% | 60.2% | 0.0% | 0.0% |  |

Shape at a glance: TigerKeyring, WinSCP, qBittorrent carry the fewest files;
Inkscape, GitForWindows, VSCode the most;
VSCode, Inkscape, ShareX the most bytes.

Excluded, as the spike excluded it: **GIMP 3.2.6** — Not acquired: the only official Windows artifact is gimp-3.2.6-setup.exe (190,358,080 bytes, SHA-256 9337cccb...), an Inno Setup 7.0.0.3 installer. No open unpacker reads Inno Setup 7 (innoextract 1.9 stops at 6.2.2, its master at 6.7.0; 7-Zip has no Inno reader), and the MSIX is a Store-only artifact, so the payload could only be obtained by executing the installer. That is disproportionate for a spike whose corpus already holds a GTK/Python/gettext application of the same shape (Inkscape).

## B. Package size

Installer bytes per application and technology (the exact bytes of the
built artifacts, `build.json`), TigerSetup against each competitor and
against the smaller of the two, and each installer as a fraction of its
payload. Minimal is in section I: its ratios are all fixed overhead.

| App | Payload | Inno Setup | NSIS | TigerSetup 0.10.0 | TS vs IS | TS vs NSIS | TS vs smaller | IS/payload | NSIS/payload | TS/payload |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ShareX | 541,619,146 | 139,596,355 | 139,987,121 | 151,478,648 | +8.5% | +8.2% | +8.5% (IS) | 0.26 | 0.26 | 0.28 |
| WinMerge | 81,636,621 | 18,784,902 | 17,195,440 | 18,988,834 | +1.1% | +10.4% | +10.4% (NSIS) | 0.23 | 0.21 | 0.23 |
| qBittorrent | 231,534,246 | 44,445,852 | 43,412,133 | 48,055,301 | +8.1% | +10.7% | +10.7% (NSIS) | 0.19 | 0.19 | 0.21 |
| VLC | 191,373,841 | 46,893,921 | 45,691,994 | 42,413,856 | −9.6% | −7.2% | −7.2% (NSIS) | 0.25 | 0.24 | 0.22 |
| TigerMarkView | 33,092,743 | 10,987,450 | 9,352,274 | 11,306,672 | +2.9% | +20.9% | +20.9% (NSIS) | 0.33 | 0.28 | 0.34 |
| TigerWrap | 15,922,990 | 5,047,799 | 3,023,323 | 4,450,380 | −11.8% | +47.2% | +47.2% (NSIS) | 0.32 | 0.19 | 0.28 |
| TigerSqlCmd | 16,023,249 | 5,293,560 | 3,267,577 | 4,714,731 | −10.9% | +44.3% | +44.3% (NSIS) | 0.33 | 0.20 | 0.29 |
| Tiger3dForge | 41,560,997 | 13,637,511 | 12,501,641 | 14,342,326 | +5.2% | +14.7% | +14.7% (NSIS) | 0.33 | 0.30 | 0.35 |
| TigerKeyring | 1,977,856 | 2,578,499 | 547,129 | 1,761,244 | −31.7% | +221.9% | +221.9% (NSIS) | 1.30 | 0.28 | 0.89 |
| GitForWindows | 403,630,328 | 91,699,969 | 67,724,032 | 68,012,084 | −25.8% | +0.4% | +0.4% (NSIS) | 0.23 | 0.17 | 0.17 |
| VSCode | 1,046,165,352 | 236,066,109 | 238,494,641 | 230,297,757 | −2.4% | −3.4% | −2.4% (IS) | 0.23 | 0.23 | 0.22 |
| Wireshark | 319,059,572 | 97,818,292 | 99,073,975 | 104,302,442 | +6.6% | +5.3% | +6.6% (IS) | 0.31 | 0.31 | 0.33 |
| WinSCP | 24,389,189 | 8,358,765 | 6,970,243 | 8,501,640 | +1.7% | +22.0% | +22.0% (NSIS) | 0.34 | 0.29 | 0.35 |
| Inkscape | 660,170,071 | 120,599,904 | 120,827,148 | 128,857,706 | +6.8% | +6.6% | +6.8% (IS) | 0.18 | 0.18 | 0.20 |
| NotepadPlusPlus | 27,100,036 | 8,219,539 | 6,353,224 | 7,664,185 | −6.8% | +20.6% | +20.6% (NSIS) | 0.30 | 0.23 | 0.28 |
| **Total** | 3,635,256,237 | 850,028,427 | 814,421,895 | 845,147,806 | −0.6% | +3.8% | | 0.23 | 0.22 | 0.23 |

The total is one number over a corpus dominated by its largest payloads;
section E is the distribution over the fifteen.

## C. Build statistics

Every build under `ProcessTreeMeter.psm1`: the compiler started suspended
in a job object of its own and resumed; wall clock until the job holds no
process; CPU time (user + kernel, exited processes included) and peak
commit (all processes together) from the job's kernel accounting; peak
working set sampled every 20 ms (the largest sum of the tree's working
sets in one sample). Builds ran serially in the order the table gives
within an application (the technology order rotates with the application),
one build each.

| App | Technology | Wall | CPU | CPU/wall | Peak commit | Peak tree WS | Installer bytes | Samples / processes |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| ShareX | TigerSetup | 143.4 s | 142.7 s | 1.00 | 279.1 MB | 282.2 MB | 151,478,648 | 4575 / 1 |
| ShareX | Inno Setup | 59.1 s | 103.5 s | 1.75 | 113.4 MB | 118.3 MB | 139,596,355 | 1888 / 1 |
| ShareX | NSIS | 147.3 s | 146.9 s | 1.00 | 98.3 MB | 140.5 MB | 139,987,121 | 4702 / 2 |
| WinMerge | Inno Setup | 8.4 s | 13.0 s | 1.54 | 113.4 MB | 117.5 MB | 18,784,902 | 271 / 1 |
| WinMerge | NSIS | 16.3 s | 16.2 s | 1.00 | 98.0 MB | 140.5 MB | 17,195,440 | 522 / 2 |
| WinMerge | TigerSetup | 18.1 s | 17.8 s | 0.98 | 228.3 MB | 231.0 MB | 18,988,834 | 580 / 1 |
| qBittorrent | NSIS | 60.0 s | 59.9 s | 1.00 | 97.7 MB | 136.5 MB | 43,412,133 | 1914 / 2 |
| qBittorrent | TigerSetup | 69.3 s | 68.8 s | 0.99 | 277.7 MB | 280.7 MB | 48,055,301 | 2213 / 1 |
| qBittorrent | Inno Setup | 39.0 s | 45.2 s | 1.16 | 112.1 MB | 117.0 MB | 44,445,852 | 1250 / 1 |
| VLC | TigerSetup | 49.2 s | 48.8 s | 0.99 | 278.8 MB | 281.4 MB | 42,413,856 | 1573 / 1 |
| VLC | Inno Setup | 22.2 s | 36.4 s | 1.64 | 113.4 MB | 117.6 MB | 46,893,921 | 710 / 1 |
| VLC | NSIS | 48.8 s | 48.7 s | 1.00 | 98.2 MB | 140.5 MB | 45,691,994 | 1557 / 2 |
| TigerMarkView | Inno Setup | 4.6 s | 6.9 s | 1.51 | 112.2 MB | 117.1 MB | 10,987,450 | 147 / 1 |
| TigerMarkView | NSIS | 9.0 s | 9.0 s | 1.00 | 97.8 MB | 136.5 MB | 9,352,274 | 290 / 2 |
| TigerMarkView | TigerSetup | 9.2 s | 8.9 s | 0.97 | 133.0 MB | 136.1 MB | 11,306,672 | 295 / 1 |
| TigerWrap | NSIS | 3.4 s | 3.4 s | 0.99 | 115.4 MB | 123.7 MB | 3,023,323 | 111 / 2 |
| TigerWrap | TigerSetup | 3.7 s | 3.4 s | 0.92 | 108.6 MB | 111.2 MB | 4,450,380 | 120 / 1 |
| TigerWrap | Inno Setup | 2.5 s | 3.0 s | 1.21 | 112.1 MB | 117.0 MB | 5,047,799 | 81 / 1 |
| TigerSqlCmd | TigerSetup | 3.8 s | 3.5 s | 0.92 | 108.7 MB | 111.6 MB | 4,714,731 | 124 / 1 |
| TigerSqlCmd | Inno Setup | 2.4 s | 3.1 s | 1.28 | 112.2 MB | 117.1 MB | 5,293,560 | 78 / 1 |
| TigerSqlCmd | NSIS | 3.4 s | 3.4 s | 1.00 | 123.0 MB | 123.8 MB | 3,267,577 | 111 / 2 |
| Tiger3dForge | Inno Setup | 6.1 s | 9.6 s | 1.56 | 112.2 MB | 117.1 MB | 13,637,511 | 198 / 1 |
| Tiger3dForge | NSIS | 12.5 s | 12.4 s | 0.99 | 97.8 MB | 140.4 MB | 12,501,641 | 400 / 2 |
| Tiger3dForge | TigerSetup | 13.3 s | 12.9 s | 0.97 | 157.7 MB | 160.2 MB | 14,342,326 | 424 / 1 |
| TigerKeyring | NSIS | 0.5 s | 0.5 s | 1.00 | 101.5 MB | 51.1 MB | 547,129 | 19 / 2 |
| TigerKeyring | TigerSetup | 1.2 s | 0.9 s | 0.78 | 61.5 MB | 63.4 MB | 1,761,244 | 38 / 1 |
| TigerKeyring | Inno Setup | 0.9 s | 0.9 s | 1.07 | 112.1 MB | 54.6 MB | 2,578,499 | 30 / 1 |
| GitForWindows | TigerSetup | 85.3 s | 85.0 s | 1.00 | 290.9 MB | 293.1 MB | 68,012,084 | 2723 / 1 |
| GitForWindows | Inno Setup | 47.6 s | 79.1 s | 1.66 | 124.7 MB | 128.8 MB | 91,699,969 | 1520 / 1 |
| GitForWindows | NSIS | 85.6 s | 85.4 s | 1.00 | 101.1 MB | 141.3 MB | 67,724,032 | 2733 / 2 |
| VSCode | Inno Setup | 138.0 s | 228.0 s | 1.65 | 115.9 MB | 120.5 MB | 236,066,109 | 4401 / 1 |
| VSCode | NSIS | 298.2 s | 297.6 s | 1.00 | 99.0 MB | 140.6 MB | 238,494,641 | 9507 / 2 |
| VSCode | TigerSetup | 294.5 s | 293.6 s | 1.00 | 281.9 MB | 285.0 MB | 230,297,757 | 9391 / 1 |
| Wireshark | NSIS | 91.3 s | 91.2 s | 1.00 | 98.3 MB | 140.5 MB | 99,073,975 | 2913 / 2 |
| Wireshark | TigerSetup | 95.8 s | 95.4 s | 1.00 | 279.4 MB | 282.5 MB | 104,302,442 | 3058 / 1 |
| Wireshark | Inno Setup | 37.6 s | 65.3 s | 1.74 | 113.3 MB | 118.5 MB | 97,818,292 | 1199 / 1 |
| WinSCP | TigerSetup | 5.8 s | 5.6 s | 0.95 | 124.6 MB | 127.7 MB | 8,501,640 | 188 / 1 |
| WinSCP | Inno Setup | 2.7 s | 4.4 s | 1.61 | 112.1 MB | 117.0 MB | 8,358,765 | 89 / 1 |
| WinSCP | NSIS | 5.4 s | 5.4 s | 1.00 | 97.7 MB | 128.4 MB | 6,970,243 | 174 / 2 |
| Inkscape | Inno Setup | 73.2 s | 117.2 s | 1.60 | 128.4 MB | 132.5 MB | 120,599,904 | 2336 / 1 |
| Inkscape | NSIS | 158.1 s | 157.8 s | 1.00 | 102.6 MB | 141.7 MB | 120,827,148 | 5043 / 2 |
| Inkscape | TigerSetup | 166.1 s | 165.5 s | 1.00 | 294.9 MB | 297.6 MB | 128,857,706 | 5297 / 1 |
| NotepadPlusPlus | NSIS | 6.1 s | 6.1 s | 1.00 | 97.7 MB | 132.5 MB | 6,353,224 | 195 / 2 |
| NotepadPlusPlus | TigerSetup | 8.5 s | 8.2 s | 0.97 | 128.1 MB | 130.6 MB | 7,664,185 | 272 / 1 |
| NotepadPlusPlus | Inno Setup | 3.9 s | 4.9 s | 1.25 | 112.1 MB | 117.2 MB | 8,219,539 | 126 / 1 |

## D. Runtime

One lab session per row on `TigerWinLab-Win11-Clean` (the VM restored to its
clean checkpoint before every install), the install job and the uninstall
job on the same VM, the session closed and the VM back to *Available*
before the next row. Install time is the installer process's lifetime plus
the exit of any process of its own name it left behind; uninstall time is
the registered uninstaller's lifetime plus the exit of what it hands off
to (NSIS's `Au_.exe`; TigerSetup's uninstaller helper) and the removal of
the install root and of the package-owned state outside it. No fixed
sleeps. The payload is verified file by file (size and SHA-256) *after*
the timed interval; *extras* are what the technology put under the root
beside the payload; *cleanup* is root gone, registration gone,
package-owned state gone.

| App | Technology | Install | Uninstall | Payload exact | Extras | Registered | Cleanup | Verdict |
|---|---|---:|---:|---|---|---|---|---|
| ShareX | TigerSetup | 19.93 s | 2.67 s | yes (1,230/1,230) | none | yes | root yes, ARP yes, state yes | PASS |
| ShareX | Inno Setup | 17.92 s | 1.82 s (1.18 s process + 0.64 s hand-off) | yes (1,230/1,230) | 2 (4,767,901 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| ShareX | NSIS | 21.59 s | 1.25 s (0.79 s process + 0.46 s hand-off) | yes (1,230/1,230) | 1 (39,267 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| WinMerge | Inno Setup | 12.59 s | 10.02 s (9.38 s process + 0.64 s hand-off) | yes (472/472) | 2 (4,585,219 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| WinMerge | NSIS | 4.71 s | 1.19 s (0.95 s process + 0.24 s hand-off) | yes (472/472) | 1 (39,267 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| WinMerge | TigerSetup | 3.92 s | 9.39 s | yes (472/472) | none | yes | root yes, ARP yes, state yes | PASS |
| qBittorrent | NSIS | 6.91 s | 0.95 s (0.72 s process + 0.23 s hand-off) | yes (38/38) | 1 (39,278 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| qBittorrent | TigerSetup | 4.05 s | 1.75 s | yes (38/38) | none | yes | root yes, ARP yes, state yes | PASS |
| qBittorrent | Inno Setup | 6.80 s | 1.59 s (0.96 s process + 0.63 s hand-off) | yes (38/38) | 2 (4,489,517 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| VLC | TigerSetup | 6.03 s | 2.04 s | yes (583/583) | none | yes | root yes, ARP yes, state yes | PASS |
| VLC | Inno Setup | 7.94 s | 2.00 s (1.35 s process + 0.65 s hand-off) | yes (583/583) | 2 (4,634,851 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| VLC | NSIS | 20.29 s | 9.28 s (8.82 s process + 0.46 s hand-off) | yes (583/583) | 1 (39,276 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerMarkView | Inno Setup | 3.47 s | 1.63 s (0.99 s process + 0.64 s hand-off) | yes (56/56) | 2 (4,494,421 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerMarkView | NSIS | 3.11 s | 1.08 s (0.85 s process + 0.23 s hand-off) | yes (56/56) | 1 (39,278 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerMarkView | TigerSetup | 3.42 s | 1.21 s | yes (56/56) | none | yes | root yes, ARP yes, state yes | PASS |
| TigerWrap | NSIS | 5.37 s | 1.22 s (0.99 s process + 0.23 s hand-off) | yes (62/62) | 1 (39,282 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerWrap | TigerSetup | 6.68 s | 0.98 s | yes (62/62) | none | yes | root yes, ARP yes, state yes | PASS |
| TigerWrap | Inno Setup | 2.87 s | 1.63 s (0.99 s process + 0.64 s hand-off) | yes (62/62) | 2 (4,500,213 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerSqlCmd | TigerSetup | 2.56 s | 1.06 s | yes (68/68) | none | yes | root yes, ARP yes, state yes | PASS |
| TigerSqlCmd | Inno Setup | 5.21 s | 1.49 s (0.85 s process + 0.64 s hand-off) | yes (68/68) | 2 (4,501,759 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerSqlCmd | NSIS | 1.96 s | 1.20 s (0.97 s process + 0.23 s hand-off) | yes (68/68) | 1 (39,280 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| Tiger3dForge | Inno Setup | 6.22 s | 1.52 s (0.89 s process + 0.63 s hand-off) | yes (89/89) | 2 (4,501,617 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| Tiger3dForge | NSIS | 4.17 s | 0.82 s (0.59 s process + 0.23 s hand-off) | yes (89/89) | 1 (39,275 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| Tiger3dForge | TigerSetup | 3.75 s | 1.18 s | yes (89/89) | none | yes | root yes, ARP yes, state yes | PASS |
| TigerKeyring | NSIS | 1.38 s | 1.61 s (1.39 s process + 0.22 s hand-off) | yes (2/2) | 1 (39,279 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| TigerKeyring | TigerSetup | 1.35 s | 1.92 s | yes (2/2) | none | yes | root yes, ARP yes, state yes | PASS |
| TigerKeyring | Inno Setup | 3.13 s | 1.56 s (0.94 s process + 0.62 s hand-off) | yes (2/2) | 2 (4,482,435 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| GitForWindows | TigerSetup | 30.84 s | 10.98 s | yes (9,584/9,584) | none | yes | root yes, ARP yes, state yes | PASS |
| GitForWindows | Inno Setup | 29.59 s | 4.17 s (3.54 s process + 0.63 s hand-off) | yes (9,584/9,584) | 2 (6,841,453 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| GitForWindows | NSIS | 42.24 s | 3.71 s (1.17 s process + 2.54 s hand-off) | yes (9,584/9,584) | 1 (39,280 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| VSCode | Inno Setup | 35.16 s | 2.27 s (1.64 s process + 0.63 s hand-off) | yes (2,484/2,484) | 2 (5,372,477 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| VSCode | NSIS | 49.10 s | 1.87 s (0.80 s process + 1.07 s hand-off) | yes (2,484/2,484) | 1 (39,266 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| VSCode | TigerSetup | 23.02 s | 4.06 s | yes (2,484/2,484) | none | yes | root yes, ARP yes, state yes | PASS |
| Wireshark | NSIS | 23.68 s | 1.12 s (0.69 s process + 0.43 s hand-off) | yes (1,354/1,354) | 1 (39,276 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| Wireshark | TigerSetup | 9.67 s | 2.53 s | yes (1,354/1,354) | none | yes | root yes, ARP yes, state yes | PASS |
| Wireshark | Inno Setup | 12.61 s | 1.73 s (1.10 s process + 0.63 s hand-off) | yes (1,354/1,354) | 2 (4,761,553 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| WinSCP | TigerSetup | 2.20 s | 1.36 s | yes (4/4) | none | yes | root yes, ARP yes, state yes | PASS |
| WinSCP | Inno Setup | 10.59 s | 1.37 s (0.74 s process + 0.63 s hand-off) | yes (4/4) | 2 (4,482,635 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| WinSCP | NSIS | 2.63 s | 0.92 s (0.70 s process + 0.22 s hand-off) | yes (4/4) | 1 (39,268 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| Inkscape | Inno Setup | 39.23 s | 4.29 s (3.65 s process + 0.64 s hand-off) | yes (12,152/12,152) | 2 (7,712,257 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |
| Inkscape | NSIS | 44.35 s | 3.51 s (0.59 s process + 2.92 s hand-off) | yes (12,152/12,152) | 1 (39,269 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| Inkscape | TigerSetup | 48.01 s | 13.92 s | yes (12,152/12,152) | none | yes | root yes, ARP yes, state yes | PASS |
| NotepadPlusPlus | NSIS | 2.70 s | 9.60 s (9.36 s process + 0.24 s hand-off) | yes (219/219) | 1 (39,284 B): Uninstall.exe | yes | root yes, ARP yes, state yes | PASS |
| NotepadPlusPlus | TigerSetup | 10.95 s | 1.21 s | yes (219/219) | none | yes | root yes, ARP yes, state yes | PASS |
| NotepadPlusPlus | Inno Setup | 3.67 s | 1.90 s (1.27 s process + 0.63 s hand-off) | yes (219/219) | 2 (4,527,595 B): unins000.dat, unins000.exe | yes | root yes, ARP yes, state yes | PASS |

TigerSetup's engine inside the measured lifetime (explanatory only; the
comparison above is the whole user-visible time): the span from the
engine's first to its last log event, what ran before it (the loader:
locating the footer, decompressing and verifying the engine, starting
it — and Windows starting a large executable for the first time on a
clean VM) and after it.

| App | Install: before | engine | after | Uninstall: before | engine | after | Engine log |
|---|---:|---:|---:|---:|---:|---:|---|
| ShareX | 13.97 s | 5.95 s | 0.01 s | 1.28 s | 1.30 s | 0.04 s | 1399 events, run_started → run_finished |
| WinMerge | 2.12 s | 1.72 s | 0.07 s | 8.83 s | 0.52 s | 0.02 s | 560 events, run_started → run_finished |
| qBittorrent | 3.22 s | 0.82 s | 0.01 s | 1.62 s | 0.09 s | 0.01 s | 61 events, run_started → run_finished |
| VLC | 3.15 s | 2.86 s | 0.00 s | 1.26 s | 0.72 s | 0.03 s | 865 events, run_started → run_finished |
| TigerMarkView | 2.12 s | 1.30 s | 0.01 s | 1.08 s | 0.10 s | 0.02 s | 87 events, run_started → run_finished |
| TigerWrap | 5.62 s | 1.05 s | 0.01 s | 0.84 s | 0.10 s | 0.02 s | 113 events, run_started → run_finished |
| TigerSqlCmd | 1.91 s | 0.62 s | 0.01 s | 0.91 s | 0.12 s | 0.02 s | 118 events, run_started → run_finished |
| Tiger3dForge | 2.14 s | 1.60 s | 0.00 s | 0.99 s | 0.13 s | 0.02 s | 123 events, run_started → run_finished |
| TigerKeyring | 1.25 s | 0.07 s | 0.02 s | 1.85 s | 0.03 s | 0.02 s | 24 events, run_started → run_finished |
| GitForWindows | 3.57 s | 27.25 s | 0.01 s | 0.87 s | 10.07 s | 0.02 s | 10468 events, run_started → run_finished |
| VSCode | 7.48 s | 15.53 s | 0.01 s | 1.04 s | 2.97 s | 0.03 s | 3156 events, run_started → run_finished |
| Wireshark | 4.29 s | 5.36 s | 0.00 s | 1.03 s | 1.45 s | 0.02 s | 1406 events, run_started → run_finished |
| WinSCP | 2.03 s | 0.17 s | 0.01 s | 1.28 s | 0.03 s | 0.02 s | 26 events, run_started → run_finished |
| Inkscape | 5.25 s | 42.75 s | 0.01 s | 1.12 s | 12.76 s | 0.03 s | 12973 events, run_started → run_finished |
| NotepadPlusPlus | 9.96 s | 0.97 s | 0.01 s | 0.88 s | 0.29 s | 0.02 s | 252 events, run_started → run_finished |

## E. Package-size distribution

TigerSetup's installer against the smaller of Inno Setup's and NSIS's, per
application, over the fifteen (Minimal excluded):

| Statistic | TS vs smaller of IS/NSIS | TS vs Inno Setup | TS vs NSIS |
|---|---:|---:|---:|
| Median | +10.7% | +1.1% | +10.7% |
| Q1 | +6.7% | −10.2% | +6.0% |
| Q3 | +21.4% | +5.9% | +21.4% |
| Minimum | −7.2% | −31.7% | −7.2% |
| Maximum | +221.9% | +8.5% | +221.9% |
| TigerSetup smaller | 2 of 15 | 7 of 15 | 2 of 15 |
| Within ±5% | 2 | 4 | 2 |
| Within ±10% | 6 | 11 | 6 |
| More than 10% larger | 9 | 0 | 9 |

The smaller competitor is Inno Setup for 4 of the fifteen and NSIS for 11.
The delta follows the payload's size, because TigerSetup's fixed overhead
(section I) is a constant term: over the 7 payloads of 100 MB or more it is
+6.6% at the median (−7.2% to +10.7%); over the 8 below 100 MB it is
+21.4% at the median (+10.4% to +221.9%).
Sorted by TigerSetup's delta against the smaller competitor:

| App | Payload | TS vs smaller | Smaller is | Files | Avg file |
|---|---:|---:|---|---:|---:|
| VLC | 182.5 MB | −7.2% | NSIS | 583 | 321 KB |
| VSCode | 997.7 MB | −2.4% | IS | 2,484 | 411 KB |
| GitForWindows | 384.9 MB | +0.4% | NSIS | 9,584 | 41 KB |
| Wireshark | 304.3 MB | +6.6% | IS | 1,354 | 230 KB |
| Inkscape | 629.6 MB | +6.8% | IS | 12,152 | 53 KB |
| ShareX | 516.5 MB | +8.5% | IS | 1,230 | 430 KB |
| WinMerge | 77.9 MB | +10.4% | NSIS | 472 | 169 KB |
| qBittorrent | 220.8 MB | +10.7% | NSIS | 38 | 5,950 KB |
| Tiger3dForge | 39.6 MB | +14.7% | NSIS | 89 | 456 KB |
| NotepadPlusPlus | 25.8 MB | +20.6% | NSIS | 219 | 121 KB |
| TigerMarkView | 31.6 MB | +20.9% | NSIS | 56 | 577 KB |
| WinSCP | 23.3 MB | +22.0% | NSIS | 4 | 5,954 KB |
| TigerSqlCmd | 15.3 MB | +44.3% | NSIS | 68 | 230 KB |
| TigerWrap | 15.2 MB | +47.2% | NSIS | 62 | 251 KB |
| TigerKeyring | 1.9 MB | +221.9% | NSIS | 2 | 484 KB |

## F. Build-resource summary

Across the fifteen applications (Minimal excluded), TigerSetup's build
against each competitor's, per application and as a distribution. A ratio
above 1 means TigerSetup took more. CPU/wall says how many cores a
compiler kept busy: Inno Setup's LZMA2 compressor is multi-threaded,
TigerSetup's zstd and NSIS's LZMA run on one.

| App | Wall IS | Wall NSIS | Wall TS | TS/IS | TS/NSIS | CPU IS | CPU NSIS | CPU TS | TS/IS | TS/NSIS | CPU/wall IS | NSIS | TS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ShareX | 59.1 s | 147.3 s | 143.4 s | 2.43× | 0.97× | 103.5 s | 146.9 s | 142.7 s | 1.38× | 0.97× | 1.75 | 1.00 | 1.00 |
| WinMerge | 8.4 s | 16.3 s | 18.1 s | 2.15× | 1.11× | 13.0 s | 16.2 s | 17.8 s | 1.37× | 1.10× | 1.54 | 1.00 | 0.98 |
| qBittorrent | 39.0 s | 60.0 s | 69.3 s | 1.77× | 1.15× | 45.2 s | 59.9 s | 68.8 s | 1.52× | 1.15× | 1.16 | 1.00 | 0.99 |
| VLC | 22.2 s | 48.8 s | 49.2 s | 2.22× | 1.01× | 36.4 s | 48.7 s | 48.8 s | 1.34× | 1.00× | 1.64 | 1.00 | 0.99 |
| TigerMarkView | 4.6 s | 9.0 s | 9.2 s | 2.01× | 1.02× | 6.9 s | 9.0 s | 8.9 s | 1.29× | 0.99× | 1.51 | 1.00 | 0.97 |
| TigerWrap | 2.5 s | 3.4 s | 3.7 s | 1.48× | 1.08× | 3.0 s | 3.4 s | 3.4 s | 1.13× | 1.01× | 1.21 | 0.99 | 0.92 |
| TigerSqlCmd | 2.4 s | 3.4 s | 3.8 s | 1.60× | 1.12× | 3.1 s | 3.4 s | 3.5 s | 1.15× | 1.04× | 1.28 | 1.00 | 0.92 |
| Tiger3dForge | 6.1 s | 12.5 s | 13.3 s | 2.16× | 1.06× | 9.6 s | 12.4 s | 12.9 s | 1.35× | 1.04× | 1.56 | 0.99 | 0.97 |
| TigerKeyring | 0.9 s | 0.5 s | 1.2 s | 1.31× | 2.12× | 0.9 s | 0.5 s | 0.9 s | 0.97× | 1.66× | 1.07 | 1.00 | 0.78 |
| GitForWindows | 47.6 s | 85.6 s | 85.3 s | 1.79× | 1.00× | 79.1 s | 85.4 s | 85.0 s | 1.07× | 0.99× | 1.66 | 1.00 | 1.00 |
| VSCode | 138.0 s | 298.2 s | 294.5 s | 2.13× | 0.99× | 228.0 s | 297.6 s | 293.6 s | 1.29× | 0.99× | 1.65 | 1.00 | 1.00 |
| Wireshark | 37.6 s | 91.3 s | 95.8 s | 2.55× | 1.05× | 65.3 s | 91.2 s | 95.4 s | 1.46× | 1.05× | 1.74 | 1.00 | 1.00 |
| WinSCP | 2.7 s | 5.4 s | 5.8 s | 2.15× | 1.08× | 4.4 s | 5.4 s | 5.6 s | 1.27× | 1.03× | 1.61 | 1.00 | 0.95 |
| Inkscape | 73.2 s | 158.1 s | 166.1 s | 2.27× | 1.05× | 117.2 s | 157.8 s | 165.5 s | 1.41× | 1.05× | 1.60 | 1.00 | 1.00 |
| NotepadPlusPlus | 3.9 s | 6.1 s | 8.5 s | 2.17× | 1.40× | 4.9 s | 6.1 s | 8.2 s | 1.67× | 1.35× | 1.25 | 1.00 | 0.97 |

| Distribution | Inno Setup | NSIS | TigerSetup | TS/IS | TS/NSIS |
|---|---|---|---|---|---|
| Build wall, median (Q1–Q3; min–max) | 8.4 s (Q1 3.3 s, Q3 43.3 s; min 0.9 s, max 138.0 s) | 16.3 s (Q1 5.7 s, Q3 88.5 s; min 0.5 s, max 298.2 s) | 18.1 s (Q1 7.2 s, Q3 90.6 s; min 1.2 s, max 294.5 s) | 2.15× (Q1 1.78×, Q3 2.19×; min 1.31×, max 2.55×) | 1.06× (Q1 1.01×, Q3 1.11×; min 0.97×, max 2.12×) |
| Build wall, aggregate | 448.2 s | 946.1 s | 967.3 s | 2.16× | 1.02× |
| CPU, median (Q1–Q3; min–max) | 13.0 s (Q1 4.6 s, Q3 72.2 s; min 0.9 s, max 228.0 s) | 16.2 s (Q1 5.7 s, Q3 88.3 s; min 0.5 s, max 297.6 s) | 17.8 s (Q1 6.9 s, Q3 90.2 s; min 0.9 s, max 293.6 s) | 1.34× (Q1 1.21×, Q3 1.40×; min 0.97×, max 1.67×) | 1.04× (Q1 1.00×, Q3 1.07×; min 0.97×, max 1.66×) |
| CPU, aggregate | 720.5 s | 944.0 s | 960.9 s | 1.33× | 1.02× |
| CPU/wall, median (min–max) | 1.56 (Q1 1.27, Q3 1.65; min 1.07, max 1.75) | 1.00 (Q1 1.00, Q3 1.00; min 0.99, max 1.00) | 0.98 (Q1 0.96, Q3 1.00; min 0.78, max 1.00) | | |
| Peak commit, median (Q1–Q3; min–max) | 112.2 MB (Q1 112.1 MB, Q3 113.4 MB; min 112.1 MB, max 128.4 MB) | 98.3 MB (Q1 97.8 MB, Q3 101.3 MB; min 97.7 MB, max 123.0 MB) | 228.3 MB (Q1 126.3 MB, Q3 279.2 MB; min 61.5 MB, max 294.9 MB) | 2.04× (medians) | 2.32× (medians) |
| Peak commit, maximum | 128.4 MB | 123.0 MB | 294.9 MB | 2.30× | 2.40× |
| Peak tree working set, median (Q1–Q3; min–max) | 117.2 MB (Q1 117.1 MB, Q3 118.4 MB; min 54.6 MB, max 132.5 MB) | 140.4 MB (Q1 130.5 MB, Q3 140.5 MB; min 51.1 MB, max 141.7 MB) | 231.0 MB (Q1 129.2 MB, Q3 282.4 MB; min 63.4 MB, max 297.6 MB) | 1.97× (medians) | 1.65× (medians) |
| Peak tree working set, maximum | 132.5 MB | 141.7 MB | 297.6 MB | 2.25× | 2.10× |

Peak memory per build:

| App | Commit IS | Commit NSIS | Commit TS | TS/IS | TS/NSIS | Tree WS IS | Tree WS NSIS | Tree WS TS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ShareX | 113.4 MB | 98.3 MB | 279.1 MB | 2.46× | 2.84× | 118.3 MB | 140.5 MB | 282.2 MB |
| WinMerge | 113.4 MB | 98.0 MB | 228.3 MB | 2.01× | 2.33× | 117.5 MB | 140.5 MB | 231.0 MB |
| qBittorrent | 112.1 MB | 97.7 MB | 277.7 MB | 2.48× | 2.84× | 117.0 MB | 136.5 MB | 280.7 MB |
| VLC | 113.4 MB | 98.2 MB | 278.8 MB | 2.46× | 2.84× | 117.6 MB | 140.5 MB | 281.4 MB |
| TigerMarkView | 112.2 MB | 97.8 MB | 133.0 MB | 1.19× | 1.36× | 117.1 MB | 136.5 MB | 136.1 MB |
| TigerWrap | 112.1 MB | 115.4 MB | 108.6 MB | 0.97× | 0.94× | 117.0 MB | 123.7 MB | 111.2 MB |
| TigerSqlCmd | 112.2 MB | 123.0 MB | 108.7 MB | 0.97× | 0.88× | 117.1 MB | 123.8 MB | 111.6 MB |
| Tiger3dForge | 112.2 MB | 97.8 MB | 157.7 MB | 1.41× | 1.61× | 117.1 MB | 140.4 MB | 160.2 MB |
| TigerKeyring | 112.1 MB | 101.5 MB | 61.5 MB | 0.55× | 0.61× | 54.6 MB | 51.1 MB | 63.4 MB |
| GitForWindows | 124.7 MB | 101.1 MB | 290.9 MB | 2.33× | 2.88× | 128.8 MB | 141.3 MB | 293.1 MB |
| VSCode | 115.9 MB | 99.0 MB | 281.9 MB | 2.43× | 2.85× | 120.5 MB | 140.6 MB | 285.0 MB |
| Wireshark | 113.3 MB | 98.3 MB | 279.4 MB | 2.46× | 2.84× | 118.5 MB | 140.5 MB | 282.5 MB |
| WinSCP | 112.1 MB | 97.7 MB | 124.6 MB | 1.11× | 1.28× | 117.0 MB | 128.4 MB | 127.7 MB |
| Inkscape | 128.4 MB | 102.6 MB | 294.9 MB | 2.30× | 2.87× | 132.5 MB | 141.7 MB | 297.6 MB |
| NotepadPlusPlus | 112.1 MB | 97.7 MB | 128.1 MB | 1.14× | 1.31× | 117.2 MB | 132.5 MB | 130.6 MB |

## G. Runtime summary

Install and uninstall wall time per application (the whole user-visible
time, as section D defines it) and TigerSetup's ratio to each competitor;
a ratio below 1 means TigerSetup was faster.

| App | Install IS | NSIS | TS | TS/IS | TS/NSIS | Uninstall IS | NSIS | TS | TS/IS | TS/NSIS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ShareX | 17.92 s | 21.59 s | 19.93 s | 1.11× | 0.92× | 1.82 s | 1.25 s | 2.67 s | 1.47× | 2.14× |
| WinMerge | 12.59 s | 4.71 s | 3.92 s | 0.31× | 0.83× | 10.02 s | 1.19 s | 9.39 s | 0.94× | 7.89× |
| qBittorrent | 6.80 s | 6.91 s | 4.05 s | 0.60× | 0.59× | 1.59 s | 0.95 s | 1.75 s | 1.10× | 1.84× |
| VLC | 7.94 s | 20.29 s | 6.03 s | 0.76× | 0.30× | 2.00 s | 9.28 s | 2.04 s | 1.02× | 0.22× |
| TigerMarkView | 3.47 s | 3.11 s | 3.42 s | 0.99× | 1.10× | 1.63 s | 1.08 s | 1.21 s | 0.74× | 1.12× |
| TigerWrap | 2.87 s | 5.37 s | 6.68 s | 2.33× | 1.24× | 1.63 s | 1.22 s | 0.98 s | 0.60× | 0.80× |
| TigerSqlCmd | 5.21 s | 1.96 s | 2.56 s | 0.49× | 1.31× | 1.49 s | 1.20 s | 1.06 s | 0.71× | 0.88× |
| Tiger3dForge | 6.22 s | 4.17 s | 3.75 s | 0.60× | 0.90× | 1.52 s | 0.82 s | 1.18 s | 0.78× | 1.44× |
| TigerKeyring | 3.13 s | 1.38 s | 1.35 s | 0.43× | 0.98× | 1.56 s | 1.61 s | 1.92 s | 1.23× | 1.19× |
| GitForWindows | 29.59 s | 42.24 s | 30.84 s | 1.04× | 0.73× | 4.17 s | 3.71 s | 10.98 s | 2.63× | 2.96× |
| VSCode | 35.16 s | 49.10 s | 23.02 s | 0.65× | 0.47× | 2.27 s | 1.87 s | 4.06 s | 1.79× | 2.17× |
| Wireshark | 12.61 s | 23.68 s | 9.67 s | 0.77× | 0.41× | 1.73 s | 1.12 s | 2.53 s | 1.46× | 2.26× |
| WinSCP | 10.59 s | 2.63 s | 2.20 s | 0.21× | 0.84× | 1.37 s | 0.92 s | 1.36 s | 0.99× | 1.48× |
| Inkscape | 39.23 s | 44.35 s | 48.01 s | 1.22× | 1.08× | 4.29 s | 3.51 s | 13.92 s | 3.24× | 3.97× |
| NotepadPlusPlus | 3.67 s | 2.70 s | 10.95 s | 2.98× | 4.06× | 1.90 s | 9.60 s | 1.21 s | 0.64× | 0.13× |

| Distribution | Inno Setup | NSIS | TigerSetup | TS/IS | TS/NSIS |
|---|---|---|---|---|---|
| Install, median (Q1–Q3; min–max) | 7.94 s (Q1 4.44 s, Q3 15.27 s; min 2.87 s, max 39.23 s) | 5.37 s (Q1 2.91 s, Q3 22.63 s; min 1.38 s, max 49.10 s) | 6.03 s (Q1 3.58 s, Q3 15.44 s; min 1.35 s, max 48.01 s) | 0.76× (Q1 0.54×, Q3 1.08×; min 0.21×, max 2.98×) | 0.90× (Q1 0.66×, Q3 1.09×; min 0.30×, max 4.06×) |
| Install, aggregate | 197.00 s | 234.19 s | 176.38 s | 0.90× | 0.75× |
| Install, TigerSetup faster / slower | | | | 10 / 5 | 10 / 5 |
| Uninstall, median (Q1–Q3; min–max) | 1.73 s (Q1 1.58 s, Q3 2.13 s; min 1.37 s, max 10.02 s) | 1.22 s (Q1 1.10 s, Q3 2.69 s; min 0.82 s, max 9.60 s) | 1.92 s (Q1 1.21 s, Q3 3.36 s; min 0.98 s, max 13.92 s) | 1.02× (Q1 0.76×, Q3 1.46×; min 0.60×, max 3.24×) | 1.48× (Q1 1.00×, Q3 2.22×; min 0.13×, max 7.89×) |
| Uninstall, aggregate | 38.99 s | 39.33 s | 56.26 s | 1.44× | 1.43× |
| Uninstall, TigerSetup faster / slower | | | | 7 / 8 | 4 / 11 |

Payload fidelity and cleanup over the 45 rows: payload exact in 45, registration present in 45, root removed in 45, registration removed in 45, package-owned state removed in 45; 45 passed.

## H. Workload shape

Runtime against the payload's size and file count, per technology. The
table is sorted by file count; the correlations are Pearson's r over the
applications with a row (a description of this corpus, not a causal
claim — bytes and files are themselves correlated across it).

| App | Files | Bytes | Install IS | NSIS | TS | Uninstall IS | NSIS | TS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| TigerKeyring | 2 | 1.9 MB | 3.13 s | 1.38 s | 1.35 s | 1.56 s | 1.61 s | 1.92 s |
| WinSCP | 4 | 23.3 MB | 10.59 s | 2.63 s | 2.20 s | 1.37 s | 0.92 s | 1.36 s |
| qBittorrent | 38 | 220.8 MB | 6.80 s | 6.91 s | 4.05 s | 1.59 s | 0.95 s | 1.75 s |
| TigerMarkView | 56 | 31.6 MB | 3.47 s | 3.11 s | 3.42 s | 1.63 s | 1.08 s | 1.21 s |
| TigerWrap | 62 | 15.2 MB | 2.87 s | 5.37 s | 6.68 s | 1.63 s | 1.22 s | 0.98 s |
| TigerSqlCmd | 68 | 15.3 MB | 5.21 s | 1.96 s | 2.56 s | 1.49 s | 1.20 s | 1.06 s |
| Tiger3dForge | 89 | 39.6 MB | 6.22 s | 4.17 s | 3.75 s | 1.52 s | 0.82 s | 1.18 s |
| NotepadPlusPlus | 219 | 25.8 MB | 3.67 s | 2.70 s | 10.95 s | 1.90 s | 9.60 s | 1.21 s |
| WinMerge | 472 | 77.9 MB | 12.59 s | 4.71 s | 3.92 s | 10.02 s | 1.19 s | 9.39 s |
| VLC | 583 | 182.5 MB | 7.94 s | 20.29 s | 6.03 s | 2.00 s | 9.28 s | 2.04 s |
| ShareX | 1,230 | 516.5 MB | 17.92 s | 21.59 s | 19.93 s | 1.82 s | 1.25 s | 2.67 s |
| Wireshark | 1,354 | 304.3 MB | 12.61 s | 23.68 s | 9.67 s | 1.73 s | 1.12 s | 2.53 s |
| VSCode | 2,484 | 997.7 MB | 35.16 s | 49.10 s | 23.02 s | 2.27 s | 1.87 s | 4.06 s |
| GitForWindows | 9,584 | 384.9 MB | 29.59 s | 42.24 s | 30.84 s | 4.17 s | 3.71 s | 10.98 s |
| Inkscape | 12,152 | 629.6 MB | 39.23 s | 44.35 s | 48.01 s | 4.29 s | 3.51 s | 13.92 s |

The two normalizations are meaningful only where the payload, not the
launch, dominates the time: the per-100 MB median is taken over the
payloads of 100 MB or more, the per-1,000-files median over those of
1,000 files or more.

| Technology | Install vs bytes (r) | Install vs files (r) | Uninstall vs bytes (r) | Uninstall vs files (r) | Install per 100 MB (median, ≥100 MB) | Install per 1,000 files (median, ≥1,000 files) |
|---|---:|---:|---:|---:|---:|---:|
| Inno Setup | 0.88 | 0.83 | 0.08 | 0.30 | 4.14 s | 9.31 s |
| NSIS | 0.91 | 0.78 | -0.02 | 0.13 | 7.04 s | 17.49 s |
| TigerSetup | 0.75 | 0.93 | 0.46 | 0.88 | 3.30 s | 7.14 s |

Per-application throughput of the install (seconds per 100 MB of payload
and per 1,000 files), the two normalizations that separate a byte-bound
row from a file-bound one:

| App | Files | Bytes | s/100 MB: IS | NSIS | TS | s/1,000 files: IS | NSIS | TS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| VSCode | 2,484 | 997.7 MB | 3.52 s | 4.92 s | 2.31 s | 14.15 s | 19.77 s | 9.27 s |
| Inkscape | 12,152 | 629.6 MB | 6.23 s | 7.04 s | 7.63 s | 3.23 s | 3.65 s | 3.95 s |
| ShareX | 1,230 | 516.5 MB | 3.47 s | 4.18 s | 3.86 s | 14.57 s | 17.55 s | 16.20 s |
| GitForWindows | 9,584 | 384.9 MB | 7.69 s | 10.97 s | 8.01 s | 3.09 s | 4.41 s | 3.22 s |
| Wireshark | 1,354 | 304.3 MB | 4.14 s | 7.78 s | 3.18 s | 9.31 s | 17.49 s | 7.14 s |
| qBittorrent | 38 | 220.8 MB | 3.08 s | 3.13 s | 1.83 s | 178.95 s | 181.84 s | 106.58 s |
| VLC | 583 | 182.5 MB | 4.35 s | 11.12 s | 3.30 s | 13.62 s | 34.80 s | 10.34 s |
| WinMerge | 472 | 77.9 MB | 16.17 s | 6.05 s | 5.04 s | 26.67 s | 9.98 s | 8.31 s |
| Tiger3dForge | 89 | 39.6 MB | 15.69 s | 10.52 s | 9.46 s | 69.89 s | 46.85 s | 42.13 s |
| TigerMarkView | 56 | 31.6 MB | 11.00 s | 9.85 s | 10.84 s | 61.96 s | 55.54 s | 61.07 s |
| NotepadPlusPlus | 219 | 25.8 MB | 14.20 s | 10.45 s | 42.37 s | 16.76 s | 12.33 s | 50.00 s |
| WinSCP | 4 | 23.3 MB | 45.53 s | 11.31 s | 9.46 s | 2,647.50 s | 657.50 s | 550.00 s |
| TigerSqlCmd | 68 | 15.3 MB | 34.09 s | 12.83 s | 16.75 s | 76.62 s | 28.82 s | 37.65 s |
| TigerWrap | 62 | 15.2 MB | 18.90 s | 35.36 s | 43.99 s | 46.29 s | 86.61 s | 107.74 s |
| TigerKeyring | 2 | 1.9 MB | 165.94 s | 73.16 s | 71.57 s | 1,565.00 s | 690.00 s | 675.00 s |

## I. Fixed overhead

The minimal no-payload installer of each technology (`packages/minimal`:
identity and metadata only, one short text file for TigerSetup, which
requires a file entry): what a generated installer starts at before any
application bytes, and what building nothing costs.

| Technology | Minimal installer | Build wall | Build CPU | Peak commit | What it carries |
|---|---:|---:|---:|---:|---|
| Inno Setup | 2,097,202 | 0.69 s | 0.69 s | 43.0 MB | SetupLdr.e64 + the LZMA2-compressed native x64 engine (Setup.e64 6,915,072 B raw) |
| NSIS | 38,332 | 0.03 s | 0.05 s | 97.4 MB | the lzma_solid-x86-unicode exehead (38,912 B) + the compiled script |
| TigerSetup | 1,222,073 | 0.78 s | 0.48 s | 61.5 MB | the C loader (74,752 B) + the zstd-compressed engine (2,490,880 B raw) + the compressed metadata |

TigerSetup's fixed overhead is −41.7% against Inno Setup's and 31.88× NSIS's.
Against the smallest application payload (TigerKeyring, 1.9 MB) it is the
dominant term; against the median payload (77.9 MB) it is a few percent.

## J. Outliers

**Package size.** TigerSetup's three best and three worst results against the smaller competitor:

- **VLC**: −7.2% vs NSIS — 583 files, 182.5 MB, avg 321 KB, largest `plugins/gui/libqt_plugin.dll` 16.7 MB; native 75.8%, text 0.3%, precompressed 0.6%
- **VSCode**: −2.4% vs IS — 2,484 files, 997.7 MB, avg 411 KB, largest `Code.exe` 224.2 MB; native 55.9%, text 25.1%, precompressed 0.5%
- **GitForWindows**: +0.4% vs NSIS — 9,584 files, 384.9 MB, avg 41 KB, largest `mingw64/bin/git-lfs.exe` 12.1 MB; native 66.6%, text 20.4%, precompressed 0.1%
- **TigerSqlCmd**: +44.3% vs NSIS — 68 files, 15.3 MB, avg 230 KB, largest `cli/runtimes/win/lib/net9.0/Microsoft.Data.SqlClient.dll` 1.8 MB; native 11.9%, text 2.1%, precompressed 0.0%
- **TigerWrap**: +47.2% vs NSIS — 62 files, 15.2 MB, avg 251 KB, largest `cli/runtimes/win/lib/net9.0/Microsoft.Data.SqlClient.dll` 1.8 MB; native 11.9%, text 7.8%, precompressed 0.0%
- **TigerKeyring**: +221.9% vs NSIS — 2 files, 1.9 MB, avg 484 KB, largest `tiger-keyring.exe` 1.3 MB; native 99.8%, text 0.2%, precompressed 0.0%

**Install time**, TigerSetup against the faster competitor (ratio below 1: TigerSetup faster):

- **qBittorrent**: 0.60× (4.05 s vs IS 6.80 s, NSIS 6.91 s) — 38 files, 220.8 MB
- **VSCode**: 0.65× (23.02 s vs IS 35.16 s, NSIS 49.10 s) — 2,484 files, 997.7 MB
- **VLC**: 0.76× (6.03 s vs IS 7.94 s, NSIS 20.29 s) — 583 files, 182.5 MB
- **TigerSqlCmd**: 1.31× (2.56 s vs IS 5.21 s, NSIS 1.96 s) — 68 files, 15.3 MB
- **TigerWrap**: 2.33× (6.68 s vs IS 2.87 s, NSIS 5.37 s) — 62 files, 15.2 MB
- **NotepadPlusPlus**: 4.06× (10.95 s vs IS 3.67 s, NSIS 2.70 s) — 219 files, 25.8 MB

**Uninstall time**, TigerSetup against the faster competitor:

- **NotepadPlusPlus**: 0.64× (1.21 s vs IS 1.90 s, NSIS 9.60 s)
- **TigerWrap**: 0.80× (0.98 s vs IS 1.63 s, NSIS 1.22 s)
- **TigerSqlCmd**: 0.88× (1.06 s vs IS 1.49 s, NSIS 1.20 s)
- **GitForWindows**: 2.96× (10.98 s vs IS 4.17 s, NSIS 3.71 s)
- **Inkscape**: 3.97× (13.92 s vs IS 4.29 s, NSIS 3.51 s)
- **WinMerge**: 7.89× (9.39 s vs IS 10.02 s, NSIS 1.19 s)

**Build.** The largest peak commit and the longest build of each technology, and TigerSetup's widest wall-clock ratios:

- Inno Setup: peak commit 128.4 MB on Inkscape; longest build 138.0 s on VSCode (997.7 MB, 2,484 files)
- NSIS: peak commit 123.0 MB on TigerSqlCmd; longest build 298.2 s on VSCode (997.7 MB, 2,484 files)
- TigerSetup: peak commit 294.9 MB on Inkscape; longest build 294.5 s on VSCode (997.7 MB, 2,484 files)
- TigerSetup's build wall against Inno Setup ranges from 1.31× (TigerKeyring) to 2.55× (Wireshark)
- TigerSetup's build wall against NSIS ranges from 0.97× (ShareX) to 2.12× (TigerKeyring)

## Determinism

Each package below was built a second time from the same definition and payload with the same compiler, into a separate artifacts root, and the two installers' SHA-256 and bytes compared: identical means the technology's output is deterministic for that input. The rebuild's timings are not part of the campaign.

| App | Technology | First build | Second build | Identical |
|---|---|---|---|---|
| Minimal | NSIS | 36a49ca27dbc10ad… (38,332 B) | 36a49ca27dbc10ad… (38,332 B) | yes |
| Minimal | TigerSetup | 099b8098c5d87204… (1,222,073 B) | 099b8098c5d87204… (1,222,073 B) | yes |
| Minimal | Inno Setup | 363821daa2b717b6… (2,097,202 B) | 363821daa2b717b6… (2,097,202 B) | yes |
| GitForWindows | TigerSetup | 1a33ab9422ec4ec1… (68,012,084 B) | 1a33ab9422ec4ec1… (68,012,084 B) | yes |
| GitForWindows | Inno Setup | 62699ded9713c3d8… (91,699,969 B) | 62699ded9713c3d8… (91,699,969 B) | yes |
| GitForWindows | NSIS | 1dd652759ca21879… (67,724,032 B) | 1dd652759ca21879… (67,724,032 B) | yes |
| NotepadPlusPlus | NSIS | 52a356e41ccd3806… (6,353,224 B) | 52a356e41ccd3806… (6,353,224 B) | yes |
| NotepadPlusPlus | TigerSetup | 4ae8c95e83b73c98… (7,664,185 B) | 4ae8c95e83b73c98… (7,664,185 B) | yes |
| NotepadPlusPlus | Inno Setup | 43e7102a5f5e7da5… (8,219,539 B) | 43e7102a5f5e7da5… (8,219,539 B) | yes |

## Provenance

| | |
|---|---|
| Host | Microsoft Windows 11 Pro 25H2 (build 26200); AMD Ryzen 7 5700X 8-Core Processor, 8 cores / 16 logical; 32,693 MB RAM; repository on C: KINGSTON SNV3S2000G (SSD, NVMe); PowerShell 7.6.6 |
| Repository | commit `4d384eb` (working tree dirty: the benchmark machinery of this campaign was uncommitted while it ran) |
| TigerSetup | 0.10.0 — builder `tiger-setup.exe` 3,006,464 B `81f504b858583eb9b06e1e759794c24a04da54b2d9a072966ee1ccaaf8b9d6ba`; engine `tigersetup-setup.exe` 2,490,880 B `3e6f332c6af5cb3b30b141181cd8d7305b78105cdce8d7bdc6471a433f64dff9`; loader `tigersetup-loader.exe` 74,752 B `5e231e9b99029351f63df0945cdf55807f5f53bd5d1fb6900bda2cdbe5523336`; every TigerSetup installer verified (`tiger-setup inspect`) to carry this engine; release builder defaults (solid zstd-19-w27, single-threaded), never `--fast` |
| Inno Setup | 7.1.0 — `C:\Program Files\Inno Setup 7\ISCC.exe` `d06ebd38f38e3cee60a3c50cc45bd449d77e0bc6a5cabc607ea9886808e4de1a`; Setup.e64 `ad12a06d09afefa9d1283c6616ef4d56289dc14a7137a12b24653a12637216bb`; SetupLdr.e64 `ee1fdf9c8c352a5e699645f8a11fa25e69ef31ab76977c201ef00f470a4d387d`; Compression=lzma2/max, SolidCompression=yes |
| NSIS | 3.12 — `C:\Program Files (x86)\NSIS\makensis.exe` `b043e554afefbfc56315669d0b4779793aeae67f0f2a7a790e2ea91f05298eff`; Bin\makensis.exe `25d1aa7081db1a9de9690b59983f9652b7409a189c3444328cc1841eff693e8d`; exehead `3507903b63ab7517fb9079e07b706c73ceeb668e99bbca269de3fef0c7376023`; SetCompressor /SOLID lzma |
| Corpus | compression-spike/results/corpus.json (sha256 aa3266b6a7a8ba8c269953e61c6f5446336401a03ae335acbea44cc4b3a9b789); compression-spike/results/corpus-payloads.json (sha256 038331757666a68a01ee628dad66c38f46fe631fa090eee90cbec8bc9c6b7134); per-file SHA-256 inventories in `results/0.10.0-broad/canonical/` |
| Lab | `TigerWinLab-Win11-Clean` — Windows 11 Enterprise 25H2 Evaluation x64 (clean fixture); Microsoft Windows 11 Enterprise Evaluation 25H2 build 26200.9457; checkpoint `BASE-CLEAN`; commands run as the job account (`TWL-WIN11C\LabAdmin`, session 0) |
| Rows | builder `tiger-setup 0.10.0`, engine `3e6f332c6af5cb3b30b141181cd8d7305b78105cdce8d7bdc6471a433f64dff9`; every row records the SHA-256 and bytes of the installer it ran (`lab-results.json`), matching `build.json` |

## Campaign notes

- Builds ran serially on the build machine on the evening of 2026-09-21 (about 40 minutes for the 48), with the host otherwise idle apart from light script editing and two short runs of the report generator during the qBittorrent and VLC builds (a second or two of one core on a 16-thread host; no build was repeated for it).
- TigerKeyring's payload, decomposed from a shipped TigerSetup installer, carries that installer's own packaged actions under `.tigersetup/` (2 of its 4 files, 3,109 bytes), a directory TigerSetup reserves and refuses in a payload (`metadata_invalid`). All three packages exclude it (TigerSetup `exclude`, Inno Setup `Excludes`, NSIS `/x`), so the installed set is 2 files in every technology; the corpus record keeps the spike's 4-file inventory and says what was excluded (`packageExcludes`). Tiger3dForge, decomposed the same way, carries no such entries.
- Before the campaign, every TigerSetup installer was inspected (`tiger-setup inspect --json`: entry count equal to the package's file count, engine 3e6f332c…) and every NSIS installer listed with 7-Zip's NSIS reader (file count equal to the package's plus `Uninstall.exe` and the `System.dll` plugin `${GetSize}` needs); Inno Setup 7 installers cannot be listed by open tooling, so their fidelity rests on the rows' file-by-file verification alone.
- Three smoke rows ran first into `results/0.10.0-broad/smoke/` (not part of the campaign): TigerKeyring-NSIS, WinSCP-InnoSetup and NotepadPlusPlus-TigerSetup. The Inno Setup row exposed a driver assumption, not a technology defect: Inno Setup's default Add/Remove Programs `DisplayName` is `<AppName> <AppVersion>` (`WinSCP 6.5.7`), and the registration check now accepts a DisplayName that begins with the application name, as the contract records. The row was re-measured and passed. Its two runs also showed how far one row can move: the identical 4-file install took 2.72 s the first time and 10.92 s the second, on the same restored checkpoint — the single-measurement noise the report's Limitations warn about, most visible on the smallest payloads.
- Launch stalls. Seven timed commands of the 90 show the same signature: a process that starts 8–10 s late and then does its work at the usual speed, in all three technologies — TigerSetup (whose engine log places it before the engine's first event: ShareX install, WinMerge uninstall, NotepadPlusPlus install; VSCode's 7.5 s before its engine is within what Windows takes to read a 230 MB executable for the first time and is not counted), Inno Setup (WinMerge uninstall, 9.4 s of process for sub-second work; WinSCP install, 10.6 s where the same row's smoke run took 2.7 s) and NSIS (VLC uninstall and NotepadPlusPlus uninstall, 9 s each). Every stalled launch is of an executable freshly written to the VM — the staged installer, TigerSetup's extracted engine, Inno Setup's second-phase copy, NSIS's Au_.exe — on a clean, online Windows 11 with Defender at its defaults, whose cloud-delivered first-sight check holds an unknown executable for up to 10 s; that is a hypothesis consistent with the signature, not something this campaign verified. No row was re-measured for it: the stall is random, re-rolling it would bias the set, and the medians and quartiles are robust to it. The rows it affects are the ones to discount when reading a single ratio (WinMerge's uninstall ratios above all).
- The determinism check rebuilt Minimal, NotepadPlusPlus and GitForWindows with all three technologies into a separate artifacts root after the campaign's builds; all nine installers were byte-identical to the campaign's (determinism.json).

## Limitations

- **Single measurements.** One build per definition and one lab row per
  installer. The 0.9.0 campaign found host activity during a build moves
  its wall clock by 2–8%; a VM row varies with the host's disk cache and
  the guest's background services. Differences of a few percent are noise;
  the distributions over fifteen applications are the finding, not any
  single row.
- **Live host.** Builds ran on a developer workstation with the operating
  system's cache as it was; the technology order rotates so no technology
  always builds first into a cold cache, and nothing was flushed.
- **VM timing.** Install and uninstall times are a 4-vCPU Hyper-V guest's,
  with a differencing disk restored from a checkpoint before every row;
  absolute times are not a physical machine's, ratios between technologies
  on the same row are the comparable figure.
- **Corpus representativeness.** Fifteen payloads chosen for shape
  variety — five of them IT Tiger's own applications — not a sample of the
  Windows application population; GIMP is absent because its Inno Setup 7
  installer cannot be unpacked by open tooling.
- **The common contract is deliberately small.** No shortcut, association,
  dependency or custom action: the rows measure payload handling and
  bookkeeping, not the functional features the first benchmark compared.
- **User scope only**, run as an administrator job account: no technology
  elevates, and the machine-scope paths (Program Files, HKLM) are not
  exercised here.
- **Launch stalls.** On the clean VM, starting a freshly written executable
  — an installer just staged, an engine or uninstaller copy just extracted
  — occasionally stalled for 8–10 s before its first instruction, in every
  technology (TigerSetup's *before engine* column in section D shows it
  directly; for the others it shows as a 9 s process lifetime on sub-second
  work). The campaign notes name the commands affected. Medians and
  quartiles absorb it; a single row's ratio may not.
- **TigerSetup 0.10.0 is frozen.** Nothing in the product was tuned on
  these results; a weakness they expose is evidence for a later version.
