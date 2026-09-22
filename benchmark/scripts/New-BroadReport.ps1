#Requires -Version 7.0
<#
    .SYNOPSIS
    Generates the broad-corpus benchmark report — benchmark/report-<Campaign>.md
    — and its CSV companions from the campaign's result files under
    benchmark/results/<Campaign>/. Nothing in the report is typed by hand;
    regenerate it whenever a result file changes.

    .DESCRIPTION
    Reads corpus.json (the fifteen payloads and their shape), toolchain.json
    (the host and the compilers), build.json (every build's wall, CPU, peak
    memory, bytes and hash), determinism.json (repeated builds compared,
    when present), lab-results.json (the runtime rows, when present) and
    campaign-notes.json (narrative: a repeated build and why, a limitation
    the numbers do not show; when present), and writes the views the
    campaign was asked for: corpus, package size, build statistics, runtime,
    the distributions and ratios across the corpus, workload-shape
    relationships, fixed overhead, outliers, provenance and limitations.

    Quartiles are by linear interpolation between order statistics (R type
    7, Excel PERCENTILE.INC); the median is the 50% point of the same
    method. Correlations are Pearson's r over the fifteen applications and
    are reported as what they are: a description of this corpus, not a
    causal claim.

    Beside the report, build.csv, sizes.csv and runtime.csv carry the same
    per-row figures for external analysis. The output is deterministic for
    a given set of result files.

    .EXAMPLE
    pwsh -File benchmark\scripts\New-BroadReport.ps1
    pwsh -File benchmark\scripts\New-BroadReport.ps1 -Campaign 0.10.0-broad
#>
[CmdletBinding()]
param(
    [string] $Campaign = '0.10.0-broad',
    [string] $OutputPath,
    # Where the result files are; defaults to benchmark/results/<Campaign>.
    [string] $ResultsRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) { $ResultsRoot = Join-Path $benchmarkRoot "results\$Campaign" }
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
if ([string]::IsNullOrWhiteSpace($OutputPath)) { $OutputPath = Join-Path $benchmarkRoot "report-$Campaign.md" }
if (-not (Test-Path -LiteralPath $ResultsRoot -PathType Container)) { throw "No campaign results under '$ResultsRoot'." }

function Read-Json { param([string] $Name) Get-Content -LiteralPath (Join-Path $ResultsRoot $Name) -Raw | ConvertFrom-Json }
function Read-JsonIf { param([string] $Name) $p = Join-Path $ResultsRoot $Name; if (Test-Path -LiteralPath $p -PathType Leaf) { Get-Content -LiteralPath $p -Raw | ConvertFrom-Json } else { $null } }
function Get-Prop { param([object] $Object, [string] $Name) if ($null -eq $Object -or $Object.PSObject.Properties.Match($Name).Count -eq 0) { return $null }; $Object.$Name }

$corpus = Read-Json 'corpus.json'
$toolchain = Read-Json 'toolchain.json'
$build = Read-Json 'build.json'
$determinism = Read-JsonIf 'determinism.json'
$lab = Read-JsonIf 'lab-results.json'
$notes = Read-JsonIf 'campaign-notes.json'

$technologies = @('InnoSetup', 'NSIS', 'TigerSetup')
$techLabel = @{ InnoSetup = 'Inno Setup'; NSIS = 'NSIS'; TigerSetup = 'TigerSetup' }
$apps = @($corpus.apps | ForEach-Object { [string] $_.app })
$corpusApp = @{}
foreach ($app in @($corpus.apps)) { $corpusApp[[string] $app.app] = $app }
$tsVersion = [string] $toolchain.tigersetup.version
$inno = $toolchain.inno_setup
$nsis = $toolchain.nsis
$hostFacts = Get-Prop $toolchain 'host'
$labRows = @(if ($null -ne $lab) { $lab.rows } else { @() })

# --- formatting -------------------------------------------------------------
function Missing { param([object] $Value) ($null -eq $Value) -or ($Value -is [string] -and [string]::IsNullOrWhiteSpace($Value)) }
function N { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N0}' -f [double] $Value) }
function MB1 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N1} MB' -f ([double] $Value / 1MB)) }
function MB0 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N0} MB' -f ([double] $Value / 1MB)) }
function KB0 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N0} KB' -f ([double] $Value / 1KB)) }
function Sec { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N2} s' -f [double] $Value) }
function Sec1 { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N1} s' -f [double] $Value) }
function Ratio { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N2}' -f [double] $Value) }
function Times { param([object] $Value) if (Missing $Value) { return 'n/a' }; ('{0:N2}×' -f [double] $Value) }
function Pct { param([object] $Value) if (Missing $Value) { return 'n/a' }; $d = [double] $Value; ('{0}{1:N1}%' -f $(if ($d -ge 0) { '+' } else { '−' }), [math]::Abs($d)) }
function Pct1 { param([object] $Part, [object] $Whole) if ((Missing $Part) -or (Missing $Whole) -or [double] $Whole -eq 0) { return 'n/a' }; ('{0:N1}%' -f (100 * [double] $Part / [double] $Whole)) }
function Short { param([string] $Hash) if ([string]::IsNullOrEmpty($Hash)) { return '' }; $Hash.Substring(0, 16) + '…' }
function Mark { param([object] $Ok) if ($null -eq $Ok) { return '—' }; if ([bool] $Ok) { 'yes' } else { '**no**' } }
function When { param([object] $Text) if (Missing $Text) { return 'n/a' }; ([DateTimeOffset] $Text).ToString('yyyy-MM-dd HH:mm zzz') }
# ConvertFrom-Json turns an ISO timestamp into a [DateTime] (Kind Utc); the CSVs carry it back as ISO 8601 UTC.
function Iso { param([object] $Value) if (Missing $Value) { return '' }; if ($Value -is [DateTime]) { $Value.ToUniversalTime().ToString('o') } else { ([DateTimeOffset] $Value).ToUniversalTime().ToString('o') } }
function DeltaPct { param([object] $New, [object] $Old) if ((Missing $New) -or (Missing $Old) -or [double] $Old -eq 0) { return $null }; 100 * ([double] $New - [double] $Old) / [double] $Old }
function Div { param([object] $A, [object] $B) if ((Missing $A) -or (Missing $B) -or [double] $B -eq 0) { return $null }; [double] $A / [double] $B }

# --- statistics -------------------------------------------------------------
function Quantile {
    <# Linear interpolation between order statistics (R type 7). #>
    param([double[]] $Values, [double] $P)
    $sorted = @($Values | Sort-Object)
    $n = $sorted.Count
    if ($n -eq 0) { return $null }
    if ($n -eq 1) { return $sorted[0] }
    $h = ($n - 1) * $P
    $lo = [math]::Floor($h)
    $hi = [math]::Ceiling($h)
    $sorted[$lo] + ($h - $lo) * ($sorted[$hi] - $sorted[$lo])
}
function Stats {
    param([object[]] $Values)
    $v = @($Values | Where-Object { -not (Missing $_) } | ForEach-Object { [double] $_ })
    if ($v.Count -eq 0) { return [ordered]@{ count = 0; min = $null; q1 = $null; median = $null; q3 = $null; max = $null; sum = $null; mean = $null } }
    [ordered]@{
        count = $v.Count
        min = (Quantile $v 0)
        q1 = (Quantile $v 0.25)
        median = (Quantile $v 0.5)
        q3 = (Quantile $v 0.75)
        max = (Quantile $v 1)
        sum = (($v | Measure-Object -Sum).Sum)
        mean = (($v | Measure-Object -Average).Average)
    }
}
function Pearson {
    param([double[]] $X, [double[]] $Y)
    $n = [math]::Min($X.Count, $Y.Count)
    if ($n -lt 3) { return $null }
    $mx = ($X | Measure-Object -Average).Average
    $my = ($Y | Measure-Object -Average).Average
    $sxy = 0.0; $sxx = 0.0; $syy = 0.0
    for ($i = 0; $i -lt $n; $i++) { $dx = $X[$i] - $mx; $dy = $Y[$i] - $my; $sxy += $dx * $dy; $sxx += $dx * $dx; $syy += $dy * $dy }
    if ($sxx -eq 0 -or $syy -eq 0) { return $null }
    $sxy / [math]::Sqrt($sxx * $syy)
}
function StatLine {
    <# median (Q1–Q3), min–max, for a formatter. #>
    param([object] $S, [scriptblock] $Format)
    if ($S.count -eq 0) { return 'n/a' }
    "$(& $Format $S.median) (Q1 $(& $Format $S.q1), Q3 $(& $Format $S.q3); min $(& $Format $S.min), max $(& $Format $S.max))"
}

