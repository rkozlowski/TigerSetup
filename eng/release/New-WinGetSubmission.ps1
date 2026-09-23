<#
    .SYNOPSIS
    Prepares the winget-pkgs submission of a published release from its own
    manifest set.

    .DESCRIPTION
    Runs after the Architect has published the release and after
    Get-ReleaseArtifacts.ps1 has retrieved and proved its assets into
    artifacts\release\<version>\assets. It proves again that those assets
    are the release workflow's (Assert-TigerSetupReleaseProvenance: the
    record, the checksums and the tag made for that record) and that the
    published URL serves the installer's exact bytes anonymously - WinGet
    downloads that URL and refuses a different hash. It then takes the
    manifest set out of the proven TigerSetup-<version>-WinGet.zip afresh,
    never from a directory anyone could have edited, and commits it to a
    branch of the winget-pkgs clone (New-TigerSetupWinGetSubmission).

    -Push pushes the branch to the clone's origin (the publisher's fork) and
    prints the compare URL; opening the pull request stays with a person.
    Nothing here edits a manifest: a set a moderator asks to change is a new
    set, generated, validated in the lab and recorded again (RELEASING.md).

    .EXAMPLE
    pwsh -File eng\release\New-WinGetSubmission.ps1 -Version 0.12.0 -WinGetPkgsRoot <winget-pkgs clone> -Push
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    # A clone of the publisher's winget-pkgs fork with an 'upstream' remote.
    [Parameter(Mandatory)] [string] $WinGetPkgsRoot,
    # Where Get-ReleaseArtifacts.ps1 put the release; defaults to artifacts\release\<version>.
    [string] $ReleaseDirectory,
    [switch] $Push
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $ReleaseDirectory) { $ReleaseDirectory = Join-Path $repoRoot "artifacts\release\$Version" }
$assetsDirectory = Join-Path $ReleaseDirectory 'assets'
$record = Assert-TigerSetupReleaseProvenance -RepositoryRoot $repoRoot -Directory $assetsDirectory -Version $Version
$installer = @($record.artifacts | Where-Object kind -CEQ 'WindowsInstaller')[0]
$archive = @($record.artifacts | Where-Object kind -CEQ 'WinGetManifests')[0]

$url = Get-TigerSetupInstallerUrl -Version $Version
$public = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-public-$([guid]::NewGuid().ToString('N')).exe"
try {
    try { Invoke-WebRequest -Uri $url -OutFile $public -UseBasicParsing }
    catch { throw "$url is not publicly served ($($_.Exception.Message)); publish the release first." }
    if ((Get-TigerSetupFileSha256 $public) -cne [string] $installer.sha256) { throw "$url does not serve the release's installer bytes." }
}
finally { Remove-Item -LiteralPath $public -Force -ErrorAction SilentlyContinue }

$manifests = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-winget-$([guid]::NewGuid().ToString('N'))"
try {
    Expand-Archive -LiteralPath (Join-Path $assetsDirectory $archive.name) -DestinationPath $manifests
    $submission = New-TigerSetupWinGetSubmission -WinGetPkgsRoot $WinGetPkgsRoot -ManifestDirectory $manifests -Version $Version -Push:$Push
}
finally { Remove-Item -LiteralPath $manifests -Recurse -Force -ErrorAction SilentlyContinue }
$checks = @(
    New-TigerSetupReleaseCheck -Id 'public' -Status PASS -Observed "$url serves the release's installer ($($installer.sha256))."
    New-TigerSetupReleaseCheck -Id 'branch' -Status PASS -Observed "$($submission.kind): $($submission.path) on $($submission.branch) at $($submission.commit)$(if ($submission.pushed) { ', pushed to origin' } else { ', not pushed' })."
)
exit (Write-TigerSetupReleaseReport -Title "WinGet submission of TigerSetup $Version" -Checks $checks -Next $(if ($submission.pushed) {
            @("Open the pull request: $($submission.compareUrl)")
        }
        else {
            @("Push $($submission.branch) to the fork (rerun with -Push), then open the pull request against microsoft/winget-pkgs.")
        }))
