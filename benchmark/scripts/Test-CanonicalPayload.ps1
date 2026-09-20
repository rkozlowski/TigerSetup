#Requires -Version 7.0
<#
    .SYNOPSIS
    Verifies that the canonical payloads on disk are byte-identical to the
    committed inventories (results/canonical-<App>.json): every inventoried
    file present with its recorded size and SHA-256, and no file the
    inventory does not name. A later campaign measures the same bytes as the
    first one only while this holds, so it runs before anything is built.

    .DESCRIPTION
    Writes a machine-readable verification record when -OutputJson is
    given, and exits 1 when any payload differs or is missing. Nothing is
    reacquired or repaired here: a differing payload is reported, and
    New-CanonicalPayload.ps1 is the tool that regenerates one.

    .EXAMPLE
    pwsh -File benchmark\scripts\Test-CanonicalPayload.ps1
    pwsh -File benchmark\scripts\Test-CanonicalPayload.ps1 -OutputJson benchmark\results\0.9.0\canonical-verification.json
#>
[CmdletBinding()]
param(
    [string[]] $Apps = @('ShareX', 'WinMerge', 'qBittorrent', 'VLC'),
    [string] $OutputJson
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$resultsRoot = Join-Path $benchmarkRoot 'results'
$Apps = @($Apps | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })

$records = [System.Collections.Generic.List[object]]::new()
$allOk = $true
foreach ($app in $Apps) {
    $inventoryPath = Join-Path $resultsRoot "canonical-$app.json"
    $inventory = Get-Content -LiteralPath $inventoryPath -Raw | ConvertFrom-Json
    # The inventory records the payload root as an absolute path of the
    # machine that derived it; the payload is looked for by its
    # repository-relative location so a checkout elsewhere verifies too.
    $payloadRoot = Join-Path $benchmarkRoot "canonical\$app\payload"
    $record = [ordered]@{
        app = $app
        inventory = $inventoryPath
        inventorySha256 = (Get-FileHash -LiteralPath $inventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        payloadRoot = $payloadRoot
        present = (Test-Path -LiteralPath $payloadRoot -PathType Container)
        expectedFiles = [int] $inventory.fileCount
        expectedBytes = [long] $inventory.totalBytes
        actualFiles = 0
        actualBytes = 0
        missing = @()
        unexpected = @()
        differing = @()
        identical = $false
        verifiedAt = [DateTimeOffset]::Now.ToString('o')
    }
    if ($record.present) {
        $expected = @{}
        foreach ($entry in $inventory.inventory) { $expected[[string] $entry.path] = $entry }
        $files = @(Get-ChildItem -LiteralPath $payloadRoot -Recurse -File)
        $record.actualFiles = $files.Count
        $record.actualBytes = [long] (($files | Measure-Object -Property Length -Sum).Sum)
        $seen = [System.Collections.Generic.HashSet[string]]::new()
        $missing = [System.Collections.Generic.List[string]]::new()
        $unexpected = [System.Collections.Generic.List[string]]::new()
        $differing = [System.Collections.Generic.List[string]]::new()
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($payloadRoot.Length).TrimStart('\').Replace('\', '/')
            $null = $seen.Add($relative)
            if (-not $expected.ContainsKey($relative)) { $unexpected.Add($relative); continue }
            $entry = $expected[$relative]
            if ([long] $entry.bytes -ne $file.Length) { $differing.Add("$relative (bytes $($file.Length), inventory $($entry.bytes))"); continue }
            $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($hash -ne ([string] $entry.sha256).ToLowerInvariant()) { $differing.Add("$relative (sha256 $hash, inventory $($entry.sha256))") }
        }
        foreach ($path in $expected.Keys) { if (-not $seen.Contains($path)) { $missing.Add($path) } }
        $record.missing = @($missing)
        $record.unexpected = @($unexpected)
        $record.differing = @($differing)
        $record.identical = ($missing.Count -eq 0 -and $unexpected.Count -eq 0 -and $differing.Count -eq 0 -and
            $record.actualFiles -eq $record.expectedFiles -and $record.actualBytes -eq $record.expectedBytes)
    }
    $allOk = $allOk -and $record.identical
    $verdict = if ($record.identical) { 'identical' } elseif (-not $record.present) { 'MISSING' } else { "DIFFERS (missing $($record.missing.Count), unexpected $($record.unexpected.Count), differing $($record.differing.Count))" }
    Write-Host ("  {0,-12} {1,6:N0} files {2,14:N0} bytes  {3}" -f $app, $record.actualFiles, $record.actualBytes, $verdict)
    $records.Add([pscustomobject] $record)
}

if (-not [string]::IsNullOrWhiteSpace($OutputJson)) {
    $outDir = Split-Path -Parent $OutputJson
    if ($outDir -and -not (Test-Path -LiteralPath $outDir)) { $null = New-Item -ItemType Directory -Path $outDir -Force }
    [ordered]@{
        verifiedAt = [DateTimeOffset]::Now.ToString('o')
        allIdentical = $allOk
        note = 'Each canonical payload on disk compared file by file (path, size, SHA-256) with its committed inventory; identical means the campaign measured the same application bytes as the inventory records.'
        payloads = @($records)
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputJson -Encoding utf8
    Write-Host "Wrote $OutputJson"
}
if (-not $allOk) { exit 1 }
exit 0
