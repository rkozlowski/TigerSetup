<#
    .SYNOPSIS
    TigerSetup's release facts and the checks every release stage shares.

    .DESCRIPTION
    The Tiger release model (TigerAiCore `docs/release-model.md`) defines the
    lifecycle; this module holds what TigerSetup releases and how its stages
    prove each other: the version source, the artifact set and its names, the
    release-bound terms frozen with it (License and PrivacyStatement), the
    release notes, the closed artifact record (`release-artifacts.json` schema 1
    and `SHA256SUMS.txt`), the commit and tag gates, and the GitHub
    Release lookup. `RELEASING.md` describes the lifecycle as TigerSetup runs
    it.

    Every function runs the same way on a developer machine and on a GitHub
    Actions runner. GitHub is reached only through `gh` (authenticated by
    `gh auth login` locally, or by GH_TOKEN in a workflow) and through `git`;
    the tests replace `gh` with Set-TigerSetupReleaseGitHubCli, and `dotnet`
    with Set-TigerSetupReleaseDotNet.
#>

Set-StrictMode -Version Latest

$script:Facts = [pscustomobject][ordered]@{
    Product = 'TigerSetup'
    Repository = 'rkozlowski/TigerSetup'
    DefaultBranch = 'main'
    ReleaseWorkflowName = 'Release TigerSetup'
    ReleaseWorkflowPath = '.github/workflows/release.yml'
    TagPrefix = 'v'
    PackageIdentifier = 'ItTiger.TigerSetup'
    PackageManifest = 'packages/tigersetup/TigerSetup.toml'
    ReleaseNotesDirectory = '.github/release-notes'
    # tiger-mark, which renders the installed help's PDF, is built from a
    # pinned commit of this repository (Build-TigerSetupTigerMark).
    TigerMarkViewRepository = 'https://github.com/rkozlowski/TigerMarkView.git'
    # The merge of TigerMarkView's active-content security fix (0.9.0): every
    # pinned source must contain it, so no older renderer is ever built.
    TigerMarkViewSecurityBaseline = 'cd94b4ac5e7b5fa034fbf569b785575bda2bab7d'
}

$script:GitHubCli = $null
$script:DotNetCli = $null

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
        The payloads of one release, in record order: the built artifacts, then
        the release-bound terms. The two records close the set.

        .DESCRIPTION
        `kind` is the release-artifacts.json vocabulary shared by the Tiger
        release model. The installer is the product; the WinGet manifest set is
        generated from that installer's exact bytes and the URL it will be
        published at, and travels with it so the release carries its own
        submission.

        `License` and `PrivacyStatement` are the terms that apply to exactly
        this release, frozen with it: the bytes the release commit holds for
        `source` in Git (Copy-TigerSetupReleaseTerms), published beside the
        installer at a URL the version fixes, which is what the release's WinGet
        LicenseUrl and PrivacyUrl name. A release set without either is not
        closed. `source` is empty for a built artifact.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)

    @(
        [pscustomobject]@{ name = "TigerSetup-$Version-Setup.exe"; kind = 'WindowsInstaller'; source = '' }
        [pscustomobject]@{ name = "TigerSetup-$Version-WinGet.zip"; kind = 'WinGetManifests'; source = '' }
        [pscustomobject]@{ name = 'LICENSE.txt'; kind = 'License'; source = 'LICENSE.txt' }
        [pscustomobject]@{ name = 'PRIVACY.md'; kind = 'PrivacyStatement'; source = 'PRIVACY.md' }
    )
}

function Get-TigerSetupReleaseTerms {
    <#
        .SYNOPSIS
        The release-bound terms of one release: the License and the
        PrivacyStatement assets, each with the repository file it is frozen from.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)
    @(Get-TigerSetupReleaseAsset -Version $Version | Where-Object { $_.source })
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

function Get-TigerSetupReleaseAssetUrl {
    <#
        .SYNOPSIS
        The public, version-specific URL a release asset is published at.

        .DESCRIPTION
        GitHub serves a release asset at a URL made of the repository, the tag
        and the asset name, so the URL is known before the release exists and
        never serves another version's file. The WinGet manifests are finalized
        with the installer's at build time, and name the terms' as LicenseUrl
        and PrivacyUrl; verification after publication downloads them and
        compares the bytes.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [string] $Name
    )
    "https://github.com/$($script:Facts.Repository)/releases/download/$(Get-TigerSetupReleaseTag -Version $Version)/$Name"
}

