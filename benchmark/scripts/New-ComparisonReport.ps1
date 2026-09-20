#Requires -Version 7.0
<#
    .SYNOPSIS
    Generates the comparison report of a later TigerSetup campaign against
    the frozen first campaign: benchmark/report-<Campaign>.md from
    benchmark/results (the historical evidence: Inno Setup, NSIS and the
    TigerSetup the first campaign froze) and benchmark/results/<Campaign>
    (the fresh evidence: all fifteen installers rebuilt and measured on the
    current host, and the new TigerSetup's lab rows). Nothing in the report
    is typed by hand; regenerate it whenever a result file changes.

    .DESCRIPTION
    The report keeps three classes of evidence apart and says which one
    every number belongs to:

      A. historical — the first campaign's runtime rows (Inno Setup, NSIS,
         TigerSetup as frozen then) and static measurements, never rerun;
      B. fresh host build evidence — every installer definition rebuilt on
         the current host under one method (ProcessTreeMeter.psm1), all
         three technologies alike;
      C. fresh runtime evidence — the new TigerSetup's rows on the same
         clean baseline, the same contracts and the same timing boundaries.

    Campaign notes — a rerun row and why, a build that was repeated, a
    limitation the numbers do not show — are read from
    results/<Campaign>/campaign-notes.json when it exists, so the report
    stays generated even where it needs a sentence.

    .EXAMPLE
    pwsh -File benchmark\scripts\New-ComparisonReport.ps1
    pwsh -File benchmark\scripts\New-ComparisonReport.ps1 -Campaign 0.9.0
#>
[CmdletBinding()]
param(
    [string] $Campaign = '0.9.0',
    [string] $OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$baseRoot = Join-Path $benchmarkRoot 'results'
$freshRoot = Join-Path $baseRoot $Campaign
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $benchmarkRoot "report-$Campaign.md" }
if (-not (Test-Path -LiteralPath $freshRoot -PathType Container)) { throw "No campaign results under '$freshRoot'." }

function Read-Json { param([string] $Root, [string] $Name) Get-Content -LiteralPath (Join-Path $Root $Name) -Raw | ConvertFrom-Json }
function Read-JsonIf { param([string] $Root, [string] $Name) $p = Join-Path $Root $Name; if (Test-Path -LiteralPath $p -PathType Leaf) { Get-Content -LiteralPath $p -Raw | ConvertFrom-Json } else { $null } }
function Get-Prop { param([object] $Object, [string] $Name) if ($null -eq $Object -or $Object.PSObject.Properties.Match($Name).Count -eq 0) { return $null }; $Object.$Name }

$appNames = @('ShareX', 'WinMerge', 'qBittorrent', 'VLC')
$allApps = @('Minimal') + $appNames
$technologies = @('InnoSetup', 'NSIS', 'TigerSetup')
$techLabel = @{ InnoSetup = 'Inno Setup'; NSIS = 'NSIS'; TigerSetup = 'TigerSetup' }

$base = @{
    toolchain = Read-Json $baseRoot 'toolchain.json'
    build = Read-Json $baseRoot 'build.json'
    lab = Read-Json $baseRoot 'lab-results.json'
    installers = @{}
}
$fresh = @{
    toolchain = Read-Json $freshRoot 'toolchain.json'
    build = Read-Json $freshRoot 'build.json'
    lab = Read-JsonIf $freshRoot 'lab-results.json'
    installers = @{}
    canonicalVerification = Read-JsonIf $freshRoot 'canonical-verification.json'
    notes = Read-JsonIf $freshRoot 'campaign-notes.json'
}
$canonical = @{}
foreach ($app in $allApps) {
    $file = "$($app.ToLowerInvariant())-installers.json"
    $base.installers[$app] = Read-Json $baseRoot $file
    $fresh.installers[$app] = Read-Json $freshRoot $file
    if ($app -ne 'Minimal') { $canonical[$app] = Read-Json $baseRoot "canonical-$app.json" }
}
$baseVersion = [string] $base.toolchain.tigersetup.version
$freshVersion = [string] $fresh.toolchain.tigersetup.version
$labBaseline = [string] $base.lab.baseline

function Missing { param([object] $Value) ($null -eq $Value) -or ($Value -is [string] -and [string]::IsNullOrWhiteSpace($Value)) }
function N { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N0}' -f [double] $Value) }
function MiB { param([object] $Value) if ($null -eq $Value) { return 'n/a' }; ('{0:N1}' -f ([double] $Value / 1MB)) }
function MB2 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N0} MB' -f ([double] $Value / 1MB)) }
function Sec { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N2} s' -f [double] $Value) }
function Sec1 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N1} s' -f [double] $Value) }
function Pct { param([double] $Part, [double] $Whole) if ($Whole -eq 0) { return 'n/a' }; ('{0:N1}%' -f (100 * $Part / $Whole)) }
function Delta { param([object] $New, [object] $Old) if ($null -eq $New -or $null -eq $Old -or [double] $Old -eq 0) { return 'n/a' }; $d = [double] $New - [double] $Old; ('{0}{1:N1}%' -f $(if ($d -ge 0) { '+' } else { '−' }), (100 * [math]::Abs($d) / [double] $Old)) }
function Times { param([object] $New, [object] $Old) if ($null -eq $New -or $null -eq $Old -or [double] $Old -eq 0) { return 'n/a' }; ('{0:N2}×' -f ([double] $New / [double] $Old)) }
function Short { param([string] $Hash) if ([string]::IsNullOrEmpty($Hash)) { return '' }; $Hash.Substring(0, 16) + '…' }
function Mark { param([object] $Ok) if ($null -eq $Ok) { return '—' }; if ([bool] $Ok) { 'yes' } else { '**no**' } }
function Installer { param([hashtable] $Set, [string] $App, [string] $Tech) @($Set.installers[$App].installers | Where-Object { $_.name -eq "$App-$Tech.exe" }) | Select-Object -First 1 }
function BuildOf { param([object] $Build, [string] $App, [string] $Tech) @($Build.builds | Where-Object { $_.app -eq $App -and $_.technology -eq $Tech }) | Select-Object -First 1 }
function Row { param([object] $Lab, [string] $App, [string] $Tech) if ($null -eq $Lab) { return $null }; @($Lab.rows | Where-Object { $_.app -eq $App -and $_.tech -eq $Tech }) | Select-Object -First 1 }
function When { param([object] $Text) if (Missing $Text) { return 'n/a' }; ([DateTimeOffset] $Text).ToString('yyyy-MM-dd HH:mm zzz') }

