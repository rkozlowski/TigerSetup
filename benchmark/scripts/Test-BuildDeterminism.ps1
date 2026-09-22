#Requires -Version 7.0
<#
    .SYNOPSIS
    Rebuilds a few packages of a campaign a second time and records whether
    each technology produced byte-identical installers: a small
    representative determinism check rather than a second full campaign.

    .DESCRIPTION
    Runs Build-Installers.ps1 again for the named packages, with the same
    corpus record and definitions, into an artifacts root and a scratch
    results root of their own (the campaign's build.json and installers are
    not touched), then compares every rebuilt installer's SHA-256 and bytes
    with the campaign's record and writes determinism.json into the
    campaign's results root. The rebuild's timings are not kept: this is a
    question about bytes.

    .EXAMPLE
    pwsh -File benchmark\scripts\Test-BuildDeterminism.ps1 -Only minimal,notepadplusplus,gitforwindows
#>
[CmdletBinding()]
param(
    [string[]] $Only = @('minimal', 'notepadplusplus', 'gitforwindows'),
    [string] $CorpusJson = (Join-Path $PSScriptRoot '..\results\0.10.0-broad\corpus.json'),
    [string] $BroadPackagesRoot = (Join-Path $PSScriptRoot '..\packages\broad'),
    [string] $ArtifactsRoot = (Join-Path $PSScriptRoot '..\artifacts\0.10.0-broad'),
    [string] $ResultsRoot = (Join-Path $PSScriptRoot '..\results\0.10.0-broad')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Only = @($Only | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim().ToLowerInvariant() } | Where-Object { $_ })
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$ArtifactsRoot = [System.IO.Path]::GetFullPath($ArtifactsRoot)
$campaign = Get-Content -LiteralPath (Join-Path $ResultsRoot 'build.json') -Raw | ConvertFrom-Json

$rebuildArtifacts = "$ArtifactsRoot-determinism"
$rebuildResults = Join-Path $rebuildArtifacts 'results'
& (Join-Path $PSScriptRoot 'Build-Installers.ps1') -CorpusJson $CorpusJson -BroadPackagesRoot $BroadPackagesRoot `
    -ArtifactsRoot $rebuildArtifacts -ResultsRoot $rebuildResults -Only $Only
if ($LASTEXITCODE -ne 0) { throw "The rebuild failed (exit $LASTEXITCODE)." }
$rebuilt = Get-Content -LiteralPath (Join-Path $rebuildResults 'build.json') -Raw | ConvertFrom-Json

$checks = [System.Collections.Generic.List[object]]::new()
$allIdentical = $true
foreach ($second in @($rebuilt.builds)) {
    $first = @($campaign.builds | Where-Object { $_.app -eq $second.app -and $_.technology -eq $second.technology }) | Select-Object -First 1
    if ($null -eq $first) { throw "The campaign's build.json has no $($second.technology) build of $($second.app)." }
    $identical = ([string] $first.installerSha256 -eq [string] $second.installerSha256) -and ([long] $first.installerBytes -eq [long] $second.installerBytes)
    $allIdentical = $allIdentical -and $identical
    $checks.Add([ordered]@{
            app = [string] $second.app
            technology = [string] $second.technology
            firstSha256 = [string] $first.installerSha256
            firstBytes = [long] $first.installerBytes
            firstBuiltAt = [string] $first.startedAt
            secondSha256 = [string] $second.installerSha256
            secondBytes = [long] $second.installerBytes
            secondBuiltAt = [string] $second.startedAt
            identical = $identical
        })
    Write-Host ("  {0,-16} {1,-10} {2}" -f $second.app, $second.technology, $(if ($identical) { 'identical' } else { 'DIFFERENT' }))
}
[ordered]@{
    checkedAt = [DateTimeOffset]::Now.ToString('o')
    note = "Each package below was built a second time from the same definition and payload with the same compiler, into a separate artifacts root, and the two installers' SHA-256 and bytes compared: identical means the technology's output is deterministic for that input. The rebuild's timings are not part of the campaign."
    allIdentical = $allIdentical
    checks = @($checks)
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $ResultsRoot 'determinism.json') -Encoding utf8
Write-Host "Wrote $(Join-Path $ResultsRoot 'determinism.json') (all identical: $allIdentical)"
if (-not $allIdentical) { exit 1 }
exit 0