function Get-TigerSetupInstallerUrl {
    <#
        .SYNOPSIS
        The public, version-specific URL the installer is published at.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string] $Version)

    $installer = (Get-TigerSetupReleaseAsset -Version $Version | Where-Object kind -CEQ 'WindowsInstaller').name
    Get-TigerSetupReleaseAssetUrl -Version $Version -Name $installer
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

function Test-TigerSetupWorkflowDefaultVersion {
    <#
        .SYNOPSIS
        Checks the version the release workflow's form is prefilled with: the
        `default` of its `workflow_dispatch` `version` input. Returns a check.

        .DESCRIPTION
        The Architect starts a release by confirming the version the form
        already shows rather than typing it, so the release commit's workflow
        must offer exactly the version its Cargo.toml records. Only the key
        `on` > `workflow_dispatch` > `inputs` > `version` > `default` counts,
        found by indentation; no YAML parser is involved.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Version
    )

    $relative = $script:Facts.ReleaseWorkflowPath
    $path = Join-Path $RepositoryRoot $relative
    $repair = "Set the default of the version input under workflow_dispatch in $relative to '$Version'."
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return New-TigerSetupReleaseCheck -Id 'workflow' -Status FAIL -Observed "$relative does not exist." -Remediation $repair
    }
    $keys = [Collections.Generic.List[object]]::new()
    $offered = $null
    foreach ($line in Get-Content -LiteralPath $path) {
        $entry = [regex]::Match($line, '^(?<indent> *)(?<key>[A-Za-z_][\w-]*):(\s+(?<value>.*?))?\s*$')
        if (-not $entry.Success) { continue }
        $indent = $entry.Groups['indent'].Length
        while ($keys.Count -and $keys[$keys.Count - 1].indent -ge $indent) { $keys.RemoveAt($keys.Count - 1) }
        $keys.Add([pscustomobject]@{ indent = $indent; key = $entry.Groups['key'].Value })
        if (($keys.key -join '/') -ceq 'on/workflow_dispatch/inputs/version/default') {
            $offered = [regex]::Match($entry.Groups['value'].Value, '^(''(?<v>[^'']*)''|"(?<v>[^"]*)"|(?<v>[^\s#]+))').Groups['v'].Value
        }
    }
    if ($offered -ceq $Version) {
        return New-TigerSetupReleaseCheck -Id 'workflow' -Status PASS -Observed "$relative offers version $Version."
    }
    New-TigerSetupReleaseCheck -Id 'workflow' -Status FAIL -Observed "$relative offers $(if ($null -eq $offered) { 'no default version' } else { "version '$offered'" }), not $Version." -Remediation $repair
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

function Export-TigerSetupGitBlob {
    <#
        .SYNOPSIS
        Writes a Git blob's exact bytes to a file: `git cat-file blob`, with no
        text conversion on the way, which a PowerShell pipeline would apply.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Object,
        [Parameter(Mandatory)] [string] $Destination
    )
    $start = [Diagnostics.ProcessStartInfo]::new('git')
    foreach ($argument in @('-C', $RepositoryRoot, 'cat-file', 'blob', $Object)) { $start.ArgumentList.Add($argument) }
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.UseShellExecute = $false
    if (Test-Path -LiteralPath $Destination) { throw "$Destination already exists." }
    $process = [Diagnostics.Process]::Start($start)
    $written = $false
    try {
        $errorText = $process.StandardError.ReadToEndAsync()
        $file = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew)
        try { $process.StandardOutput.BaseStream.CopyTo($file) } finally { $file.Dispose() }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "git cat-file blob $Object failed ($($process.ExitCode)): $($errorText.Result.Trim())" }
        $written = $true
    }
    finally {
        # A failed export leaves no partial file and no running git behind.
        if (-not $written) {
            if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
            Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        }
        $process.Dispose()
    }
}