# --- lookups ----------------------------------------------------------------
function BuildOf { param([string] $App, [string] $Tech) @($build.builds | Where-Object { $_.app -eq $App -and $_.technology -eq $Tech }) | Select-Object -First 1 }
function Row { param([string] $App, [string] $Tech) @($labRows | Where-Object { $_.app -eq $App -and $_.tech -eq $Tech }) | Select-Object -First 1 }
function Bytes { param([string] $App, [string] $Tech) $b = BuildOf $App $Tech; if ($null -eq $b) { $null } else { [long] $b.installerBytes } }
function Smaller { param([string] $App) $a = Bytes $App 'InnoSetup'; $b = Bytes $App 'NSIS'; if ($null -eq $a) { return $b }; if ($null -eq $b) { return $a }; [math]::Min($a, $b) }
function SmallerName { param([string] $App) $a = Bytes $App 'InnoSetup'; $b = Bytes $App 'NSIS'; if ($null -eq $a -or $null -eq $b) { return '' }; if ($a -le $b) { 'IS' } else { 'NSIS' } }
function InstallSeconds { param([string] $App, [string] $Tech) $r = Row $App $Tech; if ($null -eq $r) { $null } else { Get-Prop $r.install 'durationSeconds' } }
function UninstallSeconds { param([string] $App, [string] $Tech) $r = Row $App $Tech; if ($null -eq $r) { $null } else { Get-Prop $r.uninstall 'durationSeconds' } }
function AppFiles { param([string] $App) [int] $corpusApp[$App].packageFiles }
function AppBytes { param([string] $App) [long] $corpusApp[$App].packageBytes }

function FamilyPct { param([object] $App, [string] $Family) $f = Get-Prop $App.byFamily $Family; if ($null -eq $f) { '0.0%' } else { Pct1 $f.bytes $App.bytes } }

$sb = [System.Text.StringBuilder]::new()
function L { param([string] $Line = '') $null = $sb.AppendLine($Line) }

$buildOk = @($build.builds).Count
$rowsOk = @($labRows | Where-Object { $_.success }).Count
$sizeDeltas = @{}   # app -> TS vs smaller of IS/NSIS, in %
foreach ($app in $apps) { $sizeDeltas[$app] = DeltaPct (Bytes $app 'TigerSetup') (Smaller $app) }

# ===========================================================================
L "# TigerSetup $tsVersion across the broad corpus: Inno Setup $($inno.version), NSIS $($nsis.version), fifteen real payloads"
L
L "The installer-technology benchmark ([``report.md``](../report.md), [``report-0.9.0.md``](report-0.9.0.md))"
L "compared TigerSetup with Inno Setup and NSIS on four applications packaged"
L "under their real upstream contracts. This campaign asks whether those"
L "conclusions generalize: the same three technologies, **frozen TigerSetup"
L "$tsVersion**, and the **fifteen real application payloads** the payload-compression"
L "spike pinned ([``compression-spike/report.md``](compression-spike/report.md)) —"
L "from four files to twelve thousand, from 2 MB to 1 GB — each packaged under"
L "one deliberately small common contract ([``packages/broad/contract.md``](packages/broad/contract.md):"
L "install the canonical tree into the named root in user scope, register with"
L "Add/Remove Programs, uninstall from the registered uninstaller, remove"
L "everything), so what differs between two installers of one application is"
L "the technology, not the script. The minimal no-payload installer of each"
L "technology is measured beside them as the fixed-overhead anchor."
L
L "Every number below is read from ``benchmark/results/$Campaign/*.json`` by"
L "``benchmark/scripts/New-BroadReport.ps1``; nothing is transcribed by hand."
L "``build.csv``, ``sizes.csv`` and ``runtime.csv`` beside the JSON carry the same"
L "per-row figures for external analysis. Every figure is a single measurement:"
L "one build per definition on the build machine, one lab row per installer"
L "on the clean baseline (see *Limitations*). MB and KB here are 2^20 and"
L "2^10 bytes; byte columns are exact."
L
L "| Evidence | What | Where | When |"
L "|---|---|---|---|"
L "| Corpus | $($corpus.applications) payloads, $(N $corpus.totalFiles) files, $(MB0 $corpus.totalBytes), verified against the spike inventories$(if ($corpus.allVerified) { '' } else { ' — **NOT all verified**' }) | the build machine | $(When $corpus.capturedAt) |"
L "| Builds | $buildOk builds ($(@($build.builds | Where-Object { $_.technology -eq 'TigerSetup' }).Count) TigerSetup, $(@($build.builds | Where-Object { $_.technology -eq 'InnoSetup' }).Count) Inno Setup, $(@($build.builds | Where-Object { $_.technology -eq 'NSIS' }).Count) NSIS), serial, under one process-tree meter | the build machine (below) | $(When $build.builtAt) → $(When (Get-Prop $build 'finishedAt')) |"
L "| Runtime | $($labRows.Count) rows, $rowsOk passed | ``$(if ($null -ne $lab) { $lab.baseline } else { 'not run' })`` | $(if ($null -ne $lab) { (When $lab.startedAt) + ' → ' + (When $lab.updatedAt) } else { 'not run' }) |"
L

# ===========================================================================
L "## A. Corpus"
L
L "The fifteen payloads as the compression spike pinned them"
L "(``compression-spike/results/corpus.json``), verified on disk file by file"
L "against its inventories before anything was built (``corpus.json``,"
L "``canonical/<App>.json``: every file's path, size and SHA-256). The family"
L "mix is the spike's own classification of each file (native code, managed"
L "code, text, resources, already-compressed content, other), by bytes."
L
L "| App | Version | Files | Bytes | Avg file | Largest file | native | managed | text | precompressed | other | Notes |"
L "|---|---|---:|---:|---:|---|---:|---:|---:|---:|---:|---|"
foreach ($app in $apps) {
    $c = $corpusApp[$app]
    $fam = $c.byFamily
    $famPct = { param($name) $f = Get-Prop $fam $name; if ($null -eq $f) { '0.0%' } else { Pct1 $f.bytes $c.bytes } }
    $largest = $c.largestFile
    $noteParts = @()
    if ([int] $c.packageExcludedFiles -gt 0) { $noteParts += "$($c.packageExcludedFiles) reserved ``.tigersetup/`` entries excluded from every package ($(N $c.packageBytes) B, $($c.packageFiles) files installed)" }
    if ($c.origin -eq 'local-tree' -or $c.origin -eq 'tigersetup-installer') { $noteParts += 'IT Tiger release payload' }
    $resources = Get-Prop $fam 'resources'
    if ($null -ne $resources -and [double] $resources.bytes / $c.bytes -gt 0.1) { $noteParts += "resources $(Pct1 $resources.bytes $c.bytes)" }
    L "| $app | $($c.version) | $(N $c.files) | $(N $c.bytes) | $(KB0 $c.averageFileBytes) | ``$($largest.path)`` ($(MB1 $largest.bytes)) | $(& $famPct 'native') | $(& $famPct 'managed') | $(& $famPct 'text') | $(& $famPct 'precompressed') | $(& $famPct 'other') | $($noteParts -join '; ') |"
}
L
L "Shape at a glance: $(($apps | Sort-Object { AppFiles $_ } | Select-Object -First 3) -join ', ') carry the fewest files;"
L "$(($apps | Sort-Object { - (AppFiles $_) } | Select-Object -First 3) -join ', ') the most;"
L "$(($apps | Sort-Object { - (AppBytes $_) } | Select-Object -First 3) -join ', ') the most bytes."
foreach ($ex in @($corpus.excluded)) { L ""; L "Excluded, as the spike excluded it: **$($ex.app) $($ex.version)** — $($ex.reason)" }
L

# ===========================================================================
L "## B. Package size"
L
L "Installer bytes per application and technology (the exact bytes of the"
L "built artifacts, ``build.json``), TigerSetup against each competitor and"
L "against the smaller of the two, and each installer as a fraction of its"
L "payload. Minimal is in section I: its ratios are all fixed overhead."
L
L "| App | Payload | Inno Setup | NSIS | TigerSetup $tsVersion | TS vs IS | TS vs NSIS | TS vs smaller | IS/payload | NSIS/payload | TS/payload |"
L "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
foreach ($app in $apps) {
    $p = AppBytes $app
    $is = Bytes $app 'InnoSetup'; $ns = Bytes $app 'NSIS'; $ts = Bytes $app 'TigerSetup'
    L "| $app | $(N $p) | $(N $is) | $(N $ns) | $(N $ts) | $(Pct (DeltaPct $ts $is)) | $(Pct (DeltaPct $ts $ns)) | $(Pct $sizeDeltas[$app]) ($(SmallerName $app)) | $(Ratio (Div $is $p)) | $(Ratio (Div $ns $p)) | $(Ratio (Div $ts $p)) |"
}
$sumP = 0L; $sumIs = 0L; $sumNs = 0L; $sumTs = 0L
foreach ($app in $apps) { $sumP += AppBytes $app; $sumIs += [long] (Bytes $app 'InnoSetup'); $sumNs += [long] (Bytes $app 'NSIS'); $sumTs += [long] (Bytes $app 'TigerSetup') }
L "| **Total** | $(N $sumP) | $(N $sumIs) | $(N $sumNs) | $(N $sumTs) | $(Pct (DeltaPct $sumTs $sumIs)) | $(Pct (DeltaPct $sumTs $sumNs)) | | $(Ratio (Div $sumIs $sumP)) | $(Ratio (Div $sumNs $sumP)) | $(Ratio (Div $sumTs $sumP)) |"
L
L "The total is one number over a corpus dominated by its largest payloads;"
L "section E is the distribution over the fifteen."
L

