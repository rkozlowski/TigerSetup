#Requires -Version 7.0
<#
    .SYNOPSIS
    Hashes and sizes every installer under a directory and writes a
    machine-readable JSON measurement file, so report.md never transcribes a
    byte count or hash by hand.

    .EXAMPLE
    pwsh -File benchmark\scripts\Measure-StaticProperties.ps1 `
        -InstallerDirectory benchmark\artifacts\minimal `
        -OutputJson benchmark\results\minimal-installers.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $InstallerDirectory,
    [Parameter(Mandatory)]
    [string] $OutputJson,
    [string] $Filter = '*.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$files = Get-ChildItem -LiteralPath $InstallerDirectory -Filter $Filter -File | Sort-Object Name
$records = foreach ($file in $files) {
    $hash = Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256
    [ordered]@{
        name       = $file.Name
        path       = $file.FullName
        bytes      = $file.Length
        sha256     = $hash.Hash.ToLowerInvariant()
        measuredAt = (Get-Date).ToString('o')
    }
}

$result = [ordered]@{
    directory = (Resolve-Path -LiteralPath $InstallerDirectory).Path
    installers = @($records)
}

$outDir = Split-Path -Parent $OutputJson
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    $null = New-Item -ItemType Directory -Path $outDir -Force
}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputJson -Encoding utf8
Write-Host "Wrote $OutputJson ($($records.Count) installer(s))"
