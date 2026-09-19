#Requires -Version 7.0
<#
    .SYNOPSIS
    Downloads the pinned official release artifacts for every benchmark
    application into benchmark/downloads/<App>/, verifying any published
    hash and recording the actually-observed hash either way.

    .DESCRIPTION
    Reads benchmark/results/apps.json (Get-AppsManifest.ps1). For each app,
    downloads its "portable" artifact when the manifest names one (the
    canonical-payload source), and always also downloads the "installer"
    artifact (needed for qBittorrent's canonical payload, and kept for every
    app as the real upstream installer for reference/behavior checks).

    Idempotent: a file already present with the expected size is not
    re-downloaded unless -Force is passed.

    .EXAMPLE
    pwsh -File benchmark\scripts\Acquire-Downloads.ps1
    pwsh -File benchmark\scripts\Acquire-Downloads.ps1 -App qBittorrent -Force
#>
[CmdletBinding()]
param(
    [string] $App,
    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'Get-AppsManifest.ps1')
$manifest = Get-BenchmarkAppsManifest
$downloadRoot = Join-Path $PSScriptRoot '..\downloads'
$results = @()

foreach ($appEntry in $manifest.apps) {
    if ($App -and $appEntry.name -ne $App) { continue }
    $appDir = Join-Path $downloadRoot $appEntry.name
    $null = New-Item -ItemType Directory -Path $appDir -Force

    foreach ($kind in @('portable', 'installer')) {
        if ($appEntry.artifacts.PSObject.Properties.Match($kind).Count -eq 0) { continue }
        $artifact = $appEntry.artifacts.$kind
        if (-not $artifact) { continue }
        $destination = Join-Path $appDir $artifact.filename
        $needsDownload = $Force -or -not (Test-Path -LiteralPath $destination)
        if ($needsDownload) {
            Write-Host "Downloading $($appEntry.name) $kind : $($artifact.url)"
            Invoke-WebRequest -Uri $artifact.url -OutFile $destination -UseBasicParsing
        }
        else {
            Write-Host "Already present: $destination"
        }
        $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        $published = $null
        if ($artifact.PSObject.Properties.Match('sha256_published').Count -gt 0) {
            $published = $artifact.sha256_published.ToLowerInvariant()
        }
        elseif ($appEntry.PSObject.Properties.Match('sha256_published').Count -gt 0 -and
                $appEntry.sha256_published.PSObject.Properties.Match($artifact.filename).Count -gt 0) {
            $published = $appEntry.sha256_published.($artifact.filename).ToLowerInvariant()
        }
        $matched = $null
        if ($published) { $matched = ($published -eq $hash) }
        if ($published -and -not $matched) {
            throw "Hash mismatch for $destination : expected $published, got $hash"
        }
        $results += [ordered]@{
            app             = $appEntry.name
            kind            = $kind
            filename        = $artifact.filename
            path            = (Resolve-Path -LiteralPath $destination).Path
            bytes           = (Get-Item -LiteralPath $destination).Length
            sha256          = $hash
            sha256_published = $published
            hash_verified   = $matched
        }
    }
}

$outJson = Join-Path $PSScriptRoot '..\results\downloads.json'
$results | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $outJson -Encoding utf8
Write-Host "Wrote $outJson ($($results.Count) artifact(s))"
