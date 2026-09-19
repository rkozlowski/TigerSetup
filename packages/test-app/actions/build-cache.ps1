param([string] $Root, [string] $Version, [string] $Actions)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Root -PathType Container)) { throw "install root $Root is missing" }
New-Item -ItemType Directory -Force -Path $Actions | Out-Null
Set-Content -LiteralPath (Join-Path $Actions 'cache.txt') -Value "TigerSetupTestApp cache for $Version" -Encoding ASCII
Add-Content -LiteralPath (Join-Path $Actions 'record.txt') -Value "build-cache`t$Version`t$env:TIGERSETUP_OPERATION`t$env:TIGERSETUP_PHASE`t$env:TIGERSETUP_QUIET" -Encoding UTF8
Write-Output "cache built for $Version"
