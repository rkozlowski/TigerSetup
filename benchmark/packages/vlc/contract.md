# VLC 3.0.23 — benchmark functional contract

Derived from VLC's real upstream NSIS script
(`extras/package/win32/NSIS/vlc.win32.nsi.in`, read at a recent `3.0.x`
branch state). VLC's real installer offers 125+ individually checkable file
associations across audio/video/other categories; reproducing all of them in
three technologies would test list-typing stamina, not installer-technology
capability, so the benchmark uses a representative subset (documented below)
and states the simplification here rather than silently truncating it.

| Feature | Upstream default | Benchmark contract |
|---|---|---|
| Install scope | `RequestExecutionLevel admin`, `SetShellVarContext all` | machine only, all three technologies |
| Payload | VLC win64 tree minus VLC's own MSI/WiX build sources (`msi\`, not an application file) -- ~191 MB canonical | identical canonical payload, all three technologies |
| Install types | Recommended / Minimum / Full | excluded -- collapsed to one contract (see below); a 3-way install-type matrix per technology is not needed to compare packaging capability |
| Start Menu shortcut | always | always |
| Desktop shortcut | optional, unchecked | optional, default **off** |
| Context menu ("Play with VLC") | on associated extensions and on `Directory` | optional, default **on**, on the representative extension subset and on `Directory` |
| File associations | 48 audio + 60 video + ~17 other extensions, per-extension checkboxes | representative subset, always on: `.mp3 .mp4 .mkv .avi .flac .wav` (a mix of the largest audio/video categories) |
| Discs playback / AutoPlay handlers | ProgIDs for DVD/CD/Blu-ray + `AutoplayHandlers` | excluded -- optical-media handling is not materially comparable packaging work across the three technologies (documented simplification, not a missing capability) |
| Web plugins (ActiveX/NPAPI) | Full install type only | excluded -- dead browser-plugin technology on a modern baseline |
| App Paths | always, `vlc.exe` | always |
| Delete preferences/cache on uninstall | optional, unchecked | excluded -- app-runtime state cleanup, not core packaging |
| Memento (remembered selections across upgrades) | real, materially visible upgrade behavior | exercised at the Lab upgrade row where practical; not a separate typed feature |

Every option that is kept defaults to the same value as the real VLC
installer.
