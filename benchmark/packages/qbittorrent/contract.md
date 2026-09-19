# qBittorrent 5.2.3 — benchmark functional contract

Derived from qBittorrent's real upstream NSIS scripts
(`dist/windows/installer.nsh`, `dist/windows/config.nsh`, read at the pinned
tag). The real installer's `RequestExecutionLevel` is `user` but every write
it makes targets `HKLM`, so it is a per-machine installer in practice
regardless of the declared level; the benchmark contract keeps it
machine-only across all three technologies rather than reproducing that
inconsistency.

Package identity note: the canonical payload includes `qbittorrent.pdb`
(181 MB) because the real official installer genuinely ships it (see
`scripts/New-CanonicalPayload.ps1`) -- not a benchmark artifact.

| Feature | Upstream default | Benchmark contract |
|---|---|---|
| Install scope | machine only (`$PROGRAMFILES64`, all HKLM writes) | machine only, all three technologies |
| Payload | qbittorrent.exe + qbittorrent.pdb + qt.conf + translations (~232 MB canonical) | identical canonical payload, all three technologies |
| Start Menu shortcut | always | always |
| Desktop shortcut | optional, unchecked | optional, default **off** |
| Start on Windows startup | optional, unchecked, `HKCU Run` | optional, default **off**; a link in the shared Startup folder in all three technologies (the real installer writes `HKCU\Run` from an elevated process, which lands in the installing administrator's own hive rather than the machine's -- the benchmark uses the one mechanism all three express the same way) |
| Firewall authorization | optional, **checked** | optional, default **on** -- an inbound allow rule for `qbittorrent.exe` (IS and NSIS: `netsh advfirewall`, the natural form in both; TS: `[[firewall]]`) |
| Disable Windows path length limit | optional, **checked**, `HKLM ... LongPathsEnabled=1` | optional, default **on** -- each technology's registry primitive (IS `[Registry]`, NSIS `WriteRegDWORD`, TS `[[registry]] root = "HKLM"`). What happens to the setting at uninstall is each technology's own semantics, reported rather than prescribed: IS and NSIS leave the value as the real installer does; TigerSetup restores what the machine held before the install |
| File association | always, `.torrent` -> `qBittorrent.File.Torrent` | always |
| URL protocol | always, `magnet:` -> `qBittorrent.Url.Magnet` | always |
| Uninstall registration (ARP) | own `HKLM` uninstall key | each technology's native ARP registration |
| "Bonus content" / search plugin components | existed in older qBittorrent lines, **not present in 5.x** | not applicable |
| Running-instance check on install/uninstall | `FindProcDLL::FindProc` | excluded -- an NSIS-plugin-specific mechanism with no natural equivalent primitive in the other two technologies; Restart Manager coordination is TigerSetup's own typed mechanism for this class of problem, not something to fake in Inno/NSIS for symmetry |

Every option defaults to the same value as the real qBittorrent installer.
