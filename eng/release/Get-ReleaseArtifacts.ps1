<#
    .SYNOPSIS
    Retrieves a release's authoritative assets from its GitHub Release, draft
    or published, and proves them before anything is validated or submitted -
    or runs the same proof over a release set on disk, so that release
    preparation rehearses exactly the checks release validation will run.

    .DESCRIPTION
    The assets the release workflow attached are the release; nothing is
    rebuilt. This downloads them into artifacts\release\<version>\assets and
    proves the chain from the commit to these bytes:

      release     the GitHub Release for v<version> exists, is 'TigerSetup
                  <version>' and carries exactly the release set
      bytes       every file is the bytes GitHub recorded (asset digest)
      record      the set is closed and every file is the bytes
                  release-artifacts.json and SHA256SUMS.txt record
      provenance  origin's v<version> is the release workflow's annotated tag
                  (github-actions[bot]) at the record's commit, and its message
                  names this record's SHA-256 - so the set is the one that
                  workflow built, not one uploaded from anywhere else
      run         the run the tag names is the release workflow, started by
                  hand on main, for that commit, and it succeeded
      terms       LICENSE.txt (License) and PRIVACY.md (PrivacyStatement) are
                  the exact bytes the record's commit holds for them in Git:
                  the terms accepted with the release commit, frozen unchanged
      winget      the manifest set is for this package and version, names
                  the published URL and this installer's SHA-256, and its
                  LicenseUrl and PrivacyUrl name this release's frozen terms
      installer   the installer verifies and carries exactly the engine and
                  loader it installs, and the builder it installs reports this
                  version
      help        the help the release build rendered: the shipped Markdown is
                  CRLF-only and exactly docs/TigerSetup-Help.md at the record's
                  commit as Git checks it out (the lab module's own check), and
                  the shipped PDF is a PDF rendered from it
      public      once the release is published: the installer URL, LicenseUrl
                  and PrivacyUrl serve these exact bytes, anonymously
      validate    `winget validate` accepts the manifest set; BLOCKED where
                  winget is not installed, because no lab row repeats it

    It unpacks the manifest set into ...\winget and the installer's files
    into ...\payload, and ends by naming the one lab row that proves these
    bytes usable on Windows (RELEASING.md). The release is found and its assets
    are downloaded through the GitHub API with gh's login, the same way for a
    draft and a published release, so gh must be authenticated (`gh auth
    login`); a draft is visible only to an account with push access.

    -ArtifactDirectory runs every check after the download over a release set
    on disk instead: a local candidate (Build-ReleaseArtifacts.ps1), or a copy
    of a release's assets. `release` and `bytes` are NOT RUN, and provenance
    and run are NOT RUN until the release tag exists; when it does, they are
    proved as for a retrieved set. -Rehearsal says the candidate was built from
    the prepared working tree (Build-ReleaseArtifacts.ps1 -Rehearsal), so its
    terms and help are proved against that tree as Git would commit and check
    it out. A local set is never a release, whatever it passes.

    Exit codes: 0 PASS (NOT RUN checks named), 2 BLOCKED, 1 FAIL.

    .EXAMPLE
    pwsh -File eng\release\Get-ReleaseArtifacts.ps1 -Version 0.12.0

    .EXAMPLE
    pwsh -File eng\release\Get-ReleaseArtifacts.ps1 -Version 0.12.0 -ArtifactDirectory artifacts\release-candidate\0.12.0 -Rehearsal
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    # A release set on disk to prove instead of the GitHub Release.
    [string] $ArtifactDirectory,
    # With -ArtifactDirectory: the set was built from the prepared working tree.
    [switch] $Rehearsal,
    # Defaults to artifacts\release\<version>, or artifacts\release-rehearsal\<version>
    # for a set on disk.
    [string] $OutputDirectory,
    # A tiger-setup.exe that reads the installer format, to unpack the installer;
    # defaults to this workspace's release build. The installer's own builder
    # then checks the installer.
    [string] $BuilderPath,
    # The checkout whose origin names the release tag; defaults to this repository.
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$workspaceRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
# The shipped-help check is the lab's own (Get-TigerSetupCommittedTextChecks),
# so the release's help is held to exactly what the preparation rows hold a
# candidate's to.
Import-Module (Join-Path $workspaceRoot 'lab\TigerSetupLab.psm1')
$repoRoot = if ($RepositoryRoot) { $RepositoryRoot } else { $workspaceRoot }
$facts = Get-TigerSetupReleaseFacts
$tag = Get-TigerSetupReleaseTag -Version $Version
$title = "$($facts.Product) $Version"
$local = -not [string]::IsNullOrWhiteSpace($ArtifactDirectory)
if ($Rehearsal -and -not $local) { throw '-Rehearsal describes a set on disk; name it with -ArtifactDirectory.' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $workspaceRoot "artifacts\$(if ($local) { 'release-rehearsal' } else { 'release' })\$Version" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $BuilderPath) { $BuilderPath = Join-Path $workspaceRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
$checks = [Collections.Generic.List[object]]::new()
function Add-Check { param($Id, $Status, $Observed, $Remediation = '') $checks.Add((New-TigerSetupReleaseCheck -Id $Id -Status $Status -Observed $Observed -Remediation $Remediation)) }
$reportTitle = if ($local) { "Release set check of TigerSetup $Version at $ArtifactDirectory (a local set, never a release$(if ($Rehearsal) { '; prepared working tree' }))" } else { "Release artifacts of TigerSetup $Version" }
function Complete { param([string[]] $Next = @()) exit (Write-TigerSetupReleaseReport -Title $reportTitle -Checks $checks.ToArray() -Next $Next) }

# The output directory: only one this script wrote is replaced.
$assetsDirectory = Join-Path $OutputDirectory 'assets'
if ($local) {
    $ArtifactDirectory = [IO.Path]::GetFullPath($ArtifactDirectory)
    if (-not (Test-Path -LiteralPath $ArtifactDirectory -PathType Container)) { throw "$ArtifactDirectory is not a directory." }
    if ($OutputDirectory -ieq $ArtifactDirectory -or $OutputDirectory.StartsWith("$ArtifactDirectory\", [StringComparison]::OrdinalIgnoreCase) -or
        $ArtifactDirectory.StartsWith("$OutputDirectory\", [StringComparison]::OrdinalIgnoreCase)) {
        throw "The output directory $OutputDirectory and the release set $ArtifactDirectory must not contain each other."
    }
}
if (Test-Path -LiteralPath $OutputDirectory) {
    $foreign = @(Get-ChildItem -LiteralPath $OutputDirectory -Force | Where-Object { $_.Name -cnotin @('assets', 'winget', 'payload') })
    if ($foreign.Count) { throw "$OutputDirectory holds files this script did not write: $($foreign.Name -join ', ')." }
    Remove-Item -LiteralPath $OutputDirectory -Recurse -Force
}
$null = New-Item -ItemType Directory -Path $assetsDirectory -Force

$release = $null
if ($local) {
    Add-Check 'release' 'NOT RUN' 'A set on disk has no GitHub Release; retrieving the draft runs this check.'
    Add-Check 'bytes' 'NOT RUN' 'A set on disk has no GitHub asset digests; retrieving the draft runs this check.'
    # Every file, so that the record check sees anything foreign.
    foreach ($item in @(Get-ChildItem -LiteralPath $ArtifactDirectory -Force)) { Copy-Item -LiteralPath $item.FullName -Destination $assetsDirectory -Recurse }
}
else {
    # The release and its asset list.
    try { $release = Get-TigerSetupGitHubRelease -Tag $tag }
    catch {
        Add-Check 'release' BLOCKED $_.Exception.Message 'Authenticate gh (gh auth login) and rerun.'
        Complete
    }
    if ($null -eq $release) {
        Add-Check 'release' BLOCKED "No GitHub Release for $tag is visible to this session." 'Start the release workflow, or authenticate gh with push access to see a draft.'
        Complete
    }
    $names = @(Get-TigerSetupReleaseAssetName -Version $Version)
    $remote = @($release.assets)
    $remoteNames = @($remote | ForEach-Object { [string] $_.name })
    $state = if ($release.draft) { 'draft' } else { 'published' }
    if ([string] $release.name -cne $title -or (@($remoteNames | Sort-Object) -join '|') -cne (@($names | Sort-Object) -join '|')) {
        Add-Check 'release' FAIL "The $state release '$($release.name)' carries [$($remoteNames -join ', ')], not '$title' with [$($names -join ', ')]."
        Complete
    }
    Add-Check 'release' PASS "$state release '$title': $($release.html_url)"

    # The bytes, through the API for a draft and a published release alike.
    $login = Invoke-TigerSetupGitHubCli @('auth', 'token')
    if (-not $login.ok -or -not $login.output.Trim()) {
        Add-Check 'bytes' BLOCKED "gh has no token to download with: $($login.output)" 'Authenticate gh (gh auth login) and rerun.'
        Complete
    }
    $token = $login.output.Trim()
    try {
        foreach ($asset in $remote) {
            $path = Join-Path $assetsDirectory $asset.name
            try { Save-TigerSetupReleaseAsset -Url ([string] $asset.url) -Path $path -Token $token }
            catch { Add-Check "bytes/$($asset.name)" BLOCKED "The download failed: $($_.Exception.Message)" 'Rerun once GitHub answers.'; Complete }
            $digest = if ($null -ne $asset.PSObject.Properties['digest']) { [string] $asset.digest } else { '' }
            if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or $digest -cne "sha256:$(Get-TigerSetupFileSha256 $path)") {
                Add-Check "bytes/$($asset.name)" FAIL "The download does not match GitHub's recorded digest '$digest'."
                Complete
            }
        }
    }
    finally { $token = $null }
    Add-Check 'bytes' PASS "The $($remote.Count) downloads match GitHub's recorded digests."
}

# The record closes the set.
try {
    $record = Assert-TigerSetupReleaseRecord -Directory $assetsDirectory -Version $Version
    Add-Check 'record' PASS "The set is closed, and its $(@($record.artifacts).Count + 2) files are the bytes the record and checksums name, for $($record.sourceCommit)."
}
catch { Add-Check 'record' FAIL $_.Exception.Message; Complete }
$commit = [string] $record.sourceCommit

# The tag, and the run it names. A set on disk is proved the same way once the
# tag exists, and not before.
try { $tagCommit = Get-TigerSetupRemoteTagCommit -RepositoryRoot $repoRoot -Tag $tag }
catch { Add-Check 'provenance' BLOCKED $_.Exception.Message 'Make origin reachable and rerun.'; Complete }
if ($local -and $null -eq $tagCommit) {
    Add-Check 'provenance' 'NOT RUN' "origin has no $tag yet: the release action creates it, and retrieving its draft proves it."
    Add-Check 'run' 'NOT RUN' "No $tag names a run yet."
}
else {
    try {
        $null = Assert-TigerSetupReleaseProvenance -RepositoryRoot $repoRoot -Directory $assetsDirectory -Version $Version
        Add-Check 'provenance' PASS "$tag is the release workflow's tag for this record at $commit."
    }
    catch { Add-Check 'provenance' FAIL $_.Exception.Message; Complete }
    try {
        $run = Assert-TigerSetupReleaseRun -TagMessage (Get-TigerSetupRemoteTag -RepositoryRoot $repoRoot -Tag $tag).message -CommitSha $commit
        Add-Check 'run' PASS "$($run.html_url): '$($run.name)', started by hand on $($facts.DefaultBranch) for $commit, $($run.conclusion)."
    }
    catch { Add-Check 'run' $(if ($_.Exception.Data['status'] -ceq 'BLOCKED') { 'BLOCKED' } else { 'FAIL' }) $_.Exception.Message; Complete }
}

# The release-bound terms.
try {
    $terms = @(Assert-TigerSetupReleaseTerms -RepositoryRoot $repoRoot -Directory $assetsDirectory -Version $Version -CommitSha $commit -WorkingTree:$Rehearsal)
    $source = if ($Rehearsal) { 'the prepared working tree holds' } else { "$commit holds" }
    Add-Check 'terms' PASS "$(@($terms | ForEach-Object { "$($_.name) ($($_.kind))" }) -join ' and ') are the bytes $source for them."
}
catch { Add-Check 'terms' FAIL $_.Exception.Message; Complete }

# The WinGet manifest set.
$installerName = "TigerSetup-$Version-Setup.exe"
$installer = Join-Path $assetsDirectory $installerName
$installerSha256 = Get-TigerSetupFileSha256 $installer
$wingetDirectory = Join-Path $OutputDirectory 'winget'
Expand-Archive -LiteralPath (Join-Path $assetsDirectory "TigerSetup-$Version-WinGet.zip") -DestinationPath $wingetDirectory
$problems = @(Test-TigerSetupWinGetManifestSet -ManifestDirectory $wingetDirectory -Version $Version -InstallerSha256 $installerSha256)
if ($problems.Count) { Add-Check 'winget' FAIL ($problems -join '; '); Complete }
Add-Check 'winget' PASS "The manifests for $($facts.PackageIdentifier) $Version name $(Get-TigerSetupInstallerUrl -Version $Version) and SHA-256 $($installerSha256.ToUpperInvariant()), and this release's LICENSE.txt and PRIVACY.md."

# The installer and what it installs.
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) {
    Add-Check 'installer' BLOCKED "No builder at $BuilderPath to unpack the installer with." 'Run cargo build --release, or pass -BuilderPath (an installed tiger-setup.exe reads the format too).'
    Complete
}
$payloadZip = Join-Path $OutputDirectory 'payload.zip'
$unpacked = & $BuilderPath inspect $installer --output-zip $payloadZip 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { Add-Check 'installer' FAIL "tiger-setup inspect refused the installer: $unpacked"; Complete }
$payloadDirectory = Join-Path $OutputDirectory 'payload'
Expand-Archive -LiteralPath $payloadZip -DestinationPath $payloadDirectory
Remove-Item -LiteralPath $payloadZip
$shipped = Join-Path $payloadDirectory 'tiger-setup.exe'
$reported = (& $shipped --version 2>&1 | Out-String).Trim()
$inspect = & $shipped inspect $installer --json 2>$null | Out-String
$inspectExit = $LASTEXITCODE
$global:LASTEXITCODE = 0
$installerProblems = [Collections.Generic.List[string]]::new()
if ($reported -cne "tiger-setup $Version") { $installerProblems.Add("the installed builder reports '$reported'") }
if ($inspectExit -ne 0) { $installerProblems.Add("inspect exited $inspectExit") }
else {
    $json = $inspect | ConvertFrom-Json
    $engine = $json.package.engine
    if ([string] $json.verification.status -cne 'ok') { $installerProblems.Add("verification is '$($json.verification.status)'") }
    if ([string] $json.package.id -cne $facts.PackageIdentifier -or [string] $json.package.version -cne $Version) { $installerProblems.Add("it is $($json.package.id) $($json.package.version)") }
    if ([string] $engine.engine_sha256 -cne (Get-TigerSetupFileSha256 (Join-Path $payloadDirectory 'tigersetup-setup.exe'))) { $installerProblems.Add('its engine is not the engine it installs') }
    if ([string] $engine.loader_sha256 -cne (Get-TigerSetupFileSha256 (Join-Path $payloadDirectory 'tigersetup-loader.exe'))) { $installerProblems.Add('its loader is not the loader it installs') }
}
if ($installerProblems.Count) { Add-Check 'installer' FAIL ($installerProblems -join '; '); Complete }
Add-Check 'installer' PASS "Verifies; carries the engine ($($engine.engine_sha256.Substring(0, 16))...) and loader it installs; $reported."

# The help the release build rendered: the one content a build produces from
# its own checkout and its own tiger-mark rather than from the binaries.
$helpPath = 'docs/TigerSetup-Help.md'
$expected = if ($Rehearsal) { Get-TigerSetupCommittedFile -RepositoryRoot $repoRoot -WorkingTree -Path $helpPath } else { Get-TigerSetupCommittedFile -RepositoryRoot $repoRoot -Commit $commit -Path $helpPath }
$label = if ($Rehearsal) { "the prepared tree:$helpPath" } else { "$($commit.Substring(0, 12)):$helpPath" }
$markdownPath = Join-Path $payloadDirectory 'help\TigerSetup-Help.md'
$markdown = if (Test-Path -LiteralPath $markdownPath -PathType Leaf) { , [IO.File]::ReadAllBytes($markdownPath) } else { $null }
$helpChecks = @(Get-TigerSetupCommittedTextChecks -Step 'help' -Name 'the Markdown help' -Code 'help' -Shipped $markdown -Expected $expected -ExpectedLabel $label)
$pdfPath = Join-Path $payloadDirectory 'help\TigerSetup-Help.pdf'
$pdfTitle = if (Test-Path -LiteralPath $pdfPath -PathType Leaf) { Get-TigerSetupPdfTitle -Bytes ([IO.File]::ReadAllBytes($pdfPath)) } else { $null }
$failed = @($helpChecks | Where-Object status -NE 'PASS')
if ($pdfTitle -cne 'TigerSetup-Help.md') { $failed += [pscustomobject]@{ name = 'help/the PDF help is a PDF rendered from TigerSetup-Help.md'; message = "PDF title: '$pdfTitle'" } }
if ($failed.Count) { Add-Check 'help' FAIL (@($failed | ForEach-Object { "$($_.name): $($_.message)" }) -join '; '); Complete }
Add-Check 'help' PASS "The shipped Markdown is CRLF-only and exactly $label as Git checks it out; the shipped PDF was rendered from it."

# The public bytes, once published: the installer WinGet downloads, and the
# terms its LicenseUrl and PrivacyUrl name.
if ($null -ne $release -and -not $release.draft) {
    $served = @($installerName) + @(Get-TigerSetupReleaseTerms -Version $Version | ForEach-Object name)
    foreach ($name in $served) {
        $publicUrl = Get-TigerSetupReleaseAssetUrl -Version $Version -Name $name
        $wanted = Get-TigerSetupFileSha256 (Join-Path $assetsDirectory $name)
        $public = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-public-$([guid]::NewGuid().ToString('N'))"
        try {
            try { Invoke-WebRequest -Uri $publicUrl -OutFile $public -UseBasicParsing }
            catch { Add-Check "public/$name" FAIL "$publicUrl is not served anonymously: $($_.Exception.Message)"; Complete }
            $publicSha256 = Get-TigerSetupFileSha256 $public
            if ($publicSha256 -cne $wanted) { Add-Check "public/$name" FAIL "$publicUrl serves $publicSha256, not $wanted."; Complete }
            Add-Check "public/$name" PASS "$publicUrl serves these exact bytes anonymously."
        }
        finally { Remove-Item -LiteralPath $public -Force -ErrorAction SilentlyContinue }
    }
}

# The WinGet client's own check. No lab row repeats it, so it is required here.
$winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue
if ($null -eq $winget) {
    Add-Check 'validate' BLOCKED 'winget is not installed here, and release validation runs winget validate on this machine.' 'Install App Installer (WinGet) and rerun.'
}
else {
    $output = & cmd.exe /d /c winget validate --manifest "$wingetDirectory" --disable-interactivity 2>&1 | Out-String
    $code = $LASTEXITCODE
    $global:LASTEXITCODE = 0
    # 0x8A150028: validation succeeded with warnings.
    if ($code -eq 0 -or $code -eq -1978335192) { Add-Check 'validate' PASS "winget validate accepts the set$(if ($code) { ' with warnings' })." }
    else { Add-Check 'validate' FAIL "winget validate exited $code`: $($output.Trim())"; Complete }
}

$source = if ($Rehearsal) { '-SourceWorkingTree' } else { "-SourceCommit $commit" }
$lab = "pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath `"$installer`" -BuilderPath `"$shipped`" $source -Rows moderator -ManifestDirectory `"$wingetDirectory`""
Complete -Next $(if ($local) {
        @(
            'The same lab row release validation runs, on this set (RELEASING.md, Prepare):'
            "  $lab"
        )
    }
    elseif ($release.draft) {
        @(
            'Prove these bytes usable on Windows before publication, with the one release-validation row (RELEASING.md, Release validation):'
            "  $lab"
            'Then the Architect reviews and publishes the draft; after publication, rerun this script to prove the public URLs.'
        )
    }
    else {
        @("Prepare the WinGet submission: pwsh -File eng\release\New-WinGetSubmission.ps1 -Version $Version -WinGetPkgsRoot <winget-pkgs clone>")
    })
