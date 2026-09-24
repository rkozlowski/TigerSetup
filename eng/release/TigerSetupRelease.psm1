<#
    .SYNOPSIS
    TigerSetup's release facts and the checks every release stage shares.

    .DESCRIPTION
    The Tiger release model (TigerAiCore `docs/release-model.md`) defines the
    lifecycle; this module holds what TigerSetup releases and how its stages
    prove each other: the version source, the artifact set and its names, the
    release notes, the closed artifact record (`release-artifacts.json` schema 1
    and `SHA256SUMS.txt`), the commit and tag gates, and the GitHub
    Release lookup. `RELEASING.md` describes the lifecycle as TigerSetup runs
    it.

    Every function runs the same way on a developer machine and on a GitHub
    Actions runner. GitHub is reached only through `gh` (authenticated by
    `gh auth login` locally, or by GH_TOKEN in a workflow) and through `git`;
    the tests replace `gh` with Set-TigerSetupReleaseGitHubCli.
#>

Set-StrictMode -Version Latest

$script:Facts = [pscustomobject][ordered]@{
    Product = 'TigerSetup'
    Repository = 'rkozlowski/TigerSetup'
    DefaultBranch = 'main'
    ReleaseWorkflowName = 'Release TigerSetup'
    TagPrefix = 'v'
    PackageIdentifier = 'ItTiger.TigerSetup'
    PackageManifest = 'packages/tigersetup/TigerSetup.toml'
    ReleaseNotesDirectory = '.github/release-notes'
}

$script:GitHubCli = $null

function Get-TigerSetupReleaseFacts {
    <#
        .SYNOPSIS
        What TigerSetup releases and where: the project's release data.
    #>
    [CmdletBinding()]
    param()
    $script:Facts
}

function Get-TigerSetupReleaseAsset {
    <#
        .SYNOPSIS
        The closed asset set of one release: the payloads, then the two records.

        .DESCRIPTION
        `kind` is the release-artifacts.json vocabulary shared by the Tiger
        release model. The installer is the product; the WinGet manifest set is
        generated from that installer's exact bytes and the URL it will be
        published at, and travels with it so the release carries its own
        submission.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)

    @(
        [pscustomobject]@{ name = "TigerSetup-$Version-Setup.exe"; kind = 'WindowsInstaller' }
        [pscustomobject]@{ name = "TigerSetup-$Version-WinGet.zip"; kind = 'WinGetManifests' }
    )
}

function Get-TigerSetupReleaseAssetName {
    <#
        .SYNOPSIS
        Every file a release carries: the payloads and the two records.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)

    @((Get-TigerSetupReleaseAsset -Version $Version).name) + @('SHA256SUMS.txt', 'release-artifacts.json')
}

function Get-TigerSetupReleaseTag {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)
    $script:Facts.TagPrefix + $Version
}

function Get-TigerSetupInstallerUrl {
    <#
        .SYNOPSIS
        The public, version-specific URL the installer is published at.

        .DESCRIPTION
        GitHub serves a release asset at a URL made of the repository, the tag
        and the asset name, so the URL is known before the release exists. The
        WinGet manifests are finalized with it at build time; verification after
        publication downloads it and compares the bytes.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)

    $installer = (Get-TigerSetupReleaseAsset -Version $Version | Where-Object kind -CEQ 'WindowsInstaller').name
    "https://github.com/$($script:Facts.Repository)/releases/download/$(Get-TigerSetupReleaseTag -Version $Version)/$installer"
}

function Test-TigerSetupReleaseVersion {
    <#
        .SYNOPSIS
        True for a product version: exactly <Major>.<Minor>.<Patch>.
    #>
    [CmdletBinding()]
    param([AllowEmptyString()] [string] $Version)
    $Version -cmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
}

