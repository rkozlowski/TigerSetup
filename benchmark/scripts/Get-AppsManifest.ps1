#Requires -Version 7.0
<#
    .SYNOPSIS
    Loads benchmark/results/apps.json, the pinned application version/source
    manifest, as PowerShell objects. Kept as one small helper so every
    acquisition/build script reads the pin from the same place.
#>
function Get-BenchmarkAppsManifest {
    [CmdletBinding()]
    param(
        [string] $Path = (Join-Path $PSScriptRoot '..\results\apps.json')
    )
    Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
}