# ===========================================================================
L "## C. Build statistics"
L
L "Every build under ``ProcessTreeMeter.psm1``: the compiler started suspended"
L "in a job object of its own and resumed; wall clock until the job holds no"
L "process; CPU time (user + kernel, exited processes included) and peak"
L "commit (all processes together) from the job's kernel accounting; peak"
L "working set sampled every $(@($build.builds)[0].sampleIntervalMilliseconds) ms (the largest sum of the tree's working"
L "sets in one sample). Builds ran serially in the order the table gives"
L "within an application (the technology order rotates with the application),"
L "one build each."
L
L "| App | Technology | Wall | CPU | CPU/wall | Peak commit | Peak tree WS | Installer bytes | Samples / processes |"
L "|---|---|---:|---:|---:|---:|---:|---:|---:|"
foreach ($b in @($build.builds | Sort-Object { $apps.IndexOf([string] $_.app) }, { [DateTimeOffset] $_.startedAt })) {
    if ($b.app -eq 'Minimal') { continue }
    L "| $($b.app) | $($techLabel[[string] $b.technology]) | $(Sec1 $b.wallSeconds) | $(Sec1 $b.cpuSeconds) | $(Ratio (Div $b.cpuSeconds $b.wallSeconds)) | $(MB1 $b.peakJobCommitBytes) | $(MB1 $b.peakTreeWorkingSetBytes) | $(N $b.installerBytes) | $($b.samples) / $($b.processesTotal) |"
}
L

# ===========================================================================
L "## D. Runtime"
L
if ($null -eq $lab) {
    L "No runtime campaign has been recorded for this results set."
}
else {
    L "One lab session per row on ``$($lab.baseline)`` (the VM restored to its"
    L "clean checkpoint before every install), the install job and the uninstall"
    L "job on the same VM, the session closed and the VM back to *Available*"
    L "before the next row. Install time is the installer process's lifetime plus"
    L "the exit of any process of its own name it left behind; uninstall time is"
    L "the registered uninstaller's lifetime plus the exit of what it hands off"
    L "to (NSIS's ``Au_.exe``; TigerSetup's uninstaller helper) and the removal of"
    L "the install root and of the package-owned state outside it. No fixed"
    L "sleeps. The payload is verified file by file (size and SHA-256) *after*"
    L "the timed interval; *extras* are what the technology put under the root"
    L "beside the payload; *cleanup* is root gone, registration gone,"
    L "package-owned state gone."
    L
    L "| App | Technology | Install | Uninstall | Payload exact | Extras | Registered | Cleanup | Verdict |"
    L "|---|---|---:|---:|---|---|---|---|---|"
    foreach ($r in @($labRows)) {
        $i = $r.install; $u = $r.uninstall
        $extras = @(Get-Prop $i 'payloadExtras')
        $extrasText = $(if ($extras.Count -eq 0) { 'none' } else { "$($extras.Count) ($(N (Get-Prop $i 'payloadExtraBytes')) B): $(($extras | Select-Object -First 3) -join ', ')" })
        $unexpected = @(Get-Prop $i 'payloadUnexpectedExtras')
        if ($unexpected.Count -gt 0) { $extrasText += " — **unexpected: $($unexpected -join ', ')**" }
        $payloadText = $(if (-not [bool] (Get-Prop $i 'payloadVerified')) { 'not verified' } elseif ([bool] (Get-Prop $i 'payloadExact')) { "yes ($(N (Get-Prop $i 'payloadMatched'))/$(N (Get-Prop $i 'payloadExpected')))" } else { "**no** ($(@(Get-Prop $i 'payloadMissing').Count) missing, $(@(Get-Prop $i 'payloadDiffering').Count) differing)" })
        $cleanup = "root $(Mark (Get-Prop $u 'rootRemoved')), ARP $(Mark (Get-Prop $u 'arpRemoved')), state $(Mark (Get-Prop $u 'packageOwnedStateRemoved'))"
        $installText = "$(Sec (Get-Prop $i 'durationSeconds'))$(if ([double] (Get-Prop $i 'completionWaitSeconds') -gt 0.05) { " (+$(Sec (Get-Prop $i 'completionWaitSeconds')) hand-off)" })"
        $uninstallText = "$(Sec (Get-Prop $u 'durationSeconds'))$(if ([double] (Get-Prop $u 'completionWaitSeconds') -gt 0.05) { " ($(Sec (Get-Prop $u 'processSeconds')) process + $(Sec (Get-Prop $u 'completionWaitSeconds')) hand-off)" })"
        L "| $($r.app) | $($techLabel[[string] $r.tech]) | $installText | $uninstallText | $payloadText | $extrasText | $(Mark (Get-Prop $i 'arpRegistered')) | $cleanup | $(if ($r.success) { 'PASS' } else { '**FAIL**' }) |"
    }
    L
    $failed = @($labRows | Where-Object { -not $_.success })
    if ($failed.Count -gt 0) {
        L "Failed rows and what the evidence says:"
        L
        foreach ($r in $failed) {
            $why = @()
            if ([string] (Get-Prop $r.install 'jobStatus') -ne 'OK') { $why += "install job $(Get-Prop $r.install 'jobStatus')" }
            if ((Get-Prop $r.install 'exitCode') -ne 0) { $why += "install exit $(Get-Prop $r.install 'exitCode')" }
            if (-not [bool] (Get-Prop $r.install 'arpRegistered')) { $why += 'registration missing or incomplete' }
            if ([bool] (Get-Prop $r.install 'payloadVerified') -and -not [bool] (Get-Prop $r.install 'payloadExact')) { $why += "payload not exact (missing: $((@(Get-Prop $r.install 'payloadMissing') | Select-Object -First 5) -join ', '); differing: $((@(Get-Prop $r.install 'payloadDiffering') | Select-Object -First 5) -join ', '))" }
            if (@(Get-Prop $r.install 'payloadUnexpectedExtras').Count -gt 0) { $why += "unexpected extras $((@(Get-Prop $r.install 'payloadUnexpectedExtras') | Select-Object -First 5) -join ', ')" }
            if (-not [string]::IsNullOrWhiteSpace([string] (Get-Prop $r.uninstall 'skipped'))) { $why += [string] (Get-Prop $r.uninstall 'skipped') }
            if ([string] (Get-Prop $r.uninstall 'jobStatus') -ne 'OK') { $why += "uninstall job $(Get-Prop $r.uninstall 'jobStatus')" }
            if ((Get-Prop $r.uninstall 'exitCode') -ne 0) { $why += "uninstall exit $(Get-Prop $r.uninstall 'exitCode')" }
            if (-not [bool] (Get-Prop $r.uninstall 'completionSatisfied')) { $why += 'uninstall hand-off or cleanup did not complete within the timeout' }
            if (-not [bool] (Get-Prop $r.uninstall 'rootRemoved')) { $why += "install root left ($(N (Get-Prop $r.uninstall 'remainingFiles')) files)" }
            if (-not [bool] (Get-Prop $r.uninstall 'arpRemoved')) { $why += 'registration left' }
            if (-not [bool] (Get-Prop $r.uninstall 'packageOwnedStateRemoved')) { $why += 'package-owned state left' }
            L "- **$($r.row)**: $($why -join '; ')"
        }
        L
    }
    $tsRows = @($labRows | Where-Object { $_.tech -eq 'TigerSetup' })
    if ($tsRows.Count -gt 0) {
        L "TigerSetup's engine inside the measured lifetime (explanatory only; the"
        L "comparison above is the whole user-visible time): the span from the"
        L "engine's first to its last log event, what ran before it (the loader:"
        L "locating the footer, decompressing and verifying the engine, starting"
        L "it — and Windows starting a large executable for the first time on a"
        L "clean VM) and after it."
        L
        L "| App | Install: before | engine | after | Uninstall: before | engine | after | Engine log |"
        L "|---|---:|---:|---:|---:|---:|---:|---|"
        foreach ($r in $tsRows) {
            $ie = Get-Prop $r.install 'engine'; $ue = Get-Prop $r.uninstall 'engine'
            L "| $($r.app) | $(Sec (Get-Prop $ie 'beforeSeconds')) | $(Sec (Get-Prop $ie 'spanSeconds')) | $(Sec (Get-Prop $ie 'afterSeconds')) | $(Sec (Get-Prop $ue 'beforeSeconds')) | $(Sec (Get-Prop $ue 'spanSeconds')) | $(Sec (Get-Prop $ue 'afterSeconds')) | $(if ($null -ne $ie) { "$($ie.lines) events, $($ie.firstEvent) → $($ie.lastEvent)" } else { 'n/a' }) |"
        }
        L
    }
}

