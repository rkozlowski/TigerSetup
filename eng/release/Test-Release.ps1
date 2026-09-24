<#
    .SYNOPSIS
    Focused tests for the release tooling in eng\release.

    .DESCRIPTION
    Everything runs against synthetic repositories under the temporary
    directory - local bare repositories stand in for origin, for the
    winget-pkgs fork and for its upstream - and against a scripted stand-in
    for gh, so no test reaches GitHub, builds anything or touches the real
    checkout. Git identity comes from the environment for the duration of the
    run, because a CI runner has none.

    Scenarios:

      version      the <Major>.<Minor>.<Patch> rule and the Cargo.toml reader
      notes        the release-notes gate: missing, BOM, heading, thin,
                   placeholder, leak, good
      record       release-artifacts.json and SHA256SUMS.txt: written over the
                   closed set, and refused for a changed byte, a missing or
                   foreign file, another commit, another version, or a record
                   that changed in transit
      git          the commit-on-main gate and the tag lookup (annotated,
                   lightweight, absent)
      prerequisites Assert-ReleaseCommitReady.ps1 as the workflow runs it:
                   PASS for a pushed release commit, BLOCKED before the push,
                   FAIL for another version, missing notes or an existing
                   tag - and never a call to GitHub
      release      the GitHub Release lookup: draft by tag, absent, ambiguous
      publish      Publish-DraftRelease.ps1: first run tags and creates the
                   draft; a rerun over a compatible draft uploads only what is
                   missing; a changed byte, a published release, a foreign
                   asset and a tag at another commit are refused
      winget       the winget-pkgs submission: branch and commit, an idempotent
                   rerun, push to the fork, and refusal of a changed set, a
                   version upstream already has, a dirty clone and a foreign
                   upstream

    .EXAMPLE
    pwsh -File eng\release\Test-Release.ps1
#>
#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1') -Force
$script:failures = [Collections.Generic.List[string]]::new()
$script:checks = 0
$script:scenario = ''
$script:root = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-release-tests-$([guid]::NewGuid().ToString('N'))"
$null = New-Item -ItemType Directory -Path $script:root

$identity = @{ GIT_AUTHOR_NAME = 'Release Test'; GIT_AUTHOR_EMAIL = 'release-test@example.invalid'; GIT_COMMITTER_NAME = 'Release Test'; GIT_COMMITTER_EMAIL = 'release-test@example.invalid' }
$savedEnvironment = @{}
foreach ($name in @($identity.Keys) + @('GITHUB_ACTIONS', 'GITHUB_WORKFLOW', 'GITHUB_SHA', 'GITHUB_STEP_SUMMARY')) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name) }
foreach ($name in $identity.Keys) { [Environment]::SetEnvironmentVariable($name, $identity[$name]) }
[Environment]::SetEnvironmentVariable('GITHUB_ACTIONS', $null)
[Environment]::SetEnvironmentVariable('GITHUB_STEP_SUMMARY', $null)

function Start-Scenario { param([string] $Name) $script:scenario = $Name; Write-Host "  $Name" }

function Assert-True {
    param([Parameter(Mandatory)] [bool] $Condition, [Parameter(Mandatory)] [string] $Message)
    $script:checks++
    if (-not $Condition) { $script:failures.Add("$($script:scenario): $Message"); Write-Host "    FAIL  $Message" }
}

function Assert-Throws {
    param([Parameter(Mandatory)] [scriptblock] $Action, [Parameter(Mandatory)] [string] $Pattern, [Parameter(Mandatory)] [string] $Message)
    $thrown = $null
    try { $null = & $Action } catch { $thrown = $_.Exception.Message }
    Assert-True ($null -ne $thrown -and $thrown -match $Pattern) "$Message (threw: '$thrown')"
}