function Get-TigerSetupSourceVersion {
    <#
        .SYNOPSIS
        The product version as the tree records it: `[workspace.package]
        version` in the workspace Cargo.toml, the one place it is written.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $RepositoryRoot)

    $section = ''
    foreach ($line in Get-Content -LiteralPath (Join-Path $RepositoryRoot 'Cargo.toml')) {
        $trimmed = $line.Trim()
        if ($trimmed -match '^\[(?<name>[^\]]+)\]$') { $section = $Matches['name']; continue }
        if ($section -ceq 'workspace.package' -and $trimmed -match '^version\s*=\s*"(?<version>[^"]*)"') {
            return $Matches['version']
        }
    }
    throw "Cargo.toml in '$RepositoryRoot' declares no [workspace.package] version."
}

function New-TigerSetupReleaseCheck {
    <#
        .SYNOPSIS
        One gate result. PASS proceeds; BLOCKED means a human or external step
        has not happened yet (wait, then rerun); FAIL means the data is wrong;
        NOT RUN means the check could not run here, and its observation says
        where it runs instead. NOT RUN is never counted as PASS.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Id,
        [Parameter(Mandatory)] [ValidateSet('PASS', 'BLOCKED', 'FAIL', 'NOT RUN')] [string] $Status,
        [Parameter(Mandatory)] [string] $Observed,
        [string] $Remediation = ''
    )
    [pscustomobject][ordered]@{ id = $Id; status = $Status; observed = $Observed; remediation = $Remediation }
}

function Write-TigerSetupReleaseReport {
    <#
        .SYNOPSIS
        Prints a gate's checks, appends them to the workflow summary when there
        is one, and returns the exit code: 0 PASS, 2 BLOCKED, 1 FAIL. A gate
        whose other checks passed but which has NOT RUN checks reports
        "PASS, n NOT RUN" and exits 0: what did not run is named, not hidden.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Title,
        [Parameter(Mandatory)] [object[]] $Checks,
        [string[]] $Next = @(),
        [string] $StepSummaryPath = $env:GITHUB_STEP_SUMMARY
    )

    $status = if (@($Checks | Where-Object status -CEQ 'FAIL').Count) { 'FAIL' }
    elseif (@($Checks | Where-Object status -CEQ 'BLOCKED').Count) { 'BLOCKED' }
    else { 'PASS' }
    $notRun = @($Checks | Where-Object status -CEQ 'NOT RUN').Count
    $verdict = if ($status -ceq 'PASS' -and $notRun) { "PASS, $notRun NOT RUN" } else { $status }

    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add("$Title - $verdict")
    foreach ($check in $Checks) {
        $lines.Add(('  {0,-8} {1}: {2}' -f $check.status, $check.id, $check.observed))
        if ($check.status -cne 'PASS' -and $check.remediation) { $lines.Add("           -> $($check.remediation)") }
    }
    if ($status -ceq 'PASS') { foreach ($line in $Next) { $lines.Add("  NEXT     $line") } }
    foreach ($line in $lines) { Write-Host $line }

    if (-not [string]::IsNullOrWhiteSpace($StepSummaryPath)) {
        $markdown = @("### $Title - $verdict", '', '```text') + $lines.ToArray() + @('```', '')
        $markdown | Out-File -LiteralPath $StepSummaryPath -Encoding utf8 -Append
    }
    switch ($status) { 'PASS' { 0 } 'BLOCKED' { 2 } default { 1 } }
}