# ===========================================================================
L "## E. Package-size distribution"
L
$deltaVsSmaller = @($apps | ForEach-Object { $sizeDeltas[$_] })
$deltaVsIs = @($apps | ForEach-Object { DeltaPct (Bytes $_ 'TigerSetup') (Bytes $_ 'InnoSetup') })
$deltaVsNs = @($apps | ForEach-Object { DeltaPct (Bytes $_ 'TigerSetup') (Bytes $_ 'NSIS') })
$sSmaller = Stats $deltaVsSmaller
$pctFmt = { param($v) Pct $v }
L "TigerSetup's installer against the smaller of Inno Setup's and NSIS's, per"
L "application, over the fifteen (Minimal excluded):"
L
L "| Statistic | TS vs smaller of IS/NSIS | TS vs Inno Setup | TS vs NSIS |"
L "|---|---:|---:|---:|"
$sIs = Stats $deltaVsIs; $sNs = Stats $deltaVsNs
L "| Median | $(Pct $sSmaller.median) | $(Pct $sIs.median) | $(Pct $sNs.median) |"
L "| Q1 | $(Pct $sSmaller.q1) | $(Pct $sIs.q1) | $(Pct $sNs.q1) |"
L "| Q3 | $(Pct $sSmaller.q3) | $(Pct $sIs.q3) | $(Pct $sNs.q3) |"
L "| Minimum | $(Pct $sSmaller.min) | $(Pct $sIs.min) | $(Pct $sNs.min) |"
L "| Maximum | $(Pct $sSmaller.max) | $(Pct $sIs.max) | $(Pct $sNs.max) |"
$deltaVsSmaller = @($deltaVsSmaller | Where-Object { $null -ne $_ }); $deltaVsIs = @($deltaVsIs | Where-Object { $null -ne $_ }); $deltaVsNs = @($deltaVsNs | Where-Object { $null -ne $_ })
L "| TigerSetup smaller | $(@($deltaVsSmaller | Where-Object { $_ -lt 0 }).Count) of $($deltaVsSmaller.Count) | $(@($deltaVsIs | Where-Object { $_ -lt 0 }).Count) of $($deltaVsIs.Count) | $(@($deltaVsNs | Where-Object { $_ -lt 0 }).Count) of $($deltaVsNs.Count) |"
L "| Within ±5% | $(@($deltaVsSmaller | Where-Object { [math]::Abs($_) -le 5 }).Count) | $(@($deltaVsIs | Where-Object { [math]::Abs($_) -le 5 }).Count) | $(@($deltaVsNs | Where-Object { [math]::Abs($_) -le 5 }).Count) |"
L "| Within ±10% | $(@($deltaVsSmaller | Where-Object { [math]::Abs($_) -le 10 }).Count) | $(@($deltaVsIs | Where-Object { [math]::Abs($_) -le 10 }).Count) | $(@($deltaVsNs | Where-Object { [math]::Abs($_) -le 10 }).Count) |"
L "| More than 10% larger | $(@($deltaVsSmaller | Where-Object { $_ -gt 10 }).Count) | $(@($deltaVsIs | Where-Object { $_ -gt 10 }).Count) | $(@($deltaVsNs | Where-Object { $_ -gt 10 }).Count) |"
L
$smallerIsIs = @($apps | Where-Object { (SmallerName $_) -eq 'IS' }).Count
$large = @($apps | Where-Object { (AppBytes $_) -ge 100MB }); $small = @($apps | Where-Object { (AppBytes $_) -lt 100MB })
$largeStats = Stats @($large | ForEach-Object { $sizeDeltas[$_] }); $smallStats = Stats @($small | ForEach-Object { $sizeDeltas[$_] })
L "The smaller competitor is Inno Setup for $smallerIsIs of the fifteen and NSIS for $($apps.Count - $smallerIsIs)."
L "The delta follows the payload's size, because TigerSetup's fixed overhead"
L "(section I) is a constant term: over the $($large.Count) payloads of 100 MB or more it is"
L "$(Pct $largeStats.median) at the median ($(Pct $largeStats.min) to $(Pct $largeStats.max)); over the $($small.Count) below 100 MB it is"
L "$(Pct $smallStats.median) at the median ($(Pct $smallStats.min) to $(Pct $smallStats.max))."
L "Sorted by TigerSetup's delta against the smaller competitor:"
L
L "| App | Payload | TS vs smaller | Smaller is | Files | Avg file |"
L "|---|---:|---:|---|---:|---:|"
foreach ($app in @($apps | Sort-Object { $sizeDeltas[$_] })) {
    L "| $app | $(MB1 (AppBytes $app)) | $(Pct $sizeDeltas[$app]) | $(SmallerName $app) | $(N (AppFiles $app)) | $(KB0 $corpusApp[$app].averageFileBytes) |"
}
L

# ===========================================================================
L "## F. Build-resource summary"
L
L "Across the fifteen applications (Minimal excluded), TigerSetup's build"
L "against each competitor's, per application and as a distribution. A ratio"
L "above 1 means TigerSetup took more. CPU/wall says how many cores a"
L "compiler kept busy: Inno Setup's LZMA2 compressor is multi-threaded,"
L "TigerSetup's zstd and NSIS's LZMA run on one."
L
L "| App | Wall IS | Wall NSIS | Wall TS | TS/IS | TS/NSIS | CPU IS | CPU NSIS | CPU TS | TS/IS | TS/NSIS | CPU/wall IS | NSIS | TS |"
L "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
$wallRatioIs = @(); $wallRatioNs = @(); $cpuRatioIs = @(); $cpuRatioNs = @()
$wallBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }; $cpuBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }
$commitBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }; $wsBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }; $cpuWallBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }
foreach ($app in $apps) {
    $bi = BuildOf $app 'InnoSetup'; $bn = BuildOf $app 'NSIS'; $bt = BuildOf $app 'TigerSetup'
    foreach ($pair in @(@('InnoSetup', $bi), @('NSIS', $bn), @('TigerSetup', $bt))) {
        $t = $pair[0]; $b = $pair[1]
        if ($null -eq $b) { continue }
        $wallBy[$t] += [double] $b.wallSeconds; $cpuBy[$t] += [double] $b.cpuSeconds; $commitBy[$t] += [double] $b.peakJobCommitBytes; $wsBy[$t] += [double] $b.peakTreeWorkingSetBytes; $cpuWallBy[$t] += (Div $b.cpuSeconds $b.wallSeconds)
    }
    $wallRatioIs += Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bi 'wallSeconds'); $wallRatioNs += Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bn 'wallSeconds')
    $cpuRatioIs += Div (Get-Prop $bt 'cpuSeconds') (Get-Prop $bi 'cpuSeconds'); $cpuRatioNs += Div (Get-Prop $bt 'cpuSeconds') (Get-Prop $bn 'cpuSeconds')
    L "| $app | $(Sec1 (Get-Prop $bi 'wallSeconds')) | $(Sec1 (Get-Prop $bn 'wallSeconds')) | $(Sec1 (Get-Prop $bt 'wallSeconds')) | $(Times (Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bi 'wallSeconds'))) | $(Times (Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bn 'wallSeconds'))) | $(Sec1 (Get-Prop $bi 'cpuSeconds')) | $(Sec1 (Get-Prop $bn 'cpuSeconds')) | $(Sec1 (Get-Prop $bt 'cpuSeconds')) | $(Times (Div (Get-Prop $bt 'cpuSeconds') (Get-Prop $bi 'cpuSeconds'))) | $(Times (Div (Get-Prop $bt 'cpuSeconds') (Get-Prop $bn 'cpuSeconds'))) | $(Ratio (Div (Get-Prop $bi 'cpuSeconds') (Get-Prop $bi 'wallSeconds'))) | $(Ratio (Div (Get-Prop $bn 'cpuSeconds') (Get-Prop $bn 'wallSeconds'))) | $(Ratio (Div (Get-Prop $bt 'cpuSeconds') (Get-Prop $bt 'wallSeconds'))) |"
}
L
$secFmt = { param($v) Sec1 $v }
$timesFmt = { param($v) Times $v }
$mbFmt = { param($v) MB1 $v }
$ratioFmt = { param($v) Ratio $v }
L "| Distribution | Inno Setup | NSIS | TigerSetup | TS/IS | TS/NSIS |"
L "|---|---|---|---|---|---|"
L "| Build wall, median (Q1–Q3; min–max) | $(StatLine (Stats $wallBy.InnoSetup) $secFmt) | $(StatLine (Stats $wallBy.NSIS) $secFmt) | $(StatLine (Stats $wallBy.TigerSetup) $secFmt) | $(StatLine (Stats $wallRatioIs) $timesFmt) | $(StatLine (Stats $wallRatioNs) $timesFmt) |"
L "| Build wall, aggregate | $(Sec1 (Stats $wallBy.InnoSetup).sum) | $(Sec1 (Stats $wallBy.NSIS).sum) | $(Sec1 (Stats $wallBy.TigerSetup).sum) | $(Times (Div (Stats $wallBy.TigerSetup).sum (Stats $wallBy.InnoSetup).sum)) | $(Times (Div (Stats $wallBy.TigerSetup).sum (Stats $wallBy.NSIS).sum)) |"
L "| CPU, median (Q1–Q3; min–max) | $(StatLine (Stats $cpuBy.InnoSetup) $secFmt) | $(StatLine (Stats $cpuBy.NSIS) $secFmt) | $(StatLine (Stats $cpuBy.TigerSetup) $secFmt) | $(StatLine (Stats $cpuRatioIs) $timesFmt) | $(StatLine (Stats $cpuRatioNs) $timesFmt) |"
L "| CPU, aggregate | $(Sec1 (Stats $cpuBy.InnoSetup).sum) | $(Sec1 (Stats $cpuBy.NSIS).sum) | $(Sec1 (Stats $cpuBy.TigerSetup).sum) | $(Times (Div (Stats $cpuBy.TigerSetup).sum (Stats $cpuBy.InnoSetup).sum)) | $(Times (Div (Stats $cpuBy.TigerSetup).sum (Stats $cpuBy.NSIS).sum)) |"
L "| CPU/wall, median (min–max) | $(StatLine (Stats $cpuWallBy.InnoSetup) $ratioFmt) | $(StatLine (Stats $cpuWallBy.NSIS) $ratioFmt) | $(StatLine (Stats $cpuWallBy.TigerSetup) $ratioFmt) | | |"
L "| Peak commit, median (Q1–Q3; min–max) | $(StatLine (Stats $commitBy.InnoSetup) $mbFmt) | $(StatLine (Stats $commitBy.NSIS) $mbFmt) | $(StatLine (Stats $commitBy.TigerSetup) $mbFmt) | $(Times (Div (Stats $commitBy.TigerSetup).median (Stats $commitBy.InnoSetup).median)) (medians) | $(Times (Div (Stats $commitBy.TigerSetup).median (Stats $commitBy.NSIS).median)) (medians) |"
L "| Peak commit, maximum | $(MB1 (Stats $commitBy.InnoSetup).max) | $(MB1 (Stats $commitBy.NSIS).max) | $(MB1 (Stats $commitBy.TigerSetup).max) | $(Times (Div (Stats $commitBy.TigerSetup).max (Stats $commitBy.InnoSetup).max)) | $(Times (Div (Stats $commitBy.TigerSetup).max (Stats $commitBy.NSIS).max)) |"
L "| Peak tree working set, median (Q1–Q3; min–max) | $(StatLine (Stats $wsBy.InnoSetup) $mbFmt) | $(StatLine (Stats $wsBy.NSIS) $mbFmt) | $(StatLine (Stats $wsBy.TigerSetup) $mbFmt) | $(Times (Div (Stats $wsBy.TigerSetup).median (Stats $wsBy.InnoSetup).median)) (medians) | $(Times (Div (Stats $wsBy.TigerSetup).median (Stats $wsBy.NSIS).median)) (medians) |"
L "| Peak tree working set, maximum | $(MB1 (Stats $wsBy.InnoSetup).max) | $(MB1 (Stats $wsBy.NSIS).max) | $(MB1 (Stats $wsBy.TigerSetup).max) | $(Times (Div (Stats $wsBy.TigerSetup).max (Stats $wsBy.InnoSetup).max)) | $(Times (Div (Stats $wsBy.TigerSetup).max (Stats $wsBy.NSIS).max)) |"
L
L "Peak memory per build:"
L
L "| App | Commit IS | Commit NSIS | Commit TS | TS/IS | TS/NSIS | Tree WS IS | Tree WS NSIS | Tree WS TS |"
L "|---|---:|---:|---:|---:|---:|---:|---:|---:|"
foreach ($app in $apps) {
    $bi = BuildOf $app 'InnoSetup'; $bn = BuildOf $app 'NSIS'; $bt = BuildOf $app 'TigerSetup'
    L "| $app | $(MB1 (Get-Prop $bi 'peakJobCommitBytes')) | $(MB1 (Get-Prop $bn 'peakJobCommitBytes')) | $(MB1 (Get-Prop $bt 'peakJobCommitBytes')) | $(Times (Div (Get-Prop $bt 'peakJobCommitBytes') (Get-Prop $bi 'peakJobCommitBytes'))) | $(Times (Div (Get-Prop $bt 'peakJobCommitBytes') (Get-Prop $bn 'peakJobCommitBytes'))) | $(MB1 (Get-Prop $bi 'peakTreeWorkingSetBytes')) | $(MB1 (Get-Prop $bn 'peakTreeWorkingSetBytes')) | $(MB1 (Get-Prop $bt 'peakTreeWorkingSetBytes')) |"
}
L

