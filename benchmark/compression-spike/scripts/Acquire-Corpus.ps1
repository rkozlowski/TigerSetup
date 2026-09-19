#Requires -Version 7.0
<#
    .SYNOPSIS
    Downloads the pinned release artifacts of the spike corpus's additional
    OSS applications into benchmark/compression-spike/corpus/downloads/,
    verifying the published SHA-256 of each.

    .DESCRIPTION
    Reads results/corpus.json; only apps with source = "download" are
    fetched. Idempotent: a present file is hashed, not re-downloaded, unless
    -Force. Every observed hash is recorded in results/corpus-downloads.json.
    WinSCP's SourceForge URL serves the bytes only to a non-browser
    User-Agent, which the manifest names.
#>
[CmdletBinding()]
param(
    [string] $App,
    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$manifest = Get-Content -LiteralPath (Join-Path $root 'results\corpus.json') -Raw | ConvertFrom-Json
$downloads = Join-Path $root 'corpus\downloads'
$null = New-Item -ItemType Directory -Path $downloads -Force
$results = @()

foreach ($entry in $manifest.apps) {
    if ($entry.source -ne 'download') { continue }
    if ($App -and $entry.name -ne $App) { continue }
    $destination = Join-Path $downloads $entry.filename
    if ($Force -or -not (Test-Path -LiteralPath $destination)) {
        Write-Host "Downloading $($entry.name): $($entry.url)"
        $args = @('-L', '--fail', '--retry', '5', '--retry-delay', '5', '-o', $destination, $entry.url)
        if ($entry.PSObject.Properties.Match('user_agent').Count -gt 0) { $args = @('-A', $entry.user_agent) + $args }
        & curl.exe @args
        if ($LASTEXITCODE -ne 0) { throw "curl failed ($LASTEXITCODE) for $($entry.url)" }
    }
    else {
        Write-Host "Already present: $destination"
    }
    $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    $published = $entry.sha256_published.ToLowerInvariant()
    if ($hash -ne $published) { throw "Hash mismatch for $destination : expected $published, got $hash" }
    $results += [ordered]@{
        app              = $entry.name
        filename         = $entry.filename
        url              = $entry.url
        bytes            = (Get-Item -LiteralPath $destination).Length
        sha256           = $hash
        sha256_published = $published
        hash_verified    = $true
    }
}

$out = Join-Path $root 'results\corpus-downloads.json'
$existing = @()
if ((Test-Path -LiteralPath $out) -and $App) {
    $existing = @(Get-Content -LiteralPath $out -Raw | ConvertFrom-Json | Where-Object { $_.app -ne $App })
}
@($existing + $results) | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $out -Encoding utf8
Write-Host "Wrote $out"
