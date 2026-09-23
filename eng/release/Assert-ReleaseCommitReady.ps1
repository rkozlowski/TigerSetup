<#
    .SYNOPSIS
    The release workflow's first gate: may this commit be released as this
    version? Nothing is built until it passes.

    .DESCRIPTION
    Checks, in order, and reports every result:

      version        the requested version is <Major>.<Minor>.<Patch> and is
                     exactly the workspace Cargo.toml version at this commit
      readme         README.md states that version
      notes         .github/release-notes/<version>.md exists and is useful
      commit/on-main the commit is reachable from origin/main (the human push
                     happened)
      ci/run         the CI push run for this exact commit succeeded
      tag/available  origin has no tag v<version>: a release is created once
                     and a tag is never moved

    Exit codes: 0 PASS, 2 BLOCKED (a human or external step has not happened
    yet: push, CI still running, gh not authenticated), 1 FAIL. A maintainer
    runs the same script before starting the release workflow; in the workflow
    GH_TOKEN authenticates gh with actions: read and contents: read.

    .EXAMPLE
    pwsh -File eng\release\Assert-ReleaseCommitReady.ps1 -Version 0.12.0
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    # The commit to release; defaults to HEAD.
    [ValidatePattern('^([0-9a-fA-F]{40})?$')] [string] $CommitSha = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$facts = Get-TigerSetupReleaseFacts
if (-not $CommitSha) { $CommitSha = (Invoke-TigerSetupGit $repoRoot @('rev-parse', 'HEAD')).output }
$CommitSha = $CommitSha.ToLowerInvariant()

$checks = [Collections.Generic.List[object]]::new()

$source = Get-TigerSetupSourceVersion -RepositoryRoot $repoRoot
$checks.Add($(if ((Test-TigerSetupReleaseVersion $Version) -and $source -ceq $Version) {
            New-TigerSetupReleaseCheck -Id 'version' -Status PASS -Observed "Cargo.toml records $Version."
        }
        else {
            New-TigerSetupReleaseCheck -Id 'version' -Status FAIL -Observed "Requested '$Version'; Cargo.toml records '$source'." -Remediation 'Request the exact <Major>.<Minor>.<Patch> version the release commit records.'
        }))
$checks.Add((Test-TigerSetupVersionReference -RepositoryRoot $repoRoot -Version $Version))
$checks.Add((Test-TigerSetupReleaseNotes -RepositoryRoot $repoRoot -Version $Version))
$checks.Add((Test-TigerSetupCommitOnMain -RepositoryRoot $repoRoot -CommitSha $CommitSha))
$checks.Add((Get-TigerSetupCiRunCheck -CommitSha $CommitSha))

$tag = Get-TigerSetupReleaseTag -Version $Version
$tagCommit = Get-TigerSetupRemoteTagCommit -RepositoryRoot $repoRoot -Tag $tag
$checks.Add($(if ($null -eq $tagCommit) {
            New-TigerSetupReleaseCheck -Id 'tag/available' -Status PASS -Observed "origin has no tag $tag."
        }
        else {
            New-TigerSetupReleaseCheck -Id 'tag/available' -Status FAIL -Observed "origin already has $tag at $tagCommit." -Remediation 'A version is released once. Release the next version instead (RELEASING.md, Recovery).'
        }))

$code = Write-TigerSetupReleaseReport -Title "Release prerequisites for TigerSetup $Version at $CommitSha" -Checks $checks.ToArray() `
    -Next @("Start '$($facts.ReleaseWorkflowName)' for $Version on $($facts.DefaultBranch).")
exit $code
