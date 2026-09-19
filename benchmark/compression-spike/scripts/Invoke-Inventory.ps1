#Requires -Version 7.0
<#
    .SYNOPSIS
    Runs `cspike inventory` for every corpus payload (results/corpus-payloads.json)
    into results/inventory/<App>.json, a few apps at a time.
#>
[CmdletBinding()]
param(
    [string[]] $App,
    [int] $Parallel = 3,
    [switch] $Force
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$tool = Join-Path $root 'tool\target\x86_64-pc-windows-msvc\release\cspike.exe'
if (-not (Test-Path -LiteralPath $tool)) { throw "Build the spike tool first: cargo build --release in $root\tool" }
$payloads = Get-Content -LiteralPath (Join-Path $root 'results\corpus-payloads.json') -Raw | ConvertFrom-Json
$outDir = Join-Path $root 'results\inventory'
$null = New-Item -ItemType Directory -Path $outDir -Force
$jobs = @()
foreach ($p in $payloads) {
    if ($null -eq $p.payload) { continue }
    if ($App -and $p.app -notin $App) { continue }
    $out = Join-Path $outDir "$($p.app).json"
    if ((Test-Path -LiteralPath $out) -and -not $Force) { Write-Host "$($p.app): inventory present"; continue }
    while (@($jobs | Where-Object { $_.State -eq 'Running' }).Count -ge $Parallel) { Start-Sleep -Seconds 2 }
    $jobs += Start-Job -Name $p.app -ScriptBlock {
        param($tool, $app, $payload, $out)
        & $tool inventory --app $app --payload $payload --out $out 2>&1
    } -ArgumentList $tool, $p.app, $p.payload, $out
}
$jobs | Wait-Job | ForEach-Object { Receive-Job $_ } 
$jobs | Remove-Job