# ===========================================================================
L "## G. Runtime summary"
L
if ($labRows.Count -eq 0) {
    L "No runtime rows."
}
else {
    $instBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }; $uninstBy = @{ InnoSetup = @(); NSIS = @(); TigerSetup = @() }
    $instRatioIs = @(); $instRatioNs = @(); $uninstRatioIs = @(); $uninstRatioNs = @()
    L "Install and uninstall wall time per application (the whole user-visible"
    L "time, as section D defines it) and TigerSetup's ratio to each competitor;"
    L "a ratio below 1 means TigerSetup was faster."
    L
    L "| App | Install IS | NSIS | TS | TS/IS | TS/NSIS | Uninstall IS | NSIS | TS | TS/IS | TS/NSIS |"
    L "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    foreach ($app in $apps) {
        $ii = InstallSeconds $app 'InnoSetup'; $in = InstallSeconds $app 'NSIS'; $it = InstallSeconds $app 'TigerSetup'
        $ui = UninstallSeconds $app 'InnoSetup'; $un = UninstallSeconds $app 'NSIS'; $ut = UninstallSeconds $app 'TigerSetup'
        foreach ($pair in @(@('InnoSetup', $ii, $ui), @('NSIS', $in, $un), @('TigerSetup', $it, $ut))) {
            if (-not (Missing $pair[1])) { $instBy[$pair[0]] += [double] $pair[1] }
            if (-not (Missing $pair[2])) { $uninstBy[$pair[0]] += [double] $pair[2] }
        }
        $instRatioIs += Div $it $ii; $instRatioNs += Div $it $in; $uninstRatioIs += Div $ut $ui; $uninstRatioNs += Div $ut $un
        L "| $app | $(Sec $ii) | $(Sec $in) | $(Sec $it) | $(Times (Div $it $ii)) | $(Times (Div $it $in)) | $(Sec $ui) | $(Sec $un) | $(Sec $ut) | $(Times (Div $ut $ui)) | $(Times (Div $ut $un)) |"
    }
    L
    $secFmt2 = { param($v) Sec $v }
    L "| Distribution | Inno Setup | NSIS | TigerSetup | TS/IS | TS/NSIS |"
    L "|---|---|---|---|---|---|"
    L "| Install, median (Q1–Q3; min–max) | $(StatLine (Stats $instBy.InnoSetup) $secFmt2) | $(StatLine (Stats $instBy.NSIS) $secFmt2) | $(StatLine (Stats $instBy.TigerSetup) $secFmt2) | $(StatLine (Stats $instRatioIs) $timesFmt) | $(StatLine (Stats $instRatioNs) $timesFmt) |"
    L "| Install, aggregate | $(Sec (Stats $instBy.InnoSetup).sum) | $(Sec (Stats $instBy.NSIS).sum) | $(Sec (Stats $instBy.TigerSetup).sum) | $(Times (Div (Stats $instBy.TigerSetup).sum (Stats $instBy.InnoSetup).sum)) | $(Times (Div (Stats $instBy.TigerSetup).sum (Stats $instBy.NSIS).sum)) |"
    L "| Install, TigerSetup faster / slower | | | | $(@($instRatioIs | Where-Object { $null -ne $_ -and $_ -lt 1 }).Count) / $(@($instRatioIs | Where-Object { $null -ne $_ -and $_ -gt 1 }).Count) | $(@($instRatioNs | Where-Object { $null -ne $_ -and $_ -lt 1 }).Count) / $(@($instRatioNs | Where-Object { $null -ne $_ -and $_ -gt 1 }).Count) |"
    L "| Uninstall, median (Q1–Q3; min–max) | $(StatLine (Stats $uninstBy.InnoSetup) $secFmt2) | $(StatLine (Stats $uninstBy.NSIS) $secFmt2) | $(StatLine (Stats $uninstBy.TigerSetup) $secFmt2) | $(StatLine (Stats $uninstRatioIs) $timesFmt) | $(StatLine (Stats $uninstRatioNs) $timesFmt) |"
    L "| Uninstall, aggregate | $(Sec (Stats $uninstBy.InnoSetup).sum) | $(Sec (Stats $uninstBy.NSIS).sum) | $(Sec (Stats $uninstBy.TigerSetup).sum) | $(Times (Div (Stats $uninstBy.TigerSetup).sum (Stats $uninstBy.InnoSetup).sum)) | $(Times (Div (Stats $uninstBy.TigerSetup).sum (Stats $uninstBy.NSIS).sum)) |"
    L "| Uninstall, TigerSetup faster / slower | | | | $(@($uninstRatioIs | Where-Object { $null -ne $_ -and $_ -lt 1 }).Count) / $(@($uninstRatioIs | Where-Object { $null -ne $_ -and $_ -gt 1 }).Count) | $(@($uninstRatioNs | Where-Object { $null -ne $_ -and $_ -lt 1 }).Count) / $(@($uninstRatioNs | Where-Object { $null -ne $_ -and $_ -gt 1 }).Count) |"
    L
    $countRows = { param($section, $name) @($labRows | Where-Object { [bool] (Get-Prop $_.$section $name) }).Count }
    L "Payload fidelity and cleanup over the $($labRows.Count) rows: payload exact in $(& $countRows 'install' 'payloadExact'), registration present in $(& $countRows 'install' 'arpRegistered'), root removed in $(& $countRows 'uninstall' 'rootRemoved'), registration removed in $(& $countRows 'uninstall' 'arpRemoved'), package-owned state removed in $(& $countRows 'uninstall' 'packageOwnedStateRemoved'); $rowsOk passed."
    $nsisResidue = @($labRows | Where-Object { $_.tech -eq 'NSIS' } | ForEach-Object { @(Get-Prop $_.uninstall 'residue') } | Where-Object { $null -ne $_ -and $_.exists -eq $true })
    if ($nsisResidue.Count -gt 0) { L "NSIS's uninstaller copy (``%TEMP%\~nsuA.tmp``) was still present after $($nsisResidue.Count) of the NSIS uninstalls — NSIS marks it for deletion at the next reboot; recorded, not counted against cleanup." }
    L
}