function Copy-TigerSetupReleaseTerms {
    <#
        .SYNOPSIS
        Freezes a release's terms into its release directory: each License and
        PrivacyStatement asset is the exact bytes Git holds for its source file
        at the release commit. Returns each term with its blob and SHA-256.

        .DESCRIPTION
        The frozen bytes are the committed blob, read with `git cat-file blob`,
        so they do not depend on how a checkout converts line endings: anyone
        can reproduce them from the release commit (`git cat-file blob
        <commit>:<source>`), and Assert-TigerSetupReleaseTerms proves a
        retrieved set against them. A commit without the file fails; there is
        no other source of the terms.

        -WorkingTree is the rehearsal's: before the release commit exists, the
        terms are the blob Git would commit for the working-tree file
        (`git hash-object -w`, which writes that unreferenced object into the
        repository so it can be read back), so a candidate carries the terms
        being prepared rather than the previous commit's.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $CommitSha,
        [switch] $WorkingTree
    )
    foreach ($term in @(Get-TigerSetupReleaseTerms -Version $Version)) {
        if ($WorkingTree) {
            if (-not (Test-Path -LiteralPath (Join-Path $RepositoryRoot $term.source) -PathType Leaf)) { throw "The working tree has no $($term.source), the release's $($term.kind)." }
            $blob = Invoke-TigerSetupGit $RepositoryRoot @('hash-object', '-w', '--', $term.source)
            if (-not $blob.ok) { throw "git hash-object $($term.source) failed: $($blob.output)" }
        }
        else {
            $blob = Invoke-TigerSetupGit $RepositoryRoot @('rev-parse', '--verify', '--quiet', "$($CommitSha.ToLowerInvariant()):$($term.source)")
            if (-not $blob.ok) { throw "Commit $CommitSha has no $($term.source), the release's $($term.kind)." }
        }
        $path = Join-Path $Directory $term.name
        Export-TigerSetupGitBlob -RepositoryRoot $RepositoryRoot -Object $blob.output -Destination $path
        [pscustomobject][ordered]@{ name = $term.name; kind = $term.kind; source = $term.source; blob = $blob.output; sha256 = Get-TigerSetupFileSha256 -Path $path }
    }
}

function Assert-TigerSetupReleaseTerms {
    <#
        .SYNOPSIS
        Proves a release directory's License and PrivacyStatement are the exact
        bytes the commit holds for them in Git: the terms accepted with the
        release commit, unchanged by the pipeline. Returns each term.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Directory,
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $CommitSha
    )
    $scratch = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-terms-$([guid]::NewGuid().ToString('N'))"
    $null = New-Item -ItemType Directory -Path $scratch
    try {
        $expected = @(Copy-TigerSetupReleaseTerms -RepositoryRoot $RepositoryRoot -Directory $scratch -Version $Version -CommitSha $CommitSha)
        foreach ($term in $expected) {
            $path = Join-Path $Directory $term.name
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "The release set has no $($term.name), its $($term.kind)." }
            $actual = Get-TigerSetupFileSha256 -Path $path
            if ($actual -cne $term.sha256) {
                throw "$($term.name) hashes to $actual, not $($term.sha256), the bytes commit $CommitSha holds for $($term.source)."
            }
        }
        $expected
    }
    finally { Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue }
}

function Test-TigerSetupReleaseTermsReady {
    <#
        .SYNOPSIS
        The prerequisites gate's terms check: the commit holds every
        release-bound term, and TigerSetup's package declares the WinGet
        LicenseUrl and PrivacyUrl of exactly those frozen assets. Returns a
        check, so a commit without them fails before the build, not after it.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $RepositoryRoot,
        [Parameter(Mandatory)] [string] $Version,
        [Parameter(Mandatory)] [ValidatePattern('^[0-9a-fA-F]{40}$')] [string] $CommitSha
    )
    $problems = [Collections.Generic.List[string]]::new()
    foreach ($term in @(Get-TigerSetupReleaseTerms -Version $Version)) {
        $blob = Invoke-TigerSetupGit $RepositoryRoot @('rev-parse', '--verify', '--quiet', "$($CommitSha.ToLowerInvariant()):$($term.source)")
        if (-not $blob.ok) { $problems.Add("the commit has no $($term.source) ($($term.kind))") }
    }
    $manifestPath = Join-Path $RepositoryRoot $script:Facts.PackageManifest
    $package = if (Test-Path -LiteralPath $manifestPath -PathType Leaf) { Get-Content -LiteralPath $manifestPath -Raw } else { '' }
    foreach ($key in @(@('license_url', 'License'), @('privacy_url', 'PrivacyStatement'))) {
        $name, $kind = $key
        $asset = (Get-TigerSetupReleaseAsset -Version $Version | Where-Object kind -CEQ $kind).name
        $wanted = Get-TigerSetupReleaseAssetUrl -Version $Version -Name $asset
        $declared = [regex]::Match($package, "(?m)^$name\s*=\s*""(?<url>[^""]*)""")
        $resolved = if ($declared.Success) { $declared.Groups['url'].Value.Replace('{version}', $Version) } else { '' }
        if ($resolved -cne $wanted) { $problems.Add("$($script:Facts.PackageManifest) declares $name '$resolved', not $wanted") }
    }
    if ($problems.Count) {
        return New-TigerSetupReleaseCheck -Id 'terms' -Status FAIL -Observed ($problems -join '; ') -Remediation 'Commit LICENSE.txt and PRIVACY.md, and declare license_url and privacy_url as the release''s releases/download/v{version}/ assets (RELEASING.md).'
    }
    New-TigerSetupReleaseCheck -Id 'terms' -Status PASS -Observed "The commit holds LICENSE.txt and PRIVACY.md; the package names their v$Version release assets."
}

