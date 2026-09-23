<#
    .SYNOPSIS
    Retrieves a release's authoritative assets from its GitHub Release, draft
    or published, and proves them before anything is validated or submitted.

    .DESCRIPTION
    The assets the release workflow attached are the release; nothing is
    rebuilt. This downloads them into artifacts\release\<version>\assets and
    proves the chain from the commit to these bytes:

      release     the GitHub Release for v<version> exists, is 'TigerSetup
                  <version>' and carries exactly the four assets
      bytes       every file is the bytes GitHub recorded (asset digest) and
                  the bytes release-artifacts.json records (with SHA256SUMS.txt)
      provenance  origin's v<version> is the release workflow's annotated tag
                  (github-actions[bot]) at the record's commit, and its message
                  names this record's SHA-256 - so the set is the one that
                  workflow built, not one uploaded from anywhere else
      winget      the manifest set is for this package and version, and names
                  the published URL and this installer's SHA-256
      installer   the installer verifies and carries exactly the engine and
                  loader it installs, and the builder it installs reports this
                  version
      public      once the release is published: the public URL serves these
                  exact installer bytes, anonymously
      validate    `winget validate` accepts the manifest set; NOT RUN where
                  winget is not installed, and the lab's WinGet rows run it

    It unpacks the manifest set into ...\winget and the installer's files
    into ...\payload, and ends by naming the lab rows that prove these bytes
    on Windows (RELEASING.md). The release is found through `gh`, so gh must
    be authenticated (`gh auth login`); a draft is visible only to an account
    with push access to the repository.

    .EXAMPLE
    pwsh -File eng\release\Get-ReleaseArtifacts.ps1 -Version 0.12.0
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Version,
    # Defaults to artifacts\release\<version>.
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
$repoRoot = if ($RepositoryRoot) { $RepositoryRoot } else { Split-Path -Parent (Split-Path -Parent $PSScriptRoot) }
$workspaceRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$facts = Get-TigerSetupReleaseFacts
$tag = Get-TigerSetupReleaseTag -Version $Version
$title = "$($facts.Product) $Version"
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $workspaceRoot "artifacts\release\$Version" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $BuilderPath) { $BuilderPath = Join-Path $workspaceRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
$checks = [Collections.Generic.List[object]]::new()
function Add-Check { param($Id, $Status, $Observed, $Remediation = '') $checks.Add((New-TigerSetupReleaseCheck -Id $Id -Status $Status -Observed $Observed -Remediation $Remediation)) }
function Complete { param([string[]] $Next = @()) exit (Write-TigerSetupReleaseReport -Title "Release artifacts of TigerSetup $Version" -Checks $checks.ToArray() -Next $Next) }

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

# The bytes.
$assetsDirectory = Join-Path $OutputDirectory 'assets'
if (Test-Path -LiteralPath $OutputDirectory) {
    # Only a directory this script wrote is replaced.
    $foreign = @(Get-ChildItem -LiteralPath $OutputDirectory -Force | Where-Object { $_.Name -cnotin @('assets', 'winget', 'payload') })
    if ($foreign.Count) { throw "$OutputDirectory holds files this script did not write: $($foreign.Name -join ', ')." }
    Remove-Item -LiteralPath $OutputDirectory -Recurse -Force
}
$null = New-Item -ItemType Directory -Path $assetsDirectory -Force
$token = if ($release.draft) { (Invoke-TigerSetupGitHubCli @('auth', 'token')).output.Trim() } else { '' }
foreach ($asset in $remote) {
    $path = Join-Path $assetsDirectory $asset.name
    if ($release.draft) {
        Invoke-WebRequest -Uri $asset.url -OutFile $path -UseBasicParsing -Headers @{ Authorization = "Bearer $token"; Accept = 'application/octet-stream' }
    }
    else {
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $path -UseBasicParsing
    }
    $digest = if ($null -ne $asset.PSObject.Properties['digest']) { [string] $asset.digest } else { '' }
    if ($digest -cne "sha256:$(Get-TigerSetupFileSha256 $path)") {
        Add-Check "bytes/$($asset.name)" FAIL "The download does not match GitHub's recorded digest '$digest'."
        Complete
    }
}
$token = $null
Add-Check 'bytes' PASS "The four downloads match GitHub's recorded digests."
try {
    $record = Assert-TigerSetupReleaseProvenance -RepositoryRoot $repoRoot -Directory $assetsDirectory -Version $Version
    Add-Check 'provenance' PASS "The record and checksums match the files; $tag is the release workflow's tag for this record at $($record.sourceCommit)."
}
catch { Add-Check 'provenance' FAIL $_.Exception.Message; Complete }

# The WinGet manifest set.
$installerName = "TigerSetup-$Version-Setup.exe"
$installer = Join-Path $assetsDirectory $installerName
$installerSha256 = (Get-TigerSetupFileSha256 $installer).ToUpperInvariant()
$url = Get-TigerSetupInstallerUrl -Version $Version
$wingetDirectory = Join-Path $OutputDirectory 'winget'
Expand-Archive -LiteralPath (Join-Path $assetsDirectory "TigerSetup-$Version-WinGet.zip") -DestinationPath $wingetDirectory
$id = $facts.PackageIdentifier
$expectedFiles = @("$id.installer.yaml", "$id.locale.en-US.yaml", "$id.yaml")
$files = @(Get-ChildItem -LiteralPath $wingetDirectory -Recurse -File | ForEach-Object Name | Sort-Object)
$problems = [Collections.Generic.List[string]]::new()
if (($files -join '|') -cne ($expectedFiles -join '|')) { $problems.Add("the set is [$($files -join ', ')]") }
foreach ($file in @(Get-ChildItem -LiteralPath $wingetDirectory -File)) {
    $text = Get-Content -LiteralPath $file.FullName -Raw
    if ($text -notmatch "(?m)^PackageIdentifier:\s*$([regex]::Escape($id))\s*$") { $problems.Add("$($file.Name) is not $id") }
    if ($text -notmatch "(?m)^PackageVersion:\s*$([regex]::Escape($Version))\s*$") { $problems.Add("$($file.Name) is not version $Version") }
}
$installerManifest = Join-Path $wingetDirectory "$id.installer.yaml"
if (Test-Path -LiteralPath $installerManifest) {
    $text = Get-Content -LiteralPath $installerManifest -Raw
    $urls = @([regex]::Matches($text, '(?m)^\s*-?\s*InstallerUrl:\s*(\S+)\s*$') | ForEach-Object { $_.Groups[1].Value })
    $hashes = @([regex]::Matches($text, '(?m)^\s*-?\s*InstallerSha256:\s*(\S+)\s*$') | ForEach-Object { $_.Groups[1].Value })
    if ($urls.Count -eq 0 -or @($urls | Where-Object { $_ -cne $url }).Count) { $problems.Add("InstallerUrl is [$($urls -join ', ')], not $url") }
    if ($hashes.Count -ne $urls.Count -or @($hashes | Where-Object { $_ -cne $installerSha256 }).Count) { $problems.Add("InstallerSha256 is [$($hashes -join ', ')], not $installerSha256") }
}
if ($problems.Count) { Add-Check 'winget' FAIL ($problems -join '; '); Complete }
Add-Check 'winget' PASS "$($files.Count) manifests for $id $Version name $url and SHA-256 $installerSha256."

# The installer and what it installs.
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) {
    Add-Check 'installer' BLOCKED "No builder at $BuilderPath to unpack the installer with." 'Run cargo build --release, or pass -BuilderPath.'
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
$problems.Clear()
if ($reported -cne "tiger-setup $Version") { $problems.Add("the installed builder reports '$reported'") }
if ($inspectExit -ne 0) { $problems.Add("inspect exited $inspectExit") }
else {
    $json = $inspect | ConvertFrom-Json
    $engine = $json.package.engine
    if ([string] $json.verification.status -cne 'ok') { $problems.Add("verification is '$($json.verification.status)'") }
    if ([string] $json.package.id -cne $id -or [string] $json.package.version -cne $Version) { $problems.Add("it is $($json.package.id) $($json.package.version)") }
    if ([string] $engine.engine_sha256 -cne (Get-TigerSetupFileSha256 (Join-Path $payloadDirectory 'tigersetup-setup.exe'))) { $problems.Add('its engine is not the engine it installs') }
    if ([string] $engine.loader_sha256 -cne (Get-TigerSetupFileSha256 (Join-Path $payloadDirectory 'tigersetup-loader.exe'))) { $problems.Add('its loader is not the loader it installs') }
}
if ($problems.Count) { Add-Check 'installer' FAIL ($problems -join '; '); Complete }
Add-Check 'installer' PASS "Verifies; carries the engine ($($engine.engine_sha256.Substring(0, 16))...) and loader it installs; $reported."

# The public bytes, once published.
if (-not $release.draft) {
    $public = Join-Path ([IO.Path]::GetTempPath()) "tigersetup-public-$([guid]::NewGuid().ToString('N')).exe"
    try {
        try { Invoke-WebRequest -Uri $url -OutFile $public -UseBasicParsing }
        catch { Add-Check 'public' FAIL "$url is not served anonymously: $($_.Exception.Message)"; Complete }
        $publicSha256 = (Get-TigerSetupFileSha256 $public).ToUpperInvariant()
        if ($publicSha256 -cne $installerSha256) { Add-Check 'public' FAIL "$url serves $publicSha256, not $installerSha256."; Complete }
        Add-Check 'public' PASS "$url serves these exact bytes anonymously."
    }
    finally { Remove-Item -LiteralPath $public -Force -ErrorAction SilentlyContinue }
}

# The WinGet client's own check, where there is one.
$winget = Get-Command winget -CommandType Application -ErrorAction SilentlyContinue
if ($null -eq $winget) {
    Add-Check 'validate' 'NOT RUN' 'winget is not installed here; the lab WinGet rows run winget validate.'
}
else {
    $output = & cmd.exe /d /c winget validate --manifest "$wingetDirectory" --disable-interactivity 2>&1 | Out-String
    $code = $LASTEXITCODE
    $global:LASTEXITCODE = 0
    # 0x8A150028: validation succeeded with warnings.
    if ($code -eq 0 -or $code -eq -1978335192) { Add-Check 'validate' PASS "winget validate accepts the set$(if ($code) { ' with warnings' })." }
    else { Add-Check 'validate' FAIL "winget validate exited $code`: $($output.Trim())"; Complete }
}

$lab = "pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath `"$installer`" -BuilderPath `"$shipped`""
Complete -Next $(if ($release.draft) {
        @(
            'Prove these bytes on Windows before publication (RELEASING.md, Release validation):'
            "  $lab"
            "  $lab -Rows winget-user,winget-machine,moderator -ManifestDirectory `"$wingetDirectory`""
            'Then the Architect reviews and publishes the draft; after publication, rerun this script to prove the public URL.'
        )
    }
    else {
        @("Prepare the WinGet submission: pwsh -File eng\release\New-WinGetSubmission.ps1 -Version $Version -WinGetPkgsRoot <winget-pkgs clone>")
    })