function Test-TigerSetupReleaseNotes {
    <#
        .SYNOPSIS
        Checks `.github/release-notes/<version>.md`, the release's checked-in
        user-facing notes. Returns a check.

        .DESCRIPTION
        GitHub's generated notes are a changelog link, not a release
        description, so the draft is created from this file and the gate that
        runs before any build refuses a missing, empty, placeholder or leaky
        one. The notes' first heading names the version, so a copied file
        cannot carry the previous release's title.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Version
    )

    $relative = "$($script:Facts.ReleaseNotesDirectory)/$Version.md"
    $path = Join-Path $RepositoryRoot $relative
    $repair = "Write ${relative}: a '# TigerSetup $Version' heading and at least two '##' sections of real, user-facing content."
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative does not exist." -Remediation $repair
    }
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative starts with a byte-order mark." -Remediation 'Save it as UTF-8 without a BOM.'
    }
    $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    $firstHeading = [regex]::Match($text, '(?m)^#\s+(?<title>.+?)\s*$')
    if (-not $firstHeading.Success -or $firstHeading.Groups['title'].Value -cne "TigerSetup $Version") {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative does not open with '# TigerSetup $Version'." -Remediation $repair
    }
    $sections = [regex]::Matches($text, '(?m)^##\s+\S').Count
    $prose = ($text -replace '(?m)^\s*#.*$', '' -replace '\[([^\]]*)\]\([^)]*\)', '$1' -replace '\s+', ' ').Trim()
    if ($sections -lt 2 -or $prose.Length -lt 200) {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative has $sections sections and $($prose.Length) characters of prose." -Remediation $repair
    }
    $placeholder = [regex]::Match($text, '(?i)\b(TODO|TBD|FIXME|lorem ipsum)\b|<(describe|summari[sz]e|fill|add)[^>]*>|xxx+')
    if ($placeholder.Success) {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative still has placeholder text '$($placeholder.Value)'." -Remediation $repair
    }
    $leak = [regex]::Match($text, '(?i)[A-Z]:\\(Users|Projects)\\|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----')
    if ($leak.Success) {
        return New-TigerSetupReleaseCheck -Id 'notes' -Status FAIL -Observed "$relative contains a local path or a secret: '$($leak.Value)'." -Remediation 'Remove it.'
    }
    New-TigerSetupReleaseCheck -Id 'notes' -Status PASS -Observed "${relative}: $sections sections, $($prose.Length) characters."
}