# ===========================================================================
L "## H. Workload shape"
L
if ($labRows.Count -eq 0) {
    L "No runtime rows."
}
else {
    L "Runtime against the payload's size and file count, per technology. The"
    L "table is sorted by file count; the correlations are Pearson's r over the"
    L "applications with a row (a description of this corpus, not a causal"
    L "claim — bytes and files are themselves correlated across it)."
    L
    L "| App | Files | Bytes | Install IS | NSIS | TS | Uninstall IS | NSIS | TS |"
    L "|---|---:|---:|---:|---:|---:|---:|---:|---:|"
    foreach ($app in @($apps | Sort-Object { AppFiles $_ })) {
        L "| $app | $(N (AppFiles $app)) | $(MB1 (AppBytes $app)) | $(Sec (InstallSeconds $app 'InnoSetup')) | $(Sec (InstallSeconds $app 'NSIS')) | $(Sec (InstallSeconds $app 'TigerSetup')) | $(Sec (UninstallSeconds $app 'InnoSetup')) | $(Sec (UninstallSeconds $app 'NSIS')) | $(Sec (UninstallSeconds $app 'TigerSetup')) |"
    }
    L
    L "The two normalizations are meaningful only where the payload, not the"
    L "launch, dominates the time: the per-100 MB median is taken over the"
    L "payloads of 100 MB or more, the per-1,000-files median over those of"
    L "1,000 files or more."
    L
    L "| Technology | Install vs bytes (r) | Install vs files (r) | Uninstall vs bytes (r) | Uninstall vs files (r) | Install per 100 MB (median, ≥100 MB) | Install per 1,000 files (median, ≥1,000 files) |"
    L "|---|---:|---:|---:|---:|---:|---:|"
    foreach ($tech in $technologies) {
        $withRow = @($apps | Where-Object { -not (Missing (InstallSeconds $_ $tech)) })
        $x1 = @($withRow | ForEach-Object { [double] (AppBytes $_) }); $x2 = @($withRow | ForEach-Object { [double] (AppFiles $_) })
        $yi = @($withRow | ForEach-Object { [double] (InstallSeconds $_ $tech) })
        $withU = @($apps | Where-Object { -not (Missing (UninstallSeconds $_ $tech)) })
        $ux1 = @($withU | ForEach-Object { [double] (AppBytes $_) }); $ux2 = @($withU | ForEach-Object { [double] (AppFiles $_) })
        $yu = @($withU | ForEach-Object { [double] (UninstallSeconds $_ $tech) })
        $per100 = Stats @($withRow | Where-Object { (AppBytes $_) -ge 100MB } | ForEach-Object { 100 * [double] (InstallSeconds $_ $tech) / ([double] (AppBytes $_) / 1MB) })
        $perK = Stats @($withRow | Where-Object { (AppFiles $_) -ge 1000 } | ForEach-Object { 1000 * [double] (InstallSeconds $_ $tech) / [double] (AppFiles $_) })
        L "| $($techLabel[$tech]) | $(Ratio (Pearson $x1 $yi)) | $(Ratio (Pearson $x2 $yi)) | $(Ratio (Pearson $ux1 $yu)) | $(Ratio (Pearson $ux2 $yu)) | $(Sec $per100.median) | $(Sec $perK.median) |"
    }
    L
    L "Per-application throughput of the install (seconds per 100 MB of payload"
    L "and per 1,000 files), the two normalizations that separate a byte-bound"
    L "row from a file-bound one:"
    L
    L "| App | Files | Bytes | s/100 MB: IS | NSIS | TS | s/1,000 files: IS | NSIS | TS |"
    L "|---|---:|---:|---:|---:|---:|---:|---:|---:|"
    foreach ($app in @($apps | Sort-Object { - (AppBytes $_) })) {
        $mb = [double] (AppBytes $app) / 1MB; $kf = [double] (AppFiles $app) / 1000
        $per = { param($t) $s = InstallSeconds $app $t; if (Missing $s) { @($null, $null) } else { @((100 * [double] $s / $mb), ([double] $s / $kf)) } }
        $pi = & $per 'InnoSetup'; $pn = & $per 'NSIS'; $pt = & $per 'TigerSetup'
        L "| $app | $(N (AppFiles $app)) | $(MB1 (AppBytes $app)) | $(Sec $pi[0]) | $(Sec $pn[0]) | $(Sec $pt[0]) | $(Sec $pi[1]) | $(Sec $pn[1]) | $(Sec $pt[1]) |"
    }
    L
}

# ===========================================================================
L "## I. Fixed overhead"
L
L "The minimal no-payload installer of each technology (``packages/minimal``:"
L "identity and metadata only, one short text file for TigerSetup, which"
L "requires a file entry): what a generated installer starts at before any"
L "application bytes, and what building nothing costs."
L
L "| Technology | Minimal installer | Build wall | Build CPU | Peak commit | What it carries |"
L "|---|---:|---:|---:|---:|---|"
$minimalNote = @{
    InnoSetup = "SetupLdr.e64 + the LZMA2-compressed native x64 engine (Setup.e64 $(N $inno.native_x64_engine.bytes) B raw)"
    NSIS = "the lzma_solid-x86-unicode exehead ($(N $nsis.exehead.bytes) B) + the compiled script"
    TigerSetup = "the C loader ($(if ($null -ne (Get-Prop $toolchain.tigersetup 'loader')) { N $toolchain.tigersetup.loader.bytes } else { 'n/a' }) B) + the zstd-compressed engine ($(N $toolchain.tigersetup.engine.bytes) B raw) + the compressed metadata"
}
foreach ($tech in $technologies) {
    $b = BuildOf 'Minimal' $tech
    L "| $($techLabel[$tech]) | $(N (Get-Prop $b 'installerBytes')) | $(Sec (Get-Prop $b 'wallSeconds')) | $(Sec (Get-Prop $b 'cpuSeconds')) | $(MB1 (Get-Prop $b 'peakJobCommitBytes')) | $($minimalNote[$tech]) |"
}
L
$minTs = Bytes 'Minimal' 'TigerSetup'; $minIs = Bytes 'Minimal' 'InnoSetup'; $minNs = Bytes 'Minimal' 'NSIS'
L "TigerSetup's fixed overhead is $(Pct (DeltaPct $minTs $minIs)) against Inno Setup's and $(Times (Div $minTs $minNs)) NSIS's."
L "Against the smallest application payload ($(($apps | Sort-Object { AppBytes $_ } | Select-Object -First 1)), $(MB1 (($apps | Sort-Object { AppBytes $_ } | Select-Object -First 1 | ForEach-Object { AppBytes $_ })))) it is the"
L "dominant term; against the median payload ($(MB1 (Stats @($apps | ForEach-Object { AppBytes $_ })).median)) it is a few percent."
L