function Test-TigerSetupWinGetTermsUrl {
    <#
        .SYNOPSIS
        Checks that a manifest set's LicenseUrl and PrivacyUrl name the terms
        frozen with this version - the release's License and PrivacyStatement
        assets - and returns what is wrong, if anything.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $ManifestDirectory,
        [Parameter(Mandatory)] [string] $Version
    )
    $locale = Join-Path $ManifestDirectory "$($script:Facts.PackageIdentifier).locale.en-US.yaml"
    if (-not (Test-Path -LiteralPath $locale -PathType Leaf)) { return @("the set has no $(Split-Path -Leaf $locale)") }
    $text = Get-Content -LiteralPath $locale -Raw
    $problems = [Collections.Generic.List[string]]::new()
    foreach ($field in @(@('LicenseUrl', 'License'), @('PrivacyUrl', 'PrivacyStatement'))) {
        $name, $kind = $field
        $asset = (Get-TigerSetupReleaseAsset -Version $Version | Where-Object kind -CEQ $kind).name
        $wanted = Get-TigerSetupReleaseAssetUrl -Version $Version -Name $asset
        $values = @([regex]::Matches($text, "(?m)^$name\s*:\s*(?<url>\S+?)\s*$") | ForEach-Object { $_.Groups['url'].Value.Trim('''', '"') })
        if ($values.Count -ne 1 -or $values[0] -cne $wanted) { $problems.Add("$name is [$($values -join ', ')], not $wanted") }
    }
    @($problems)
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

function Set-TigerSetupReleaseDotNet {
    <#
        .SYNOPSIS
        Replaces `dotnet` for this session, as Set-TigerSetupReleaseGitHubCli
        replaces gh. Tests use it; $null restores the real dotnet.
    #>
    [CmdletBinding()]
    param([AllowNull()] [scriptblock] $Command)
    $script:DotNetCli = $Command
}

function Invoke-TigerSetupDotNet {
    <#
        .SYNOPSIS
        Runs dotnet with the given arguments. Returns ok, exitCode and output.
    #>
    [CmdletBinding()]
    param([Parameter(Mandatory)] [string[]] $Arguments)

    $previous = $PSNativeCommandUseErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        if ($null -ne $script:DotNetCli) {
            $output = & $script:DotNetCli @Arguments
        }
        else {
            if ($null -eq (Get-Command dotnet -CommandType Application -ErrorAction SilentlyContinue)) {
                return [pscustomobject]@{ ok = $false; exitCode = -1; output = 'dotnet is not installed.' }
            }
            $output = & dotnet @Arguments 2>&1
        }
        $code = $global:LASTEXITCODE
        [pscustomobject]@{ ok = ($code -eq 0); exitCode = $code; output = (ConvertTo-TigerSetupCommandText -Output $output -Failed $true) }
    }
    finally {
        $PSNativeCommandUseErrorActionPreference = $previous
        $global:LASTEXITCODE = 0
    }
}

function Test-TigerSetupPinnedCommit {
    <#
        .SYNOPSIS
        Whether the text pins a commit: its full 40-character lower-case SHA.
        A branch, a tag, a version or an abbreviated SHA names something that
        can move or become ambiguous, and is not a pin.
    #>
    [CmdletBinding()]
    param([AllowNull()] [AllowEmptyString()] [string] $Commit)
    $null -ne $Commit -and $Commit -cmatch '^[0-9a-f]{40}$'
}

function Assert-TigerSetupSourceCheckout {
    <#
        .SYNOPSIS
        Proves a checkout is the pinned source: HEAD is exactly the commit, the
        tree is clean, and the commit contains the security baseline.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Path,
        [Parameter(Mandatory)] [string] $Commit,
        [Parameter(Mandatory)] [string] $Baseline
    )
    $head = Invoke-TigerSetupGit $Path @('rev-parse', '--verify', 'HEAD^{commit}')
    if (-not $head.ok -or $head.output -cne $Commit) { throw "The TigerMarkView checkout is at '$($head.output)', not the pinned $Commit." }
    $status = Invoke-TigerSetupGit $Path @('status', '--porcelain', '--untracked-files=all')
    if (-not $status.ok -or $status.output) { throw "The TigerMarkView checkout of $Commit is not clean: $($status.output)" }
    $contains = Invoke-TigerSetupGit $Path @('merge-base', '--is-ancestor', $Baseline, $Commit)
    if (-not $contains.ok) {
        throw "TigerMarkView $Commit does not contain $Baseline, the active-content security baseline; no older tiger-mark is built. $($contains.output)".Trim()
    }
}

function Build-TigerSetupTigerMark {
    <#
        .SYNOPSIS
        Builds tiger-mark from one pinned TigerMarkView commit and proves it
        runs. Returns path, version, commit and reported.

        .DESCRIPTION
        Clones the repository into <Directory>\source, checks the commit out
        detached and proves the checkout (Assert-TigerSetupSourceCheckout),
        then publishes src\TigerMarkView.Cli alone - with TigerMarkView.Core
        and TigerMarkView.Pdf, its project references - framework-dependent
        for win-x64 into <Directory>\tiger-mark, as TigerMarkView stages the
        command for its own installer. The published tiger-mark.exe must run
        and report the version the commit's Version.props declares. Directory
        must be empty or absent: nothing already built is reused. Every
        failure throws; there is no other source of tiger-mark.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $Commit,
        [Parameter(Mandatory)] [string] $Directory,
        [string] $Repository = $script:Facts.TigerMarkViewRepository,
        [string] $SecurityBaseline = $script:Facts.TigerMarkViewSecurityBaseline
    )

    if (-not (Test-TigerSetupPinnedCommit $Commit)) { throw "'$Commit' does not pin a TigerMarkView commit: the full 40-character lower-case SHA is required." }
    $Directory = [IO.Path]::GetFullPath($Directory)
    if ((Test-Path -LiteralPath $Directory) -and @(Get-ChildItem -LiteralPath $Directory -Force).Count) {
        throw "$Directory is not empty; tiger-mark is built from the pinned source alone."
    }
    $null = New-Item -ItemType Directory -Path $Directory -Force
    $source = Join-Path $Directory 'source'
    $output = Join-Path $Directory 'tiger-mark'

    $clone = Invoke-TigerSetupGit $Directory @('clone', '--quiet', '--no-checkout', '--', $Repository, $source)
    if (-not $clone.ok) { throw "Cloning TigerMarkView from $Repository failed: $($clone.output)" }
    $checkout = Invoke-TigerSetupGit $source @('-c', 'advice.detachedHead=false', 'checkout', '--quiet', '--detach', $Commit)
    if (-not $checkout.ok) { throw "Checking out TigerMarkView $Commit failed: $($checkout.output)" }
    Assert-TigerSetupSourceCheckout -Path $source -Commit $Commit -Baseline $SecurityBaseline

    $versionProps = Join-Path $source 'Version.props'
    $version = if (Test-Path -LiteralPath $versionProps -PathType Leaf) {
        @(([xml] (Get-Content -LiteralPath $versionProps -Raw)).Project.PropertyGroup | ForEach-Object { $_.PSObject.Properties['Version'] } | Where-Object { $_ } | ForEach-Object { "$($_.Value)" })[0]
    }
    if (-not $version) { throw "TigerMarkView $Commit declares no Version in Version.props." }
    $project = Join-Path $source 'src\TigerMarkView.Cli\TigerMarkView.Cli.csproj'
    if (-not (Test-Path -LiteralPath $project -PathType Leaf)) { throw "TigerMarkView $Commit has no src\TigerMarkView.Cli\TigerMarkView.Cli.csproj." }

    $publish = Invoke-TigerSetupDotNet @('publish', $project, '--configuration', 'Release', '--runtime', 'win-x64',
        '--self-contained', 'false', '--output', $output, '-m:1', '--disable-build-servers', '--nologo')
    if (-not $publish.ok) { throw "dotnet publish of TigerMarkView.Cli at $Commit failed ($($publish.exitCode)):`n$($publish.output)" }
    $tigerMark = Join-Path $output 'tiger-mark.exe'
    if (-not (Test-Path -LiteralPath $tigerMark -PathType Leaf)) { throw "The build of TigerMarkView $Commit produced no $tigerMark." }

    $previous = $PSNativeCommandUseErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        $reported = (& $tigerMark --version 2>&1 | Out-String).Trim()
        $code = $global:LASTEXITCODE
    }
    catch { throw "$tigerMark does not run: $($_.Exception.Message)" }
    finally { $PSNativeCommandUseErrorActionPreference = $previous; $global:LASTEXITCODE = 0 }
    if ($code -ne 0 -or $reported -notmatch "(?<![0-9.])$([regex]::Escape($version))(?![0-9.])") {
        throw "$tigerMark --version reported '$reported' (exit $code), not TigerMarkView $version."
    }
    [pscustomobject]@{ path = $tigerMark; version = $version; commit = $Commit; reported = $reported }
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