function Test-TigerSetupVersionReference {
    <#
        .SYNOPSIS
        Checks the one statement of the current version outside Cargo.toml:
        README.md's "TigerSetup is at version **<version>**". Returns a check.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Version
    )
    $readme = Get-Content -LiteralPath (Join-Path $RepositoryRoot 'README.md') -Raw
    $stated = [regex]::Match($readme, 'TigerSetup is at version \*\*(?<version>[^*]+)\*\*')
    if ($stated.Success -and $stated.Groups['version'].Value -ceq $Version) {
        return New-TigerSetupReleaseCheck -Id 'readme' -Status PASS -Observed "README.md states version $Version."
    }
    New-TigerSetupReleaseCheck -Id 'readme' -Status FAIL -Observed "README.md states version '$(if ($stated.Success) { $stated.Groups['version'].Value })', not $Version." `
        -Remediation 'Update README.md with the release: its version line and the installer names in its examples.'
}

function Get-TigerSetupFileSha256 {
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Write-TigerSetupReleaseRecord {
    <#
        .SYNOPSIS
        Closes a release directory: writes release-artifacts.json and
        SHA256SUMS.txt over exactly the release's payload files, and returns
        the SHA-256 of release-artifacts.json.

        .DESCRIPTION
        The record is the Tiger release model's schema 1, byte-compatible with
        the one TigerMarkView and TigerQuery publish: the version, the commit the
        payloads were built from, and each payload's name, kind, length and
        SHA-256. A directory holding anything else, or missing a payload, is
        refused, so the record always describes a closed set.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $CommitSha
    )

    $assets = @(Get-TigerSetupReleaseAsset -Version $Version)
    $present = @(Get-ChildItem -LiteralPath $Directory -File | ForEach-Object Name)
    $missing = @($assets.name | Where-Object { $_ -cnotin $present })
    $extra = @($present | Where-Object { $_ -cnotin $assets.name })
    if ($missing.Count -or $extra.Count) {
        throw "The release directory is not the closed payload set. Missing: $($missing -join ', '); unexpected: $($extra -join ', ')."
    }

    $artifacts = @(foreach ($asset in $assets) {
            $path = Join-Path $Directory $asset.name
            [ordered]@{
                name = $asset.name
                kind = $asset.kind
                length = (Get-Item -LiteralPath $path).Length
                sha256 = Get-TigerSetupFileSha256 -Path $path
            }
        })
    $recordPath = Join-Path $Directory 'release-artifacts.json'
    [ordered]@{
        schemaVersion = 1
        releaseVersion = $Version
        sourceCommit = $CommitSha.ToLowerInvariant()
        generatedAtUtc = [DateTime]::UtcNow.ToString('o')
        artifacts = $artifacts
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $recordPath -Encoding utf8NoBOM
    @($artifacts | ForEach-Object { "$($_.sha256)  $($_.name)" }) |
        Set-Content -LiteralPath (Join-Path $Directory 'SHA256SUMS.txt') -Encoding utf8NoBOM
    Get-TigerSetupFileSha256 -Path $recordPath
}

function Assert-TigerSetupReleaseRecord {
    <#
        .SYNOPSIS
        Proves a directory holds exactly the bytes its release record names, for
        the expected version and commit, and returns the record.

        .DESCRIPTION
        -ExpectedRecordSha256 is the transfer check: the build records the
        record's own hash and every later stage that receives the directory
        repeats it, so a record changed in transit cannot vouch for itself.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [string] $Version,
        [ValidatePattern('^([0-9a-fA-F]{40})?$')] [string] $CommitSha = '',
        [ValidatePattern('^([0-9a-fA-F]{64})?$')] [string] $ExpectedRecordSha256 = ''
    )

    $recordPath = Join-Path $Directory 'release-artifacts.json'
    if (-not (Test-Path -LiteralPath $recordPath -PathType Leaf)) { throw "$recordPath does not exist." }
    if ($ExpectedRecordSha256) {
        $actual = Get-TigerSetupFileSha256 -Path $recordPath
        if ($actual -cne $ExpectedRecordSha256.ToLowerInvariant()) {
            throw "release-artifacts.json hashes to $actual, not the recorded $($ExpectedRecordSha256.ToLowerInvariant()); it changed in transit."
        }
    }
    $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
    if ($record.schemaVersion -ne 1 -or [string] $record.releaseVersion -cne $Version) {
        throw "release-artifacts.json describes schema $($record.schemaVersion), version '$($record.releaseVersion)', not schema 1, version $Version."
    }
    if ([string] $record.sourceCommit -notmatch '^[0-9a-f]{40}$') { throw 'release-artifacts.json names no source commit.' }
    if ($CommitSha -and [string] $record.sourceCommit -cne $CommitSha.ToLowerInvariant()) {
        throw "release-artifacts.json names commit $($record.sourceCommit), not $($CommitSha.ToLowerInvariant())."
    }

    $assets = @(Get-TigerSetupReleaseAsset -Version $Version)
    $entries = @($record.artifacts)
    $recorded = @($entries | ForEach-Object { "$($_.name)|$($_.kind)" })
    $expected = @($assets | ForEach-Object { "$($_.name)|$($_.kind)" })
    if (($recorded -join ';') -cne ($expected -join ';')) {
        throw "release-artifacts.json records [$($recorded -join ', ')], not the release set [$($expected -join ', ')]."
    }

    $present = @(Get-ChildItem -LiteralPath $Directory -File | ForEach-Object Name)
    $allowed = @(Get-TigerSetupReleaseAssetName -Version $Version)
    $missing = @($allowed | Where-Object { $_ -cnotin $present })
    $extra = @($present | Where-Object { $_ -cnotin $allowed })
    if ($missing.Count -or $extra.Count) {
        throw "The release directory is not the closed set. Missing: $($missing -join ', '); unexpected: $($extra -join ', ')."
    }
    foreach ($entry in $entries) {
        $path = Join-Path $Directory $entry.name
        if ((Get-Item -LiteralPath $path).Length -ne [long] $entry.length -or (Get-TigerSetupFileSha256 -Path $path) -cne [string] $entry.sha256) {
            throw "$($entry.name) is not the bytes release-artifacts.json records."
        }
    }
    $sums = @($entries | ForEach-Object { "$($_.sha256)  $($_.name)" }) -join "`n"
    $actualSums = ((Get-Content -LiteralPath (Join-Path $Directory 'SHA256SUMS.txt') -Raw) -replace "`r`n", "`n").TrimEnd("`n")
    if ($actualSums -cne $sums) { throw 'SHA256SUMS.txt does not match release-artifacts.json.' }
    $record
}

