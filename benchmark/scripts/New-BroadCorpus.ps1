#Requires -Version 7.0
<#
    .SYNOPSIS
    Enumerates and verifies the broad benchmark corpus — the fifteen real
    application payloads the compression spike pinned — and writes the
    campaign's corpus record and a per-file SHA-256 inventory of every
    payload, so the runtime rows can verify an installed payload file by
    file.

    .DESCRIPTION
    The corpus is not defined here. It is read from the compression spike's
    committed evidence (benchmark/compression-spike/results/corpus-payloads.json,
    the payload trees it materialized, and results/inventory/<App>.json, its
    per-file inventory of each tree), and every payload on disk is checked
    against that evidence before anything is built from it:

      - the tree is present at the location the spike recorded;
      - its file count and byte total are the ones corpus-payloads.json
        records;
      - the set of (path, bytes) pairs is exactly the spike inventory's;
      - the four payloads the installer-technology benchmark derived first
        (ShareX, WinMerge, qBittorrent, VLC) are additionally verified
        file by file against their committed SHA-256 inventories
        (benchmark/results/canonical-<App>.json).

    A payload that fails any check is reported and the script exits 1;
    nothing is reacquired, renormalized or repaired here. GIMP, which the
    spike recorded as unavailable, stays excluded with the spike's reason.

    The corpus record (corpus.json) carries, per application, the shape
    facts the report explains outliers with — bytes, files, average and
    largest file, and the file-family and extension mix read from the spike
    inventory (no new classifier) — and the identity of the evidence each
    payload was verified against. The inventories (canonical/<App>.json) have
    the same shape as benchmark/results/canonical-<App>.json.

    .EXAMPLE
    pwsh -File benchmark\scripts\New-BroadCorpus.ps1 -ResultsRoot benchmark\results\0.10.0-broad