# ===========================================================================
L "## J. Outliers"
L
$byDelta = @($apps | Sort-Object { $sizeDeltas[$_] })
L "**Package size.** TigerSetup's three best and three worst results against the smaller competitor:"
L
foreach ($app in @($byDelta | Select-Object -First 3)) {
    $c = $corpusApp[$app]
    L "- **$app**: $(Pct $sizeDeltas[$app]) vs $(SmallerName $app) — $(N $c.packageFiles) files, $(MB1 $c.packageBytes), avg $(KB0 $c.averageFileBytes), largest ``$($c.largestFile.path)`` $(MB1 $c.largestFile.bytes); native $(FamilyPct $c 'native'), text $(FamilyPct $c 'text'), precompressed $(FamilyPct $c 'precompressed')"
}
foreach ($app in @($byDelta | Select-Object -Last 3)) {
    $c = $corpusApp[$app]
    L "- **$app**: $(Pct $sizeDeltas[$app]) vs $(SmallerName $app) — $(N $c.packageFiles) files, $(MB1 $c.packageBytes), avg $(KB0 $c.averageFileBytes), largest ``$($c.largestFile.path)`` $(MB1 $c.largestFile.bytes); native $(FamilyPct $c 'native'), text $(FamilyPct $c 'text'), precompressed $(FamilyPct $c 'precompressed')"
}
L
if ($labRows.Count -gt 0) {
    $instDelta = @{}
    foreach ($app in $apps) {
        $it = InstallSeconds $app 'TigerSetup'
        $best = @((InstallSeconds $app 'InnoSetup'), (InstallSeconds $app 'NSIS')) | Where-Object { -not (Missing $_) } | Sort-Object | Select-Object -First 1
        $instDelta[$app] = Div $it $best
    }
    $withInst = @($apps | Where-Object { $null -ne $instDelta[$_] } | Sort-Object { $instDelta[$_] })
    if ($withInst.Count -gt 0) {
        L "**Install time**, TigerSetup against the faster competitor (ratio below 1: TigerSetup faster):"
        L
        foreach ($app in @($withInst | Select-Object -First 3)) { L "- **$app**: $(Times $instDelta[$app]) ($(Sec (InstallSeconds $app 'TigerSetup')) vs IS $(Sec (InstallSeconds $app 'InnoSetup')), NSIS $(Sec (InstallSeconds $app 'NSIS'))) — $(N (AppFiles $app)) files, $(MB1 (AppBytes $app))" }
        foreach ($app in @($withInst | Select-Object -Last 3)) { L "- **$app**: $(Times $instDelta[$app]) ($(Sec (InstallSeconds $app 'TigerSetup')) vs IS $(Sec (InstallSeconds $app 'InnoSetup')), NSIS $(Sec (InstallSeconds $app 'NSIS'))) — $(N (AppFiles $app)) files, $(MB1 (AppBytes $app))" }
        L
    }
    $uninstDelta = @{}
    foreach ($app in $apps) {
        $ut = UninstallSeconds $app 'TigerSetup'
        $best = @((UninstallSeconds $app 'InnoSetup'), (UninstallSeconds $app 'NSIS')) | Where-Object { -not (Missing $_) } | Sort-Object | Select-Object -First 1
        $uninstDelta[$app] = Div $ut $best
    }
    $withUninst = @($apps | Where-Object { $null -ne $uninstDelta[$_] } | Sort-Object { $uninstDelta[$_] })
    if ($withUninst.Count -gt 0) {
        L "**Uninstall time**, TigerSetup against the faster competitor:"
        L
        foreach ($app in @($withUninst | Select-Object -First 3)) { L "- **$app**: $(Times $uninstDelta[$app]) ($(Sec (UninstallSeconds $app 'TigerSetup')) vs IS $(Sec (UninstallSeconds $app 'InnoSetup')), NSIS $(Sec (UninstallSeconds $app 'NSIS')))" }
        foreach ($app in @($withUninst | Select-Object -Last 3)) { L "- **$app**: $(Times $uninstDelta[$app]) ($(Sec (UninstallSeconds $app 'TigerSetup')) vs IS $(Sec (UninstallSeconds $app 'InnoSetup')), NSIS $(Sec (UninstallSeconds $app 'NSIS')))" }
        L
    }
}
L "**Build.** The largest peak commit and the longest build of each technology, and TigerSetup's widest wall-clock ratios:"
L
foreach ($tech in $technologies) {
    $bs = @($build.builds | Where-Object { $_.technology -eq $tech -and $_.app -ne 'Minimal' })
    $maxCommit = $bs | Sort-Object { - [double] $_.peakJobCommitBytes } | Select-Object -First 1
    $maxWall = $bs | Sort-Object { - [double] $_.wallSeconds } | Select-Object -First 1
    L "- $($techLabel[$tech]): peak commit $(MB1 (Get-Prop $maxCommit 'peakJobCommitBytes')) on $((Get-Prop $maxCommit 'app')); longest build $(Sec1 (Get-Prop $maxWall 'wallSeconds')) on $((Get-Prop $maxWall 'app')) ($(MB1 (AppBytes (Get-Prop $maxWall 'app'))), $(N (AppFiles (Get-Prop $maxWall 'app'))) files)"
}
$wallRatioByApp = @{}
foreach ($app in $apps) {
    $bt = BuildOf $app 'TigerSetup'; $bi = BuildOf $app 'InnoSetup'; $bn = BuildOf $app 'NSIS'
    $wallRatioByApp[$app] = @{ IS = (Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bi 'wallSeconds')); NSIS = (Div (Get-Prop $bt 'wallSeconds') (Get-Prop $bn 'wallSeconds')) }
}
$widestIs = @($apps | Where-Object { $null -ne $wallRatioByApp[$_].IS } | Sort-Object { - $wallRatioByApp[$_].IS })
$widestNs = @($apps | Where-Object { $null -ne $wallRatioByApp[$_].NSIS } | Sort-Object { - $wallRatioByApp[$_].NSIS })
if ($widestIs.Count -gt 0) { L "- TigerSetup's build wall against Inno Setup ranges from $(Times $wallRatioByApp[$widestIs[-1]].IS) ($($widestIs[-1])) to $(Times $wallRatioByApp[$widestIs[0]].IS) ($($widestIs[0]))" }
if ($widestNs.Count -gt 0) { L "- TigerSetup's build wall against NSIS ranges from $(Times $wallRatioByApp[$widestNs[-1]].NSIS) ($($widestNs[-1])) to $(Times $wallRatioByApp[$widestNs[0]].NSIS) ($($widestNs[0]))" }
L

# ===========================================================================
L "## Determinism"
L
if ($null -eq $determinism) {
    L "No determinism check was recorded."
}
else {
    L "$($determinism.note)"
    L
    L "| App | Technology | First build | Second build | Identical |"
    L "|---|---|---|---|---|"
    foreach ($d in @($determinism.checks)) {
        L "| $($d.app) | $($techLabel[[string] $d.technology]) | $(Short $d.firstSha256) ($(N $d.firstBytes) B) | $(Short $d.secondSha256) ($(N $d.secondBytes) B) | $(Mark $d.identical) |"
    }
    L
}

# ===========================================================================
L "## Provenance"
L
L "| | |"
L "|---|---|"
if ($null -ne $hostFacts -and $hostFacts -isnot [string]) {
    $win = Get-Prop $hostFacts 'windows'; $cpu = Get-Prop $hostFacts 'cpu'; $drive = Get-Prop $hostFacts 'repositoryDrive'
    L "| Host | $(Get-Prop $win 'caption') $(Get-Prop $win 'displayVersion') (build $(Get-Prop $win 'build')); $(Get-Prop $cpu 'name'), $(Get-Prop $cpu 'cores') cores / $(Get-Prop $cpu 'logicalProcessors') logical; $(MB0 (Get-Prop $hostFacts 'memoryBytes')) RAM; repository on $(if ($null -ne $drive -and $drive -isnot [string]) { "$($drive.letter) $($drive.model) ($($drive.mediaType), $($drive.busType))" } else { 'n/a' }); PowerShell $(Get-Prop $hostFacts 'powershell') |"
}
L "| Repository | commit ``$($toolchain.tigersetup.git_commit)``$(if ([bool] $toolchain.tigersetup.working_tree_dirty) { ' (working tree dirty: the benchmark machinery of this campaign was uncommitted while it ran)' } else { '' }) |"
$loaderText = ''
if ($null -ne (Get-Prop $toolchain.tigersetup 'loader')) { $loaderText = "; loader ``tigersetup-loader.exe`` $(N $toolchain.tigersetup.loader.bytes) B ``$($toolchain.tigersetup.loader.sha256)``" }
L "| TigerSetup | $tsVersion — builder ``tiger-setup.exe`` $(N $toolchain.tigersetup.builder.bytes) B ``$($toolchain.tigersetup.builder.sha256)``; engine ``tigersetup-setup.exe`` $(N $toolchain.tigersetup.engine.bytes) B ``$($toolchain.tigersetup.engine.sha256)``$loaderText; every TigerSetup installer verified (``tiger-setup inspect``) to carry this engine; release builder defaults (solid zstd-19-w27, single-threaded), never ``--fast`` |"
L "| Inno Setup | $($inno.version) — ``$($inno.compiler.path)`` ``$($inno.compiler.sha256)``; Setup.e64 ``$($inno.native_x64_engine.sha256)``; SetupLdr.e64 ``$($inno.loader_stub.sha256)``; $($inno.compression) |"
$nsisBinText = ''
if ($null -ne (Get-Prop $nsis 'compiler_bin')) { $nsisBinText = "; Bin\makensis.exe ``$($nsis.compiler_bin.sha256)``" }
L "| NSIS | $($nsis.version) — ``$($nsis.compiler.path)`` ``$($nsis.compiler.sha256)``$nsisBinText; exehead ``$($nsis.exehead.sha256)``; $($nsis.compression) |"
L "| Corpus | $($corpus.spikeEvidence.corpus); $($corpus.spikeEvidence.payloads); per-file SHA-256 inventories in ``results/$Campaign/canonical/`` |"
if ($null -ne $lab) {
    $env = Get-Prop $lab 'labEnvironment'
    if ($null -ne $env) {
        $os = Get-Prop $env 'os'
        L "| Lab | ``$($lab.baseline)`` — $(Get-Prop $env 'baselineDisplayName'); $(Get-Prop $os 'caption') $(Get-Prop $os 'displayVersion') build $(Get-Prop $os 'build').$(Get-Prop $os 'updateBuildRevision'); checkpoint ``$(Get-Prop $env 'checkpoint')``; commands run as the job account (``$(Get-Prop (Get-Prop $env 'account') 'name')``, session $(Get-Prop (Get-Prop $env 'account') 'sessionId')) |"
    }
    L "| Rows | builder ``$($lab.builder)``, engine ``$($lab.engineSha256)``; every row records the SHA-256 and bytes of the installer it ran (``lab-results.json``), matching ``build.json`` |"
}
L