function ConvertTo-TigerSetupCommandText {
    <#
        .SYNOPSIS
        A native command's merged output as text: standard output only, unless
        the command failed, when standard error is kept for the message.
    #>
    [CmdletBinding()]
    param([AllowNull()] [object[]] $Output, [bool] $Failed)
    $lines = @($Output | Where-Object { $Failed -or $_ -isnot [Management.Automation.ErrorRecord] } | ForEach-Object { "$_" })
    $lines -join "`n"
}

function Set-TigerSetupReleaseGitHubCli {
    <#
        .SYNOPSIS
        Replaces `gh` for this session: a script block taking gh's arguments and
        returning its standard output, with $global:LASTEXITCODE set. Tests use
        it; $null restores the real gh.
    #>
    [CmdletBinding()]
    param([AllowNull()] [scriptblock] $Command)
    $script:GitHubCli = $Command
}

function Invoke-TigerSetupGitHubCli {
    <#
        .SYNOPSIS
        Runs gh with the given arguments. Returns ok, exitCode and output:
        standard output, with standard error added when gh failed.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string[]] $Arguments)

    $previous = $PSNativeCommandUseErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        if ($null -ne $script:GitHubCli) {
            $output = & $script:GitHubCli @Arguments
        }
        else {
            if ($null -eq (Get-Command gh -CommandType Application -ErrorAction SilentlyContinue)) {
                return [pscustomobject]@{ ok = $false; exitCode = -1; output = 'gh is not installed.' }
            }
            $output = & gh @Arguments 2>&1
        }
        $code = $global:LASTEXITCODE
        [pscustomobject]@{ ok = ($code -eq 0); exitCode = $code; output = (ConvertTo-TigerSetupCommandText -Output $output -Failed ($code -ne 0)) }
    }
    finally {
        $PSNativeCommandUseErrorActionPreference = $previous
        $global:LASTEXITCODE = 0
    }
}

function Invoke-TigerSetupGitHubApi {
    <#
        .SYNOPSIS
        GET a GitHub REST path through gh. Returns ok, data (the parsed JSON)
        and error (gh's message when it failed).
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Path)

    $result = Invoke-TigerSetupGitHubCli @('api', $Path)
    if (-not $result.ok) {
        return [pscustomobject]@{ ok = $false; data = $null; error = $result.output }
    }
    [pscustomobject]@{ ok = $true; data = ($result.output | ConvertFrom-Json); error = '' }
}

function Invoke-TigerSetupGit {
    <#
        .SYNOPSIS
        Runs git in the repository; returns ok, exitCode and trimmed output.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string[]] $Arguments
    )
    $previous = $PSNativeCommandUseErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        $output = & git -C $RepositoryRoot @Arguments 2>&1
        $code = $global:LASTEXITCODE
        [pscustomobject]@{ ok = ($code -eq 0); exitCode = $code; output = (ConvertTo-TigerSetupCommandText -Output $output -Failed ($code -ne 0)).Trim() }
    }
    finally {
        $PSNativeCommandUseErrorActionPreference = $previous
        $global:LASTEXITCODE = 0
    }
}

function Test-TigerSetupCommitOnMain {
    <#
        .SYNOPSIS
        Proves the commit is reachable from origin/main, after fetching it. The
        release follows a human push; it is never inferred from a nearby commit.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $CommitSha
    )

    $branch = $script:Facts.DefaultBranch
    $null = Invoke-TigerSetupGit $RepositoryRoot @('fetch', '--quiet', 'origin', "+refs/heads/${branch}:refs/remotes/origin/${branch}")
    $tip = Invoke-TigerSetupGit $RepositoryRoot @('rev-parse', '--verify', '--quiet', "refs/remotes/origin/$branch")
    if (-not $tip.ok) {
        return New-TigerSetupReleaseCheck -Id 'commit/on-main' -Status BLOCKED -Observed "origin/$branch is unknown here." -Remediation "Fetch origin/$branch and rerun."
    }
    $ancestor = Invoke-TigerSetupGit $RepositoryRoot @('merge-base', '--is-ancestor', $CommitSha, $tip.output)
    if ($ancestor.ok) {
        return New-TigerSetupReleaseCheck -Id 'commit/on-main' -Status PASS -Observed "$CommitSha is reachable from origin/$branch ($($tip.output))."
    }
    New-TigerSetupReleaseCheck -Id 'commit/on-main' -Status BLOCKED -Observed "$CommitSha is not reachable from origin/$branch ($($tip.output))." -Remediation "Push the release commit to $branch first."
}

