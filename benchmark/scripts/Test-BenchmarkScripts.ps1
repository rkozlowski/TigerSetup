#Requires -Version 7.0
<#
    .SYNOPSIS
    Focused checks of the benchmark machinery, before a campaign spends an
    hour of build time or an afternoon of lab time on it.

    .DESCRIPTION
    Six checks, all local and cheap:

    - **Static.** Every script and module under benchmark\scripts passes
      lab\Test-LabScripts.ps1 (syntax, undeclared reads under strict mode,
      shadowed parameters, jobs without a lease policy).
    - **The process-tree meter.** A command shell running a batch file that
      starts ping is measured as one tree: the exit code is the
      tree's, every process is counted, the wall clock spans the child's
      wait, CPU and commit are non-zero.
    - **The payload comparison.** Compare-InstalledPayload (BenchmarkRow.psm1)
      reports exact, missing, differing, extra and excluded entries
      correctly on a hand-made inventory.
    - **The package generator.** New-BroadPackages.ps1 is deterministic (a
      second run writes nothing) and the committed definitions under
      packages\broad are exactly what it generates from the campaign's
      corpus record.
    - **The report generator.** New-BroadReport.ps1 produces byte-identical
      output on a second run over the same results, and its CSV companions
      exist.
    - **The campaign record.** Every row of the committed lab-results.json
      resolves on its own: to the build record of the exact installer it
      ran, and to a canonical inventory whose document is committed and
      whose SHA-256 is the one the row recorded. That is what makes the raw
      lab evidence disposable (README.md, *What a campaign commits*).

    .EXAMPLE
    pwsh -File benchmark\scripts\Test-BenchmarkScripts.ps1
#>
[CmdletBinding()]
param(
    [string] $Campaign = '0.10.0-broad'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $benchmarkRoot '..')).Path
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) "tigersetup-benchmark-tests-$PID"
$null = New-Item -ItemType Directory -Path $scratch -Force
$failures = [System.Collections.Generic.List[string]]::new()
function Check { param([string] $Name, [bool] $Ok, [string] $Detail = '') if ($Ok) { Write-Host "  ok    $Name" } else { Write-Host "  FAIL  $Name $Detail"; $failures.Add("$Name $Detail") } }