$sb = [System.Text.StringBuilder]::new()
function L { param([string] $Line = '') $null = $sb.AppendLine($Line) }

$hostFacts = Get-Prop $fresh.toolchain 'host'
$ts = $fresh.toolchain.tigersetup
$inno = $fresh.toolchain.inno_setup
$nsis = $fresh.toolchain.nsis
$freshLab = $fresh.lab
$freshRows = @(if ($null -ne $freshLab) { $freshLab.rows } else { @() })
$sameInno = ([string] $inno.compiler.sha256 -eq [string] $base.toolchain.inno_setup.compiler.sha256) -and ([string] $inno.native_x64_engine.sha256 -eq [string] $base.toolchain.inno_setup.native_x64_engine.sha256)
$sameNsis = ([string] $nsis.compiler.sha256 -eq [string] $base.toolchain.nsis.compiler.sha256) -and ([string] $nsis.exehead.sha256 -eq [string] $base.toolchain.nsis.exehead.sha256)

# ---------------------------------------------------------------------------
L "# TigerSetup $freshVersion against the benchmark baseline: Inno Setup $($inno.version), NSIS $($nsis.version), TigerSetup $baseVersion"
L
L "The installer-technology benchmark ([``report.md``](../report.md)) measured"
L "**TigerSetup $baseVersion**, **Inno Setup $($base.toolchain.inno_setup.version)** and **NSIS $($base.toolchain.nsis.version)**"
L "packaging the same four open-source applications, statically on the build"
L "machine and at runtime on TigerWinLab's clean Windows 11 baseline. That"
L "report is frozen: its numbers are historical evidence and are not regenerated"
L "here. This report re-runs the TigerSetup side of the same experiment for"
L "**TigerSetup $freshVersion** — the same canonical payloads, the same package"
L "definitions, the same lab baseline, contracts and timing boundaries — and"
L "adds the build-side evidence the first campaign lacked: every one of the"
L "fifteen installer definitions rebuilt on one host under one method, with"
L "wall clock, CPU time and peak memory of the compiler's whole process tree."
L
L "Every number below is read from ``benchmark/results/*.json`` (historical)"
L "and ``benchmark/results/$Campaign/*.json`` (fresh) by"
L "``benchmark/scripts/New-ComparisonReport.ps1``; nothing is transcribed by"
L "hand. The three classes of evidence are kept apart throughout:"
L
L "| Class | What | Where measured | When |"
L "|---|---|---|---|"
L "| **A. Historical** | Inno Setup, NSIS and TigerSetup ${baseVersion}: installer bytes, build wall clock, the runtime rows | the build machine; ``$labBaseline`` | $(When $base.lab.startedAt) (lab); $(When $base.build.builtAt) (builds) |"
L "| **B. Fresh host builds** | all 15 installers rebuilt: bytes, wall, CPU, peak memory | this host (below), serial, one run each | $(When $fresh.build.builtAt) → $(When (Get-Prop $fresh.build 'finishedAt')) |"
L "| **C. Fresh runtime** | TigerSetup $freshVersion only: the four application rows | ``$(if ($null -ne $freshLab) { $freshLab.baseline } else { $labBaseline })`` | $(if ($null -ne $freshLab) { When $freshLab.startedAt } else { 'not run' }) |"
L
L "A comparison across classes is a comparison across campaigns: package"
L "bytes are exact and compare freely; build figures compare within class B"
L "(one host, one method, one afternoon); runtime figures compare across A and"
L "C because the lab, the baseline checkpoint, the contracts and the timing"
L "boundaries are the same, but they remain single observations on a VM and"
L "differences of a second or two are noise."
L
# ---------------------------------------------------------------------------
L "## Provenance"
L
L "**Product under test.** TigerSetup $freshVersion, commit ``$($ts.git_commit)``$(if ($ts.working_tree_dirty) { ' (uncommitted working tree at build time: the benchmark scripts of this campaign)' }),"
L "built with ``cargo build --release``: builder ``tiger-setup.exe``"
L "$(N $ts.builder.bytes) bytes ``$($ts.builder.sha256)``; engine"
$loaderText = if ($null -ne (Get-Prop $ts 'loader')) { '; loader `tigersetup-loader.exe` ' + (N $ts.loader.bytes) + ' bytes `' + $ts.loader.sha256 + '`' } else { '' }
L "``tigersetup-setup.exe`` $(N $ts.engine.bytes) bytes ``$($ts.engine.sha256)``$loaderText."
L "Every TigerSetup installer built here was checked with ``tiger-setup inspect``"
L "to carry exactly that engine. The historical TigerSetup $baseVersion engine was"
L "``$($base.toolchain.tigersetup.engine.sha256)`` (commit ``$($base.toolchain.tigersetup.git_commit)``); it was not rebuilt or rerun."
L
L "**Compilers.** Inno Setup $($inno.version) (``$($inno.compiler.path)``,"
L "``$($inno.compiler.sha256)``; engine ``Setup.e64`` ``$(Short $inno.native_x64_engine.sha256)``) and"
$nsisBinText = if ($null -ne (Get-Prop $nsis 'compiler_bin')) { ', the compiler it launches `Bin\makensis.exe` `' + (Short $nsis.compiler_bin.sha256) + '`' } else { '' }
L "NSIS $($nsis.version) (``$($nsis.compiler.path)`` ``$($nsis.compiler.sha256)``$nsisBinText; exehead ``$(Short $nsis.exehead.sha256)``)."
L "$(if ($sameInno -and $sameNsis) { 'Both are byte-identical to the compilers the historical campaign used.' } elseif ($sameInno) { 'Inno Setup is byte-identical to the historical campaign''s; NSIS is not.' } elseif ($sameNsis) { 'NSIS is byte-identical to the historical campaign''s; Inno Setup is not.' } else { 'Neither compiler is byte-identical to the historical campaign''s.' })"
L "Compression settings are the first campaign's: Inno Setup ``$($inno.compression)``, NSIS ``$($nsis.compression)``,"
L "TigerSetup's release build (its default compression search; never ``--fast``)."
L
if ($null -ne $hostFacts -and $hostFacts -isnot [string]) {
    $w = Get-Prop $hostFacts 'windows'; $c = Get-Prop $hostFacts 'cpu'; $d = Get-Prop $hostFacts 'repositoryDrive'
    L "**Host (class B).** $(if ($w -is [string]) { $w } else { "$($w.caption) $(Get-Prop $w 'displayVersion') (build $($w.build))" });"
    L "$(if ($c -is [string]) { $c } else { "$($c.name), $($c.cores) cores / $($c.logicalProcessors) logical processors" });"
    L "$(MiB (Get-Prop $hostFacts 'memoryBytes')) MiB RAM;"
    L "repository on $(if ($d -is [string]) { $d } else { "$($d.letter) $($d.model) ($($d.mediaType), $($d.busType), $(MiB $d.sizeBytes) MiB)" });"
    L "PowerShell $(Get-Prop $hostFacts 'powershell'). Single-run measurements on a live workstation, not an isolated"
    L "benchmark host; the OS file cache was not flushed between builds, and the"
    L "technology order rotates between applications so no technology always"
    L "builds first into a cold cache or last into a warm one."
    L
}
L "**Lab (class C).** TigerWinLab baseline ``$(if ($null -ne $freshLab) { $freshLab.baseline } else { $labBaseline })``, the checkpoint the"
L "historical rows used; one session per row, the VM restored to the checkpoint"
L "before the install job, the uninstall job on the same VM, the session closed"
L "and the VM reported ``Available`` before the next row. Every row runs as the"
L "lab's job account (an administrator with no interactive desktop), silently."
L
if ($null -ne $fresh.canonicalVerification) {
    $cv = $fresh.canonicalVerification
    L "**Canonical payloads.** Verified before anything was built"
    L "($(When $cv.verifiedAt)): $(if ([bool] $cv.allIdentical) { 'all four application payloads on disk are byte-identical to the committed inventories' } else { '**at least one payload differs from its inventory**' })"
    L "(``benchmark/results/$Campaign/canonical-verification.json``, from ``Test-CanonicalPayload.ps1``):"
    L
    L "| App | Files | Bytes | Against inventory |"
    L "|---|---:|---:|---|"
    foreach ($p in $cv.payloads) { L "| $($p.app) | $(N $p.actualFiles) | $(N $p.actualBytes) | $(if ([bool] $p.identical) { 'identical' } else { "**differs** (missing $(@($p.missing).Count), unexpected $(@($p.unexpected).Count), differing $(@($p.differing).Count))" }) |" }
    L
}
L "**Package definitions.** Unchanged from the first campaign, with one build-side"
L "exception: each NSIS script's ``OutFile`` now takes ``/DOUTFILE=`` so a campaign"
L "can build into artifacts of its own (the default path is the definition's"
L "own; the installer's bytes do not depend on where it is written)."
L
# ---------------------------------------------------------------------------
L "## 1. Package size"
L
L "Exact bytes, class A for Inno Setup, NSIS and TigerSetup $baseVersion (the"
L "historical installers) and class B for TigerSetup $freshVersion. The Inno Setup"
L "and NSIS installers were rebuilt in class B as well; the ``rebuilt`` column"
L "says whether the fresh build reproduced the historical bytes exactly."
L
L "| App | Inno Setup | NSIS | TigerSetup $baseVersion | TigerSetup $freshVersion | $freshVersion vs $baseVersion | $freshVersion vs smallest of IS/NSIS | rebuilt IS / NSIS identical |"
L "|---|---:|---:|---:|---:|---:|---:|---|"
foreach ($app in $allApps) {
    $i = Installer $base $app 'InnoSetup'; $n = Installer $base $app 'NSIS'; $t0 = Installer $base $app 'TigerSetup'; $t1 = Installer $fresh $app 'TigerSetup'
    $fi = Installer $fresh $app 'InnoSetup'; $fn = Installer $fresh $app 'NSIS'
    $smallest = [math]::Min([long] $i.bytes, [long] $n.bytes)
    $sameI = [string] $fi.sha256 -eq [string] $i.sha256; $sameN = [string] $fn.sha256 -eq [string] $n.sha256
    L "| $app | $(N $i.bytes) | $(N $n.bytes) | $(N $t0.bytes) | **$(N $t1.bytes)** | $(Delta $t1.bytes $t0.bytes) ($(N ([long] $t1.bytes - [long] $t0.bytes))) | $(Delta $t1.bytes $smallest) vs $(if ([long] $i.bytes -le [long] $n.bytes) { 'IS' } else { 'NSIS' }) | $(if ($sameI) { 'yes' } else { "no ($(N $fi.bytes))" }) / $(if ($sameN) { 'yes' } else { "no ($(N $fn.bytes))" }) |"
}
L
L "``Minimal`` is the fixed cost of a technology before any payload: its"
L "TigerSetup installer carries one 38-byte file. The installer-to-payload"
L "ratio, for the four applications:"
L
L "| App | Canonical bytes | Inno Setup | NSIS | TigerSetup $baseVersion | TigerSetup $freshVersion |"
L "|---|---:|---:|---:|---:|---:|"
foreach ($app in $appNames) {
    $cb = [long] $canonical[$app].totalBytes
    L "| $app | $(N $cb) | $(Pct (Installer $base $app 'InnoSetup').bytes $cb) | $(Pct (Installer $base $app 'NSIS').bytes $cb) | $(Pct (Installer $base $app 'TigerSetup').bytes $cb) | $(Pct (Installer $fresh $app 'TigerSetup').bytes $cb) |"
}
L
# ---------------------------------------------------------------------------
L "## 2. Fresh host build statistics (class B)"
L
L "All fifteen builds on the host above, serial, one run each, every compiler"
L "under the same meter (``benchmark/scripts/ProcessTreeMeter.psm1``): the"
L "compiler starts suspended inside a job object of its own and is resumed, so"
L "every process it creates is measured with it (NSIS's ``makensis.exe`` is a"
L "launcher for ``Bin\makensis.exe``). **Wall** runs from the resume until the job"
L "holds no process. **CPU** is the job's kernel-maintained accounting (user +"
L "kernel time of every process that ever belonged to it). **Peak commit** is"
L "the job's ``PeakJobMemoryUsed`` — the highest commit charge of all its"
L "processes together at any instant — also kernel-maintained and exact."
L "**Peak working set** is sampled every $($fresh.build.builds[0].sampleIntervalMilliseconds) ms: the largest sum of the"
L "processes' working sets seen in one sample (the tree) and the largest"
L "Windows-tracked per-process peak read while that process was alive (the"
L "largest single process). Windows keeps no job-wide working-set peak, so the"
L "sampled figures can miss a spike shorter than the interval or a process that"
L "lives and dies between two samples; the sample count and the processes seen"
L "against the job's total say how much a build was covered — a build of a few"
L "tens of milliseconds is covered by its commit figures, not by its samples."
L
L "| App | Technology | Installer bytes | Build wall | CPU | CPU / wall | Peak tree WS | Largest process WS | Peak commit (job) | Samples / processes |"
L "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|"
foreach ($app in $allApps) {
    foreach ($tech in $technologies) {
        $b = BuildOf $fresh.build $app $tech
        if ($null -eq $b) { L "| $app | $($techLabel[$tech]) | **not built** | | | | | | | |"; continue }
        $cpuRatio = if ([double] $b.wallSeconds -gt 0) { '{0:N2}' -f ([double] $b.cpuSeconds / [double] $b.wallSeconds) } else { 'n/a' }
        L "| $app | $($techLabel[$tech]) | $(N $b.installerBytes) | $(Sec $b.wallSeconds) | $(Sec $b.cpuSeconds) | $cpuRatio | $(MB2 $b.peakTreeWorkingSetBytes) | $(MB2 $b.peakProcessWorkingSetBytes) | $(MB2 $b.peakJobCommitBytes) | $($b.samples) / $($b.processesSeen) of $($b.processesTotal) |"
    }
}
L
L "The historical campaign recorded a wall clock around each compiler"
L "invocation (class A, $(When $base.build.builtAt), the same host a day earlier, no"
L "memory or CPU figures; ``not measured`` where it did not). Shown beside the"
L "fresh wall clock for orientation only — the two are different campaigns, not"
L "repeated samples of one:"
L
L "| App | Technology | Wall, historical campaign | Wall, this campaign | Peak memory, historical |"
L "|---|---|---:|---:|---|"
foreach ($app in $allApps) {
    foreach ($tech in $technologies) {
        $b0 = BuildOf $base.build $app $tech; $b1 = BuildOf $fresh.build $app $tech
        $label = if ($tech -eq 'TigerSetup') { "TigerSetup ($baseVersion → $freshVersion)" } else { $techLabel[$tech] }
        L "| $app | $label | $(Sec1 (Get-Prop $b0 'buildSeconds')) | $(Sec1 (Get-Prop $b1 'wallSeconds')) | not measured |"
    }
}
L
# ---------------------------------------------------------------------------
L "## 3. Runtime: install, uninstall, contract, payload"
L
L "Class A rows (Inno Setup, NSIS, TigerSetup $baseVersion) and class C rows"
L "(TigerSetup $freshVersion) side by side. Timing boundaries are the first"
L "campaign's: install is the installer process's lifetime; uninstall is the"
L "uninstaller's lifetime plus the exit of whatever it hands off to (NSIS's"
L "``Au_.exe``) and the removal of the install root, the completion wait shown"
L "as ``(+wait)``. A TigerSetup $freshVersion installer is a small loader that"
L "extracts its engine and **waits for it**, so the installer process's lifetime"
L "already spans the whole operation; the $freshVersion rows additionally waited for"
L "any process of the installer's own name (the engine keeps the package's"
L "file name) and record that wait separately, so a hand-off could not have"
L "escaped the clock. ``Contract`` is the number of install-time contract"
L "probes that held, of those the application's contract names; ``Payload"
L "exact`` is the installed payload against the canonical one: for the class C"
L "rows every file's SHA-256 (the guest reader returned per-file hashes;"
L "``extras`` are what the technology added under the install root), for the"
L "class A rows the file count and byte total the first campaign recorded."
L
L "| App | Technology | Class | Row | Install | Uninstall (+wait) | Contract | Payload exact | Extras under root | ARP | Start Menu link |"
L "|---|---|---|---|---:|---:|---:|---|---:|---|---|"
foreach ($app in $appNames) {
    $rowsToShow = @(foreach ($tech in $technologies) { @{ tech = $tech; class = 'A'; row = (Row $base.lab $app $tech); version = $(if ($tech -eq 'TigerSetup') { $baseVersion } else { '' }) } }) + @(@{ tech = 'TigerSetup'; class = 'C'; row = (Row $freshLab $app 'TigerSetup'); version = $freshVersion })
    foreach ($entry in $rowsToShow) {
        $r = $entry.row
        $label = $techLabel[$entry.tech] + $(if ($entry.version) { " $($entry.version)" } else { '' })
        if ($null -eq $r) { L "| $app | $label | $($entry.class) | **not run** | | | | | | | |"; continue }
        $i = $r.install; $u = $r.uninstall
        $status = if ([bool] $r.success) { 'ok' } else { '**FAILED**' }
        $probesHeld = @($i.contract | Where-Object { [bool] $_.present }).Count; $probes = @($i.contract).Count
        $exact = if ($null -ne (Get-Prop $i 'payloadVerified') -and [bool] $i.payloadVerified) { if ([bool] $i.payloadExact) { "yes (SHA-256, $(N $i.payloadMatched) files)" } else { "**no** (missing $(@($i.payloadMissing).Count), differing $(@($i.payloadDiffering).Count))" } } else { "$(if ([long] $i.installedBytes - [long] $i.extraBytes -eq [long] $i.canonicalBytes -and [int] $i.installedFiles - [int] $i.extraFiles -eq [int] $i.canonicalFiles) { 'by count and bytes' } else { '**count/bytes differ**' })" }
        $extras = if ($null -ne (Get-Prop $i 'payloadExtras')) { "$(@($i.payloadExtras).Count) ($(N $i.extraBytes) B)" } else { "$(N $i.extraFiles) ($(N $i.extraBytes) B)" }
        L "| $app | $label | $($entry.class) | $status | $(Sec $i.durationSeconds) | $(Sec $u.durationSeconds) (+$(Sec (Get-Prop $u 'completionWaitSeconds'))) | $probesHeld / $probes | $exact | $extras | $(Mark $i.arpPresent) → $(Mark $u.arpRemoved) | $(Mark $i.startMenuLinkOk) → $(Mark $u.startMenuLinkRemoved) |"
    }
}
L
L "Read ``ARP`` and ``Start Menu link`` as *present after install → removed after"
L "uninstall*. ``Row`` is the harness's verdict on the whole row: both jobs ran,"
L "both processes exited 0, the completion wait was satisfied, every"
L "install-time probe held, the payload was exact where it could be verified,"
L "and every app-owned resource was gone after the uninstall."
L
if ($freshRows.Count -gt 0) {
    L "**Contract probes after uninstall, TigerSetup $freshVersion** (a resource the"
    L "application owns must be gone; a *setting* is reported as observed; an"
    L "*absent* probe must still hold):"
    L
    L "| App | Probe | After install | After uninstall |"
    L "|---|---|---|---|"
    foreach ($app in $appNames) {
        $r = Row $freshLab $app 'TigerSetup'
        if ($null -eq $r) { continue }
        foreach ($pi in @($r.install.contract)) {
            $pu = @($r.uninstall.contract | Where-Object { $_.feature -eq $pi.feature }) | Select-Object -First 1
            $after = switch ($pi.kind) {
                'setting' { "observed: $(($pu.detail -split '=')[-1])" }
                { $_ -like 'absent-*' } { if ([bool] $pu.present) { 'still absent' } else { '**appeared**' } }
                default { if ([bool] $pu.present) { '**LEFT BEHIND**' } else { 'gone' } }
            }
            L "| $app | $($pi.feature) | $(if ([bool] $pi.present) { 'ok' } else { '**MISSING**' }) — $($pi.mechanism) | $after |"
        }
    }
    L
}
L "**Totals over the four applications** (single measurements summed; a coarse"
L "view, not a second measurement):"
L
L "| Technology | Class | Install, all four | Uninstall, all four |"
L "|---|---|---:|---:|"
$totals = @{}
foreach ($entry in @(@{ tech = 'InnoSetup'; lab = $base.lab; class = 'A'; label = 'Inno Setup' }, @{ tech = 'NSIS'; lab = $base.lab; class = 'A'; label = 'NSIS' }, @{ tech = 'TigerSetup'; lab = $base.lab; class = 'A'; label = "TigerSetup $baseVersion" }, @{ tech = 'TigerSetup'; lab = $freshLab; class = 'C'; label = "TigerSetup $freshVersion" })) {
    $sumI = 0.0; $sumU = 0.0; $n = 0
    foreach ($app in $appNames) { $r = Row $entry.lab $app $entry.tech; if ($null -ne $r -and $null -ne $r.install.durationSeconds -and $null -ne $r.uninstall.durationSeconds) { $sumI += [double] $r.install.durationSeconds; $sumU += [double] $r.uninstall.durationSeconds; $n++ } }
    $totals[$entry.label] = @{ i = $sumI; u = $sumU; n = $n }
    L "| $($entry.label) | $($entry.class) | $(if ($n -eq 4) { Sec $sumI } else { "n/a ($n of 4 rows)" }) | $(if ($n -eq 4) { Sec $sumU } else { "n/a ($n of 4 rows)" }) |"
}
L
# ---------------------------------------------------------------------------
L "## 4. TigerSetup evolution: $baseVersion → $freshVersion"
L
L "The same packages, the same payloads, the same lab rows; class A against"
L "class C for runtime, exact bytes for size."
L
L "| App | Installer bytes | Install | Uninstall (+wait) | Payload exact / extras under root | Where the bookkeeping is |"
L "|---|---:|---:|---:|---|---|"
foreach ($app in $appNames) {
    $r0 = Row $base.lab $app 'TigerSetup'; $r1 = Row $freshLab $app 'TigerSetup'
    $b0 = [long] (Installer $base $app 'TigerSetup').bytes; $b1 = [long] (Installer $fresh $app 'TigerSetup').bytes
    $cells = @("$(N $b0) → $(N $b1) ($(Delta $b1 $b0))")
    if ($null -ne $r0 -and $null -ne $r1) {
        $cells += "$(Sec $r0.install.durationSeconds) → $(Sec $r1.install.durationSeconds) ($(Times $r1.install.durationSeconds $r0.install.durationSeconds))"
        $cells += "$(Sec $r0.uninstall.durationSeconds) (+$(Sec (Get-Prop $r0.uninstall 'completionWaitSeconds'))) → $(Sec $r1.uninstall.durationSeconds) (+$(Sec (Get-Prop $r1.uninstall 'completionWaitSeconds'))) ($(Times $r1.uninstall.durationSeconds $r0.uninstall.durationSeconds))"
        $cells += "${baseVersion}: by count/bytes, $(N $r0.install.extraFiles) extra; ${freshVersion}: $(if ([bool] (Get-Prop $r1.install 'payloadExact')) { 'SHA-256 exact' } else { '**not exact**' }), $(@(Get-Prop $r1.install 'payloadExtras').Count) extra"
        $cells += 'state database and uninstaller in the scope''s TigerSetup state directory (both versions; not under the root, so not in `extras`)'
    }
    else { $cells += 'not run', 'not run', 'not run', '' }
    L "| $app | $($cells -join ' | ') |"
}
L
if ($freshRows.Count -gt 0) {
    L "**Where the $freshVersion install time goes.** Each $freshVersion row also brought back the"
    L "engine's own log and the guest's clock at the process's start and end,"
    L "so the loader's share of the lifetime is measured rather than assumed:"
    L "*before* is from the process start to the engine's first event"
    L "(``run_started``) — the loader locating the footer, decompressing and"
    L "verifying the engine, starting it, and the engine opening its log;"
    L "*engine* is the span from that event to ``run_finished``; *after* is the"
    L "engine's exit and the loader's clean-up of its extraction directory."
    L
    L "| App | Step | Process lifetime | Before engine | Engine span | After engine | Loader share |"
    L "|---|---|---:|---:|---:|---:|---:|"
    foreach ($app in $appNames) {
        $r = Row $freshLab $app 'TigerSetup'
        if ($null -eq $r) { continue }
        foreach ($step in @('install', 'uninstall')) {
            $e = Get-Prop $r.$step 'engine'
            if ($null -eq $e) { L "| $app | $step | $(Sec $r.$step.processSeconds) | n/a | n/a | n/a | n/a |"; continue }
            $overhead = $null
            if ($null -ne $e.beforeSeconds -and $null -ne $e.afterSeconds) { $overhead = [double] $e.beforeSeconds + [double] $e.afterSeconds }
            L "| $app | $step | $(Sec $r.$step.processSeconds) | $(Sec $e.beforeSeconds) | $(Sec $e.spanSeconds) | $(Sec $e.afterSeconds) | $(if ($null -ne $overhead) { Pct $overhead ([double] $r.$step.processSeconds) } else { 'n/a' }) |"
        }
    }
    L
    $beforeByApp = @(foreach ($app in $appNames) { $r = Row $freshLab $app 'TigerSetup'; $e = Get-Prop $r.install 'engine'; if ($null -ne $e -and $null -ne $e.beforeSeconds) { [pscustomobject]@{ app = $app; bytes = [long] (Installer $fresh $app 'TigerSetup').bytes; before = [double] $e.beforeSeconds; uninstallBefore = [double] (Get-Prop (Get-Prop $r.uninstall 'engine') 'beforeSeconds') } } })
    $beforeByApp = @($beforeByApp | Sort-Object bytes)
    if ($beforeByApp.Count -gt 0) {
        L "*Before engine* on install is not the loader's decompression alone. It"
        L "grows with the installer's size — $(($beforeByApp | ForEach-Object { "$($_.app) $(MiB $_.bytes) MiB: $(Sec $_.before)" }) -join '; ') —"
        L "while what the loader reads is the same engine block every time (the whole ``Minimal`` installer is $(MiB (Installer $fresh 'Minimal' 'TigerSetup').bytes) MiB) and"
        L "what the engine does before its first event is to open the package and"
        L "check the metadata block's hash, kilobytes. The uninstall, the same file"
        L "on the same VM a minute later, spends $(Sec (($beforeByApp | Measure-Object -Property uninstallBefore -Minimum).Minimum))–$(Sec (($beforeByApp | Measure-Object -Property uninstallBefore -Maximum).Maximum)) there. A cost that"
        L "scales with the file's bytes on its first launch and largely disappears on"
        L "its second is consistent with the platform's handling of a never-run"
        L "executable of that size on a clean Windows with real-time protection on;"
        L "this benchmark does not isolate it, and an Inno Setup or NSIS installer of"
        L "the same size goes through the same first launch inside its own install"
        L "time. What is TigerSetup's own in the column is the loader's work and the"
        L "engine's start-up, at most the uninstall's figure."
        L
    }
}
L "**Fixed overhead.** A TigerSetup $baseVersion installer began with its"
L "$(N $base.toolchain.tigersetup.engine.bytes)-byte engine stored uncompressed"
L "(``Minimal``: $(N (Installer $base 'Minimal' 'TigerSetup').bytes) bytes); a $freshVersion installer begins with"
L "a $(if ($null -ne (Get-Prop $ts 'loader')) { "$(N $ts.loader.bytes)-byte C loader and" } else { 'a loader and' }) the engine as one zstd block"
L "(``Minimal``: $(N (Installer $fresh 'Minimal' 'TigerSetup').bytes) bytes). Against the smallest application payload"
L "here (WinMerge, $(N $canonical['WinMerge'].totalBytes) bytes) that fixed cost is"
L "$(Pct (Installer $fresh 'Minimal' 'TigerSetup').bytes (Installer $fresh 'WinMerge' 'TigerSetup').bytes) of the $freshVersion installer, where it was"
L "$(Pct (Installer $base 'Minimal' 'TigerSetup').bytes (Installer $base 'WinMerge' 'TigerSetup').bytes) of the $baseVersion one; Inno Setup's fixed cost is"
L "$(N (Installer $base 'Minimal' 'InnoSetup').bytes) bytes and NSIS's $(N (Installer $base 'Minimal' 'NSIS').bytes)."
L
# ---------------------------------------------------------------------------
L "## 5. Build-resource trade-off (class B)"
L
L "What TigerSetup $freshVersion's ``zstd-19-w27`` solid payload costs at build time"
L "against Inno Setup's ``lzma2/max`` and NSIS's ``/SOLID lzma``, on one host"
L "under one method, beside what it produces. Ratios are TigerSetup over the"
L "named technology; a ratio above 1 means TigerSetup used more."
L
L "| App | Installer bytes: TS / IS / NSIS | Build wall: TS / IS / NSIS | TS wall vs IS / NSIS | CPU: TS / IS / NSIS | Peak commit: TS / IS / NSIS | TS commit vs IS / NSIS | Peak tree WS: TS / IS / NSIS |"
L "|---|---|---|---|---|---|---|---|"
foreach ($app in $allApps) {
    $t = BuildOf $fresh.build $app 'TigerSetup'; $i = BuildOf $fresh.build $app 'InnoSetup'; $n = BuildOf $fresh.build $app 'NSIS'
    if ($null -eq $t -or $null -eq $i -or $null -eq $n) { L "| $app | not built | | | | | | |"; continue }
    L "| $app | $(N $t.installerBytes) / $(N $i.installerBytes) / $(N $n.installerBytes) | $(Sec1 $t.wallSeconds) / $(Sec1 $i.wallSeconds) / $(Sec1 $n.wallSeconds) | $(Times $t.wallSeconds $i.wallSeconds) / $(Times $t.wallSeconds $n.wallSeconds) | $(Sec1 $t.cpuSeconds) / $(Sec1 $i.cpuSeconds) / $(Sec1 $n.cpuSeconds) | $(MB2 $t.peakJobCommitBytes) / $(MB2 $i.peakJobCommitBytes) / $(MB2 $n.peakJobCommitBytes) | $(Times $t.peakJobCommitBytes $i.peakJobCommitBytes) / $(Times $t.peakJobCommitBytes $n.peakJobCommitBytes) | $(MB2 $t.peakTreeWorkingSetBytes) / $(MB2 $i.peakTreeWorkingSetBytes) / $(MB2 $n.peakTreeWorkingSetBytes) |"
}
L
$sumWall = @{}; $sumCpu = @{}; $maxCommit = @{}
foreach ($tech in $technologies) {
    $sumWall[$tech] = [double] (@($appNames | ForEach-Object { (BuildOf $fresh.build $_ $tech).wallSeconds }) | Measure-Object -Sum).Sum
    $sumCpu[$tech] = [double] (@($appNames | ForEach-Object { (BuildOf $fresh.build $_ $tech).cpuSeconds }) | Measure-Object -Sum).Sum
    $maxCommit[$tech] = [long] (@($appNames | ForEach-Object { (BuildOf $fresh.build $_ $tech).peakJobCommitBytes }) | Measure-Object -Maximum).Maximum
}
L "Over the four applications together: build wall TigerSetup $(Sec1 $sumWall.TigerSetup), Inno Setup"
L "$(Sec1 $sumWall.InnoSetup), NSIS $(Sec1 $sumWall.NSIS) ($(Times $sumWall.TigerSetup $sumWall.InnoSetup) Inno Setup's, $(Times $sumWall.TigerSetup $sumWall.NSIS) NSIS's); CPU"
L "$(Sec1 $sumCpu.TigerSetup) / $(Sec1 $sumCpu.InnoSetup) / $(Sec1 $sumCpu.NSIS); the largest peak commit of any build $(MB2 $maxCommit.TigerSetup) /"
L "$(MB2 $maxCommit.InnoSetup) / $(MB2 $maxCommit.NSIS). The CPU-to-wall ratio in §2 says how each compiler"
L "uses the host's cores: a ratio near 1 is one thread compressing; above it,"
L "parallel compression."
L
# ---------------------------------------------------------------------------
L "## What the measurements answer"
L
$tsVsSmall = @(foreach ($app in $appNames) { $i = [long] (Installer $base $app 'InnoSetup').bytes; $n = [long] (Installer $base $app 'NSIS').bytes; $t = [long] (Installer $fresh $app 'TigerSetup').bytes; [pscustomobject]@{ app = $app; d = (100.0 * ($t - [math]::Min($i, $n)) / [math]::Min($i, $n)) } })
$shrink = @(foreach ($app in $appNames) { $t0 = [long] (Installer $base $app 'TigerSetup').bytes; $t1 = [long] (Installer $fresh $app 'TigerSetup').bytes; [pscustomobject]@{ app = $app; d = (100.0 * ($t1 - $t0) / $t0) } })
function Range { param([object[]] $Values, [string] $Unit = '%') $min = ($Values | Measure-Object -Minimum).Minimum; $max = ($Values | Measure-Object -Maximum).Maximum; "$('{0:N1}' -f $min)$Unit to $('{0:N1}' -f $max)$Unit" }
L "- **How much larger or smaller is TigerSetup $freshVersion than Inno Setup and NSIS?**"
L "  Against the smaller of the two on each application: $(($tsVsSmall | ForEach-Object { "$($_.app) $(if ($_.d -ge 0) { '+' })$('{0:N1}' -f $_.d)%" }) -join ', ')."
L "  The remaining difference is the fixed loader-plus-engine block"
L "  ($(N (Installer $fresh 'Minimal' 'TigerSetup').bytes) bytes against $(N (Installer $base 'Minimal' 'NSIS').bytes) for NSIS) and the codec: Zstandard"
L "  level 19 with a 128 MiB window against LZMA/LZMA2, a trade the product"
L "  made for decode speed inside the installation transaction"
L "  (``TigerSetup-Design.md`` §10.4, ``benchmark/compression-spike/report.md``)."
L "- **How much did TigerSetup shrink from ${baseVersion}?** $(($shrink | ForEach-Object { "$($_.app) $('{0:N1}' -f $_.d)%" }) -join ', '); ``Minimal``"
L "  $(Delta (Installer $fresh 'Minimal' 'TigerSetup').bytes (Installer $base 'Minimal' 'TigerSetup').bytes). Exact bytes, not an estimate."
$wallRatioI = @($appNames | ForEach-Object { [double] (BuildOf $fresh.build $_ 'TigerSetup').wallSeconds / [double] (BuildOf $fresh.build $_ 'InnoSetup').wallSeconds })
$wallRatioN = @($appNames | ForEach-Object { [double] (BuildOf $fresh.build $_ 'TigerSetup').wallSeconds / [double] (BuildOf $fresh.build $_ 'NSIS').wallSeconds })
L "- **How much longer does TigerSetup take to build?** On this host, per"
L "  application, $(Range $wallRatioI '×') Inno Setup's wall clock and $(Range $wallRatioN '×') NSIS's;"
L "  over the four applications $(Times $sumWall.TigerSetup $sumWall.InnoSetup) and $(Times $sumWall.TigerSetup $sumWall.NSIS). In CPU time the"
L "  picture is $(Times $sumCpu.TigerSetup $sumCpu.InnoSetup) Inno Setup's and $(Times $sumCpu.TigerSetup $sumCpu.NSIS) NSIS's: TigerSetup's"
L "  compressor is single-threaded, Inno Setup's LZMA2 is not (§2, CPU / wall)."
$commitI = @($appNames | ForEach-Object { [double] (BuildOf $fresh.build $_ 'TigerSetup').peakJobCommitBytes / [double] (BuildOf $fresh.build $_ 'InnoSetup').peakJobCommitBytes })
$commitN = @($appNames | ForEach-Object { [double] (BuildOf $fresh.build $_ 'TigerSetup').peakJobCommitBytes / [double] (BuildOf $fresh.build $_ 'NSIS').peakJobCommitBytes })
L "- **How much peak build memory does it consume?** Peak commit of the whole"
L "  build tree: $(($appNames | ForEach-Object { "$_ $(MB2 (BuildOf $fresh.build $_ 'TigerSetup').peakJobCommitBytes)" }) -join ', ') — $(Range $commitI '×')"
L "  Inno Setup's and $(Range $commitN '×') NSIS's on the same application (§5). Peak working"
L "  set, sampled, is in §2 beside it."
if ($totals.ContainsKey("TigerSetup $freshVersion") -and $totals["TigerSetup $freshVersion"].n -eq 4) {
    $t1 = $totals["TigerSetup $freshVersion"]; $t0 = $totals["TigerSetup $baseVersion"]; $ti = $totals['Inno Setup']; $tn = $totals['NSIS']
    L "- **How does $freshVersion install time compare with $baseVersion, Inno Setup and NSIS?**"
    L "  Over the four applications: $(Sec $t1.i) against $(Sec $t0.i) for $baseVersion ($(Times $t1.i $t0.i)),"
    L "  $(Sec $ti.i) for Inno Setup ($(Times $t1.i $ti.i)) and $(Sec $tn.i) for NSIS ($(Times $t1.i $tn.i)); per application in §3."
    L "- **How does $freshVersion uninstall time compare?** $(Sec $t1.u) against $(Sec $t0.u) for"
    L "  $baseVersion ($(Times $t1.u $t0.u)), $(Sec $ti.u) for Inno Setup ($(Times $t1.u $ti.u)) and $(Sec $tn.u) for NSIS ($(Times $t1.u $tn.u))."
    L "- **Did the transaction optimizations materially change runtime behaviour?**"
    L "  §4 is the answer per application: the install and uninstall ratios"
    L "  $baseVersion → $freshVersion, with the payload verified file by file and the same"
    L "  contract probes holding, and the split of each $freshVersion lifetime into loader"
    L "  and engine. What changed between the versions is recorded in"
    L "  ``benchmark/README.md`` (the local A/B) and ``TigerSetup-Design.md`` §5.4,"
    L "  §5.10; this report measures the outcome on the clean VM."
    L "- **Does the fixed loader/engine overhead remain significant on small"
    L "  packages?** In bytes, §4 (*Fixed overhead*); in time, the *before engine*"
    L "  and *after engine* columns of §4 on the smallest package here (WinMerge)."
}
else {
    L "- The runtime questions (install and uninstall time against $baseVersion, Inno Setup"
    L "  and NSIS; the transaction optimizations' effect; the loader/engine"
    L "  overhead in time) need the four class C rows, which this results set does"
    L "  not hold."
}
L "- **Which costs occur at build/distribution time versus during the"
L "  installation transaction?** Build time, CPU and peak memory (§2, §5) are"
L "  paid once per release on the build machine; installer bytes (§1) are paid"
L "  per download; install and uninstall time (§3, §4) are paid per machine,"
L "  inside the transaction. The benchmark measures each where it occurs and"
L "  does not weigh one against another: which trade is right depends on how"
L "  many machines install a release and how often it is built, which this"
L "  experiment does not measure. Nor does it measure user preference."
L
# ---------------------------------------------------------------------------
if ($null -ne $fresh.notes) {
    $notes = $fresh.notes
    $anomalies = @(Get-Prop $notes 'anomalies'); $limitations = @(Get-Prop $notes 'limitations')
    L "## Anomalies, repeated rows and limitations of this campaign"
    L
    if ($anomalies.Count -gt 0) { foreach ($a in $anomalies) { L "- **$($a.subject)** — $($a.text)" } } else { L "- No build or row was repeated; every figure above is its first and only measurement." }
    foreach ($l in $limitations) { L "- $l" }
    L
}
L "## Limitations that apply to every number here"
L
L "- Class B is a single run of each build on a live workstation, in one"
L "  sitting, with the OS cache warm or cold as the rotation left it; a"
L "  difference of ten per cent between two builds of a minute is within what a"
L "  second sitting could move. The ratios between technologies on one"
L "  application are the robust reading; the absolute seconds are this host's."
L "- Peak working set is sampled and can under-read a short build; peak commit"
L "  is exact. Where the two disagree on a build of milliseconds, the commit"
L "  figure is the one to trust."
L "- Class C is one measurement per row, as class A was; the boot-per-row"
L "  cost of the lab makes repeats materially more expensive without changing"
L "  what a second or two could show. The class A rows were measured on"
L "  $(When $base.lab.startedAt) and the class C rows on $(if ($null -ne $freshLab) { When $freshLab.startedAt } else { 'n/a' }): the same lab,"
L "  baseline checkpoint and driver, a day apart, on a host that was not idle"
L "  either time."
L "- Inno Setup and NSIS were not rerun in the lab; their runtime rows are the"
L "  first campaign's, and nothing here says whether they would repeat to the"
L "  second."
L "- Four applications are evidence about these four applications."
L

[System.IO.File]::WriteAllText($OutputPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote $OutputPath"