function Get-TigerSetupRemoteTagCommit {
    <#
        .SYNOPSIS
        The commit a release tag names on origin, dereferencing an annotated tag,
        or $null when origin has no such tag. Plain git; no GitHub API.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Tag
    )
    $result = Invoke-TigerSetupGit $RepositoryRoot @('ls-remote', '--tags', 'origin', "refs/tags/$Tag", "refs/tags/$Tag^{}")
    if (-not $result.ok) { throw "Could not list origin's tags: $($result.output)" }
    $refs = @{}
    foreach ($line in ($result.output -split "`n" | Where-Object { $_ })) {
        $sha, $ref = $line -split "`t", 2
        $refs[$ref.Trim()] = $sha.Trim().ToLowerInvariant()
    }
    if ($refs.ContainsKey("refs/tags/$Tag^{}")) { return $refs["refs/tags/$Tag^{}"] }
    if ($refs.ContainsKey("refs/tags/$Tag")) { return $refs["refs/tags/$Tag"] }
    $null
}

function Get-TigerSetupGitHubRelease {
    <#
        .SYNOPSIS
        The GitHub Release for a tag, draft or published, or $null.

        .DESCRIPTION
        GitHub's by-tag endpoint does not return drafts, so this lists the
        releases and selects by tag name. Drafts are visible only to a session
        with push access (a maintainer's gh, or a workflow's contents: write
        token); anonymously only published releases are listed.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Tag)

    $response = Invoke-TigerSetupGitHubApi "repos/$($script:Facts.Repository)/releases?per_page=100"
    if (-not $response.ok) { throw "Could not list the GitHub Releases: $($response.error)" }
    $matching = @($response.data | Where-Object { [string] $_.tag_name -ceq $Tag })
    if ($matching.Count -gt 1) { throw "More than one GitHub Release names tag $Tag." }
    if ($matching.Count -eq 0) { return $null }
    $matching[0]
}

function Get-TigerSetupTagMessage {
    <#
        .SYNOPSIS
        The annotated release tag's message: the title, the SHA-256 of the
        release record the tag was made for, and the run that built it.

        .DESCRIPTION
        The record describes the release set but cannot vouch for itself. The
        tag can: only the release workflow creates it (Publish-DraftRelease.ps1
        refuses anywhere else), as github-actions[bot], at the release commit,
        and a tag is never moved - so the record hash it carries is what every
        later stage checks a retrieved set against.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{64}$')] [string] $RecordSha256,
        [string] $RunUrl = ''
    )
    $lines = @("$($script:Facts.Product) $Version", '', "release-artifacts.json sha256 $($RecordSha256.ToLowerInvariant())")
    if ($RunUrl) { $lines += "built by $RunUrl" }
    $lines -join "`n"
}

