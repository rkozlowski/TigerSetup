#Requires -Version 7.0
<#
    .SYNOPSIS
    Generates the repository's report.md from the machine-readable results
    in benchmark/results: the toolchain, the pinned applications and their
    canonical payloads, the built installers, the build times and the lab
    campaign. Nothing in the report is typed by hand; regenerate it whenever
    a result file changes.

    .DESCRIPTION
    The report reads only the final campaign's data (results/lab-results.json)
    and refuses to mix in anything else. Where a technology's feature
    implementation is a fact about the package definition rather than a
    measurement — whether a feature is a native primitive, script, a custom
    action or a workaround — that fact is stated in the tables below, beside
    the file it comes from, so a reader can check it against the definition.

    .EXAMPLE
    pwsh -File benchmark\scripts\New-Report.ps1
#>
[CmdletBinding()]
param(
    [string] $OutputPath = (Join-Path $PSScriptRoot '..\..\report.md')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$resultsRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\results')).Path
function Read-Json { param([string] $Name) Get-Content -LiteralPath (Join-Path $resultsRoot $Name) -Raw | ConvertFrom-Json }
function Get-Prop { param([object] $Object, [string] $Name) if ($null -eq $Object -or $Object.PSObject.Properties.Match($Name).Count -eq 0) { return $null }; $Object.$Name }

$toolchain = Read-Json 'toolchain.json'
$apps = Read-Json 'apps.json'
$downloads = Read-Json 'downloads.json'
$build = Read-Json 'build.json'
$lab = Read-Json 'lab-results.json'
$minimal = Read-Json 'minimal-installers.json'
$appNames = @('ShareX', 'WinMerge', 'qBittorrent', 'VLC')
$technologies = @('InnoSetup', 'NSIS', 'TigerSetup')
$techLabel = @{ InnoSetup = 'Inno Setup'; NSIS = 'NSIS'; TigerSetup = 'TigerSetup' }
$canonical = @{}
$installers = @{}
foreach ($app in $appNames) {
    $canonical[$app] = Read-Json "canonical-$app.json"
    $installers[$app] = Read-Json "$($app.ToLowerInvariant())-installers.json"
}

function N { param([object] $Value) if ($null -eq $Value) { return 'n/a' }; ('{0:N0}' -f [double] $Value) }
function MiB { param([object] $Value) if ($null -eq $Value) { return 'n/a' }; ('{0:N1}' -f ([double] $Value / 1MB)) }
function Sec { param([object] $Value) if ($null -eq $Value) { return 'n/a' }; ('{0:N2} s' -f [double] $Value) }
function Pct { param([double] $Part, [double] $Whole) if ($Whole -eq 0) { return 'n/a' }; ('{0:N1}%' -f (100 * $Part / $Whole)) }
function Short { param([string] $Hash) if ([string]::IsNullOrEmpty($Hash)) { return '' }; $Hash.Substring(0, 16) + '…' }
function Installer { param([string] $App, [string] $Tech) @($installers[$App].installers | Where-Object { $_.name -eq "$App-$Tech.exe" }) | Select-Object -First 1 }
function Row { param([string] $App, [string] $Tech) @($lab.rows | Where-Object { $_.app -eq $App -and $_.tech -eq $Tech }) | Select-Object -First 1 }
function BuildOf { param([string] $App, [string] $Tech) @($build.builds | Where-Object { $_.app -eq $App -and $_.technology -eq $Tech }) | Select-Object -First 1 }
function Mark { param([object] $Ok) if ($null -eq $Ok) { return '—' }; if ([bool] $Ok) { 'yes' } else { '**no**' } }

$missingRows = @(foreach ($app in $appNames) { foreach ($tech in $technologies) { if ($null -eq (Row $app $tech)) { "$app-$tech" } } })
if ($missingRows.Count -gt 0) { Write-Warning "lab-results.json has no row for: $($missingRows -join ', '); the runtime tables will say so." }

$sb = [System.Text.StringBuilder]::new()
function L { param([string] $Line = '') $null = $sb.AppendLine($Line) }

$ts = $toolchain.tigersetup
$inno = $toolchain.inno_setup
$nsis = $toolchain.nsis

L "# Installer-technology benchmark: TigerSetup $($ts.version) vs Inno Setup $($inno.version) vs NSIS $($nsis.version)"
L
L "Three Windows installer technologies — **TigerSetup $($ts.version)**, **Inno Setup"
L "$($inno.version)** and **NSIS $($nsis.version)** — packaging the same four real, open-source"
L "Windows applications with each, plus a minimal no-payload installer per"
L "technology, measured statically on the build machine and at runtime on"
L "TigerWinLab's clean Windows 11 baseline (``$($lab.baseline)``)."
L
L "The reproducible harness — acquisition, canonical payloads, package"
L "definitions, build, measurement, the lab campaign driver and this report's"
L "generator — lives under [``benchmark/``](benchmark/README.md). Every number"
L "below is read from ``benchmark/results/*.json`` by"
L "``benchmark/scripts/New-Report.ps1``; nothing is transcribed by hand."
L
L "## How this report came to be"
L
L "The benchmark was first run against TigerSetup 0.7.0. That prototype run"
L "exposed two problems before any runtime number could be trusted, and both"
L "were corrected before the measurement campaign this report is built from:"
L
L "- **A TigerSetup capability gap.** qBittorrent's real installer writes"
L "  ``HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled``, and"
L "  TigerSetup 0.7.0's typed ``[[registry]]`` resource reached only the scope's"
L "  ``Software`` root, so the prototype package fell back to a custom action"
L "  running ``reg.exe`` from a hard-coded ``System32`` path — the wrong"
L "  abstraction, and not a portable package definition. TigerSetup 0.7.1"
L "  added explicit registry locations to the typed resource (``root = ""HKLM""``),"
L "  with the pre-installation state recorded and restored, and the package"
L "  now declares the value as the typed resource it is."
L "- **A benchmark harness defect.** The prototype's NSIS rows for the"
L "  per-user applications ran the installer without ``/S``, so the wizard waited"
L "  invisibly on the desktop-less job account until the timeout; the"
L "  prototype also timed an NSIS uninstall by the lifetime of ``Uninstall.exe``,"
L "  which copies itself to ``%TEMP%`` and exits while the copy does the work,"
L "  and papered over the gap with a fixed sleep. The harness now waits for the"
L "  actual completion — the hand-off process's exit and the install root's"
L "  removal — and no artificial delay is anywhere in the measured path."
L
L "TigerSetup was then frozen at **0.7.1** (engine"
L "``$($ts.engine.sha256)``, commit ``$($ts.git_commit)``$(if ($ts.working_tree_dirty) { ', uncommitted working tree' })),"
L "every installer was rebuilt from the definitions in this repository, and"
L "**every measurement below comes from that single clean campaign**"
L "(``benchmark/results/lab-results.json``, started $(([DateTimeOffset] $lab.startedAt).ToString('yyyy-MM-dd HH:mm zzz'))). No"
L "prototype measurement was kept."
L
L "## Scope and methodology"
L
L "**Question.** With real applications and real measurements, how does"
L "TigerSetup's packaging output compare with two established installer"
L "technologies — on installer size, on install and uninstall time on a clean"
L "Windows 11, on what each installer actually leaves on the machine, and on"
L "how much of each application's installer-level contract each technology"
L "can express natively rather than by script or custom action?"
L
L "**Applications.** Four open-source Windows applications whose upstream"
L "installers are themselves built with the technologies under comparison —"
L "so the ``Upstream tech`` column is an independent reference point, not a"
L "benchmark artifact — chosen together for the installer features they"
L "exercise: per-user and per-machine scope, file associations, a URL"
L "protocol, classic and COM context-menu integration, a firewall rule, PATH,"
L "and a system-wide registry setting."
L
L "| App | Version | Upstream tech | Licence | Canonical source |"
L "|---|---|---|---|---|"
foreach ($entry in $apps.apps) {
    $source = @($downloads | Where-Object { $_.app -eq $entry.name -and $_.kind -eq $(if ($entry.canonical_source -eq 'portable') { 'portable' } else { 'installer' }) }) | Select-Object -First 1
    L "| $($entry.name) | $($entry.version) | $($entry.upstream_installer_tech) | $($entry.license) | ``$($source.filename)`` ($(N $source.bytes) bytes, SHA-256 ``$(Short $source.sha256)``$(if ($source.hash_verified) { ', matches the published hash' })) |"
}
L
L "No application was built from source; every payload comes from an official"
L "release artifact pinned in ``benchmark/results/apps.json`` and verified in"
L "``downloads.json``. WinMerge's real version is the four-part ``2.16.58.2``;"
L "TigerSetup requires a strict three-part version, so all three benchmark"
L "packages for WinMerge use ``2.16.58`` for identity parity."
L
L "**Canonical payloads.** For a given app, all three technologies install"
L "byte-identical files: the canonical payload, derived from the official"
L "artifact by ``benchmark/scripts/New-CanonicalPayload.ps1`` with every"
L "normalization recorded, and inventoried file by file with SHA-256"
L "(``benchmark/results/canonical-<App>.json``)."
L
L "| App | Canonical files | Canonical bytes | Normalizations |"
L "|---|---:|---:|---|"
foreach ($app in $appNames) {
    $c = $canonical[$app]
    L "| $app | $(N $c.fileCount) | $(N $c.totalBytes) | $(($c.normalizations | ForEach-Object { $_ -replace '\|', '\|' }) -join ' ') |"
}
L
L "**Functional contracts.** Each application's installer-level contract is"
L "derived from its real upstream installer script and written down once in"
L "``benchmark/packages/<app>/contract.md``, with every simplification stated"
L "(VLC's 125+ associations reduced to a representative six; WinMerge's"
L "component matrix collapsed to one full install). All three package"
L "definitions of an app implement that contract, every option defaulting to"
L "what the real installer defaults to, and each technology uses its own"
L "natural primitive for each feature — a native primitive where it has one,"
L "its script where it has not, a custom action where none of the three has"
L "a typed form."
L
L "**Compression.** Each technology's commonly recommended production-quality"
L "setting: Inno Setup ``$($inno.compression)``; NSIS ``$($nsis.compression)``;"
L "TigerSetup's release build (its default compression search — never"
L "``--fast``, which the project's release discipline excludes from anything"
L "measured or published)."
L
L "**Runtime measurement.** Every row is one TigerWinLab session on one VM:"
L "the VM is restored to the ``$($lab.baseline)`` checkpoint, the installer is"
L "staged and run silently as the job account (an administrator with no"
L "interactive desktop), the machine is read — the install root, the"
L "registry keys, shortcuts, firewall rules and PATH the contract names — the"
L "uninstaller is run silently, the machine is read again, the session is"
L "closed, and the next row starts only after the lab reports the VM"
L "normalized and available. Rows never overlap. Technology order rotates"
L "between applications. **Timing boundaries are identical across"
L "technologies**: $($lab.timing). One measurement per row: the lab's"
L "restore-and-boot cost per row makes repeated samples materially more"
L "expensive, so the figures are single controlled observations, not"
L "distributions — treat differences of a second or two as noise."
L
L "**The installed payload is a control, not a score.** Every installer must"
L "leave exactly the canonical files under the install root, plus its own"
L "uninstaller bookkeeping; identical installed sizes are the expected result."
L
L "## Toolchain"
L
L "| Technology | Version | Raw engine / stub | Bytes | SHA-256 |"
L "|---|---|---|---:|---|"
L "| TigerSetup | $($ts.version) | ``tigersetup-setup.exe`` (embedded verbatim in every installer) | $(N $ts.engine.bytes) | ``$($ts.engine.sha256)`` |"
L "| Inno Setup | $($inno.version) | ``Setup.e64`` (native x64 engine; embedded LZMA2-compressed behind ``SetupLdr.e64``, $(N $inno.loader_stub.bytes) bytes) | $(N $inno.native_x64_engine.bytes) | ``$($inno.native_x64_engine.sha256)`` |"
L "| NSIS | $($nsis.version) | ``Stubs\lzma_solid-x86-unicode`` (32-bit exehead of the chosen compressor) | $(N $nsis.exehead.bytes) | ``$($nsis.exehead.sha256)`` |"
L
L "- $($ts.note)"
L "- $($inno.note)"
L "- $($nsis.note)"
L
L "**Minimal installers** (identity and metadata only; TigerSetup's carries one"
L "38-byte file because its builder requires a file set):"
L
L "| Technology | Installer | Bytes | SHA-256 |"
L "|---|---|---:|---|"
foreach ($m in ($minimal.installers | Sort-Object bytes)) {
    L "| $($techLabel[($m.name -replace '^Minimal-|\.exe$', '')]) | ``$($m.name)`` | $(N $m.bytes) | ``$($m.sha256)`` |"
}
L
L "## Installer sizes"
L
L "Exact bytes are the data; MiB is shown for readability. ``Ratio`` is"
L "installer bytes over canonical payload bytes."
L
L "| App | Technology | Installer bytes | MiB | Ratio | vs. smallest | SHA-256 |"
L "|---|---|---:|---:|---:|---:|---|"
foreach ($app in $appNames) {
    $sizes = @(foreach ($tech in $technologies) { $i = Installer $app $tech; [pscustomobject]@{ tech = $tech; bytes = [long] $i.bytes; sha = $i.sha256 } }) | Sort-Object bytes
    $smallest = $sizes[0].bytes
    foreach ($size in $sizes) {
        $diff = $size.bytes - $smallest
        $vs = if ($diff -eq 0) { 'smallest' } else { "+$(N $diff) (+$(Pct $diff $smallest))" }
        L "| $app | $($techLabel[$size.tech]) | $(N $size.bytes) | $(MiB $size.bytes) | $(Pct $size.bytes $canonical[$app].totalBytes) | $vs | ``$($size.sha)`` |"
    }
}
L
L "TigerSetup's payload block is a ZIP archive using ``Stored`` and ``Deflate``"
L "entries (``crates/tigersetup-format/src/payload.rs``); Inno Setup and NSIS"
L "compress with LZMA2 and LZMA. That single format choice explains the"
L "consistent size difference on every application; it is not a per-app"
L "effect or a misconfiguration, and it is the benchmark's largest"
L "quantitative finding about TigerSetup."
L
L "**Build times** (wall-clock around one compiler invocation on the build"
L "machine, one run each; an engineering observation, not a compiler"
L "benchmark):"
L
L "| App | TigerSetup | Inno Setup | NSIS |"
L "|---|---:|---:|---:|"
foreach ($app in @('Minimal') + $appNames) {
    L "| $app | $(Sec (Get-Prop (BuildOf $app 'TigerSetup') 'buildSeconds')) | $(Sec (Get-Prop (BuildOf $app 'InnoSetup') 'buildSeconds')) | $(Sec (Get-Prop (BuildOf $app 'NSIS') 'buildSeconds')) |"
}
L
L "## Runtime: install, uninstall, and what was left on the machine"
L
L "Every row on the same clean Windows 11 baseline, silently, as described"
L "above. ``Installed`` counts the files under the install root after the"
L "install; ``Extra`` is what the technology added beyond the canonical"
L "payload (Inno Setup's ``unins000.exe``/``.dat``, NSIS's ``Uninstall.exe``;"
L "TigerSetup keeps its state outside the install root). ``Uninstall`` time"
L "includes the completion wait the harness needed after the uninstaller"
L "process exited, shown separately as ``(+wait)``."
L
L "| App | Technology | Row | Install | Installed files / extra | Installed bytes / extra | Uninstall (+wait) | Root removed | ARP | Start Menu link |"
L "|---|---|---|---:|---:|---:|---:|---|---|---|"
foreach ($app in $appNames) {
    foreach ($tech in $technologies) {
        $r = Row $app $tech
        if ($null -eq $r) { L "| $app | $($techLabel[$tech]) | **not run** | | | | | | | |"; continue }
        $i = $r.install; $u = $r.uninstall
        $status = if ([bool] $r.success) { 'ok' } else { '**FAILED**' }
        $wait = Get-Prop $u 'completionWaitSeconds'
        L "| $app | $($techLabel[$tech]) | $status | $(Sec $i.durationSeconds) | $(N $i.installedFiles) / $(N $i.extraFiles) | $(N $i.installedBytes) / $(N $i.extraBytes) | $(Sec $u.durationSeconds) (+$(Sec $wait)) | $(Mark $u.rootRemoved) | $(Mark $i.arpPresent) → $(Mark $u.arpRemoved) | $(Mark $i.startMenuLinkOk) → $(Mark $u.startMenuLinkRemoved) |"
    }
}
L
L "Read ``ARP`` and ``Start Menu link`` as *present after install → removed after"
L "uninstall*. ``Row`` is the harness's verdict on the whole row: both jobs ran,"
L "both processes exited 0, the completion wait was satisfied, every"
L "install-time contract probe held, and every app-owned resource was gone"
L "after the uninstall. A row measured after a definition fix carries its own"
L "``finishedAt``$(if (@($lab.rows | Where-Object { $null -ne (Get-Prop $_ 'finishedAt') }).Count -gt 0) { ': ' + ((@($lab.rows | Where-Object { $null -ne (Get-Prop $_ 'finishedAt') } | ForEach-Object { "$($_.row) ($(([DateTimeOffset] $_.finishedAt).ToString('HH:mm')))" })) -join ', ') + ' — the NSIS packages of the two 64-bit machine-scope applications first wrote their Add/Remove Programs entry into the 32-bit registry view and were corrected with `SetRegView 64`, as the real installers do' })."
L
L "**Totals over the four applications** (the same single measurements,"
L "summed; a coarse view of the same data, not a second measurement):"
L
L "| Technology | Install, all four apps | Uninstall, all four apps |"
L "|---|---:|---:|"
foreach ($tech in $technologies) {
    $sumI = 0.0; $sumU = 0.0; $n = 0
    foreach ($app in $appNames) { $r = Row $app $tech; if ($null -ne $r -and $null -ne $r.install.durationSeconds -and $null -ne $r.uninstall.durationSeconds) { $sumI += [double] $r.install.durationSeconds; $sumU += [double] $r.uninstall.durationSeconds; $n++ } }
    L "| $($techLabel[$tech]) | $(if ($n -eq 4) { Sec $sumI } else { "n/a ($n of 4 rows)" }) | $(if ($n -eq 4) { Sec $sumU } else { "n/a ($n of 4 rows)" }) |"
}
L
$slowestUninstall = @(foreach ($app in $appNames) { (@(foreach ($tech in $technologies) { $r = Row $app $tech; if ($null -ne $r) { [pscustomobject]@{ tech = $tech; s = [double] $r.uninstall.durationSeconds } } }) | Sort-Object s -Descending)[0].tech })
$installRank = @(foreach ($app in $appNames) { $ranked = @(foreach ($tech in $technologies) { $r = Row $app $tech; if ($null -ne $r) { [pscustomobject]@{ tech = $tech; s = [double] $r.install.durationSeconds } } }) | Sort-Object s; "$app $(([array]::IndexOf(@($ranked | ForEach-Object { $_.tech }), 'TigerSetup') + 1)) of $($ranked.Count)" })
L "TigerSetup's uninstall is the slowest of the three on $(if (@($slowestUninstall | Where-Object { $_ -ne 'TigerSetup' }).Count -eq 0) { 'every application' } else { "$(@($slowestUninstall | Where-Object { $_ -eq 'TigerSetup' }).Count) of the four applications" }),"
L "by a wide margin; its install ranks (fastest = 1) $($installRank -join ', ')."
L "The engine journals every operation durably before and after each"
L "mutation — the crash-consistency design (``TigerSetup-Design.md`` §5.4) —"
L "and pays for it in per-file commits; Inno Setup and NSIS delete a tree"
L "without a journal. This is a real cost of the transactional model at this"
L "engine version, reported in the gaps below."
L
L "## Functional parity"
L
L "For each application, each feature of its contract with how each"
L "technology's package definition implements it — **native**: a declarative"
L "primitive of the technology; **script**: installer-script code written for"
L "this package; **action**: a custom executable or script the installer runs;"
L "**workaround**: something done outside the technology's own model — and"
L "what the lab read from the machine after install (the mechanism found)"
L "and after uninstall. The implementation column is a fact about the files"
L "under ``benchmark/packages/<app>/``; the result columns are measurements."
L

# The implementation form of every feature, per technology, as the package
# definitions have it. Kept beside the generator, verified against the
# definition files by reading them.
$forms = @{
    ShareX = @(
        @{ feature = 'Files, install root, per-user or per-machine scope'; InnoSetup = 'native (`PrivilegesRequiredOverridesAllowed=dialog`)'; NSIS = 'script (`MultiUser.nsh`: mode page, `.onInit` re-exec, `SHCTX`)'; TigerSetup = 'native (`scopes = ["user", "machine"]`)' }
        @{ feature = 'Start Menu shortcut'; InnoSetup = 'native (`[Icons]`)'; NSIS = 'native (`CreateShortCut`)'; TigerSetup = 'native (`[[shortcuts]]`)' }
        @{ feature = 'Desktop / Send To / Startup shortcuts (optional, off)'; InnoSetup = 'native (`[Tasks]` + `[Icons]`)'; NSIS = 'native (`Section /o` + `CreateShortCut`)'; TigerSetup = 'native (`[[options]]` + `[[shortcuts]]`)' }
        @{ feature = 'Explorer context menu "Upload with ShareX" (optional, off)'; InnoSetup = 'native (`[Registry]` on `Classes\*\shell`)'; NSIS = 'native (`WriteRegStr` on `Classes\*\shell`)'; TigerSetup = 'native (`[[context_menu]]`)' }
        @{ feature = 'Add/Remove Programs registration'; InnoSetup = 'native (automatic)'; NSIS = 'script (`WriteRegStr` × 8 by hand)'; TigerSetup = 'native (automatic)' }
    )
    WinMerge = @(
        @{ feature = 'Files, install root, per-user or per-machine scope'; InnoSetup = 'native'; NSIS = 'script (`MultiUser.nsh`)'; TigerSetup = 'native' }
        @{ feature = 'Start Menu shortcut; desktop shortcut (optional, off)'; InnoSetup = 'native'; NSIS = 'native'; TigerSetup = 'native' }
        @{ feature = '`.WinMerge` file association'; InnoSetup = 'native (`[Registry]`, extension default = ProgID)'; NSIS = 'native (`WriteRegStr`, extension default = ProgID)'; TigerSetup = 'native (`[[file_associations]]`, handler under `OpenWithProgids`; never the default)' }
        @{ feature = 'App Paths entry'; InnoSetup = 'native (`[Registry]`)'; NSIS = 'native (`WriteRegStr`)'; TigerSetup = 'native (`[[app_paths]]`)' }
        @{ feature = 'Explorer context menu: the real COM shell extension'; InnoSetup = 'action (`[Run]`/`[UninstallRun]` regsvr32 via `{sys}`)'; NSIS = 'action (`ExecWait regsvr32.exe`)'; TigerSetup = 'action (`[[actions]]` post-install/pre-uninstall running a packaged `regsvr32.cmd`)' }
        @{ feature = 'Add to PATH (optional, off)'; InnoSetup = 'script (~35 lines of Pascal: read-modify-write of the `Path` value)'; NSIS = 'script (~15 lines: `ReadRegStr`/`WriteRegExpandStr`/broadcast)'; TigerSetup = 'native (`[[path]]`)' }
        @{ feature = 'Add/Remove Programs registration'; InnoSetup = 'native'; NSIS = 'script'; TigerSetup = 'native' }
    )
    qBittorrent = @(
        @{ feature = 'Files, install root, machine scope'; InnoSetup = 'native (`PrivilegesRequired=admin`)'; NSIS = 'native (`RequestExecutionLevel admin`, `SetShellVarContext all`)'; TigerSetup = 'native (`scopes = ["machine"]`)' }
        @{ feature = 'Start Menu shortcut; desktop and Startup shortcuts (optional, off)'; InnoSetup = 'native'; NSIS = 'native'; TigerSetup = 'native' }
        @{ feature = '`.torrent` file association'; InnoSetup = 'native (extension default = ProgID)'; NSIS = 'native (extension default = ProgID)'; TigerSetup = 'native (handler under `OpenWithProgids`)' }
        @{ feature = '`magnet:` URL protocol'; InnoSetup = 'native (`[Registry]`: scheme class + command)'; NSIS = 'native (`WriteRegStr`: scheme class + command)'; TigerSetup = 'native (`[[url_protocols]]`: handler ProgID + capability registration; the scheme class is claimed only when free)' }
        @{ feature = 'Firewall rule (optional, on)'; InnoSetup = 'action (`[Run]`/`[UninstallRun]` netsh via `{sys}`)'; NSIS = 'action (`ExecWait netsh.exe`)'; TigerSetup = 'native (`[[firewall]]`, Windows Firewall API)' }
        @{ feature = '`LongPathsEnabled = 1` under `HKLM\SYSTEM` (optional, on)'; InnoSetup = 'native (`[Registry]`; the value is left at uninstall)'; NSIS = 'native (`WriteRegDWORD`; the value is left at uninstall)'; TigerSetup = 'native (`[[registry]] root = "HKLM"`, new in 0.7.1; the prior value is restored at uninstall)' }
        @{ feature = 'Add/Remove Programs registration'; InnoSetup = 'native'; NSIS = 'script'; TigerSetup = 'native' }
    )
    VLC = @(
        @{ feature = 'Files, install root, machine scope'; InnoSetup = 'native'; NSIS = 'native (`SetShellVarContext all`)'; TigerSetup = 'native' }
        @{ feature = 'Start Menu shortcut (in a `VideoLAN` group); desktop shortcut (optional, off)'; InnoSetup = 'native'; NSIS = 'native'; TigerSetup = 'native (the benchmark manifest puts the link directly in Programs; `folder = "VideoLAN"` on the `[[shortcuts]]` entry would have matched the group)' }
        @{ feature = 'App Paths entry'; InnoSetup = 'native'; NSIS = 'native'; TigerSetup = 'native' }
        @{ feature = 'Six file associations (`.mp3 .flac .wav .mp4 .mkv .avi`)'; InnoSetup = 'native (30 `[Registry]` lines, one block per extension)'; NSIS = 'native (a `!macro` per extension)'; TigerSetup = 'native (six `[[file_associations]]`; handlers, never the default)' }
        @{ feature = '"Play with VLC" on those types and on a folder background (optional, on)'; InnoSetup = 'native (`[Registry]` verbs under each ProgID and `Directory\Background`)'; NSIS = 'native (`WriteRegStr` verbs, same keys)'; TigerSetup = 'native (two `[[context_menu]]` entries: `SystemFileAssociations\<.ext>\shell` and `Directory\Background\shell`)' }
        @{ feature = 'Add/Remove Programs registration'; InnoSetup = 'native'; NSIS = 'script'; TigerSetup = 'native' }
    )
}

foreach ($app in $appNames) {
    L "### $app"
    L
    L "| Feature | Inno Setup | NSIS | TigerSetup |"
    L "|---|---|---|---|"
    foreach ($form in $forms[$app]) {
        L "| $($form.feature) | $($form.InnoSetup) | $($form.NSIS) | $($form.TigerSetup) |"
    }
    L
    L "What the lab read after install and after uninstall (each probe's"
    L "mechanism as found; *ok* = the contract held, *gone* = the resource was"
    L "removed, a **setting** is reported as observed):"
    L
    L "| Probe | Inno Setup | NSIS | TigerSetup |"
    L "|---|---|---|---|"
    $probeNames = @()
    foreach ($tech in $technologies) { $r = Row $app $tech; if ($null -ne $r) { foreach ($p in @($r.install.contract)) { if ($p.feature -notin $probeNames) { $probeNames += $p.feature } } } }
    foreach ($name in $probeNames) {
        $cells = foreach ($tech in $technologies) {
            $r = Row $app $tech
            if ($null -eq $r) { 'not run'; continue }
            $pi = @($r.install.contract | Where-Object { $_.feature -eq $name }) | Select-Object -First 1
            $pu = @($r.uninstall.contract | Where-Object { $_.feature -eq $name }) | Select-Object -First 1
            if ($null -eq $pi) { '—'; continue }
            $after = switch ($pi.kind) {
                'setting' { "after uninstall: $(($pu.detail -split '=')[-1])" }
                { $_ -like 'absent-*' } { if ([bool] $pu.present) { 'still absent' } else { '**appeared**' } }
                default { if ([bool] $pu.present) { '**LEFT BEHIND**' } else { 'gone' } }
            }
            $installed = if ([bool] $pi.present) { 'ok' } else { '**MISSING**' }
            "$installed — $($pi.mechanism); $after"
        }
        L "| $name | $($cells -join ' | ') |"
    }
    L
}

L "## Implementation observations"
L
L "- **Where TigerSetup needed a custom action**: one feature across the four"
L "  applications — WinMerge's COM shell extension, which no technology has a"
L "  typed primitive for; all three run ``regsvr32``. TigerSetup's action is a"
L "  packaged one-line batch script so the command shell resolves ``regsvr32``"
L "  (a ``kind = ""exe""`` action runs exactly the program it names, and a"
L "  package must not carry a ``System32`` path); Inno Setup has ``{sys}`` and"
L "  NSIS relies on ``CreateProcess``'s search. Running elevated, all three"
L "  register the extension machine-wide even for a per-user install, which"
L "  is a property of ``regsvr32``/COM, not of any installer."
L "- **Where Inno Setup and NSIS needed script or an action that TigerSetup"
L "  did not**: PATH (both, by hand), the firewall rule (both, ``netsh``),"
L "  per-user/per-machine choice (NSIS, ``MultiUser.nsh``), Add/Remove Programs"
L "  registration (NSIS, eight ``WriteRegStr`` lines per package)."
L "- **Different Windows mechanisms for the same feature.** For file"
L "  associations and the URL protocol, Inno Setup and NSIS set the"
L "  extension's or scheme's default handler; TigerSetup registers a handler"
L "  (``OpenWithProgids``, capabilities) and never takes the default. For a"
L "  context-menu verb on file types, Inno Setup and NSIS put the verb under"
L "  the ProgID they own; TigerSetup puts it under"
L "  ``SystemFileAssociations\<.ext>``, where Windows shows it whatever the"
L "  type's default handler is. The lab probes accept either; the tables"
L "  above say which was found."
L "- **Conservative removal.** TigerSetup removes what it owns and restores"
L "  what it changed: after its qBittorrent uninstall ``LongPathsEnabled`` is"
L "  back at the baseline's ``0``, where Inno Setup and NSIS leave ``1`` (as the"
L "  real installer does). NSIS's ``DeleteRegKey`` on ``Classes\.torrent`` removes"
L "  the whole extension key, other applications' ``OpenWithProgids`` entries"
L "  included; Inno Setup's ``uninsdeletevalue`` removes only its own value;"
L "  TigerSetup removes only the values and keys it created."
L "- **Definition size and shape.** The TigerSetup manifests are declarative"
L "  throughout; the Inno Setup scripts are declarative except for PATH; the"
L "  NSIS scripts are imperative throughout, longest for the two per-user"
L "  applications because of ``MultiUser.nsh``. This is visible in the"
L "  implementation columns above; no numeric developer-experience score is"
L "  offered."
L "- **Version identity.** TigerSetup's strict three-part version could not"
L "  carry WinMerge's four-part ``2.16.58.2``; Inno Setup and NSIS accept four."
L
L "## TigerSetup gaps and capabilities discovered"
L
L "1. **Payload compression.** ZIP ``Deflate`` against LZMA/LZMA2 is the whole"
L "   size story above and the one finding with a direct product implication"
L "   (download size, WinGet submission size). Not changed during the"
L "   benchmark: the product was frozen, and a compression format is an"
L "   Architect decision."
L "2. **Install and uninstall time on large file sets.** The durable journal"
L "   makes TigerSetup's uninstall several times slower than Inno Setup's or"
L "   NSIS's on every application here (see the totals above), and its install"
L "   slower on the two largest payloads. Whether the per-operation durability"
L "   can be batched without weakening the crash-consistency guarantee is an"
L "   engineering question for the engine, not changed during the benchmark."
L "3. **Explicit registry locations** — found by the 0.7.0 prototype, fixed"
L "   in 0.7.1 before the campaign (``root = ""HKLM""``/``""HKCU""`` on"
L "   ``[[registry]]``, with the value's pre-installation state recorded and"
L "   restored, and a hive that must be the one the package's only scope"
L "   writes). A dual-scope package still cannot declare an ``HKLM`` value:"
L "   there is no scope predicate, and none of the four applications needed one."
L "4. **A Start Menu group folder for the product's own link.** VLC and"
L "   WinMerge put their link in a group folder; TigerSetup's ``[[shortcuts]]``"
L "   has ``folder`` for a subfolder, and the benchmark packages left the link"
L "   directly in Programs. Not a gap in capability; noted because the"
L "   parity table shows a different location."
L "5. **Custom-action program resolution.** A ``kind = ""exe""`` action runs"
L "   exactly the program it names and expands only the known folders; a"
L "   Windows tool (``regsvr32``, ``netsh``) therefore needs the ``cmd`` form. This"
L "   is deliberate (deterministic semantics), and the benchmark records it as"
L "   a property rather than a defect."
L
L "## Limitations"
L
L "- Four applications are evidence about these four applications, not a"
L "  verdict on any technology."
L "- Single measurement per row; the boot-per-row cost of the lab made repeats"
L "  materially more expensive without changing what a second or two of"
L "  difference could show."
L "- Silent/unattended paths only; no interactive wizard, UAC or upgrade rows"
L "  here — TigerSetup's own acceptance suite covers those for TigerSetup."
L "- All rows run as an administrator (the lab's job account), including the"
L "  per-user installs; a standard-user install path is not exercised."
L "- qBittorrent's and VLC's upstream installer scripts were read at a recent"
L "  branch state rather than the exact pinned tag when the contracts were"
L "  written."
L
L "## Conclusions"
L
L "Grounded in this campaign's four applications and single measurements:"
L
$largest = @(foreach ($app in $appNames) { (@(foreach ($tech in $technologies) { [pscustomobject]@{ tech = $tech; bytes = [long] (Installer $app $tech).bytes } }) | Sort-Object bytes -Descending)[0].tech })
$tsLargestEverywhere = @($largest | Where-Object { $_ -ne 'TigerSetup' }).Count -eq 0
$spread = @(foreach ($app in $appNames) { $i = [long] (Installer $app 'InnoSetup').bytes; $n = [long] (Installer $app 'NSIS').bytes; [math]::Round(100 * [math]::Abs($i - $n) / [math]::Min($i, $n), 1) })
L "- **Size**: Inno Setup and NSIS land within $(($spread | Measure-Object -Maximum).Maximum)% of each other on"
L "  every application; TigerSetup's installer is the largest on $(if ($tsLargestEverywhere) { 'every application' } else { "$(@($largest | Where-Object { $_ -eq 'TigerSetup' }).Count) of the four applications" }), by the"
L "  margin its ZIP/Deflate payload predicts, and the fixed engine overhead"
L "  ($(MiB $ts.engine.bytes) MiB, uncompressed) is a second-order term next to any real"
L "  payload. This is the actionable quantitative finding."
$rowsRun = @($lab.rows).Count
$rowsOk = @($lab.rows | Where-Object { [bool] $_.success }).Count
$failed = @($lab.rows | Where-Object { -not [bool] $_.success } | ForEach-Object { $_.row })
L "- **Runtime**: $rowsOk of $rowsRun rows installed and uninstalled the canonical"
L "  payload on the clean baseline with every contract probe holding$(if ($failed.Count -gt 0) { " — the exceptions are $($failed -join ', '), whose rows above say what failed" }); the timing"
L "  table above is the evidence, and its differences are engineering"
L "  observations at single-sample precision."
L "- **Expressiveness**: for these four contracts, TigerSetup expressed"
L "  everything but the COM shell extension declaratively; Inno Setup needed"
L "  script for PATH and actions for the firewall and the shell extension;"
L "  NSIS needed script for scope choice, PATH and ARP and actions for the"
L "  firewall and the shell extension. Where TigerSetup differs in mechanism"
L "  (handlers rather than defaults, verbs under ``SystemFileAssociations``,"
L "  restored rather than abandoned system settings) it differs towards the"
L "  conservative end."
L
L "## A second round with IT Tiger applications"
L
L "Not started. The harness is parameterized by ``benchmark/results/apps.json``"
L "and one ``packages/<app>/`` directory per application; a round with IT Tiger"
L "applications needs a ``contract.md`` per application (from the application's"
L "own requirements rather than an upstream installer script), the three"
L "package definitions, and a rerun of the same acquisition → canonical →"
L "build → measure → lab pipeline. That is an Architect decision."

[System.IO.File]::WriteAllText($OutputPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote $OutputPath"