function Invoke-Git {
    # Plain $args, not bound parameters: git's own -d, -a and -m must not
    # prefix-match a PowerShell parameter such as -Debug.
    $Path = $args[0]
    $Rest = @($args | Select-Object -Skip 1 | ForEach-Object { "$_" })
    $output = & git -C $Path @Rest 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($Rest -join ' ') failed in ${Path}: $output" }
    ($output | Out-String).Trim()
}

function New-Directory { param([string] $Name) $path = Join-Path $script:root $Name; $null = New-Item -ItemType Directory -Path $path -Force; $path }

function New-Repository {
    <# A bare origin and a clone of it with one commit on main. #>
    param([string] $Name)
    $bare = New-Directory "$Name.git"
    Invoke-Git $bare init --quiet --bare --initial-branch=main | Out-Null
    $clone = Join-Path $script:root $Name
    & git clone --quiet $bare $clone 2>&1 | Out-Null
    Set-Content -LiteralPath (Join-Path $clone 'README.md') -Value 'test'
    Invoke-Git $clone add README.md | Out-Null
    Invoke-Git $clone commit --quiet -m 'first' | Out-Null
    Invoke-Git $clone push --quiet origin HEAD:main | Out-Null
    $clone
}

function Set-FakeGitHub {
    <# gh answers from $global:FakeGh: releases (array), apiFails, and records every call. #>
    $global:FakeGh = @{ releases = @(); apiFails = $false; calls = [Collections.Generic.List[string]]::new() }
    Set-TigerSetupReleaseGitHubCli {
        $call = $args -join ' '
        $global:FakeGh.calls.Add($call)
        $global:LASTEXITCODE = 0
        if ($args[0] -eq 'api') {
            if ($global:FakeGh.apiFails) { $global:LASTEXITCODE = 1; return 'HTTP 403: forbidden' }
            if ($args[1] -like '*/releases?*') { return (ConvertTo-Json -InputObject @($global:FakeGh.releases) -Depth 6) }
            $global:LASTEXITCODE = 1; return 'HTTP 404'
        }
        if ($args[0] -eq 'release' -and $args[1] -eq 'create') {
            $files = @($args | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Where-Object { $_ -notlike '*.md' })
            $global:FakeGh.releases = @([pscustomobject]@{
                    tag_name = $args[2]; name = $args[@($args).IndexOf('--title') + 1]; draft = $true; html_url = "https://example.invalid/$($args[2])"
                    assets = @($files | ForEach-Object { New-FakeAsset $_ })
                })
            return 'created'
        }
        if ($args[0] -eq 'release' -and $args[1] -eq 'upload') {
            $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets) + @(New-FakeAsset $args[3])
            return 'uploaded'
        }
        $global:LASTEXITCODE = 1
        "unexpected gh call: $call"
    }
}

function New-FakeAsset {
    param([string] $Path)
    [pscustomobject]@{ name = (Split-Path -Leaf $Path); size = (Get-Item -LiteralPath $Path).Length; digest = "sha256:$((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant())" }
}

$goodNotes = @'
# TigerSetup 1.2.3

TigerSetup builds small, self-contained Windows installers from a declarative
manifest, with transactional installation state and a quiet mode for automation.

## What is new

Everything in this release is described here in real sentences, for a person who
installs TigerSetup and wants to know what changed and why it matters to them.

## Install

Download the installer below and run it, or install it with WinGet once the
package is available there.
'@

function New-ReleaseSet {
    <# A closed release directory for version 1.2.3, recorded against $Commit. #>
    param([string] $Name, [string] $Commit)
    $directory = New-Directory $Name
    [IO.File]::WriteAllBytes((Join-Path $directory 'TigerSetup-1.2.3-Setup.exe'), [byte[]](1..200))
    [IO.File]::WriteAllBytes((Join-Path $directory 'TigerSetup-1.2.3-WinGet.zip'), [byte[]](50..90))
    $sha = Write-TigerSetupReleaseRecord -Directory $directory -Version '1.2.3' -CommitSha $Commit
    [pscustomobject]@{ directory = $directory; recordSha256 = $sha }
}