function Get-TigerSetupRemoteTag {
    <#
        .SYNOPSIS
        Origin's release tag as it is: the commit it names, whether it is
        annotated, its tagger and its message; $null when origin has no such
        tag. Fetches into FETCH_HEAD only, so no local ref changes.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Tag
    )
    if ($null -eq (Get-TigerSetupRemoteTagCommit -RepositoryRoot $RepositoryRoot -Tag $Tag)) { return $null }
    $fetched = Invoke-TigerSetupGit $RepositoryRoot @('fetch', '--quiet', '--no-tags', 'origin', "refs/tags/$Tag")
    if (-not $fetched.ok) { throw "Could not fetch origin's $Tag`: $($fetched.output)" }
    $object = (Invoke-TigerSetupGit $RepositoryRoot @('rev-parse', 'FETCH_HEAD')).output
    $commit = (Invoke-TigerSetupGit $RepositoryRoot @('rev-parse', "$object^{commit}")).output.ToLowerInvariant()
    $annotated = (Invoke-TigerSetupGit $RepositoryRoot @('cat-file', '-t', $object)).output -ceq 'tag'
    $tagger = ''
    $message = ''
    if ($annotated) {
        $text = (Invoke-TigerSetupGit $RepositoryRoot @('cat-file', '-p', $object)).output -replace "`r`n", "`n"
        $header, $body = $text -split "`n`n", 2
        $taggerLine = @($header -split "`n" | Where-Object { $_ -like 'tagger *' })
        if ($taggerLine.Count) { $tagger = ($taggerLine[0] -replace '^tagger\s+', '' -replace '\s*<.*$', '') }
        $message = if ($null -ne $body) { $body.Trim() } else { '' }
    }
    [pscustomobject][ordered]@{ tag = $Tag; commit = $commit; annotated = $annotated; tagger = $tagger; message = $message }
}

function Assert-TigerSetupReleaseProvenance {
    <#
        .SYNOPSIS
        Proves a retrieved release set is the one the release workflow built
        and tagged, and returns its record.

        .DESCRIPTION
        The directory must hold exactly the bytes its record names
        (Assert-TigerSetupReleaseRecord), and origin's v<version> must be an
        annotated tag by github-actions[bot] at the record's commit whose
        message names this record's SHA-256. A set built anywhere else, or a
        record replaced on the release, fails here.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [string] $Version
    )
    $record = Assert-TigerSetupReleaseRecord -Directory $Directory -Version $Version
    $recordSha256 = Get-TigerSetupFileSha256 (Join-Path $Directory 'release-artifacts.json')
    $tagName = Get-TigerSetupReleaseTag -Version $Version
    $tag = Get-TigerSetupRemoteTag -RepositoryRoot $RepositoryRoot -Tag $tagName
    if ($null -eq $tag) { throw "origin has no tag $tagName." }
    if (-not $tag.annotated -or $tag.tagger -cne 'github-actions[bot]') {
        throw "origin's $tagName is not the release workflow's annotated tag (annotated: $($tag.annotated), tagger: '$($tag.tagger)')."
    }
    if ($tag.commit -cne [string] $record.sourceCommit) {
        throw "release-artifacts.json names $($record.sourceCommit); origin's $tagName names $($tag.commit)."
    }
    if ($tag.message -notmatch "(?m)^release-artifacts\.json sha256 $recordSha256$") {
        throw "origin's $tagName was made for another release record; this one hashes to $recordSha256."
    }
    $record
}

