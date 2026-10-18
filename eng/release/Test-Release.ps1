<#
    .SYNOPSIS
    Focused tests for the release tooling in eng\release.

    .DESCRIPTION
    Everything runs against synthetic repositories under the temporary
    directory - local bare repositories stand in for origin, for the
    winget-pkgs fork, for its upstream and for TigerMarkView - and against
    scripted stand-ins for gh, dotnet and the release-asset download, so no
    test reaches GitHub, builds anything or touches the real checkout. Git
    identity comes from the environment for the duration of the run, because a
    CI runner has none, and the runner's GITHUB_* variables are cleared, so the
    runner preflight runs these tests as a developer machine does.

    Scenarios:

      version      the <Major>.<Minor>.<Patch> rule and the Cargo.toml reader
      notes        the release-notes gate: missing, BOM, heading, thin,
                   placeholder, leak, good; the README version line; the
                   release workflow's prefilled version: missing, absent,
                   another version, and the version in each YAML spelling
                   (another key's default never counts)
      record       release-artifacts.json and SHA256SUMS.txt: written over the
                   closed set, the terms as License and PrivacyStatement, and
                   refused for a changed byte, a missing or foreign file, a
                   missing, doubled or mis-kinded term, another commit,
                   another version, or a record that changed in transit
      terms        the release-bound terms: frozen as the committed bytes
                   whatever the checkout's line endings, proved against the
                   commit, refused when changed, missing or uncommitted; a
                   rehearsal freezes the prepared working tree's; manifests
                   whose LicenseUrl or PrivacyUrl is not this version's frozen
                   asset are refused, and TigerSetup's own package declares
                   exactly those URLs
      git          the commit-on-main gate and the tag lookup (annotated,
                   lightweight, absent)
      prerequisites Assert-ReleaseCommitReady.ps1 as the workflow runs it:
                   PASS for a pushed release commit, BLOCKED before the push,
                   the ref NOT RUN on a checkout, and inside GitHub Actions
                   PASS on refs/heads/main and FAIL on dev, another branch, a
                   tag, a pull request or no ref; FAIL for another version,
                   missing notes, a commit without its licence and privacy
                   statement, a package whose terms URL is not the frozen
                   asset, a workflow prefilled with another version or an
                   existing tag - and never a call to GitHub
      release      the GitHub Release lookup: draft by tag, absent, ambiguous
      publish      Publish-DraftRelease.ps1: a run started on any ref but main
                   tags and drafts nothing; first run tags and creates the
                   draft with the six files; a rerun over a compatible draft
                   uploads only what is missing; terms that are not the
                   commit's, a changed byte, a published release, a foreign
                   asset and a tag at another commit are refused
      tigermark    Build-TigerMark.ps1 and the source pin: only a full SHA
                   pins; the checkout is exactly the pin, clean, and contains
                   the security baseline; only the CLI is published; an
                   unobtainable source or commit, a failed build, a missing,
                   unrunnable or wrong-version tiger-mark fail; the release
                   workflow pins a full commit and names no TigerMarkView
                   release or installer to fall back to
      winget      the winget-pkgs submission: branch and commit, an idempotent
                   rerun, push to the fork, and refusal of a changed set, a
                   version upstream already has, a dirty clone and a foreign
                   upstream
      ref          the ref check in and outside GitHub Actions
      workflows    release.yml and ci.yml as GitHub runs them: started by hand
                   only, read-only but for the publish job in the release
                   environment, prerequisites before the build and the build
                   before publishing with no condition between them, the
                   runner check and tiger-mark's render before the release
                   build, every job on the run's commit, and the preflight on
                   exactly the release workflow's actions, building no release
      runner       Test-ReleaseRunner.ps1 without git, Rust, .NET or gh: each
                   fails, and every check is reported
      render       the PDF title, a tiger-mark that renders nothing or is not
                   there, and Build-TigerMark.ps1 building the release
                   workflow's pin by default
      manifests    the manifest-set check: a set naming the installer passes;
                   a missing or extra file, another version, URL or hash, and
                   terms that are not the release's are refused
      retrieve     Get-ReleaseArtifacts.ps1 up to the installer: a draft and a
                   published release downloaded alike through the API and
                   proved (bytes, record, tag, run, terms, manifests); a run on
                   dev, of another workflow, for another commit, failed or
                   unknown, a wrong digest or download, a failed download
                   (BLOCKED), a foreign or missing
                   asset, another name, no release, a refusing GitHub, no
                   token and a tag made by a person are refused; a set on disk
                   is proved without downloading, refused with a foreign file,
                   its tag and run NOT RUN before the tag exists, and a
                   rehearsal's prepared terms proved only with -Rehearsal
      readiness    Test-ReleaseReadiness.ps1: ready with the settings and a
                   preflight of this tree; a change to the tooling, a missing
                   preflight or an elevated-tests run instead, an environment
                   for every branch, for dev too, for protected branches or for
                   a tag, no environment, an active tag ruleset, Actions off,
                   SHA pinning or local actions only, a private repository, a
                   login without push access, an unreadable repository and a
                   main without the workflow are each reported

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
$runnerVariables = @('GITHUB_ACTIONS', 'GITHUB_WORKFLOW', 'GITHUB_SHA', 'GITHUB_REF', 'GITHUB_STEP_SUMMARY')
foreach ($name in @($identity.Keys) + $runnerVariables) { $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name) }
foreach ($name in $identity.Keys) { [Environment]::SetEnvironmentVariable($name, $identity[$name]) }
# On a runner these name the real run; every scenario starts outside one.
foreach ($name in $runnerVariables) { [Environment]::SetEnvironmentVariable($name, $null) }

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
    <#
        gh answers from $global:FakeGh: releases (array), api (path -> object,
        answered as JSON), apiFails, token (`gh auth token`), and records every
        call.
    #>
    $global:FakeGh = @{ releases = @(); api = @{}; apiFails = $false; token = 'fake-token'; calls = [Collections.Generic.List[string]]::new() }
    Set-TigerSetupReleaseGitHubCli {
        $call = $args -join ' '
        $global:FakeGh.calls.Add($call)
        $global:LASTEXITCODE = 0
        if ($args[0] -eq 'api') {
            if ($global:FakeGh.apiFails) { $global:LASTEXITCODE = 1; return 'HTTP 403: forbidden' }
            if ($global:FakeGh.api.ContainsKey($args[1])) {
                # A string answer is gh's error for that path.
                if ($global:FakeGh.api[$args[1]] -is [string]) { $global:LASTEXITCODE = 1; return $global:FakeGh.api[$args[1]] }
                return (ConvertTo-Json -InputObject $global:FakeGh.api[$args[1]] -Depth 8)
            }
            if ($args[1] -like '*/releases?*') { return (ConvertTo-Json -InputObject @($global:FakeGh.releases) -Depth 6) }
            $global:LASTEXITCODE = 1; return 'HTTP 404'
        }
        if ($args[0] -eq 'auth' -and $args[1] -eq 'token') {
            if (-not $global:FakeGh.token) { $global:LASTEXITCODE = 1; return 'not logged in' }
            return $global:FakeGh.token
        }
        if ($args[0] -eq 'release' -and $args[1] -eq 'create') {
            $notesFile = $args[@($args).IndexOf('--notes-file') + 1]
            $files = @($args | Where-Object { $_ -cne $notesFile -and (Test-Path -LiteralPath $_ -PathType Leaf) })
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
    $name = Split-Path -Leaf $Path
    [pscustomobject]@{ name = $name; size = (Get-Item -LiteralPath $Path).Length; digest = "sha256:$((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant())"; url = "https://api.invalid/assets/$name" }
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

function Get-ReleaseWorkflowText {
    <# A release workflow whose version input defaults to $Default, or has no default. #>
    param([string] $Default)
    $line = if ($Default) { "        default: $Default`n" } else { '' }
    "name: Release TigerSetup`non:`n  workflow_dispatch:`n    inputs:`n      channel:`n        default: '1.2.3'`n      version:`n        description: The version`n        required: true`n$line        type: string`njobs:`n  build:`n    steps:`n      - name: Step`n        with:`n          default: '1.2.3'`n"
}

function New-ReleaseSet {
    <#
        A closed release directory for version 1.2.3, recorded against $Commit.
        With -Repository its terms are frozen from that repository's commit, as
        the release build freezes them; without, they are stand-in bytes.
    #>
    param([string] $Name, [string] $Commit, [string] $Repository = '')
    $directory = New-Directory $Name
    [IO.File]::WriteAllBytes((Join-Path $directory 'TigerSetup-1.2.3-Setup.exe'), [byte[]](1..200))
    [IO.File]::WriteAllBytes((Join-Path $directory 'TigerSetup-1.2.3-WinGet.zip'), [byte[]](50..90))
    if ($Repository) { $null = Copy-TigerSetupReleaseTerms -RepositoryRoot $Repository -Directory $directory -Version '1.2.3' -CommitSha $Commit }
    else {
        [IO.File]::WriteAllText((Join-Path $directory 'LICENSE.txt'), "MIT License`n")
        [IO.File]::WriteAllText((Join-Path $directory 'PRIVACY.md'), "# Privacy`n")
    }
    $sha = Write-TigerSetupReleaseRecord -Directory $directory -Version '1.2.3' -CommitSha $Commit
    [pscustomobject]@{ directory = $directory; recordSha256 = $sha }
}

function Add-Terms {
    <# Commits a licence and a privacy statement to $Repository, LF in Git. #>
    param([string] $Repository, [string] $Privacy = "# Privacy`n`nNo telemetry.`n")
    [IO.File]::WriteAllText((Join-Path $Repository 'LICENSE.txt'), "MIT License`n`nCopyright (c) 2026 Test`n")
    [IO.File]::WriteAllText((Join-Path $Repository 'PRIVACY.md'), $Privacy)
    Invoke-Git $Repository add LICENSE.txt PRIVACY.md | Out-Null
    Invoke-Git $Repository commit --quiet -m terms | Out-Null
}

function Get-LocaleManifestText {
    <# A locale manifest naming $License and $Privacy, either omitted when empty. #>
    param([string] $License, [string] $Privacy)
    $lines = @('PackageIdentifier: ItTiger.TigerSetup', 'PackageVersion: 1.2.3', 'PackageLocale: en-US', 'PublisherSupportUrl: https://github.com/rkozlowski/TigerSetup/issues')
    if ($Privacy) { $lines += "PrivacyUrl: $Privacy" }
    $lines += 'License: MIT'
    if ($License) { $lines += "LicenseUrl: $License" }
    ($lines + @('ManifestType: defaultLocale', 'ManifestVersion: 1.12.0')) -join "`n"
}

function Write-ManifestSet {
    <# The three manifests of ItTiger.TigerSetup 1.2.3 for an installer at $Url with $Sha256. #>
    param([string] $Directory, [string] $Url, [string] $Sha256)
    $id = 'ItTiger.TigerSetup'
    Set-Content -LiteralPath (Join-Path $Directory "$id.yaml") -Value "PackageIdentifier: $id`nPackageVersion: 1.2.3`nDefaultLocale: en-US`nManifestType: version`nManifestVersion: 1.12.0"
    Set-Content -LiteralPath (Join-Path $Directory "$id.installer.yaml") -Value "PackageIdentifier: $id`nPackageVersion: 1.2.3`nInstallers:`n- Architecture: x64`n  InstallerUrl: $Url`n  InstallerSha256: $($Sha256.ToUpperInvariant())`nManifestType: installer`nManifestVersion: 1.12.0"
    Set-Content -LiteralPath (Join-Path $Directory "$id.locale.en-US.yaml") -Value (Get-LocaleManifestText (Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name 'LICENSE.txt') (Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name 'PRIVACY.md'))
}

function Get-CheckStatus {
    <# The status a gate's report gives check $Id, or '' when it does not name it. #>
    param([string] $Text, [string] $Id)
    $line = [regex]::Match($Text, "(?m)^\s+(?<status>PASS|FAIL|BLOCKED|NOT RUN)\s+$([regex]::Escape($Id)):")
    if ($line.Success) { $line.Groups['status'].Value } else { '' }
}

function Get-WorkflowJob {
    <# A workflow's jobs, by name, each as the text of its block. No YAML parser is involved. #>
    param([string] $Text)
    $jobs = [ordered]@{}
    $name = $null
    $inJobs = $false
    foreach ($line in ($Text -split "`r?`n")) {
        if ($line -ceq 'jobs:') { $inJobs = $true; continue }
        if (-not $inJobs) { continue }
        if ($line -match '^  (?<job>[A-Za-z0-9_-]+):\s*$') { $name = $Matches['job']; $jobs[$name] = ''; continue }
        if ($null -ne $name) { $jobs[$name] += "$line`n" }
    }
    $jobs
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
    $workflow = { (Test-TigerSetupWorkflowDefaultVersion -RepositoryRoot $notesRepo -Version '1.2.3').status }
    Assert-True ((& $workflow) -ceq 'FAIL') 'a missing release workflow fails'
    $workflowPath = Join-Path (New-Item -ItemType Directory -Path (Join-Path $notesRepo '.github\workflows') -Force) 'release.yml'
    Set-Content -LiteralPath $workflowPath -Value (Get-ReleaseWorkflowText)
    Assert-True ((& $workflow) -ceq 'FAIL') 'a version input without a default fails, whatever other keys default to'
    Set-Content -LiteralPath $workflowPath -Value (Get-ReleaseWorkflowText "'1.2.2'")
    Assert-True ((& $workflow) -ceq 'FAIL') 'a workflow offering the previous version fails'
    foreach ($spelling in @("'1.2.3'", '"1.2.3"', '1.2.3 # the release')) {
        Set-Content -LiteralPath $workflowPath -Value (Get-ReleaseWorkflowText $spelling)
        Assert-True ((& $workflow) -ceq 'PASS') "a workflow offering the version as $spelling passes"
    }

    Start-Scenario 'record'
    $set = New-ReleaseSet 'record' $sha40
    $record = Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha $sha40 -ExpectedRecordSha256 $set.recordSha256
    Assert-True ($record.schemaVersion -eq 1 -and @($record.artifacts).Count -eq 4) 'the record is schema 1 with the four payloads'
    Assert-True (@($record.artifacts.kind) -join ',' -ceq 'WindowsInstaller,WinGetManifests,License,PrivacyStatement') 'the payload kinds are recorded, the terms as License and PrivacyStatement'
    Assert-True (@($record.artifacts.name) -join ',' -ceq 'TigerSetup-1.2.3-Setup.exe,TigerSetup-1.2.3-WinGet.zip,LICENSE.txt,PRIVACY.md') 'the terms keep their file names'
    $sums = (Get-Content -LiteralPath (Join-Path $set.directory 'SHA256SUMS.txt'))
    Assert-True ($sums.Count -eq 4 -and $sums[0] -match '^[0-9a-f]{64}  TigerSetup-1\.2\.3-Setup\.exe$') 'SHA256SUMS.txt is in sha256sum format'
    Assert-True ((@($sums | ForEach-Object { ($_ -split '  ', 2)[1] }) -join ',') -ceq (@($record.artifacts.name) -join ',')) 'SHA256SUMS.txt names the same files in the record''s order'
    $recordPath = Join-Path $set.directory 'release-artifacts.json'
    $recordText = Get-Content -LiteralPath $recordPath -Raw
    $edited = { param([scriptblock] $Change) $json = $recordText | ConvertFrom-Json; & $Change $json; $json | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $recordPath -Encoding utf8NoBOM }
    & $edited { param($json) $json.artifacts = @($json.artifacts | Where-Object kind -CNE 'PrivacyStatement') }
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'not the release set' 'a record without the PrivacyStatement is refused'
    & $edited { param($json) $json.artifacts = @($json.artifacts | Where-Object kind -CNE 'License') }
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'not the release set' 'a record without the License is refused'
    & $edited { param($json) $json.artifacts = @($json.artifacts) + @($json.artifacts | Where-Object kind -CEQ 'License') }
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'not the release set' 'a record with a second License is refused'
    & $edited { param($json) ($json.artifacts | Where-Object kind -CEQ 'PrivacyStatement').kind = 'License' }
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'not the release set' 'the privacy statement recorded under another kind is refused'
    Set-Content -LiteralPath $recordPath -Value $recordText -NoNewline -Encoding utf8NoBOM
    $privacyPath = Join-Path $set.directory 'PRIVACY.md'
    $privacyBytes = [IO.File]::ReadAllBytes($privacyPath)
    [IO.File]::WriteAllText($privacyPath, "# Privacy, changed`n")
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'PRIVACY\.md is not the bytes' 'a changed privacy statement is refused'
    Remove-Item -LiteralPath $privacyPath
    Assert-Throws { Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' } 'Missing: PRIVACY\.md' 'a set without its privacy statement is refused'
    Assert-Throws { Write-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha $sha40 } 'Missing: PRIVACY\.md' 'a set without its privacy statement is never recorded'
    [IO.File]::WriteAllBytes($privacyPath, $privacyBytes)
    $licenseBytes = [IO.File]::ReadAllBytes((Join-Path $set.directory 'LICENSE.txt'))
    Remove-Item -LiteralPath (Join-Path $set.directory 'LICENSE.txt')
    Assert-Throws { Write-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -CommitSha $sha40 } 'Missing: LICENSE\.txt' 'a set without its licence is never recorded'
    [IO.File]::WriteAllBytes((Join-Path $set.directory 'LICENSE.txt'), $licenseBytes)
    $null = Assert-TigerSetupReleaseRecord -Directory $set.directory -Version '1.2.3' -ExpectedRecordSha256 $set.recordSha256
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

    Start-Scenario 'terms'
    $terms = @(Get-TigerSetupReleaseTerms -Version '1.2.3')
    Assert-True ((@($terms | ForEach-Object { "$($_.name)|$($_.kind)|$($_.source)" }) -join ';') -ceq 'LICENSE.txt|License|LICENSE.txt;PRIVACY.md|PrivacyStatement|PRIVACY.md') 'the release-bound terms are LICENSE.txt and PRIVACY.md'
    # A checkout that converts line endings, as a Windows runner's does: the
    # frozen terms are the committed bytes, not the checkout's.
    $repo = New-Repository 'terms'
    Invoke-Git $repo config core.autocrlf true | Out-Null
    $withoutTerms = Invoke-Git $repo rev-parse HEAD
    Add-Terms $repo
    $commit = Invoke-Git $repo rev-parse HEAD
    Remove-Item -LiteralPath (Join-Path $repo 'LICENSE.txt'), (Join-Path $repo 'PRIVACY.md')
    Invoke-Git $repo checkout --quiet -- . | Out-Null
    Assert-True ((Get-Content -LiteralPath (Join-Path $repo 'PRIVACY.md') -Raw) -match "`r`n") 'the checkout carries CRLF'
    $frozen = New-Directory 'terms-frozen'
    $copied = @(Copy-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $frozen -Version '1.2.3' -CommitSha $commit)
    Assert-True ($copied.Count -eq 2 -and [IO.File]::ReadAllText((Join-Path $frozen 'PRIVACY.md')) -ceq "# Privacy`n`nNo telemetry.`n") 'the frozen privacy statement is the committed bytes'
    Assert-True ([IO.File]::ReadAllText((Join-Path $frozen 'LICENSE.txt')) -ceq "MIT License`n`nCopyright (c) 2026 Test`n") 'the frozen licence is the committed bytes'
    Assert-True ($copied[1].sha256 -ceq (Get-TigerSetupFileSha256 (Join-Path $frozen 'PRIVACY.md'))) 'each frozen term reports its SHA-256'
    Assert-True (@(Assert-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $frozen -Version '1.2.3' -CommitSha $commit).Count -eq 2) 'frozen terms prove against their commit'
    $checkoutCopy = New-Directory 'terms-checkout'
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE.txt'), (Join-Path $repo 'PRIVACY.md') -Destination $checkoutCopy
    Assert-Throws { Assert-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $checkoutCopy -Version '1.2.3' -CommitSha $commit } 'not [0-9a-f]{64}, the bytes commit' 'the checkout''s converted copy is not the frozen terms'
    [IO.File]::WriteAllText((Join-Path $frozen 'PRIVACY.md'), "# Privacy`n`nSome telemetry.`n")
    Assert-Throws { Assert-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $frozen -Version '1.2.3' -CommitSha $commit } 'PRIVACY\.md hashes to' 'a changed privacy statement is refused'
    Remove-Item -LiteralPath (Join-Path $frozen 'LICENSE.txt')
    Assert-Throws { Assert-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $frozen -Version '1.2.3' -CommitSha $commit } 'no LICENSE\.txt, its License' 'a missing licence is refused'
    Assert-Throws { Copy-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory (New-Directory 'terms-none') -Version '1.2.3' -CommitSha $withoutTerms } 'has no LICENSE\.txt' 'a commit without the terms freezes nothing'
    # The rehearsal: the working tree's terms, as Git would commit them.
    [IO.File]::WriteAllText((Join-Path $repo 'PRIVACY.md'), "# Privacy`r`n`r`nPrepared.`r`n")
    $rehearsal = New-Directory 'terms-rehearsal'
    $null = Copy-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $rehearsal -Version '1.2.3' -CommitSha $commit -WorkingTree
    Assert-True ([IO.File]::ReadAllText((Join-Path $rehearsal 'PRIVACY.md')) -ceq "# Privacy`n`nPrepared.`n") 'a rehearsal freezes the prepared terms as Git would commit them'
    Assert-True ((Invoke-Git $repo status --porcelain) -ceq 'M PRIVACY.md') 'a rehearsal changes nothing in the checkout'
    Assert-Throws { Assert-TigerSetupReleaseTerms -RepositoryRoot $repo -Directory $rehearsal -Version '1.2.3' -CommitSha $commit } 'PRIVACY\.md hashes to' 'uncommitted terms are not the commit''s'

    # The manifests' LicenseUrl and PrivacyUrl name this version's frozen terms.
    $licenseUrl = Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name 'LICENSE.txt'
    $privacyUrl = Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name 'PRIVACY.md'
    Assert-True ($licenseUrl -ceq 'https://github.com/rkozlowski/TigerSetup/releases/download/v1.2.3/LICENSE.txt' -and $privacyUrl -ceq 'https://github.com/rkozlowski/TigerSetup/releases/download/v1.2.3/PRIVACY.md') 'the terms are published at the release''s asset URLs'
    $manifests = New-Directory 'terms-manifests'
    $locale = Join-Path $manifests 'ItTiger.TigerSetup.locale.en-US.yaml'
    $urls = { param([string] $License, [string] $Privacy) Set-Content -LiteralPath $locale -Value (Get-LocaleManifestText $License $Privacy); @(Test-TigerSetupWinGetTermsUrl -ManifestDirectory $manifests -Version '1.2.3') }
    Assert-True (@(& $urls $licenseUrl $privacyUrl).Count -eq 0) 'manifests naming the frozen terms pass'
    foreach ($case in @(
            @{ name = 'a privacy statement on main'; license = $licenseUrl; privacy = 'https://github.com/rkozlowski/TigerSetup/blob/main/PRIVACY.md' },
            @{ name = 'a licence at the tag in the repository'; license = 'https://github.com/rkozlowski/TigerSetup/blob/v1.2.3/LICENSE.txt'; privacy = $privacyUrl },
            @{ name = 'the latest release''s terms'; license = $licenseUrl; privacy = 'https://github.com/rkozlowski/TigerSetup/releases/latest/download/PRIVACY.md' },
            @{ name = 'another version''s terms'; license = $licenseUrl.Replace('1.2.3', '1.2.2'); privacy = $privacyUrl },
            @{ name = 'no PrivacyUrl'; license = $licenseUrl; privacy = '' },
            @{ name = 'no LicenseUrl'; license = ''; privacy = $privacyUrl })) {
        Assert-True (@(& $urls $case.license $case.privacy).Count -eq 1) "$($case.name) is refused"
    }
    # TigerSetup's own package declares exactly these URLs, by version.
    $package = Get-Content -LiteralPath (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'packages\tigersetup\TigerSetup.toml') -Raw
    foreach ($key in @(@('license_url', 'LICENSE.txt'), @('privacy_url', 'PRIVACY.md'))) {
        $declared = [regex]::Match($package, "(?m)^$($key[0])\s*=\s*""([^""]*)""")
        Assert-True ($declared.Success -and $declared.Groups[1].Value.Replace('{version}', '1.2.3') -ceq (Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name $key[1])) "TigerSetup's $($key[0]) names the release's frozen $($key[1]) by version"
    }

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
    $null = New-Item -ItemType Directory -Path (Join-Path $repo '.github\workflows') -Force
    Set-Content -LiteralPath (Join-Path $repo '.github\workflows\release.yml') -Value (Get-ReleaseWorkflowText "'1.2.3'")
    $null = New-Item -ItemType Directory -Path (Join-Path $repo 'packages\tigersetup') -Force
    $packageText = "[winget]`nlicense_url = `"https://github.com/rkozlowski/TigerSetup/releases/download/v{version}/LICENSE.txt`"`nprivacy_url = `"https://github.com/rkozlowski/TigerSetup/releases/download/v{version}/PRIVACY.md`"`n"
    Set-Content -LiteralPath (Join-Path $repo 'packages\tigersetup\TigerSetup.toml') -Value $packageText
    Invoke-Git $repo add . | Out-Null
    Invoke-Git $repo commit --quiet -m 'release 1.2.3 without its terms' | Out-Null
    $withoutTerms = Invoke-Git $repo rev-parse HEAD
    Add-Terms $repo
    $commit = Invoke-Git $repo rev-parse HEAD
    $gate = { param([string] $Version = '1.2.3') & $assert -Version $Version -CommitSha $commit -RepositoryRoot $repo *>&1 | Out-Null; $LASTEXITCODE }
    Set-FakeGitHub
    Assert-True ((& $gate) -eq 2) 'before the push the gate is BLOCKED'
    Invoke-Git $repo push --quiet origin HEAD:main | Out-Null
    Assert-True ((& $gate) -eq 0) 'a pushed release commit passes, with no CI run anywhere'
    $report = (& $assert -Version '1.2.3' -CommitSha $commit -RepositoryRoot $repo 6>&1 | Out-String)
    Assert-True ($report -match '(?m)^\s+NOT RUN\s+ref: ') 'on a maintainer''s checkout the ref is NOT RUN, and named'
    # Inside GitHub Actions the ref is always checked, on GITHUB_REF: the
    # same commit, otherwise ready, fails when the run was started elsewhere.
    $gateIn = {
        param([AllowEmptyString()] [string] $Ref, [string] $Named = '')
        [Environment]::SetEnvironmentVariable('GITHUB_ACTIONS', 'true')
        [Environment]::SetEnvironmentVariable('GITHUB_REF', $Ref)
        try { $text = & $assert -Version '1.2.3' -CommitSha $commit -RepositoryRoot $repo -WorkflowRef $Named 6>&1 | Out-String; [pscustomobject]@{ code = $LASTEXITCODE; text = $text } }
        finally { foreach ($name in 'GITHUB_ACTIONS', 'GITHUB_REF') { [Environment]::SetEnvironmentVariable($name, $null) } }
    }
    $inMain = & $gateIn 'refs/heads/main'
    Assert-True ($inMain.code -eq 0 -and $inMain.text -match '(?m)^\s+PASS\s+ref: The release workflow was started on refs/heads/main') 'a run started on main passes'
    foreach ($ref in @('refs/heads/dev', 'refs/heads/feature/x', 'refs/heads/main-old', 'refs/tags/v1.2.3', 'refs/pull/1/merge', '')) {
        $other = & $gateIn $ref
        Assert-True ($other.code -eq 1 -and $other.text -match '(?m)^\s+FAIL\s+ref: ') "a run started on '$ref' fails at the ref"
    }
    Assert-True ((& $gateIn 'refs/heads/main' 'refs/heads/dev').code -eq 1) 'a named ref that is not the run''s fails'
    $simulated = & { & $assert -Version '1.2.3' -CommitSha $commit -RepositoryRoot $repo -WorkflowRef 'refs/heads/dev' *>&1 | Out-Null; $LASTEXITCODE }
    Assert-True ($simulated -eq 1) 'outside GitHub Actions a simulated dev run fails too'
    Assert-True ((& $gate '1.2.4') -eq 1) 'another version fails'
    Remove-Item -LiteralPath (Join-Path $repo '.github\release-notes\1.2.3.md')
    Assert-True ((& $gate) -eq 1) 'missing notes fail'
    Invoke-Git $repo checkout --quiet -- .github | Out-Null
    Set-Content -LiteralPath (Join-Path $repo '.github\workflows\release.yml') -Value (Get-ReleaseWorkflowText "'1.2.2'")
    Assert-True ((& $gate) -eq 1) 'a workflow prefilled with another version fails'
    Invoke-Git $repo checkout --quiet -- .github | Out-Null
    $gateAt = { param([string] $At) & $assert -Version '1.2.3' -CommitSha $At -RepositoryRoot $repo *>&1 | Out-Null; $LASTEXITCODE }
    Assert-True ((& $gateAt $withoutTerms) -eq 1) 'a commit without its licence and privacy statement fails before any build'
    Set-Content -LiteralPath (Join-Path $repo 'packages\tigersetup\TigerSetup.toml') -Value $packageText.Replace('releases/download/v{version}/PRIVACY.md', 'blob/main/PRIVACY.md')
    Assert-True ((& $gate) -eq 1) 'a package whose PrivacyUrl is not the frozen statement fails'
    Invoke-Git $repo checkout --quiet -- packages | Out-Null
    Assert-True ((Test-TigerSetupReleaseTermsReady -RepositoryRoot $repo -Version '1.2.3' -CommitSha $commit).status -ceq 'PASS') 'a commit with its terms and the frozen-terms URLs passes the terms check'
    Assert-True ((& $gate) -eq 0) 'the restored release commit passes again'
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
    Add-Terms $repo
    Invoke-Git $repo push --quiet origin HEAD:main | Out-Null
    $commit = Invoke-Git $repo rev-parse HEAD
    $set = New-ReleaseSet 'publish-set' $commit $repo
    $run = {
        param([switch] $PlanOnly, [switch] $Outside, [string] $Record = $set.recordSha256, [string] $Directory = $set.directory, [string] $Ref = 'refs/heads/main',
            [string] $RunUrl = 'https://example.invalid/run/1')
        # The release workflow's run for this commit, started on $Ref, unless -Outside.
        [Environment]::SetEnvironmentVariable('GITHUB_ACTIONS', $(if ($Outside) { $null } else { 'true' }))
        [Environment]::SetEnvironmentVariable('GITHUB_WORKFLOW', $(if ($Outside) { $null } else { 'Release TigerSetup' }))
        [Environment]::SetEnvironmentVariable('GITHUB_SHA', $(if ($Outside) { $null } else { $commit }))
        [Environment]::SetEnvironmentVariable('GITHUB_REF', $(if ($Outside) { $null } else { $Ref }))
        try { & $publish -Version '1.2.3' -ArtifactDirectory $Directory -CommitSha $commit -ExpectedRecordSha256 $Record -RepositoryRoot $repo -RunUrl $RunUrl -PlanOnly:$PlanOnly *>&1 | Out-Null; $LASTEXITCODE }
        finally { foreach ($name in 'GITHUB_ACTIONS', 'GITHUB_WORKFLOW', 'GITHUB_SHA', 'GITHUB_REF') { [Environment]::SetEnvironmentVariable($name, $null) } }
    }
    Set-FakeGitHub
    # The release workflow started on any other ref tags and drafts nothing,
    # even when every other fact is right.
    foreach ($ref in @('refs/heads/dev', 'refs/heads/feature/x', 'refs/tags/v1.2.3', '')) {
        Assert-Throws { & $run -Ref $ref } 'Nothing is tagged or drafted' "a run started on '$ref' is refused"
    }
    Assert-True ($null -eq (Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.3') -and @($global:FakeGh.calls | Where-Object { $_ -like 'release *' }).Count -eq 0) 'a run on another ref tags and drafts nothing'
    # Terms that are not the commit's, in a set whose record is consistent
    # with them, are refused before anything is tagged or drafted.
    $foreignTerms = New-ReleaseSet 'publish-foreign-terms' $commit
    Assert-Throws { & $run -Record $foreignTerms.recordSha256 -Directory $foreignTerms.directory } 'LICENSE\.txt hashes to .* the bytes commit' 'terms that are not the release commit''s are refused'
    Assert-True ($null -eq (Get-TigerSetupRemoteTagCommit -RepositoryRoot $repo -Tag 'v1.2.3') -and $global:FakeGh.calls.Count -eq 0) 'refused terms tag and draft nothing'
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
    Assert-True ((@($global:FakeGh.releases[0].assets.name) -join ',') -ceq 'TigerSetup-1.2.3-Setup.exe,TigerSetup-1.2.3-WinGet.zip,LICENSE.txt,PRIVACY.md,SHA256SUMS.txt,release-artifacts.json') 'the draft carries the six files of the release set, the terms among them'
    Assert-True (@($global:FakeGh.releases[0].assets | Where-Object { $_.digest -cne "sha256:$(Get-TigerSetupFileSha256 (Join-Path $set.directory $_.name))" }).Count -eq 0) 'the draft carries the frozen bytes'
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release create *' -or $_ -like 'release upload *' }).Count -eq 0) 'a rerun over a complete draft passes and changes nothing'
    Assert-True ((& $run -PlanOnly -Outside) -eq 0) '-PlanOnly over an existing draft reports'
    $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets | Where-Object name -NE 'SHA256SUMS.txt')
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0) 'a rerun over a partial draft passes'
    Assert-True (@($global:FakeGh.calls | Where-Object { $_ -like 'release upload *SHA256SUMS.txt*' }).Count -eq 1 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release create *' }).Count -eq 0) 'the rerun uploads only the missing asset'
    $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets | Where-Object name -NE 'PRIVACY.md')
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0 -and (@($global:FakeGh.calls | Where-Object { $_ -like 'release upload *' }) -join '|') -like 'release upload v1.2.3 *PRIVACY.md --repo *') 'a draft missing its privacy statement gets exactly that file'
    $saveAssets = $global:FakeGh.releases[0].assets
    $global:FakeGh.releases[0].assets = @()
    $global:FakeGh.calls.Clear()
    Assert-True ((& $run) -eq 0 -and @($global:FakeGh.calls | Where-Object { $_ -like 'release upload *' }).Count -eq 6) 'a draft with no assets gets all six'
    $global:FakeGh.releases[0].assets = $saveAssets
    $privacyAsset = @($global:FakeGh.releases[0].assets | Where-Object name -CEQ 'PRIVACY.md')[0]
    $privacyDigest = $privacyAsset.digest
    $privacyAsset.digest = 'sha256:' + ('0' * 64)
    Assert-Throws { & $run } 'PRIVACY\.md is not byte-identical' 'a draft privacy statement with other bytes is refused'
    $privacyAsset.digest = $privacyDigest
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
    $other = New-ReleaseSet 'provenance-other' $commit $repo
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

    Start-Scenario 'tigermark'
    foreach ($bad in @('', 'main', 'v0.9.0', 'cd94b4a', ('A' * 40), ('a' * 39), ('a' * 41), "$('a' * 40) ")) { Assert-True (-not (Test-TigerSetupPinnedCommit $bad)) "'$bad' does not pin a commit" }
    Assert-True (Test-TigerSetupPinnedCommit ('0123456789abcdef' * 3).Substring(0, 40)) 'a full lower-case SHA pins a commit'
    # A stand-in TigerMarkView history: old -> baseline -> pin -> a commit
    # declaring another version. The stand-in "tiger-mark" is a copy of
    # Windows' curl.exe, which runs anywhere and reports its own version.
    $curl = Join-Path $env:SystemRoot 'System32\curl.exe'
    $curlVersion = [regex]::Match((& $curl --version | Select-Object -First 1), '^curl (\d+\.\d+\.\d+)').Groups[1].Value
    $tmv = New-Repository 'tmv'
    $old = Invoke-Git $tmv rev-parse HEAD
    $props = { param([string] $Version) Set-Content -LiteralPath (Join-Path $tmv 'Version.props') -Value "<Project>`n  <PropertyGroup>`n    <Version>$Version</Version>`n  </PropertyGroup>`n</Project>" }
    & $props $curlVersion
    $null = New-Item -ItemType Directory -Path (Join-Path $tmv 'src\TigerMarkView.Cli') -Force
    Set-Content -LiteralPath (Join-Path $tmv 'src\TigerMarkView.Cli\TigerMarkView.Cli.csproj') -Value '<Project Sdk="Microsoft.NET.Sdk" />'
    Invoke-Git $tmv add . | Out-Null
    Invoke-Git $tmv commit --quiet -m 'baseline' | Out-Null
    $baseline = Invoke-Git $tmv rev-parse HEAD
    Set-Content -LiteralPath (Join-Path $tmv 'README.md') -Value 'pinned'
    Invoke-Git $tmv commit --quiet -am 'pin' | Out-Null
    $pin = Invoke-Git $tmv rev-parse HEAD
    & $props '0.0.1'
    Invoke-Git $tmv commit --quiet -am 'another version' | Out-Null
    $otherVersion = Invoke-Git $tmv rev-parse HEAD
    Invoke-Git $tmv push --quiet origin HEAD:main | Out-Null
    $tmvOrigin = Join-Path $script:root 'tmv.git'

    $global:FakeDotNet = @{ exitCode = 0; produce = 'curl'; curl = $curl; calls = [Collections.Generic.List[string]]::new() }
    Set-TigerSetupReleaseDotNet {
        $global:FakeDotNet.calls.Add($args -join ' ')
        $global:LASTEXITCODE = $global:FakeDotNet.exitCode
        if ($global:FakeDotNet.exitCode -ne 0) { return 'error MSB0000: the build failed' }
        $output = $args[[array]::IndexOf($args, '--output') + 1]
        $null = New-Item -ItemType Directory -Path $output -Force
        switch ($global:FakeDotNet.produce) {
            'curl' { Copy-Item -LiteralPath $global:FakeDotNet.curl -Destination (Join-Path $output 'tiger-mark.exe') }
            'garbage' { Set-Content -LiteralPath (Join-Path $output 'tiger-mark.exe') -Value 'not a program' }
        }
        'published'
    }
    $build = {
        param([string] $Commit, [string] $Name, [string] $Repository = $tmvOrigin)
        $global:FakeDotNet.calls.Clear()
        Build-TigerSetupTigerMark -Commit $Commit -Directory (Join-Path $script:root $Name) -Repository $Repository -SecurityBaseline $baseline
    }
    $built = & $build $pin 'tm-pass'
    Assert-True ($built.commit -ceq $pin -and $built.version -ceq $curlVersion -and (Test-Path -LiteralPath $built.path -PathType Leaf)) 'the pinned commit builds a tiger-mark reporting its Version.props version'
    Assert-True ((Invoke-Git (Join-Path $script:root 'tm-pass\source') rev-parse HEAD) -ceq $pin) 'the checkout is exactly the pinned commit'
    Assert-True ($global:FakeDotNet.calls.Count -eq 1 -and $global:FakeDotNet.calls[0] -match '^publish \S+\\src\\TigerMarkView\.Cli\\TigerMarkView\.Cli\.csproj --configuration Release --runtime win-x64 --self-contained false --output \S+\\tm-pass\\tiger-mark ') 'only the CLI project is published, framework-dependent for win-x64'
    Assert-Throws { & $build $pin 'tm-pass' } 'not empty' 'a directory holding an earlier build is refused'
    foreach ($bad in @('cd94b4a', 'main', $pin.ToUpperInvariant())) {
        Assert-Throws { & $build $bad "tm-bad-$([guid]::NewGuid().ToString('N'))" } 'does not pin' "'$bad' is refused before anything is fetched"
    }
    Assert-Throws { & $build $pin 'tm-noclone' (Join-Path $script:root 'no-such-repository.git') } 'Cloning TigerMarkView' 'a source that cannot be obtained fails'
    Assert-Throws { & $build ('0' * 40) 'tm-nocommit' } 'Checking out TigerMarkView' 'a commit the source does not hold fails'
    Assert-Throws { & $build $old 'tm-old' } 'security baseline' 'a commit older than the security baseline is refused'
    Assert-True ($global:FakeDotNet.calls.Count -eq 0) 'a refused source is never built'
    Assert-Throws { Assert-TigerSetupSourceCheckout -Path (Join-Path $script:root 'tm-pass\source') -Commit $baseline -Baseline $baseline } 'not the pinned' 'a checkout at another commit is refused'
    Set-Content -LiteralPath (Join-Path $script:root 'tm-pass\source\stray.txt') -Value 'x'
    Assert-Throws { Assert-TigerSetupSourceCheckout -Path (Join-Path $script:root 'tm-pass\source') -Commit $pin -Baseline $baseline } 'not clean' 'a checkout that is not clean is refused'
    $global:FakeDotNet.exitCode = 1
    Assert-Throws { & $build $pin 'tm-buildfail' } 'dotnet publish .* failed \(1\)[\s\S]*MSB0000' 'a failed build fails with its output'
    $global:FakeDotNet.exitCode = 0
    $global:FakeDotNet.produce = 'nothing'
    Assert-Throws { & $build $pin 'tm-noexe' } 'produced no' 'a build without tiger-mark.exe fails'
    $global:FakeDotNet.produce = 'garbage'
    Assert-Throws { & $build $pin 'tm-garbage' } 'does not run' 'a tiger-mark.exe that cannot run fails'
    $global:FakeDotNet.produce = 'curl'
    Assert-Throws { & $build $otherVersion 'tm-wrong' } 'not TigerMarkView 0\.0\.1' 'a tiger-mark reporting another version fails'
    # The script offers no way around the real baseline: a history without
    # TigerMarkView's security fix is refused however it is pinned.
    $script = Join-Path $PSScriptRoot 'Build-TigerMark.ps1'
    $githubOutput = Join-Path $script:root 'tm-github-output.txt'
    $global:FakeDotNet.calls.Clear()
    Assert-Throws { & $script -Commit $pin -Repository $tmvOrigin -Directory (Join-Path $script:root 'tm-script') -GitHubOutput $githubOutput 6>$null } "does not contain $((Get-TigerSetupReleaseFacts).TigerMarkViewSecurityBaseline)" 'the script enforces the real security baseline'
    Assert-True (-not (Test-Path -LiteralPath $githubOutput) -and $global:FakeDotNet.calls.Count -eq 0) 'a refused source hands the workflow nothing and builds nothing'
    Assert-Throws { & $script -Commit 'main' -Repository $tmvOrigin -Directory (Join-Path $script:root 'tm-script-main') 6>$null } 'does not pin' 'the script refuses a floating reference'
    # The release workflow builds tiger-mark from a pinned commit and nothing
    # else: no TigerMarkView release, installer or hash of one is left to fall
    # back to.
    $releaseRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $workflow = Get-Content -LiteralPath (Join-Path $releaseRoot '.github\workflows\release.yml') -Raw
    $pinned = [regex]::Match($workflow, "(?m)^\s+TIGERMARKVIEW_COMMIT:\s*'([^']*)'\s*$")
    Assert-True ($pinned.Success -and (Test-TigerSetupPinnedCommit $pinned.Groups[1].Value)) 'the release workflow pins a full TigerMarkView commit'
    Assert-True ($workflow -match '\./eng/release/Build-TigerMark\.ps1\s+-Commit \$env:TIGERMARKVIEW_COMMIT\s') 'the release workflow builds tiger-mark from that commit'
    Assert-True ($workflow -notmatch 'Install-TigerMark|TIGERMARKVIEW_(VERSION|SHA256)|releases/download|-setup\.exe') 'the release workflow names no TigerMarkView release or installer'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'Install-TigerMark.ps1'))) 'the installer-based provisioning is gone'
    $installerReference = 'TigerMarkView/releases|TigerMarkView-[^/\s]*-setup\.exe'
    $referencing = @(Get-ChildItem -LiteralPath $PSScriptRoot -File | Where-Object { $_.Name -cne 'Test-Release.ps1' -and (Get-Content -LiteralPath $_.FullName -Raw) -match $installerReference })
    Assert-True ($referencing.Count -eq 0) "no release script downloads a TigerMarkView release ($(@($referencing | ForEach-Object Name) -join ', '))"

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

    Start-Scenario 'ref'
    Assert-True ((Test-TigerSetupWorkflowRef).status -ceq 'NOT RUN') 'outside a run, with no ref named, the check is NOT RUN'
    Assert-True ((Test-TigerSetupWorkflowRef -Ref 'refs/heads/main').status -ceq 'PASS') 'a simulated run on main passes'
    Assert-True ((Test-TigerSetupWorkflowRef -Ref 'refs/heads/dev').status -ceq 'FAIL') 'a simulated run on dev fails'
    [Environment]::SetEnvironmentVariable('GITHUB_ACTIONS', 'true')
    try {
        foreach ($case in @(@{ ref = 'refs/heads/main'; status = 'PASS' }, @{ ref = 'refs/heads/Main'; status = 'FAIL' }, @{ ref = 'main'; status = 'FAIL' },
                @{ ref = 'refs/heads/dev'; status = 'FAIL' }, @{ ref = 'refs/tags/v1.2.3'; status = 'FAIL' }, @{ ref = ''; status = 'FAIL' })) {
            [Environment]::SetEnvironmentVariable('GITHUB_REF', $case.ref)
            Assert-True ((Test-TigerSetupWorkflowRef).status -ceq $case.status) "inside GitHub Actions, GITHUB_REF '$($case.ref)' is $($case.status)"
        }
        [Environment]::SetEnvironmentVariable('GITHUB_REF', 'refs/heads/dev')
        Assert-True ((Test-TigerSetupWorkflowRef -Ref 'refs/heads/main').status -ceq 'FAIL') 'inside GitHub Actions a named main cannot stand in for the run''s own dev'
    }
    finally { foreach ($name in 'GITHUB_ACTIONS', 'GITHUB_REF') { [Environment]::SetEnvironmentVariable($name, $null) } }

    Start-Scenario 'workflows'
    # The workflows as GitHub runs them: what starts them, the order of jobs
    # and steps, where each script is called, and the preflight using the
    # release workflow's own actions.
    $releaseText = Get-Content -LiteralPath (Join-Path $releaseRoot '.github\workflows\release.yml') -Raw
    $ciText = Get-Content -LiteralPath (Join-Path $releaseRoot (Get-TigerSetupReleaseFacts).RunnerDiagnosticsWorkflowPath) -Raw
    foreach ($pair in @(@('release.yml', $releaseText), @('ci.yml', $ciText))) {
        Assert-True ($pair[1] -match '(?m)^  workflow_dispatch:' -and $pair[1] -notmatch '(?m)^  (push|pull_request|pull_request_target|schedule|workflow_run|workflow_call):') "$($pair[0]) is started by hand only"
        Assert-True ($pair[1] -match '(?m)^permissions:\s*\r?\n\s+contents: read\s*$') "$($pair[0]) reads by default"
        Assert-True ($pair[1] -notmatch 'continue-on-error') "no step of $($pair[0]) may fail without failing its job"
    }
    $releaseJobs = Get-WorkflowJob $releaseText
    Assert-True ((@($releaseJobs.Keys) -join ',') -ceq 'prerequisites,build,publish') 'the release workflow is prerequisites, build, publish'
    Assert-True ($releaseJobs.prerequisites -match '\./eng/release/Assert-ReleaseCommitReady\.ps1 -Version \$env:RELEASE_VERSION -CommitSha \$env:GITHUB_SHA' -and $releaseJobs.prerequisites -notmatch '(?m)^\s+if:') 'the prerequisites job always runs the gate, which checks the ref first'
    Assert-True ($releaseJobs.build -match '(?m)^\s+needs: prerequisites\s*$' -and $releaseJobs.publish -match '(?m)^\s+needs: build\s*$') 'nothing is built before the prerequisites pass, and nothing published before the build'
    Assert-True ($releaseJobs.build -notmatch '(?m)^\s+if:' -and $releaseJobs.publish -notmatch '(?m)^\s+if:') 'no condition lets a job run past a failed one'
    $order = @('Test-ReleaseRunner\.ps1', 'Build-TigerMark\.ps1', 'Build-ReleaseArtifacts\.ps1' | ForEach-Object { [regex]::Match($releaseJobs.build, $_).Index })
    Assert-True ($order[0] -gt 0 -and $order[0] -lt $order[1] -and $order[1] -lt $order[2]) 'the build job checks the runner, then builds tiger-mark and proves it renders, then builds the release'
    Assert-True ($releaseJobs.publish -match '(?m)^\s+environment: release\s*$' -and $releaseJobs.publish -match '(?m)^\s+contents: write\s*$' -and $releaseJobs.publish -match 'Publish-DraftRelease\.ps1') 'only the publish job writes, in the release environment'
    Assert-True (@([regex]::Matches($releaseText, 'contents: write')).Count -eq 1) 'no other job of the release workflow may write'
    $checkouts = @([regex]::Matches($releaseText, 'uses: actions/checkout@')).Count
    Assert-True ($checkouts -eq 3 -and @([regex]::Matches($releaseText, 'ref: \$\{\{ github\.sha \}\}')).Count -eq $checkouts) 'every release job checks out exactly the commit the run names'
    $ciJobs = Get-WorkflowJob $ciText
    $facts = Get-TigerSetupReleaseFacts
    Assert-True ($ciText -match '(?m)^run-name: Runner diagnostics - \$\{\{ inputs\.diagnostic \}\}\s*$' -and $facts.RunnerReadinessRunTitle -ceq 'Runner diagnostics - release-readiness' -and
        $ciText -match '(?m)^\s+default: release-readiness\s*$' -and $ciText -match '(?m)^\s+- release-readiness\s*$') 'a release-readiness run is titled as Test-ReleaseReadiness.ps1 looks for it'
    $uses = { param([string[]] $Texts) @($Texts | ForEach-Object { [regex]::Matches($_, '(?m)^\s+(-\s+)?uses:\s*(?<a>\S+)') | ForEach-Object { $_.Groups['a'].Value } } | Sort-Object -Unique) -join ',' }
    $preflightJobs = @($ciJobs.Keys | Where-Object { $_ -like 'release-readiness*' })
    Assert-True ($preflightJobs.Count -eq 2 -and (& $uses @($preflightJobs | ForEach-Object { $ciJobs[$_] })) -ceq (& $uses @($releaseText))) 'the preflight uses exactly the release workflow''s actions, at its versions'
    $preflight = $ciJobs['release-readiness']
    Assert-True ($preflight -match 'Test-ReleaseRunner\.ps1 -GitHubAccess' -and $preflight -match 'Build-TigerMark\.ps1 -Directory' -and $preflight -notmatch 'Build-TigerMark\.ps1[^\r\n]*-Commit' -and
        $preflight -match 'cargo fetch --locked' -and $preflight -match 'Test-Release\.ps1') 'the preflight checks the runner, builds tiger-mark from the release pin and renders, fetches the crates and runs these tests'
    Assert-True ($ciText -notmatch 'contents: write|Publish-DraftRelease|Build-ReleaseArtifacts|environment:') 'the runner diagnostics build no release and write nothing'

    Start-Scenario 'runner'
    # Without git, Rust, .NET or gh on PATH the check fails on each, in one report.
    $runnerScript = Join-Path $PSScriptRoot 'Test-ReleaseRunner.ps1'
    $pwshPath = (Get-Process -Id $PID).Path
    $savedPath = $env:PATH
    try { $env:PATH = "$env:SystemRoot\System32"; $text = & $pwshPath -NoProfile -File $runnerScript 2>&1 | Out-String; $code = $LASTEXITCODE }
    finally { $env:PATH = $savedPath }
    Assert-True ($code -eq 1) "a machine without the tools fails (exit $code)"
    foreach ($id in 'git', 'rust', 'dotnet', 'gh') { Assert-True ((Get-CheckStatus $text $id) -ceq 'FAIL') "missing $id fails" }
    foreach ($id in 'powershell', 'git', 'rust', 'msvc', 'windows-sdk', 'dotnet', 'webview2', 'gh') { Assert-True ((Get-CheckStatus $text $id) -ne '') "the report names $id" }

    Start-Scenario 'render'
    $pdf = [Text.Encoding]::Latin1.GetBytes("%PDF-1.4`n1 0 obj << /Title (help.md) >> endobj`n%%EOF`n")
    Assert-True ((Get-TigerSetupPdfTitle -Bytes $pdf) -ceq 'help.md') 'a PDF''s title is read'
    Assert-True ($null -eq (Get-TigerSetupPdfTitle -Bytes ([Text.Encoding]::Latin1.GetBytes("1 0 obj << /Title (help.md) >>`n%%EOF`n")))) 'bytes that are not a PDF have no title'
    Assert-True ($null -eq (Get-TigerSetupPdfTitle -Bytes ([Text.Encoding]::Latin1.GetBytes("%PDF-1.4`n<< /Title (help.md) >>")))) 'a truncated PDF has no title'
    Assert-True ($null -eq (Get-TigerSetupPdfTitle -Bytes $null)) 'nothing has no title'
    Assert-Throws { Test-TigerSetupTigerMarkRender -TigerMarkPath $curl -Directory (New-Directory 'render-curl') } 'did not render' 'a tiger-mark that writes no PDF fails'
    Assert-Throws { Test-TigerSetupTigerMarkRender -TigerMarkPath (Join-Path $script:root 'no-tiger-mark.exe') -Directory (New-Directory 'render-none') } 'does not run' 'a tiger-mark that is not there fails'
    $releasePin = Get-TigerSetupTigerMarkViewPin -RepositoryRoot $releaseRoot
    Assert-True ($releasePin -ceq $pinned.Groups[1].Value) 'the pin is read from the release workflow'
    Assert-Throws { & $script -Repository $tmvOrigin -Directory (Join-Path $script:root 'tm-script-default') 6>$null } "Checking out TigerMarkView $releasePin" 'without -Commit the script builds the release workflow''s pin'

    Start-Scenario 'manifests'
    $installerSha = 'ab' * 32
    $url = Get-TigerSetupInstallerUrl -Version '1.2.3'
    $check = { param([scriptblock] $Change) $directory = New-Directory "manifests-$([guid]::NewGuid().ToString('N'))"; Write-ManifestSet $directory $url $installerSha; if ($Change) { & $Change $directory }; @(Test-TigerSetupWinGetManifestSet -ManifestDirectory $directory -Version '1.2.3' -InstallerSha256 $installerSha) }
    Assert-True (@(& $check $null).Count -eq 0) 'the manifest set of the installer passes'
    foreach ($case in @(
            @{ name = 'a missing locale manifest'; change = { param($d) Remove-Item -LiteralPath (Join-Path $d 'ItTiger.TigerSetup.locale.en-US.yaml') } },
            @{ name = 'a fourth file'; change = { param($d) Set-Content -LiteralPath (Join-Path $d 'extra.yaml') -Value 'PackageIdentifier: ItTiger.TigerSetup' } },
            @{ name = 'another version'; change = { param($d) $p = Join-Path $d 'ItTiger.TigerSetup.yaml'; Set-Content -LiteralPath $p -Value ((Get-Content -LiteralPath $p -Raw).Replace('1.2.3', '1.2.4')) } },
            @{ name = 'another installer URL'; change = { param($d) $p = Join-Path $d 'ItTiger.TigerSetup.installer.yaml'; Set-Content -LiteralPath $p -Value ((Get-Content -LiteralPath $p -Raw).Replace('/v1.2.3/', '/v1.2.2/')) } },
            @{ name = 'another installer hash'; change = { param($d) $p = Join-Path $d 'ItTiger.TigerSetup.installer.yaml'; Set-Content -LiteralPath $p -Value ((Get-Content -LiteralPath $p -Raw).Replace(('AB' * 32), ('CD' * 32))) } },
            @{ name = 'a second installer entry with another hash'; change = { param($d) Add-Content -LiteralPath (Join-Path $d 'ItTiger.TigerSetup.installer.yaml') -Value "- Architecture: arm64`n  InstallerUrl: $url`n  InstallerSha256: $('CD' * 32)" } },
            @{ name = 'a privacy statement on main'; change = { param($d) Set-Content -LiteralPath (Join-Path $d 'ItTiger.TigerSetup.locale.en-US.yaml') -Value (Get-LocaleManifestText (Get-TigerSetupReleaseAssetUrl -Version '1.2.3' -Name 'LICENSE.txt') 'https://github.com/rkozlowski/TigerSetup/blob/main/PRIVACY.md') } })) {
        Assert-True (@(& $check $case.change).Count -gt 0) "$($case.name) is refused"
    }

    Start-Scenario 'retrieve'
    # Get-ReleaseArtifacts.ps1 as release validation runs it, up to the
    # installer, which needs a real builder: the real release proves that part
    # in preparation (RELEASING.md). A draft is retrieved through the API, its
    # bytes proved against GitHub's digests, the record, the tag the release
    # workflow made, the run that tag names and the commit's terms.
    $retrieve = Join-Path $PSScriptRoot 'Get-ReleaseArtifacts.ps1'
    $runId = '42'
    $newReleasedRepository = {
        param([string] $Name, [switch] $Untagged)
        $repository = New-Repository $Name
        Add-Terms $repository
        Invoke-Git $repository push --quiet origin HEAD:main | Out-Null
        $sha = Invoke-Git $repository rev-parse HEAD
        $directory = New-Directory "$Name-set"
        [IO.File]::WriteAllBytes((Join-Path $directory 'TigerSetup-1.2.3-Setup.exe'), [byte[]](1..250))
        $manifestDirectory = New-Directory "$Name-manifests"
        Write-ManifestSet $manifestDirectory (Get-TigerSetupInstallerUrl -Version '1.2.3') (Get-TigerSetupFileSha256 (Join-Path $directory 'TigerSetup-1.2.3-Setup.exe'))
        Compress-Archive -Path (Join-Path $manifestDirectory '*') -DestinationPath (Join-Path $directory 'TigerSetup-1.2.3-WinGet.zip')
        $null = Copy-TigerSetupReleaseTerms -RepositoryRoot $repository -Directory $directory -Version '1.2.3' -CommitSha $sha
        $record = Write-TigerSetupReleaseRecord -Directory $directory -Version '1.2.3' -CommitSha $sha
        if (-not $Untagged) {
            # The tag as the release workflow makes it: annotated, by the bot,
            # naming the record and the run.
            $message = Get-TigerSetupTagMessage -Version '1.2.3' -RecordSha256 $record -RunUrl "https://github.com/rkozlowski/TigerSetup/actions/runs/$runId"
            $saved = @{ GIT_COMMITTER_NAME = $env:GIT_COMMITTER_NAME; GIT_COMMITTER_EMAIL = $env:GIT_COMMITTER_EMAIL }
            $env:GIT_COMMITTER_NAME = 'github-actions[bot]'; $env:GIT_COMMITTER_EMAIL = '41898282+github-actions[bot]@users.noreply.github.com'
            try { Invoke-Git $repository tag -a v1.2.3 $sha -m $message | Out-Null } finally { foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) } }
            Invoke-Git $repository push --quiet origin v1.2.3 | Out-Null
        }
        [pscustomobject]@{ repository = $repository; commit = $sha; directory = $directory }
    }
    $released = & $newReleasedRepository 'retrieve'
    $setFiles = @(Get-ChildItem -LiteralPath $released.directory -File)
    $draft = { [pscustomobject]@{ tag_name = 'v1.2.3'; name = 'TigerSetup 1.2.3'; draft = $true; html_url = 'https://example.invalid/v1.2.3'; assets = @($setFiles | ForEach-Object { New-FakeAsset $_.FullName }) } }
    $runRecord = { param([hashtable] $Change = @{}) $value = @{ path = '.github/workflows/release.yml'; status = 'completed'; event = 'workflow_dispatch'; head_branch = 'main'; head_sha = $released.commit; conclusion = 'success'; name = 'Release TigerSetup 1.2.3'; html_url = "https://github.com/rkozlowski/TigerSetup/actions/runs/$runId" }; foreach ($k in $Change.Keys) { $value[$k] = $Change[$k] }; $value }
    $global:FakeDownloads = [Collections.Generic.List[string]]::new()
    $global:FakeDownloadSource = $released.directory
    $global:FakeDownloadCorrupt = ''
    Set-TigerSetupReleaseDownloader {
        param($Url, $Path, $Token)
        $global:FakeDownloads.Add("$Url|$Token")
        $name = Split-Path -Leaf $Url
        if ("throw:$name" -ceq $global:FakeDownloadCorrupt) { throw 'Response status code does not indicate success: 502 (Bad Gateway).' }
        if ($name -ceq $global:FakeDownloadCorrupt) { [IO.File]::WriteAllBytes($Path, [byte[]](9, 9, 9)) }
        else { Copy-Item -LiteralPath (Join-Path $global:FakeDownloadSource $name) -Destination $Path }
    }
    $validate = {
        param([hashtable] $With = @{}, [string] $Repository = $released.repository)
        $arguments = @{ Version = '1.2.3'; RepositoryRoot = $Repository; OutputDirectory = (Join-Path $script:root "retrieve-out-$([guid]::NewGuid().ToString('N'))"); BuilderPath = (Join-Path $script:root 'no-builder.exe') } + $With
        $text = & $retrieve @arguments 6>&1 | Out-String
        [pscustomobject]@{ code = $LASTEXITCODE; text = $text; output = $arguments.OutputDirectory }
    }
    $reset = { Set-FakeGitHub; $global:FakeGh.releases = @(& $draft); $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord); $global:FakeDownloads.Clear(); $global:FakeDownloadCorrupt = '' }

    & $reset
    $result = & $validate
    Assert-True ($result.code -eq 2 -and (Get-CheckStatus $result.text 'installer') -ceq 'BLOCKED') "a draft is proved up to the installer, which needs a builder (exit $($result.code))"
    foreach ($id in 'release', 'bytes', 'record', 'provenance', 'run', 'terms', 'winget') { Assert-True ((Get-CheckStatus $result.text $id) -ceq 'PASS') "the draft's $id passes" }
    Assert-True ($global:FakeDownloads.Count -eq 6 -and @($global:FakeDownloads | Where-Object { $_ -notlike 'https://api.invalid/assets/*|fake-token' }).Count -eq 0) 'every asset is downloaded through the API with gh''s token'
    Assert-True ((@(Get-ChildItem -LiteralPath (Join-Path $result.output 'assets') -File).Name | Sort-Object) -join ',' -ceq (@($setFiles.Name | Sort-Object) -join ',')) 'the six files are retrieved'
    Assert-True (@(Get-ChildItem -LiteralPath (Join-Path $result.output 'winget') -File).Count -eq 3) 'the manifest set is unpacked'
    $published = & $draft; $published.draft = $false
    & $reset; $global:FakeGh.releases = @($published)
    $result = & $validate
    Assert-True ((Get-CheckStatus $result.text 'release') -ceq 'PASS' -and $global:FakeDownloads.Count -eq 6 -and @($global:FakeDownloads | Where-Object { $_ -notlike 'https://api.invalid/*|fake-token' }).Count -eq 0) 'a published release is retrieved exactly as a draft is'

    foreach ($case in @(
            @{ name = 'a run started on dev'; check = 'run'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord @{ head_branch = 'dev' }) } },
            @{ name = 'a run of another workflow'; check = 'run'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord @{ path = '.github/workflows/ci.yml' }) } },
            @{ name = 'a run for another commit'; check = 'run'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord @{ head_sha = ('c' * 40) }) } },
            @{ name = 'a run still in progress'; check = 'run'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord @{ status = 'in_progress'; conclusion = $null }) } },
            @{ name = 'a run that failed'; check = 'run'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["repos/rkozlowski/TigerSetup/actions/runs/$runId"] = (& $runRecord @{ conclusion = 'failure' }) } },
            @{ name = 'a run GitHub does not know'; check = 'run'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api.Remove("repos/rkozlowski/TigerSetup/actions/runs/$runId") } },
            @{ name = 'an asset whose digest differs'; check = 'bytes/TigerSetup-1.2.3-WinGet.zip'; status = 'FAIL'; code = 1; setup = { @($global:FakeGh.releases[0].assets | Where-Object name -CEQ 'TigerSetup-1.2.3-WinGet.zip')[0].digest = 'sha256:' + ('0' * 64) } },
            @{ name = 'a download that is not the recorded bytes'; check = 'bytes/PRIVACY.md'; status = 'FAIL'; code = 1; setup = { $global:FakeDownloadCorrupt = 'PRIVACY.md' } },
            @{ name = 'a download that fails'; check = 'bytes/LICENSE.txt'; status = 'BLOCKED'; code = 2; setup = { $global:FakeDownloadCorrupt = 'throw:LICENSE.txt' } },
            @{ name = 'a foreign asset'; check = 'release'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets) + @([pscustomobject]@{ name = 'other.exe'; size = 1; digest = 'sha256:00'; url = 'https://api.invalid/assets/other.exe' }) } },
            @{ name = 'a missing asset'; check = 'release'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.releases[0].assets = @($global:FakeGh.releases[0].assets | Where-Object name -CNE 'LICENSE.txt') } },
            @{ name = 'another release name'; check = 'release'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.releases[0].name = 'TigerSetup 1.2.4' } },
            @{ name = 'no release'; check = 'release'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.releases = @() } },
            @{ name = 'a GitHub that refuses'; check = 'release'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.apiFails = $true } },
            @{ name = 'gh with no token'; check = 'bytes'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.token = '' } })) {
        & $reset
        & $case.setup
        $result = & $validate
        Assert-True ($result.code -eq $case.code -and (Get-CheckStatus $result.text $case.check) -ceq $case.status) "$($case.name): $($case.check) is $($case.status) (exit $($result.code), $($case.check) $(Get-CheckStatus $result.text $case.check))"
    }
    # A tag a person made for the same record is not the release workflow's.
    $personal = & $newReleasedRepository 'retrieve-personal' -Untagged
    $personalRecord = Get-TigerSetupFileSha256 (Join-Path $personal.directory 'release-artifacts.json')
    Invoke-Git $personal.repository tag -a v1.2.3 $personal.commit -m (Get-TigerSetupTagMessage -Version '1.2.3' -RecordSha256 $personalRecord -RunUrl "https://github.com/rkozlowski/TigerSetup/actions/runs/$runId") | Out-Null
    Invoke-Git $personal.repository push --quiet origin v1.2.3 | Out-Null
    & $reset
    $global:FakeDownloadSource = $personal.directory
    $global:FakeGh.releases = @([pscustomobject]@{ tag_name = 'v1.2.3'; name = 'TigerSetup 1.2.3'; draft = $true; html_url = 'https://example.invalid/v1.2.3'; assets = @(Get-ChildItem -LiteralPath $personal.directory -File | ForEach-Object { New-FakeAsset $_.FullName }) })
    $result = & $validate -Repository $personal.repository
    Assert-True ($result.code -eq 1 -and (Get-CheckStatus $result.text 'provenance') -ceq 'FAIL') 'a set tagged by a person is refused'
    $global:FakeDownloadSource = $released.directory

    # The same checks over a set on disk: what preparation rehearses.
    & $reset
    $result = & $validate @{ ArtifactDirectory = $released.directory }
    Assert-True ($result.code -eq 2 -and $global:FakeDownloads.Count -eq 0) 'a set on disk is proved without downloading anything, up to the installer'
    foreach ($id in 'release', 'bytes') { Assert-True ((Get-CheckStatus $result.text $id) -ceq 'NOT RUN') "a set on disk has no $id to check" }
    foreach ($id in 'record', 'provenance', 'run', 'terms', 'winget') { Assert-True ((Get-CheckStatus $result.text $id) -ceq 'PASS') "a tagged set on disk proves its $id" }
    Assert-True (@(Get-ChildItem -LiteralPath $released.directory -File).Count -eq 6) 'the set on disk is not changed'
    $withForeign = New-Directory 'retrieve-foreign'
    Copy-Item -Path (Join-Path $released.directory '*') -Destination $withForeign
    Set-Content -LiteralPath (Join-Path $withForeign 'notes.txt') -Value 'x'
    $result = & $validate @{ ArtifactDirectory = $withForeign }
    Assert-True ($result.code -eq 1 -and (Get-CheckStatus $result.text 'record') -ceq 'FAIL') 'a set on disk with a foreign file is refused'
    # Before the release action: no tag yet, and a candidate of the prepared tree.
    $untagged = & $newReleasedRepository 'retrieve-untagged' -Untagged
    $result = & $validate @{ ArtifactDirectory = $untagged.directory } -Repository $untagged.repository
    Assert-True ($result.code -eq 2 -and (Get-CheckStatus $result.text 'provenance') -ceq 'NOT RUN' -and (Get-CheckStatus $result.text 'run') -ceq 'NOT RUN' -and (Get-CheckStatus $result.text 'terms') -ceq 'PASS') 'before the tag exists, provenance and run are NOT RUN and the commit''s terms are proved'
    [IO.File]::WriteAllText((Join-Path $untagged.repository 'PRIVACY.md'), "# Privacy`n`nPrepared for 1.2.3.`n")
    $candidate = New-Directory 'retrieve-candidate'
    Copy-Item -LiteralPath (Join-Path $untagged.directory 'TigerSetup-1.2.3-Setup.exe'), (Join-Path $untagged.directory 'TigerSetup-1.2.3-WinGet.zip') -Destination $candidate
    $null = Copy-TigerSetupReleaseTerms -RepositoryRoot $untagged.repository -Directory $candidate -Version '1.2.3' -CommitSha $untagged.commit -WorkingTree
    $null = Write-TigerSetupReleaseRecord -Directory $candidate -Version '1.2.3' -CommitSha $untagged.commit
    $result = & $validate @{ ArtifactDirectory = $candidate; Rehearsal = $true } -Repository $untagged.repository
    Assert-True ($result.code -eq 2 -and (Get-CheckStatus $result.text 'terms') -ceq 'PASS') 'a rehearsal proves the prepared working tree''s terms'
    $result = & $validate @{ ArtifactDirectory = $candidate } -Repository $untagged.repository
    Assert-True ($result.code -eq 1 -and (Get-CheckStatus $result.text 'terms') -ceq 'FAIL') 'without -Rehearsal, terms that are not the commit''s are refused'
    Assert-Throws { & $retrieve -Version '1.2.3' -Rehearsal -RepositoryRoot $released.repository -OutputDirectory (New-Directory 'retrieve-bad') 6>$null } 'ArtifactDirectory' '-Rehearsal without a set on disk is refused'
    $occupied = New-Directory 'retrieve-occupied'
    Set-Content -LiteralPath (Join-Path $occupied 'mine.txt') -Value 'x'
    Assert-Throws { & $retrieve -Version '1.2.3' -ArtifactDirectory $released.directory -RepositoryRoot $released.repository -OutputDirectory $occupied 6>$null } 'did not write' 'an output directory holding other files is not emptied'
    Assert-True (Test-Path -LiteralPath (Join-Path $occupied 'mine.txt')) 'the other files are still there'

    Start-Scenario 'readiness'
    $readiness = Join-Path $PSScriptRoot 'Test-ReleaseReadiness.ps1'
    $repo = New-Repository 'readiness'
    foreach ($file in @('.github\workflows\release.yml', 'eng\release\Tool.ps1', '.cargo\config.toml', 'Cargo.lock')) {
        $path = Join-Path $repo $file
        $null = New-Item -ItemType Directory -Path (Split-Path -Parent $path) -Force
        Set-Content -LiteralPath $path -Value "# $file"
    }
    Invoke-Git $repo add . | Out-Null
    Invoke-Git $repo commit --quiet -m 'runner-facing files' | Out-Null
    Invoke-Git $repo push --quiet origin HEAD:main | Out-Null
    $preflighted = Invoke-Git $repo rev-parse HEAD
    Set-Content -LiteralPath (Join-Path $repo 'README.md') -Value 'documentation only'
    Invoke-Git $repo commit --quiet -am 'docs' | Out-Null
    $docsOnly = Invoke-Git $repo rev-parse HEAD
    Set-Content -LiteralPath (Join-Path $repo 'eng\release\Tool.ps1') -Value '# changed'
    Invoke-Git $repo commit --quiet -am 'tooling' | Out-Null
    $tooling = Invoke-Git $repo rev-parse HEAD
    $api = 'repos/rkozlowski/TigerSetup'
    $configured = {
        Set-FakeGitHub
        $global:FakeGh.api[$api] = @{ visibility = 'public'; default_branch = 'main'; archived = $false; permissions = @{ push = $true } }
        $global:FakeGh.api["$api/actions/permissions"] = @{ enabled = $true; allowed_actions = 'all'; sha_pinning_required = $false }
        $global:FakeGh.api["$api/environments/release"] = @{ name = 'release'; deployment_branch_policy = @{ custom_branch_policies = $true; protected_branches = $false }; protection_rules = @(@{ type = 'branch_policy' }) }
        $global:FakeGh.api["$api/environments/release/deployment-branch-policies"] = @{ branch_policies = @(@{ name = 'main'; type = 'branch' }) }
        $global:FakeGh.api["$api/rulesets?includes_parents=true&per_page=100"] = @()
        $global:FakeGh.api["$api/actions/workflows/ci.yml/runs?status=success&per_page=50"] = @{ workflow_runs = @(
                @{ display_title = 'Runner diagnostics - elevated-tests'; conclusion = 'success'; head_sha = $tooling; head_branch = 'dev'; created_at = [DateTimeOffset]::UtcNow.ToString('o'); html_url = 'https://example.invalid/runs/8' },
                @{ display_title = 'Runner diagnostics - release-readiness'; conclusion = 'success'; head_sha = $preflighted; head_branch = 'dev'; created_at = [DateTimeOffset]::UtcNow.AddDays(-1).ToString('o'); html_url = 'https://example.invalid/runs/7' }) }
    }
    $ready = {
        param([string] $At = $docsOnly)
        $text = & $readiness -CommitSha $At -RepositoryRoot $repo 6>&1 | Out-String
        [pscustomobject]@{ code = $LASTEXITCODE; text = $text }
    }
    & $configured
    $result = & $ready
    Assert-True ($result.code -eq 0) "a configured repository with a preflight of this tree's runner-facing files is ready (exit $($result.code))"
    Assert-True ($result.text -match 'runs/7 succeeded') 'the preflight that covers the tree is named'
    $result = & $ready $tooling
    Assert-True ($result.code -eq 2 -and (Get-CheckStatus $result.text 'runner/preflight') -ceq 'BLOCKED') 'a change to the release tooling needs a new preflight; an elevated-tests run is not one'
    foreach ($case in @(
            @{ name = 'a release environment for every branch'; check = 'config/environment'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/environments/release"].deployment_branch_policy = $null } },
            @{ name = 'a release environment for main and dev'; check = 'config/environment'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/environments/release/deployment-branch-policies"] = @{ branch_policies = @(@{ name = 'main'; type = 'branch' }, @{ name = 'dev'; type = 'branch' }) } } },
            @{ name = 'a release environment for protected branches'; check = 'config/environment'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/environments/release"].deployment_branch_policy = @{ custom_branch_policies = $false; protected_branches = $true } } },
            @{ name = 'a release environment for a tag named main'; check = 'config/environment'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/environments/release/deployment-branch-policies"] = @{ branch_policies = @(@{ name = 'main'; type = 'tag' }) } } },
            @{ name = 'no release environment'; check = 'config/environment'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api.Remove("$api/environments/release") } },
            @{ name = 'a release environment this login cannot read'; check = 'config/environment'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.api["$api/environments/release"] = 'HTTP 403: Resource not accessible' } },
            @{ name = 'Actions disabled, answered without allowed_actions'; check = 'config/actions'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/actions/permissions"] = @{ enabled = $false } } },
            @{ name = 'an environment answered without protection rules'; check = 'config/environment'; status = 'PASS'; code = 0; setup = { $global:FakeGh.api["$api/environments/release"].Remove('protection_rules') } },
            @{ name = 'an active tag ruleset'; check = 'config/rulesets'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.api["$api/rulesets?includes_parents=true&per_page=100"] = @(@{ name = 'tags'; target = 'tag'; enforcement = 'active' }) } },
            @{ name = 'Actions disabled'; check = 'config/actions'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/actions/permissions"].enabled = $false } },
            @{ name = 'actions pinned by SHA'; check = 'config/actions'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/actions/permissions"].sha_pinning_required = $true } },
            @{ name = 'local actions only'; check = 'config/actions'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api["$api/actions/permissions"].allowed_actions = 'local_only' } },
            @{ name = 'a private repository'; check = 'config/repository'; status = 'FAIL'; code = 1; setup = { $global:FakeGh.api[$api].visibility = 'private' } },
            @{ name = 'no preflight at all'; check = 'runner/preflight'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.api["$api/actions/workflows/ci.yml/runs?status=success&per_page=50"] = @{ workflow_runs = @() } } },
            @{ name = 'a login without push access'; check = 'local/gh'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.api[$api].permissions.push = $false } },
            @{ name = 'gh that cannot read the repository'; check = 'config/repository'; status = 'BLOCKED'; code = 2; setup = { $global:FakeGh.apiFails = $true } })) {
        & $configured
        & $case.setup
        $result = & $ready
        Assert-True ($result.code -eq $case.code -and (Get-CheckStatus $result.text $case.check) -ceq $case.status) "$($case.name): $($case.check) is $($case.status) (exit $($result.code), $($case.check) $(Get-CheckStatus $result.text $case.check))"
    }
    & $configured
    $global:FakeGh.api["$api/actions/permissions"].allowed_actions = 'selected'
    $global:FakeGh.api["$api/actions/permissions/selected-actions"] = @{ github_owned_allowed = $true }
    Assert-True ((& $ready).code -eq 0) 'selected actions that include the GitHub-owned ones are enough'
    Invoke-Git (Join-Path $script:root 'readiness.git') config receive.denyDeleteCurrent ignore | Out-Null
    Invoke-Git $repo push --quiet origin --delete main | Out-Null
    $result = & $ready
    Assert-True ((Get-CheckStatus $result.text 'config/workflow') -ceq 'FAIL') 'a main without the release workflow cannot start it'
}
finally {
    Set-TigerSetupReleaseGitHubCli $null
    Set-TigerSetupReleaseDotNet $null
    Set-TigerSetupReleaseDownloader $null
    foreach ($name in 'FakeGh', 'FakeDotNet', 'FakeDownloads', 'FakeDownloadSource', 'FakeDownloadCorrupt') { Remove-Variable -Name $name -Scope Global -ErrorAction SilentlyContinue }
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