try {
    Write-Host 'Release tooling tests'
    $sha40 = 'a' * 40

    Start-Scenario 'version'
    foreach ($good in @('0.12.0', '1.0.0', '10.20.30')) { Assert-True (Test-TigerSetupReleaseVersion $good) "$good is a version" }
    foreach ($bad in @('', '1.0', '1.0.0.0', 'v1.0.0', '01.0.0', '1.0.0-rc1', ' 1.0.0')) { Assert-True (-not (Test-TigerSetupReleaseVersion $bad)) "'$bad' is not a version" }
    $cargo = New-Directory 'cargo'
    Set-Content -LiteralPath (Join-Path $cargo 'Cargo.toml') -Value "[package]`nversion = `"9.9.9`"`n`n[workspace.package]`nedition = `"2021`"`nversion = `"1.2.3`"`n"
    Assert-True ((Get-TigerSetupSourceVersion -RepositoryRoot $cargo) -ceq '1.2.3') 'the [workspace.package] version is read, not another section''s'

    Start-Scenario 'notes'
    $notesRepo = New-Directory 'notes'
    $notesDir = New-Item -ItemType Directory -Path (Join-Path $notesRepo '.github\release-notes') -Force
    $notesPath = Join-Path $notesDir '1.2.3.md'
    $status = { (Test-TigerSetupReleaseNotes -RepositoryRoot $notesRepo -Version '1.2.3').status }
    Assert-True ((& $status) -ceq 'FAIL') 'missing notes fail'
    [IO.File]::WriteAllText($notesPath, $goodNotes, [Text.UTF8Encoding]::new($true))
    Assert-True ((& $status) -ceq 'FAIL') 'a byte-order mark fails'
    [IO.File]::WriteAllText($notesPath, $goodNotes.Replace('# TigerSetup 1.2.3', '# TigerSetup 1.2.2'), [Text.UTF8Encoding]::new($false))
    Assert-True ((& $status) -ceq 'FAIL') 'another version''s heading fails'
    [IO.File]::WriteAllText($notesPath, "# TigerSetup 1.2.3`n`n## One`n`nShort.`n`n## Two`n`nShort.`n", [Text.UTF8Encoding]::new($false))
    Assert-True ((& $status) -ceq 'FAIL') 'thin notes fail'
    [IO.File]::WriteAllText($notesPath, $goodNotes + "`nTODO: finish`n", [Text.UTF8Encoding]::new($false))
    Assert-True ((& $status) -ceq 'FAIL') 'placeholder text fails'
    [IO.File]::WriteAllText($notesPath, $goodNotes + "`nBuilt in C:\Users\someone\src.`n", [Text.UTF8Encoding]::new($false))
    Assert-True ((& $status) -ceq 'FAIL') 'a local path fails'
    [IO.File]::WriteAllText($notesPath, $goodNotes, [Text.UTF8Encoding]::new($false))
    Assert-True ((& $status) -ceq 'PASS') 'good notes pass'
    Set-Content -LiteralPath (Join-Path $notesRepo 'README.md') -Value 'TigerSetup is at version **1.2.2**.'
    Assert-True ((Test-TigerSetupVersionReference -RepositoryRoot $notesRepo -Version '1.2.3').status -ceq 'FAIL') 'a README stating the previous version fails'
    Set-Content -LiteralPath (Join-Path $notesRepo 'README.md') -Value 'TigerSetup is at version **1.2.3**.'
    Assert-True ((Test-TigerSetupVersionReference -RepositoryRoot $notesRepo -Version '1.2.3').status -ceq 'PASS') 'a README stating the version passes'

    Start-Scenario 'record'
    $set = New-ReleaseSet 'record' $sha40
    $record = Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha $sha40 -ExpectedRecordSha256 $set.recordSha256
    Assert-True ($record.schemaVersion -eq 1 -and @($record.artifacts).Count -eq 2) 'the record is schema 1 with both payloads'
    Assert-True (@($record.artifacts.kind) -join ',' -ceq 'WindowsInstaller,WinGetManifests') 'the payload kinds are recorded'
    $sums = (Get-Content -LiteralPath (Join-Path $set.directory 'SHA256SUMS.txt'))
    Assert-True ($sums.Count -eq 2 -and $sums[0] -match '^[0-9a-f]{64}  TigerSetup-1\.2\.3-Setup\.exe$') 'SHA256SUMS.txt is in sha256sum format'
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha ('b' * 40) } 'names commit' 'another commit is refused'
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.4' } 'version 1\.2\.4' 'another version is refused'
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -ExpectedRecordSha256 ('0' * 64) } 'in transit' 'a record changed in transit is refused'
    Set-Content -LiteralPath (Join-Path $set.directory 'extra.txt') -Value 'x'
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'unexpected: extra\.txt' 'a foreign file is refused'
    Remove-Item -LiteralPath (Join-Path $set.directory 'extra.txt')
    $installerPath = Join-Path $set.directory 'TigerSetup-1.2.3-Setup.exe'
    $bytes = [IO.File]::ReadAllBytes($installerPath); $bytes[5] = 0; [IO.File]::WriteAllBytes($installerPath, $bytes)
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'not the bytes' 'a changed byte is refused'
    Remove-Item -LiteralPath $installerPath
    Assert-Throws { Write-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha $sha40 } 'Missing: TigerSetup-1\.2\.3-Setup\.exe' 'a record over an incomplete set is refused'

    Start-Scenario 'git'
    $repo = New-Repository 'git'
    $first = Invoke-Git $repo rev-parse HEAD
    Assert-True ((Test-TigerSetupCommitOnMain -RepositoryRoot $repo -CommitSha $first).status -ceq 'PASS') 'a pushed commit is on main'
    Set-Content -LiteralPath (Join-Path $repo 'second.txt') -Value 'x'
    Invoke-Git $repo add second.txt | Out-Null
    Invoke-Git $repo commit --quiet -m second | Out-Null
    $second = Invoke-Git $repo rev-parse HEAD
    Assert-True ((Test-TigerSetupCommitOnMain -RepositoryRoot $repo -CommitSha $second).status -ceq 'BLOCKED') 'an unpushed commit is blocked'
    Assert-True ($null -eq (Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.3')) 'an absent tag names nothing'
    Invoke-Git $repo tag -a v1.2.3 $first -m 'annotated' | Out-Null
    Invoke-Git $repo tag v1.2.4 $first | Out-Null
    Invoke-Git $repo push --quiet origin v1.2.3 v1.2.4 | Out-Null
    Assert-True ((Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.3') -ceq $first) 'an annotated tag is dereferenced to its commit'
    Assert-True ((Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.4') -ceq $first) 'a lightweight tag names its commit'

    Start-Scenario 'prerequisites'
    # The release workflow's first job, whole: it needs git and the commit's
    # own files, and no hosted run of anything is one of its prerequisites.
    $assert = Join-Path $PSScriptRoot 'Assert-ReleaseCommitReady.ps1'
    $repo = New-Repository 'prerequisites'
    Set-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Value "[workspace.package]`nversion = `"1.2.3`"`n"
    Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'TigerSetup is at version **1.2.3**.'
    $null = New-Item -ItemType Directory -Path (Join-Path $repo '.github\release-notes') -Force
    [IO.File]::WriteAllText((Join-Path $repo '.github\release-notes\1.2.3.md'), $goodNotes, [Text.UTF8Encoding]::new($false))
    Invoke-Git $repo add . | Out-Null
    Invoke-Git $repo commit --quiet -m 'release 1.2.3' | Out-Null
    $commit = Invoke-Git $repo rev-parse HEAD
    $gate = { param([string] $Version = '1.2.3') & $assert -Version $Version -CommitSha $commit -RepositoryRoot $repo *>&1 | Out-Null; $LASTEXITCODE }
    Set-FakeGitHub
    Assert-True ((& $gate) -eq 2) 'before the push the gate is BLOCKED'
    Invoke-Git $repo push --quiet origin HEAD:main | Out-Null
    Assert-True ((& $gate) -eq 0) 'a pushed release commit passes, with no CI run anywhere'
    Assert-True ((& $gate '1.2.4') -eq 1) 'another version fails'
    Remove-Item -LiteralPath (Join-Path $repo '.github\release-notes\1.2.3.md')
    Assert-True ((& $gate) -eq 1) 'missing notes fail'
    Invoke-Git $repo checkout --quiet -- .github | Out-Null
    Invoke-Git $repo tag -a v1.2.3 $commit -m 'released' | Out-Null
    Invoke-Git $repo push --quiet origin v1.2.3 | Out-Null
    Assert-True ((& $gate) -eq 1) 'an existing tag fails'
    Assert-True ($global:FakeGh.calls.Count -eq 0) 'the gate never calls GitHub'

    Start-Scenario 'release'
    Set-FakeGitHub
    Assert-True ($null -eq (Get-TigerSetupGitHubRelease -Tag 'v1.2.3')) 'no release is $null'
    $global:FakeGh.releases = @([pscustomobject]@{ tag_name = 'v1.2.2'; draft = $false }, [pscustomobject]@{ tag_name = 'v1.2.3'; draft = $true })
    Assert-True ((Get-TigerSetupGitHubRelease -Tag 'v1.2.3').draft) 'a draft is found by its tag'
    $global:FakeGh.releases = @([pscustomobject]@{ tag_name = 'v1.2.3'; draft = $true }, [pscustomobject]@{ tag_name = 'v1.2.3'; draft = $false })
    Assert-Throws { Get-TigerSetupGitHubRelease -Tag 'v1.2.3' } 'More than one' 'two releases for one tag are refused'

    Start-Scenario 'publish'
    $publish = Join-Path $PSScriptRoot 'Publish-DraftRelease.ps1'
    $repo = New-Repository 'publish'
    $null = New-Item -ItemType Directory -Path (Join-Path $repo '.github\release-notes') -Force
    [IO.File]::WriteAllText((Join-Path $repo '.github\release-notes\1.2.3.md'), $goodNotes, [Text.UTF8Encoding]::new($false))
    Invoke-Git $repo add . | Out-Null
    Invoke-Git $repo commit --quiet -m notes | Out-Null
    Invoke-Git $repo push --quiet origin HEAD:main | Out-Null
    $commit = Invoke-Git $repo rev-parse HEAD
    $set = New-ReleaseSet 'publish-set' $commit
    $run = {
        param([switch] $PlanOnly, [switch] $Outside, [string] $Record = $set.recordSha256, [string] $Directory = $set.directory)
        # The release workflow's run for this commit, unless -Outside.
        [Environment]::SetEnvironmentVariable('GITHUB_ACTIONS', $(if ($Outside) { $null } else { 'true' }))
        [Environment]::SetEnvironmentVariable('GITHUB_WORKFLOW', $(if ($Outside) { $null } else { 'Release TigerSetup' }))
        [Environment]::SetEnvironmentVariable('GITHUB_SHA', $(if ($Outside) { $null } else { $commit }))
        try { & $publish -Version '1.2.3' -ArtifactDirectory $Directory -CommitSha $commit -ExpectedRecordSha256 $Record -RepositoryRoot $repo -RunUrl 'https://example.invalid/run/1' -PlanOnly:$PlanOnly *>&1 | Out-Null; $LASTEXITCODE }
        finally { foreach ($name in 'GITHUB_ACTIONS', 'GITHUB_WORKFLOW', 'GITHUB_SHA') { [Environment]::SetEnvironmentVariable($name, $null) } }
    }
    Set-FakeGitHub
    Assert-Throws { & $run -Outside } 'Only the' 'outside the release workflow nothing is published'
    Assert-True ((& $run -PlanOnly -Outside) -eq 0) '-PlanOnly runs anywhere'
    Assert-True ($null -eq (Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.3') -and @($global:FakeGh.calls | Where-Object { $_ -like 'release *' }).Count -eq 0) '-PlanOnly changes nothing'
    Assert-True ((& $run) -eq 0) 'the first run passes'
    $tag = Get-TigerSetupRemoteTag -RepositoryRoot $repo -Tag 'v1.2.3'
    Assert-True ($tag.commit -ceq $commit -and $tag.annotated) 'the annotated tag names the release commit on origin'
    Assert-True ($tag.tagger -ceq 'github-actions[bot]') 'the tag is made by github-actions[bot]'
    Assert-True ($tag.message -match "(?m)^release-artifacts\.json sha256 $($set.recordSha256)$" -and $tag.message -match 'built by https://example\.invalid/run/1') 'the tag names the release record and the run'
    # The checkout's own configuration, where an absent name - the usual case
    # on a CI runner, which has no identity at all - makes git exit 1.
    $checkoutName = & git -C $repo config --local --get user.name 2>$null
    Assert-True ($LASTEXITCODE -in 0, 1 -and $checkoutName -cne 'github-actions[bot]') 'the bot identity is not written into the checkout'
    Assert-True ([Environment]::GetEnvironmentVariable('GIT_COMMITTER_NAME') -ceq 'Release Test') 'the bot identity does not outlive the tag'
    $create = @($global:FakeGh.calls | Where-Object { $_ -like 'release create *' })
    Assert-True ($create.Count -eq 1 -and $create[0] -match '--draft' -and $create[0] -match '--verify-tag' -and $create[0] -match '--title TigerSetup 1\.2\.3' -and $create[0] -match '1\.2\.3\.md') 'the draft is created from the notes, verifying the tag'
    Assert-True (@($global:FakeGh.releases[0].assets).Count -eq 4) 'the draft carries the four assets'
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release create *' -or $_ -like 'release upload *' }).Count -eq 0) 'a rerun over a complete draft passes and changes nothing'
    Assert-True ((& $run -PlanOnly -Outside) -eq 0) '-PlanOnly over an existing draft reports'
    $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets | Where-Object name -NE 'SHA256SUMS.txt')
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0) 'a rerun over a partial draft passes'
    Assert-True (@($global:FakeGh.calls | Where-Object { $_ -like 'release upload *SHA256SUMS.txt*' }).Count -eq 1 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release create *' }).Count -eq 0) 'the rerun uploads only the missing asset'
    $saveAssets = $global:FakeGh.releases[0].assets
    $global:FakeGh.releases[0].assets = @()
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release upload *' }).Count -eq 4) 'a draft with no assets gets all four'
    $global:FakeGh.releases[0].assets = $saveAssets
    $global:FakeGh.releases[0].assets[0].digest = 'sha256:' + ('0' * 64)
    Assert-Throws { & $run } 'not byte-identical' 'a draft asset with other bytes is refused'
    $global:FakeGh.releases[0].assets[0] = New-FakeAsset (Join-Path $set.directory $global:FakeGh.releases[0].assets[0].name)
    $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets) + @([pscustomobject]@{ name = 'other.exe'; size = 1; digest = 'sha256:00' })
    Assert-Throws { & $run } 'not this release' 'a foreign asset is refused'
    $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets | Where-Object name -NE 'other.exe')
    $global:FakeGh.releases[0].draft = $false
    Assert-Throws { & $run } 'not the draft' 'a published release is refused'
    $global:FakeGh.releases[0].draft = $true

    Start-Scenario 'provenance'
    $assetsCopy = New-Directory 'provenance-assets'
    Copy-Item -Path (Join-Path $set.directory '*') -Destination $assetsCopy
    $record = Assert-TigerSetupReleaseProvenance -RepositoryRoot $repo -Directory $assetsCopy -Version '1.2.3'
    Assert-True ([string] $record.sourceCommit -ceq $commit) 'the tagged set proves its provenance'
    $other = New-ReleaseSet 'provenance-other' $commit
    Assert-Throws { Assert-TigerSetupReleaseProvenance -RepositoryRoot $repo -Directory $other.directory -Version '1.2.3' } 'another release record' 'a set built elsewhere for the same commit is refused'
    Assert-Throws { & $run -Record $other.recordSha256 -Directory $other.directory } 'not the release workflow''s tag for this release record' 'the tag is not reused for another set'
    Invoke-Git $repo push --quiet origin :refs/tags/v1.2.3 | Out-Null
    Invoke-Git $repo tag -d v1.2.3 | Out-Null
    Invoke-Git $repo tag -a v1.2.3 $commit -m (Get-TigerSetupTagMessage -Version '1.2.3' -RecordSha256 $set.recordSha256) | Out-Null
    Invoke-Git $repo push --quiet origin v1.2.3 | Out-Null
    Assert-Throws { Assert-TigerSetupReleaseProvenance -RepositoryRoot $repo -Directory $assetsCopy -Version '1.2.3' } 'not the release workflow' 'a tag made by a person is refused'
    Invoke-Git $repo push --quiet origin :refs/tags/v1.2.3 | Out-Null
    Invoke-Git $repo tag -d v1.2.3 | Out-Null
    Invoke-Git $repo tag -a v1.2.3 (Invoke-Git $repo rev-parse HEAD~1) -m 'elsewhere' | Out-Null
    Invoke-Git $repo push --quiet origin v1.2.3 | Out-Null
    Assert-Throws { & $run } 'never moved' 'a tag at another commit is refused'

    Start-Scenario 'report'
    $text = Write-TigerSetupReleaseReport -Title 'T' -StepSummaryPath '' -Checks @(
        New-TigerSetupReleaseCheck -Id 'a' -Status PASS -Observed 'ran'
        New-TigerSetupReleaseCheck -Id 'b' -Status 'NOT RUN' -Observed 'runs in the lab') 6>&1
    Assert-True ($text[-1] -eq 0 -and ($text -join "`n") -match 'T - PASS, 1 NOT RUN') 'a NOT RUN check is named, not counted as PASS'

    Start-Scenario 'winget'
    $upstream = New-Directory 'winget-upstream.git'
    Invoke-Git $upstream init --quiet --bare --initial-branch=master | Out-Null
    $seed = Join-Path $script:root 'winget-seed'
    & git clone --quiet $upstream $seed 2>&1 | Out-Null
    $null = New-Item -ItemType Directory -Path (Join-Path $seed 'manifests\o\Other\Tool\1.0.0') -Force
    Set-Content -LiteralPath (Join-Path $seed 'manifests\o\Other\Tool\1.0.0\Other.Tool.yaml') -Value 'PackageIdentifier: Other.Tool'
    Invoke-Git $seed add . | Out-Null
    Invoke-Git $seed commit --quiet -m seed | Out-Null
    Invoke-Git $seed push --quiet origin HEAD:master | Out-Null
    $fork = New-Directory 'winget-fork.git'
    Invoke-Git $fork init --quiet --bare --initial-branch=master | Out-Null
    $clone = Join-Path $script:root 'winget-clone'
    & git clone --quiet $upstream $clone 2>&1 | Out-Null
    Invoke-Git $clone remote rename origin upstream | Out-Null
    Invoke-Git $clone remote add origin $fork | Out-Null
    $manifests = New-Directory 'winget-manifests'
    foreach ($name in @('ItTiger.TigerSetup.yaml', 'ItTiger.TigerSetup.installer.yaml', 'ItTiger.TigerSetup.locale.en-US.yaml')) {
        Set-Content -LiteralPath (Join-Path $manifests $name) -Value "PackageIdentifier: ItTiger.TigerSetup`nPackageVersion: 1.2.3"
    }
    $submit = { param([switch] $Push) New-TigerSetupWinGetSubmission -WinGetPkgsRoot $clone -ManifestDirectory $manifests -Version '1.2.3' -Upstream $upstream -Push:$Push }
    $result = & $submit
    Assert-True ($result.branch -ceq 'ItTiger-TigerSetup-1.2.3' -and $result.kind -ceq 'New package' -and -not $result.pushed) 'a first submission is a new package on its own branch'
    $tree = Invoke-Git $clone ls-tree -r --name-only $result.branch -- manifests/i/ItTiger/TigerSetup/1.2.3
    Assert-True (@($tree -split "`n").Count -eq 3) 'the branch holds exactly the three manifests'
    Assert-True ((Invoke-Git $clone log -1 --format=%s $result.branch) -ceq 'New package: ItTiger.TigerSetup version 1.2.3') 'the commit subject follows winget-pkgs'
    Assert-True ((Invoke-Git $clone rev-parse "$($result.branch)~1") -ceq (Invoke-Git $clone rev-parse refs/remotes/upstream/master)) 'the branch starts from upstream master'
    $again = & $submit -Push
    Assert-True ($again.commit -ceq $result.commit -and $again.pushed) 'a rerun changes nothing and pushes'
    Assert-True ((Invoke-Git $fork rev-parse refs/heads/ItTiger-TigerSetup-1.2.3) -ceq $result.commit) 'the fork has the branch'
    Set-Content -LiteralPath (Join-Path $manifests 'ItTiger.TigerSetup.yaml') -Value 'PackageIdentifier: ItTiger.TigerSetup'
    Assert-Throws { & $submit } 'does not hold exactly' 'a changed set on an existing branch is refused'
    Set-Content -LiteralPath (Join-Path $clone 'dirty.txt') -Value 'x'
    Assert-Throws { & $submit } 'uncommitted' 'a dirty clone is refused'
    Remove-Item -LiteralPath (Join-Path $clone 'dirty.txt')
    Assert-Throws { New-TigerSetupWinGetSubmission -WinGetPkgsRoot $clone -ManifestDirectory $manifests -Version '1.2.3' } 'upstream' 'a clone of another upstream is refused'
    $null = New-Item -ItemType Directory -Path (Join-Path $seed 'manifests\i\ItTiger\TigerSetup\1.2.4') -Force
    Set-Content -LiteralPath (Join-Path $seed 'manifests\i\ItTiger\TigerSetup\1.2.4\ItTiger.TigerSetup.yaml') -Value 'PackageIdentifier: ItTiger.TigerSetup'
    Invoke-Git $seed add . | Out-Null
    Invoke-Git $seed commit --quiet -m 'upstream has 1.2.4' | Out-Null
    Invoke-Git $seed push --quiet origin HEAD:master | Out-Null
    Assert-Throws { New-TigerSetupWinGetSubmission -WinGetPkgsRoot $clone -ManifestDirectory $manifests -Version '1.2.4' -Upstream $upstream } 'already has' 'a version upstream already has is refused'
    $next = New-TigerSetupWinGetSubmission -WinGetPkgsRoot $clone -ManifestDirectory $manifests -Version '1.2.5' -Upstream $upstream
    Assert-True ($next.kind -ceq 'New version') 'a later version of a known package is a new version'
}
finally {
    Set-TigerSetupReleaseGitHubCli $null
    Remove-Variable -Name FakeGh -Scope Global -ErrorAction SilentlyContinue
    foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name]) }
    Remove-Item -LiteralPath $script:root -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
if ($script:failures.Count) {
    Write-Host "FAILED: $($script:failures.Count) of $($script:checks) checks"
    foreach ($failure in $script:failures) { Write-Host "  $failure" }
    exit 1
}
Write-Host "PASSED: $($script:checks) checks"
exit 0