#>
[CmdletBinding()]
param(
    [string] $ResultsRoot = (Join-Path $PSScriptRoot '..\results\0.10.0-broad'),
    [string] $SpikeResults = (Join-Path $PSScriptRoot '..\compression-spike\results')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$benchmarkRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$SpikeResults = (Resolve-Path -LiteralPath $SpikeResults).Path
$inventoryRoot = Join-Path $ResultsRoot 'canonical'
$null = New-Item -ItemType Directory -Path $inventoryRoot -Force

$corpusPayloadsPath = Join-Path $SpikeResults 'corpus-payloads.json'
$corpusPath = Join-Path $SpikeResults 'corpus.json'
$spikePayloads = @(Get-Content -LiteralPath $corpusPayloadsPath -Raw | ConvertFrom-Json)
$spikeCorpus = Get-Content -LiteralPath $corpusPath -Raw | ConvertFrom-Json

function Get-Prop { param([object] $Object, [string] $Name) if ($null -eq $Object -or $Object.PSObject.Properties.Match($Name).Count -eq 0) { return $null }; $Object.$Name }

function ConvertTo-PackageVersion {
    <#
        TigerSetup requires a strict major.minor.patch version; a fourth
        component (WinMerge 2.16.58.2, Tiger3dForge 0.17.0.2, Git for
        Windows 2.55.0.5) is dropped for every technology alike, so the three
        packages of one application carry one identity.
    #>
    param([string] $Version)
    $parts = @($Version -split '\.')
    if ($parts.Count -lt 3) { $parts += @('0') * (3 - $parts.Count) }
    ($parts[0..2] -join '.')
}

function Get-RelativePayloadPath {
    <# The payload's location relative to benchmark/, so no record carries a machine path. #>
    param([string] $Absolute)
    $full = [System.IO.Path]::GetFullPath($Absolute)
    if (-not $full.StartsWith($benchmarkRoot, [System.StringComparison]::OrdinalIgnoreCase)) { throw "Payload '$full' is not under '$benchmarkRoot'." }
    $full.Substring($benchmarkRoot.Length).TrimStart('\').Replace('\', '/')
}

$apps = [System.Collections.Generic.List[object]]::new()
$excluded = [System.Collections.Generic.List[object]]::new()
$allVerified = $true
foreach ($entry in $spikePayloads) {
    $name = [string] $entry.app
    if ($null -eq (Get-Prop $entry 'payload')) {
        $excluded.Add([ordered]@{ app = $name; version = [string] $entry.version; reason = (@($entry.normalizations) -join ' ') })
        Write-Host ("  {0,-16} excluded: {1}" -f $name, ((@($entry.normalizations) -join ' ').Substring(0, 60) + '…'))
        continue
    }
    $payloadRoot = [System.IO.Path]::GetFullPath([string] $entry.payload)
    $spikeInventory = Get-Content -LiteralPath (Join-Path $SpikeResults "inventory\$name.json") -Raw | ConvertFrom-Json
    $corpusEntry = @($spikeCorpus.apps | Where-Object { $_.name -eq $name }) | Select-Object -First 1

    $verification = [ordered]@{
        present = (Test-Path -LiteralPath $payloadRoot -PathType Container)
        expectedFiles = [int] $entry.files
        expectedBytes = [long] $entry.bytes
        actualFiles = 0
        actualBytes = 0
        countMatches = $false
        bytesMatch = $false
        inventoryMatches = $false      # (path, bytes) set equals the spike inventory's
        inventoryMissing = @()
        inventoryUnexpected = @()
        inventoryDiffering = @()
        sha256Evidence = $null         # the committed SHA-256 inventory this payload was verified against, where one exists
        sha256Matches = $null
        sha256Differing = @()
        verifiedAt = [DateTimeOffset]::Now.ToString('o')
    }
    $inventory = [System.Collections.Generic.List[object]]::new()
    $largest = $null
    if ($verification.present) {
        $files = @(Get-ChildItem -LiteralPath $payloadRoot -Recurse -File -Force | Sort-Object FullName)
        $verification.actualFiles = $files.Count
        $verification.actualBytes = [long] (($files | Measure-Object -Property Length -Sum).Sum)
        $verification.countMatches = ($verification.actualFiles -eq $verification.expectedFiles)
        $verification.bytesMatch = ($verification.actualBytes -eq $verification.expectedBytes)

        $expected = @{}
        foreach ($file in @($spikeInventory.files)) { $expected[[string] $file.path] = [long] $file.bytes }
        $seen = [System.Collections.Generic.HashSet[string]]::new()
        $missing = [System.Collections.Generic.List[string]]::new()
        $unexpected = [System.Collections.Generic.List[string]]::new()
        $differing = [System.Collections.Generic.List[string]]::new()
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($payloadRoot.Length).TrimStart('\').Replace('\', '/')
            $null = $seen.Add($relative)
            $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            $inventory.Add([ordered]@{ path = $relative; bytes = [long] $file.Length; sha256 = $hash })
            if ($null -eq $largest -or $file.Length -gt $largest.bytes) { $largest = [ordered]@{ path = $relative; bytes = [long] $file.Length } }
            if (-not $expected.ContainsKey($relative)) { $unexpected.Add($relative); continue }
            if ($expected[$relative] -ne [long] $file.Length) { $differing.Add("$relative (bytes $($file.Length), inventory $($expected[$relative]))") }
        }
        foreach ($path in $expected.Keys) { if (-not $seen.Contains($path)) { $missing.Add($path) } }
        $verification.inventoryMissing = @($missing)
        $verification.inventoryUnexpected = @($unexpected)
        $verification.inventoryDiffering = @($differing)
        $verification.inventoryMatches = ($missing.Count -eq 0 -and $unexpected.Count -eq 0 -and $differing.Count -eq 0)

        # The four payloads the first benchmark derived have a committed
        # SHA-256 inventory; the hashes just read are compared with it.
        $committed = Join-Path $benchmarkRoot "results\canonical-$name.json"
        if (Test-Path -LiteralPath $committed -PathType Leaf) {
            $committedInventory = Get-Content -LiteralPath $committed -Raw | ConvertFrom-Json
            $verification.sha256Evidence = "results/canonical-$name.json (sha256 $((Get-FileHash -LiteralPath $committed -Algorithm SHA256).Hash.ToLowerInvariant()))"
            $committedHashes = @{}
            foreach ($file in @($committedInventory.inventory)) { $committedHashes[[string] $file.path] = ([string] $file.sha256).ToLowerInvariant() }
            $shaDiffering = [System.Collections.Generic.List[string]]::new()
            foreach ($file in $inventory) {
                if (-not $committedHashes.ContainsKey($file.path)) { $shaDiffering.Add("$($file.path) (not in the committed inventory)"); continue }
                if ($committedHashes[$file.path] -ne $file.sha256) { $shaDiffering.Add("$($file.path) (sha256 $($file.sha256), committed $($committedHashes[$file.path]))") }
            }
            $verification.sha256Differing = @($shaDiffering)
            $verification.sha256Matches = ($shaDiffering.Count -eq 0 -and $committedHashes.Count -eq $inventory.Count)
        }
    }
    $verified = $verification.present -and $verification.countMatches -and $verification.bytesMatch -and $verification.inventoryMatches -and ($null -eq $verification.sha256Matches -or $verification.sha256Matches)
    $verification.verified = $verified
    $allVerified = $allVerified -and $verified

    # Shape facts from the spike inventory: no new classifier, the families
    # and extensions cspike recorded (family: native, managed, text,
    # resources, precompressed, other).
    $byFamily = [ordered]@{}
    foreach ($property in @((Get-Prop $spikeInventory 'by_family').PSObject.Properties | Sort-Object Name)) {
        $byFamily[$property.Name] = [ordered]@{ files = [int] $property.Value.files; bytes = [long] $property.Value.bytes }
    }
    $byExtension = @($spikeInventory.files | Group-Object { ([string] $_.ext).ToLowerInvariant() } | ForEach-Object {
            [ordered]@{ ext = $(if ($_.Name -eq '') { '(none)' } else { $_.Name }); files = $_.Count; bytes = [long] (($_.Group | Measure-Object -Property bytes -Sum).Sum) }
        } | Sort-Object { - $_.bytes } | Select-Object -First 8)
    $fileCount = [int] $verification.actualFiles

    # A payload decomposed from a shipped TigerSetup installer carries that
    # installer's own packaged actions under .tigersetup/, a directory
    # TigerSetup reserves for its entries and refuses in a payload. Those
    # files are the previous installer's bookkeeping, not the application's,
    # and no package installs them: every technology excludes the directory
    # (contract.md), and a row verifies the installed tree against the
    # inventory without them.
    $reserved = @($inventory | Where-Object { $_.path -like '.tigersetup/*' })
    $packageExcludes = @(if ($reserved.Count -gt 0) { '.tigersetup/**' } else { @() })
    $reservedBytes = [long] 0
    foreach ($file in $reserved) { $reservedBytes += [long] $file.bytes }
    $record = [ordered]@{
        app = $name
        version = [string] $entry.version
        packageVersion = (ConvertTo-PackageVersion ([string] $entry.version))
        license = [string] $entry.license
        origin = [string] $(if ($null -ne $corpusEntry) { Get-Prop $corpusEntry 'source' } else { '' })
        source = [string] $entry.source
        payloadRoot = (Get-RelativePayloadPath $payloadRoot)
        normalizations = @($entry.normalizations)
        files = $fileCount
        bytes = [long] $verification.actualBytes
        averageFileBytes = $(if ($fileCount -gt 0) { [long] [math]::Round([double] $verification.actualBytes / $fileCount) } else { 0 })
        largestFile = $largest
        byFamily = $byFamily
        topExtensionsByBytes = @($byExtension)
        inventory = "canonical/$name.json"
        packageExcludes = $packageExcludes
        packageExcludedFiles = $reserved.Count
        packageExcludedBytes = $reservedBytes
        packageExcludeReason = $(if ($reserved.Count -gt 0) { '.tigersetup/ holds the shipped TigerSetup installer''s own packaged actions, a directory TigerSetup reserves and refuses in a payload; excluded from all three packages, not from the corpus record.' } else { '' })
        packageFiles = ($fileCount - $reserved.Count)
        packageBytes = ([long] $verification.actualBytes - $reservedBytes)
        verification = $verification
    }
    $apps.Add($record)

    $verdict = if ($verified) { 'verified' } elseif (-not $verification.present) { 'MISSING' } else { "DIFFERS (count $($verification.countMatches), bytes $($verification.bytesMatch), inventory $($verification.inventoryMatches), sha256 $($verification.sha256Matches))" }
    Write-Host ("  {0,-16} {1,6:N0} files {2,14:N0} bytes  {3}{4}" -f $name, $fileCount, $verification.actualBytes, $verdict, $(if ($null -ne $verification.sha256Matches) { ' (SHA-256 against the committed inventory)' } else { '' }))

    [ordered]@{
        app = $name
        source = [string] $entry.source
        payloadRoot = (Get-RelativePayloadPath $payloadRoot)
        fileCount = $fileCount
        totalBytes = [long] $verification.actualBytes
        normalizations = @($entry.normalizations)
        inventory = @($inventory)
    } | ConvertTo-Json -Depth 4 -Compress | Set-Content -LiteralPath (Join-Path $inventoryRoot "$name.json") -Encoding utf8
}

$totalFiles = 0; $totalBytes = [long] 0
foreach ($app in $apps) { $totalFiles += [int] $app.files; $totalBytes += [long] $app.bytes }
[ordered]@{
    capturedAt = [DateTimeOffset]::Now.ToString('o')
    note = 'The broad benchmark corpus: the fifteen real application payloads the compression spike pinned (benchmark/compression-spike/results/corpus.json, corpus-payloads.json), verified on disk against the spike inventory (path and bytes per file) and, for the four payloads the first benchmark derived, against their committed SHA-256 inventories. GIMP stays excluded with the spike''s reason. canonical/<App>.json is each payload''s per-file SHA-256 inventory, read here, the evidence a runtime row verifies an installed payload against.'
    spikeEvidence = [ordered]@{
        corpus = "compression-spike/results/corpus.json (sha256 $((Get-FileHash -LiteralPath $corpusPath -Algorithm SHA256).Hash.ToLowerInvariant()))"
        payloads = "compression-spike/results/corpus-payloads.json (sha256 $((Get-FileHash -LiteralPath $corpusPayloadsPath -Algorithm SHA256).Hash.ToLowerInvariant()))"
    }
    allVerified = $allVerified
    applications = $apps.Count
    totalFiles = $totalFiles
    totalBytes = $totalBytes
    apps = @($apps)
    excluded = @($excluded)
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $ResultsRoot 'corpus.json') -Encoding utf8
Write-Host ("Corpus: {0} applications, {1:N0} files, {2:N0} bytes; all verified: {3}; wrote {4}" -f $apps.Count, $totalFiles, $totalBytes, $allVerified, (Join-Path $ResultsRoot 'corpus.json'))
if (-not $allVerified) { exit 1 }
exit 0