try {
    # --- static ---------------------------------------------------------------
    Write-Host '== static checks'
    $scripts = @(Get-ChildItem -LiteralPath $PSScriptRoot -File -Include '*.ps1', '*.psm1' -Recurse | Where-Object { $_.Name -ne 'Test-BenchmarkScripts.ps1' })
    foreach ($script in $scripts) {
        $output = & pwsh -NoProfile -File (Join-Path $repoRoot 'lab\Test-LabScripts.ps1') -Path $script.FullName 2>&1 | Out-String
        Check "lint $($script.Name)" ($LASTEXITCODE -eq 0) ($output.Trim())
    }

    # --- the meter -------------------------------------------------------------
    Write-Host '== process-tree meter'
    Import-Module (Join-Path $PSScriptRoot 'ProcessTreeMeter.psm1') -Force
    $log = Join-Path $scratch 'meter.log'
    # A batch file that starts ping (a second process, alive for a second) and exits with 7.
    $batch = Join-Path $scratch 'tree.cmd'
    Set-Content -LiteralPath $batch -Value "@echo off`r`nping -n 2 127.0.0.1 > nul`r`nexit /b 7`r`n" -Encoding ascii
    $m = Invoke-MeasuredProcess -FilePath "$env:SystemRoot\System32\cmd.exe" -ArgumentList @('/c', $batch) -LogPath $log -SampleIntervalMilliseconds 20 -TimeoutSeconds 60
    Check 'meter: exit code of the tree' ($m.exitCode -eq 7) "got $($m.exitCode)"
    Check 'meter: not timed out' (-not $m.timedOut)
    Check 'meter: two processes (cmd, ping)' ($m.processesTotal -ge 2) "got $($m.processesTotal)"
    Check 'meter: wall spans the child wait (>= 0.9 s)' ($m.wallSeconds -ge 0.9) "got $($m.wallSeconds)"
    Check 'meter: cpu accounted' ($m.cpuSeconds -ge 0 -and $null -ne $m.cpuSeconds)
    Check 'meter: peak job commit > 0' ($m.peakJobCommitBytes -gt 0)
    Check 'meter: samples taken' ($m.samples -ge 10) "got $($m.samples)"
    Check 'meter: log written' (Test-Path -LiteralPath $log -PathType Leaf)

    # --- the payload comparison --------------------------------------------------
    Write-Host '== payload comparison'
    Import-Module (Join-Path $PSScriptRoot 'BenchmarkRow.psm1') -Force
    $canonical = [pscustomobject]@{ inventory = @(
            [pscustomobject]@{ path = 'a.txt'; bytes = 3; sha256 = 'aa' }
            [pscustomobject]@{ path = 'sub/b.bin'; bytes = 5; sha256 = 'bb' }
            [pscustomobject]@{ path = 'gone.txt'; bytes = 1; sha256 = 'cc' }
            [pscustomobject]@{ path = '.tigersetup/actions/x.ps1'; bytes = 9; sha256 = 'dd' }
        ) }
    $inventory = [pscustomobject]@{ hashes = @(
            [pscustomobject]@{ path = 'A.txt'; bytes = 3; sha256 = 'AA' }
            [pscustomobject]@{ path = 'sub/b.bin'; bytes = 5; sha256 = 'ee' }
            [pscustomobject]@{ path = 'unins000.exe'; bytes = 100; sha256 = 'ff' }
            [pscustomobject]@{ path = 'unins000.dat'; bytes = 50; sha256 = '00' }
        ) }
    $c = Compare-InstalledPayload -Inventory $inventory -Canonical $canonical -Exclude @('.tigersetup/*')
    Check 'compare: verified' ([bool] $c.verified)
    Check 'compare: not exact' (-not [bool] $c.exact)
    Check 'compare: expected counts the non-excluded entries' ($c.expected -eq 3) "got $($c.expected)"
    Check 'compare: case-insensitive match with lower-cased hash' ($c.matched -eq 1) "got $($c.matched)"
    Check 'compare: missing' (@($c.missing) -join ',' -eq 'gone.txt') "got $(@($c.missing) -join ',')"
    Check 'compare: differing' (@($c.differing) -join ',' -eq 'sub/b.bin') "got $(@($c.differing) -join ',')"
    Check 'compare: extras sorted, excluded entries not among them' ((@($c.extras) -join ',') -eq 'unins000.dat,unins000.exe') "got $(@($c.extras) -join ',')"
    Check 'compare: extra bytes' ($c.extraBytes -eq 150) "got $($c.extraBytes)"
    $exact = Compare-InstalledPayload -Inventory ([pscustomobject]@{ hashes = @([pscustomobject]@{ path = 'a.txt'; bytes = 3; sha256 = 'aa' }) }) -Canonical ([pscustomobject]@{ inventory = @([pscustomobject]@{ path = 'a.txt'; bytes = 3; sha256 = 'aa' }) })
    Check 'compare: exact when everything matches' ([bool] $exact.exact -and $exact.extras.Count -eq 0)
    $unverified = Compare-InstalledPayload -Inventory ([pscustomobject]@{ fileCount = 1 }) -Canonical $canonical
    Check 'compare: not verified without hashes' (-not [bool] $unverified.verified -and $null -eq $unverified.exact)

    # --- the package generator ---------------------------------------------------
    Write-Host '== package generator'
    $corpusJson = Join-Path $benchmarkRoot "results\$Campaign\corpus.json"
    if (Test-Path -LiteralPath $corpusJson -PathType Leaf) {
        $generated = Join-Path $scratch 'packages'
        $first = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'New-BroadPackages.ps1') -CorpusJson $corpusJson -PackagesRoot $generated 2>&1 | Out-String
        $second = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'New-BroadPackages.ps1') -CorpusJson $corpusJson -PackagesRoot $generated 2>&1 | Out-String
        Check 'generator: first run writes every definition' ($first -match '\(45 file\(s\) written\)') $first.Trim()
        Check 'generator: second run writes nothing' ($second -match '\(0 file\(s\) written\)') $second.Trim()
        $committed = Join-Path $benchmarkRoot 'packages\broad'
        $differences = [System.Collections.Generic.List[string]]::new()
        foreach ($file in @(Get-ChildItem -LiteralPath $generated -Recurse -File)) {
            $relative = $file.FullName.Substring($generated.Length).TrimStart('\')
            $other = Join-Path $committed $relative
            if (-not (Test-Path -LiteralPath $other -PathType Leaf)) { $differences.Add("$relative (not committed)"); continue }
            if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $other -Algorithm SHA256).Hash) { $differences.Add("$relative (differs)") }
        }
        Check 'generator: the committed definitions are what the corpus record generates' ($differences.Count -eq 0) ($differences -join '; ')
    }
    else { Write-Host "  skip  package generator (no $corpusJson)" }

    # --- the report generator ----------------------------------------------------
    Write-Host '== report generator'
    $resultsRoot = Join-Path $benchmarkRoot "results\$Campaign"
    if ((Test-Path -LiteralPath (Join-Path $resultsRoot 'build.json') -PathType Leaf) -and (Test-Path -LiteralPath (Join-Path $resultsRoot 'toolchain.json') -PathType Leaf)) {
        # The generator writes its CSVs beside the results, so it runs over a copy.
        $copy = Join-Path $scratch 'results'
        $null = New-Item -ItemType Directory -Path $copy -Force
        Copy-Item -Path (Join-Path $resultsRoot '*.json') -Destination $copy -Force
        $reportA = Join-Path $scratch 'report-a.md'; $reportB = Join-Path $scratch 'report-b.md'
        $null = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'New-BroadReport.ps1') -Campaign $Campaign -ResultsRoot $copy -OutputPath $reportA 2>&1 | Out-String
        $okA = $LASTEXITCODE -eq 0
        $null = & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'New-BroadReport.ps1') -Campaign $Campaign -ResultsRoot $copy -OutputPath $reportB 2>&1 | Out-String
        Check 'report: generates' ($okA -and (Test-Path -LiteralPath $reportA -PathType Leaf))
        Check 'report: deterministic' ((Get-FileHash -LiteralPath $reportA -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $reportB -Algorithm SHA256).Hash)
        Check 'report: build.csv and sizes.csv written' ((Test-Path -LiteralPath (Join-Path $copy 'build.csv')) -and (Test-Path -LiteralPath (Join-Path $copy 'sizes.csv')))
        $committedReport = Join-Path $benchmarkRoot "report-$Campaign.md"
        if (Test-Path -LiteralPath $committedReport -PathType Leaf) {
            Check 'report: the committed report is what the results generate' ((Get-FileHash -LiteralPath $reportA -Algorithm SHA256).Hash -eq (Get-FileHash -LiteralPath $committedReport -Algorithm SHA256).Hash)
        }
    }
    else { Write-Host "  skip  report generator (no build.json under $resultsRoot)" }

    # --- the campaign record -----------------------------------------------------
    Write-Host '== campaign record'
    $labResultsPath = Join-Path $resultsRoot 'lab-results.json'
    if (Test-Path -LiteralPath $labResultsPath -PathType Leaf) {
        $lab = Get-Content -LiteralPath $labResultsPath -Raw | ConvertFrom-Json
        $builds = @{}
        foreach ($b in @((Get-Content -LiteralPath (Join-Path $resultsRoot 'build.json') -Raw | ConvertFrom-Json).builds)) {
            $builds["$($b.app)-$($b.technology)"] = $b
        }
        $inventoryHash = @{}
        $unresolved = [System.Collections.Generic.List[string]]::new()
        foreach ($row in @($lab.rows)) {
            $name = [string] $row.row
            # The exact bytes the row ran, as the build record has them.
            if (-not $builds.ContainsKey($name)) { $unresolved.Add("${name}: no build record") }
            else {
                $built = $builds[$name]
                if ([string] $built.installerSha256 -ne [string] $row.installerSha256) { $unresolved.Add("${name}: installer SHA-256 is not build.json's") }
                if ([long] $built.installerBytes -ne [long] $row.installerBytes) { $unresolved.Add("${name}: installer bytes are not build.json's") }
                if ([string] $built.toolVersion -ne [string] $row.toolVersion) { $unresolved.Add("${name}: tool version is not build.json's") }
            }
            # The canonical inventory the payload verdict was decided against.
            $anchor = Get-Prop $row 'canonicalInventory'
            if ($null -eq $anchor) { $unresolved.Add("${name}: no canonicalInventory"); continue }
            $relative = [string] (Get-Prop $anchor 'path')
            $file = Join-Path $resultsRoot $relative
            if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { $unresolved.Add("${name}: canonical inventory '$relative' is not committed"); continue }
            if (-not $inventoryHash.ContainsKey($relative)) { $inventoryHash[$relative] = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() }
            if ($inventoryHash[$relative] -ne ([string] (Get-Prop $anchor 'sha256')).ToLowerInvariant()) { $unresolved.Add("${name}: canonical inventory '$relative' is not the document the row recorded") }
            # The verdicts the raw evidence would otherwise have to be read
            # for: present, whatever their value on a row that failed.
            $install = Get-Prop $row 'install'
            $uninstall = Get-Prop $row 'uninstall'
            foreach ($field in 'jobId', 'payloadVerified', 'payloadExact', 'payloadExpected', 'payloadMatched', 'payloadMissing', 'payloadDiffering', 'payloadExtras', 'arpRegistered') {
                if ($null -eq $install -or $install.PSObject.Properties.Match($field).Count -eq 0) { $unresolved.Add("${name}: install.$field is absent") }
            }
            foreach ($field in 'jobId', 'rootRemoved', 'arpRemoved', 'packageOwnedStateRemoved', 'residue') {
                if ($null -eq $uninstall -or $uninstall.PSObject.Properties.Match($field).Count -eq 0) { $unresolved.Add("${name}: uninstall.$field is absent") }
            }
        }
        Check "record: every row resolves to its installer, its canonical inventory and its verdicts ($(@($lab.rows).Count) rows)" ($unresolved.Count -eq 0) (@($unresolved | Select-Object -First 5) -join '; ')
    }
    else { Write-Host "  skip  campaign record (no lab-results.json under $resultsRoot)" }
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
if ($failures.Count -gt 0) { Write-Host "benchmark scripts: $($failures.Count) failure(s)."; exit 1 }
Write-Host 'benchmark scripts: all checks passed.'
exit 0