function New-TigerSetupWinGetSubmission {
    <#
        .SYNOPSIS
        Commits a release's manifest set to a winget-pkgs clone, on its own
        branch from upstream's master, and optionally pushes that branch.

        .DESCRIPTION
        The clone is a working copy of the publisher's winget-pkgs fork:
        `origin` is the fork and `upstream` is -Upstream. Its working tree must be
        clean. The branch ItTiger-TigerSetup-<version> is created from
        upstream/master with exactly the three manifests under
        manifests/i/ItTiger/TigerSetup/<version>; a version winget-pkgs already
        has is refused. An existing branch is accepted only when it already
        holds exactly these bytes, so a rerun changes nothing. -Push pushes the
        branch to origin, never forced; the pull request is opened by a person.
        Returns the branch, the commit and, when pushed, the compare URL.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $WinGetPkgsRoot,
        [Parameter(Mandatory)] [string] $ManifestDirectory,
        [Parameter(Mandatory)] [string] $Version,
        [string] $Upstream = 'https://github.com/microsoft/winget-pkgs',
        [switch] $Push
    )

    $id = $script:Facts.PackageIdentifier
    $publisher, $name = $id -split '\.', 2
    $relative = "manifests/$($publisher.Substring(0, 1).ToLowerInvariant())/$publisher/$name/$Version"
    $branch = "$publisher-$name-$Version"
    $files = @(Get-ChildItem -LiteralPath $ManifestDirectory -File | Sort-Object Name)
    if ($files.Count -ne 3) { throw "$ManifestDirectory holds $($files.Count) files, not the three manifests." }
    $git = { param([string[]] $Arguments) Invoke-TigerSetupGit -RepositoryRoot $WinGetPkgsRoot -Arguments $Arguments }
    $normalize = { param([string] $Url) ($Url.Trim() -replace '\.git$', '' -replace '/+$', '').ToLowerInvariant() }

    $upstreamUrl = & $git @('remote', 'get-url', 'upstream')
    if (-not $upstreamUrl.ok -or (& $normalize $upstreamUrl.output) -cne (& $normalize $Upstream)) {
        throw "$WinGetPkgsRoot's 'upstream' remote is '$($upstreamUrl.output)', not $Upstream."
    }
    $originUrl = & $git @('remote', 'get-url', 'origin')
    if (-not $originUrl.ok) { throw "$WinGetPkgsRoot has no 'origin' remote (the publisher's fork)." }
    $status = & $git @('status', '--porcelain')
    if ($status.output) { throw "$WinGetPkgsRoot has uncommitted changes:`n$($status.output)" }
    $fetched = & $git @('fetch', '--quiet', 'upstream', '+refs/heads/master:refs/remotes/upstream/master')
    if (-not $fetched.ok) { throw "Could not fetch upstream master: $($fetched.output)" }
    if ((& $git @('ls-tree', '-d', '--name-only', 'refs/remotes/upstream/master', '--', $relative)).output) {
        throw "winget-pkgs already has $relative."
    }
    $kind = if ((& $git @('ls-tree', '-d', '--name-only', 'refs/remotes/upstream/master', '--', (Split-Path -Parent $relative).Replace('\', '/'))).output) { 'New version' } else { 'New package' }

    # The blobs these files become, to compare an existing branch against.
    $wanted = @($files | ForEach-Object { "$((& $git @('hash-object', '--', $_.FullName)).output) $relative/$($_.Name)" })
    $existing = & $git @('rev-parse', '--verify', '--quiet', "refs/heads/$branch")
    if ($existing.ok) {
        $held = @((& $git @('ls-tree', '-r', "refs/heads/$branch", '--', $relative)).output -split "`n" | Where-Object { $_ } | ForEach-Object {
                $meta, $path = $_ -split "`t", 2; "$(($meta -split ' ')[2]) $path"
            })
        if (($held -join '|') -cne ($wanted -join '|')) { throw "Branch $branch exists and does not hold exactly this manifest set; it is not changed." }
        $commit = $existing.output
    }
    else {
        $switched = & $git @('switch', '--quiet', '--no-track', '--create', $branch, 'refs/remotes/upstream/master')
        if (-not $switched.ok) { throw "Could not create $branch`: $($switched.output)" }
        $target = Join-Path $WinGetPkgsRoot $relative
        $null = New-Item -ItemType Directory -Path $target -Force
        foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $target $file.Name) }
        $added = & $git @('add', '--', $relative)
        if (-not $added.ok) { throw "git add failed: $($added.output)" }
        $committed = & $git @('commit', '--quiet', '-m', "$kind`: $id version $Version")
        if (-not $committed.ok) { throw "git commit failed: $($committed.output)" }
        $commit = (& $git @('rev-parse', 'HEAD')).output
    }

    $compare = ''
    if ($Push) {
        $pushed = & $git @('push', 'origin', "refs/heads/${branch}:refs/heads/$branch")
        if (-not $pushed.ok) { throw "Could not push $branch to origin: $($pushed.output)" }
        if ($originUrl.output -match 'github\.com[:/](?<owner>[^/]+)/(?<repo>[^/]+?)(\.git)?/?$') {
            $compare = "$(& $normalize $Upstream)/compare/master...$($Matches['owner']):$($Matches['repo']):$branch`?expand=1"
        }
    }
    [pscustomobject][ordered]@{ branch = $branch; commit = $commit; kind = $kind; path = $relative; pushed = [bool] $Push; compareUrl = $compare }
}

Export-ModuleMember -Function *-TigerSetup*