if ($null -ne $notes) {
    L "## Campaign notes"
    L
    foreach ($note in @(Get-Prop $notes 'notes')) { L "- $note" }
    L
}

# ===========================================================================
L "## Limitations"
L
L "- **Single measurements.** One build per definition and one lab row per"
L "  installer. The 0.9.0 campaign found host activity during a build moves"
L "  its wall clock by 2–8%; a VM row varies with the host's disk cache and"
L "  the guest's background services. Differences of a few percent are noise;"
L "  the distributions over fifteen applications are the finding, not any"
L "  single row."
L "- **Live host.** Builds ran on a developer workstation with the operating"
L "  system's cache as it was; the technology order rotates so no technology"
L "  always builds first into a cold cache, and nothing was flushed."
L "- **VM timing.** Install and uninstall times are a 4-vCPU Hyper-V guest's,"
L "  with a differencing disk restored from a checkpoint before every row;"
L "  absolute times are not a physical machine's, ratios between technologies"
L "  on the same row are the comparable figure."
L "- **Corpus representativeness.** Fifteen payloads chosen for shape"
L "  variety — five of them IT Tiger's own applications — not a sample of the"
L "  Windows application population; GIMP is absent because its Inno Setup 7"
L "  installer cannot be unpacked by open tooling."
L "- **The common contract is deliberately small.** No shortcut, association,"
L "  dependency or custom action: the rows measure payload handling and"
L "  bookkeeping, not the functional features the first benchmark compared."
L "- **User scope only**, run as an administrator job account: no technology"
L "  elevates, and the machine-scope paths (Program Files, HKLM) are not"
L "  exercised here."
L "- **Launch stalls.** On the clean VM, starting a freshly written executable"
L "  — an installer just staged, an engine or uninstaller copy just extracted"
L "  — occasionally stalled for 8–10 s before its first instruction, in every"
L "  technology (TigerSetup's *before engine* column in section D shows it"
L "  directly; for the others it shows as a 9 s process lifetime on sub-second"
L "  work). The campaign notes name the commands affected. Medians and"
L "  quartiles absorb it; a single row's ratio may not."
L "- **TigerSetup $tsVersion is frozen.** Nothing in the product was tuned on"
L "  these results; a weakness they expose is evidence for a later version."

[System.IO.File]::WriteAllText($OutputPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))
Write-Host "Wrote $OutputPath"

# --- CSV companions ---------------------------------------------------------
$buildCsv = @($build.builds | ForEach-Object {
        [pscustomobject][ordered]@{
            app = $_.app; technology = $_.technology; toolVersion = $_.toolVersion; toolSha256 = $_.toolSha256
            startedAt = (Iso $_.startedAt); finishedAt = (Iso $_.finishedAt)
            wallSeconds = $_.wallSeconds; cpuSeconds = $_.cpuSeconds; userSeconds = $_.userSeconds; kernelSeconds = $_.kernelSeconds
            cpuPerWall = $(if ([double] $_.wallSeconds -gt 0) { [math]::Round([double] $_.cpuSeconds / [double] $_.wallSeconds, 3) } else { $null })
            peakJobCommitBytes = $_.peakJobCommitBytes; peakProcessCommitBytes = $_.peakProcessCommitBytes
            peakTreeWorkingSetBytes = $_.peakTreeWorkingSetBytes; peakProcessWorkingSetBytes = $_.peakProcessWorkingSetBytes
            samples = $_.samples; processesSeen = $_.processesSeen; processesTotal = $_.processesTotal
            installerBytes = $_.installerBytes; installerSha256 = $_.installerSha256
            payloadFiles = (Get-Prop $_ 'payloadFiles'); payloadBytes = (Get-Prop $_ 'payloadBytes'); installerToPayloadRatio = (Get-Prop $_ 'installerToPayloadRatio')
            success = $_.success
        }
    })
$buildCsv | Export-Csv -LiteralPath (Join-Path $ResultsRoot 'build.csv') -NoTypeInformation -Encoding utf8
$sizesCsv = @($apps | ForEach-Object {
        $app = $_
        [pscustomobject][ordered]@{
            app = $app; version = $corpusApp[$app].version; files = (AppFiles $app); bytes = (AppBytes $app); averageFileBytes = $corpusApp[$app].averageFileBytes; largestFileBytes = $corpusApp[$app].largestFile.bytes
            innoSetupBytes = (Bytes $app 'InnoSetup'); nsisBytes = (Bytes $app 'NSIS'); tigerSetupBytes = (Bytes $app 'TigerSetup')
            tsVsInnoSetupPct = [math]::Round((DeltaPct (Bytes $app 'TigerSetup') (Bytes $app 'InnoSetup')), 2)
            tsVsNsisPct = [math]::Round((DeltaPct (Bytes $app 'TigerSetup') (Bytes $app 'NSIS')), 2)
            tsVsSmallerPct = [math]::Round($sizeDeltas[$app], 2); smaller = (SmallerName $app)
            innoSetupRatio = [math]::Round((Div (Bytes $app 'InnoSetup') (AppBytes $app)), 4); nsisRatio = [math]::Round((Div (Bytes $app 'NSIS') (AppBytes $app)), 4); tigerSetupRatio = [math]::Round((Div (Bytes $app 'TigerSetup') (AppBytes $app)), 4)
        }
    })
$sizesCsv | Export-Csv -LiteralPath (Join-Path $ResultsRoot 'sizes.csv') -NoTypeInformation -Encoding utf8
if ($labRows.Count -gt 0) {
    $runtimeCsv = @($labRows | ForEach-Object {
            $r = $_; $i = $r.install; $u = $r.uninstall; $ie = Get-Prop $i 'engine'; $ue = Get-Prop $u 'engine'
            [pscustomobject][ordered]@{
                app = $r.app; technology = $r.tech; installerSha256 = $r.installerSha256; installerBytes = $r.installerBytes
                canonicalFiles = $r.canonicalFiles; canonicalBytes = $r.canonicalBytes
                installSeconds = (Get-Prop $i 'durationSeconds'); installProcessSeconds = (Get-Prop $i 'processSeconds'); installCompletionWaitSeconds = (Get-Prop $i 'completionWaitSeconds'); installExitCode = (Get-Prop $i 'exitCode')
                installedFiles = (Get-Prop $i 'installedFiles'); installedBytes = (Get-Prop $i 'installedBytes')
                payloadExact = (Get-Prop $i 'payloadExact'); payloadMatched = (Get-Prop $i 'payloadMatched'); payloadMissing = @(Get-Prop $i 'payloadMissing').Count; payloadDiffering = @(Get-Prop $i 'payloadDiffering').Count
                extras = @(Get-Prop $i 'payloadExtras').Count; extraBytes = (Get-Prop $i 'payloadExtraBytes'); arpRegistered = (Get-Prop $i 'arpRegistered')
                uninstallSeconds = (Get-Prop $u 'durationSeconds'); uninstallProcessSeconds = (Get-Prop $u 'processSeconds'); uninstallCompletionWaitSeconds = (Get-Prop $u 'completionWaitSeconds'); uninstallExitCode = (Get-Prop $u 'exitCode')
                rootRemoved = (Get-Prop $u 'rootRemoved'); arpRemoved = (Get-Prop $u 'arpRemoved'); packageOwnedStateRemoved = (Get-Prop $u 'packageOwnedStateRemoved')
                engineInstallBefore = (Get-Prop $ie 'beforeSeconds'); engineInstallSpan = (Get-Prop $ie 'spanSeconds'); engineInstallAfter = (Get-Prop $ie 'afterSeconds')
                engineUninstallBefore = (Get-Prop $ue 'beforeSeconds'); engineUninstallSpan = (Get-Prop $ue 'spanSeconds'); engineUninstallAfter = (Get-Prop $ue 'afterSeconds')
                success = $r.success; session = $r.session
            }
        })
    $runtimeCsv | Export-Csv -LiteralPath (Join-Path $ResultsRoot 'runtime.csv') -NoTypeInformation -Encoding utf8
}
Write-Host "Wrote build.csv, sizes.csv$(if ($labRows.Count -gt 0) { ', runtime.csv' }) under $ResultsRoot"
