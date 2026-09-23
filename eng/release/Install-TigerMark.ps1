<#
    .SYNOPSIS
    Provisions tiger-mark, the TigerMarkView command line that renders the
    installed help's PDF, on a machine that does not have it registered - the
    release workflow's runner.

    .DESCRIPTION
    A developer machine resolves tiger-mark as the registered TigerMarkView tool
    (eng\TigerAiCore.psm1). A GitHub-hosted runner has no TigerAiCore
    configuration, so the release build installs one pinned TigerMarkView
    release instead: the published installer is downloaded, refused unless its
    SHA-256 is the pinned one, installed per user into -Directory without the
    PATH task, and the installed tiger-mark must report the pinned version.
    The path goes to GITHUB_OUTPUT as `path`, which the build passes to
    Build-Package.ps1 -TigerMarkPath.

    The installer needs the .NET 10 Desktop Runtime and the WebView2 Runtime,
    and tiger-mark renders through a shown (off-screen) WebView2 window;
    GitHub's Windows images carry both runtimes and run jobs in an interactive
    session.

    -VerifyOnly downloads and checks the installer and installs nothing: the
    local proof of the pin.

    .EXAMPLE
    pwsh -File eng\release\Install-TigerMark.ps1 -Version 0.8.1 -Sha256 <hash> -Directory $env:RUNNER_TEMP\tigermarkview
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [ValidatePattern('^\d+\.\d+\.\d+$')] [string] $Version,
    [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{64}$')] [string] $Sha256,
    [string] $Directory = (Join-Path ([IO.Path]::GetTempPath()) 'tigermarkview'),
    [switch] $VerifyOnly,
    [string] $GitHubOutput
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$name = "TigerMarkView-$Version-win-x64-setup.exe"
$uri = "https://github.com/rkozlowski/TigerMarkView/releases/download/v$Version/$name"
$download = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-release-$([guid]::NewGuid().ToString('N'))-$name"
try {
    Write-Host "Downloading $uri"
    Invoke-WebRequest -Uri $uri -OutFile $download -UseBasicParsing
    $actual = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash
    if ($actual -ne $Sha256.ToUpperInvariant()) {
        throw "$name hashes to $actual, not the pinned $($Sha256.ToUpperInvariant()); it is not installed."
    }
    Write-Host "$name matches the pinned SHA-256."
    if ($VerifyOnly) { return }

    $log = Join-Path ([IO.Path]::GetTempPath()) "tigermarkview-$Version-install.log"
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/NOICONS',
        '/MERGETASKS=!addtopath,!desktopicon', "/DIR=`"$Directory`"", "/LOG=`"$log`"")
    $process = Start-Process -FilePath $download -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        $tail = if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log -Tail 30) -join "`n" } else { '(no log)' }
        throw "The TigerMarkView installer exited $($process.ExitCode).`n$tail"
    }
}
finally {
    Remove-Item -LiteralPath $download -Force -ErrorAction SilentlyContinue
}

$tigerMark = Join-Path $Directory 'tiger-mark.exe'
if (-not (Test-Path -LiteralPath $tigerMark -PathType Leaf)) { throw "The installation holds no $tigerMark." }
$reported = (& $tigerMark --version 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $reported -notmatch "\b$([regex]::Escape($Version))\b") {
    throw "tiger-mark --version reported '$reported' (exit $LASTEXITCODE), not $Version."
}
Write-Host "tiger-mark: $tigerMark ($reported)"
if ($GitHubOutput) { "path=$tigerMark" | Out-File -LiteralPath $GitHubOutput -Encoding utf8 -Append }
exit 0
