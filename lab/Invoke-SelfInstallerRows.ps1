#Requires -Version 7.0
<#
    .SYNOPSIS
    The self-hosted TigerSetup installer end to end on a clean Windows 11, in
    each scope: a quiet install, the installed `tiger-setup` answering with
    the version the installer carries, `verify`, the registration and PATH
    entry as Windows holds them, a quiet uninstall, and nothing left behind —
    the install root, the state directory, the registration, the PATH entry
    and the loader's extraction directories included.

    .DESCRIPTION
    One lab session, one VM, two chained jobs per row, each read with the
    generic guest reader (guest\Invoke-SetupCommands.ps1):

      install     `Setup.exe install --quiet --scope <scope>`; then the
                  installed `tiger-setup.exe --version`, the VERSIONINFO of
                  every installed executable, `verify --json` and
                  `inspect --json`; the install root is inventoried with
                  hashes, the state directory, the registration key, both
                  PATH values and the loader's extraction roots are read
      uninstall   `Setup.exe uninstall --quiet --scope <scope>`; the same
                  reads, which must now find nothing of the product

    The row derives what it expects from the installer itself
    (`tiger-setup inspect --json`): the package version, the install root of
    the scope, the registration key and the PATH entry the `path` option
    declares. A row is one scope; `-Rows user,machine` (the default) runs
    both from the baseline. Both run as the job account — an administrator in
    session 0 — so the machine scope needs no prompt; the elevation rows
    (`Invoke-ElevationRows.ps1`) are where the wizard, the shield and the
    genuine UAC prompt are proved.

    The loader extracts the engine under `%TEMP%\TigerSetup\<pid>-<tick>-
    <attempt>` (unelevated) or `%SystemRoot%\Temp\TigerSetup-<pid>-<tick>-
    <attempt>` (elevated) and removes the directory on every path; after each
    step both roots must hold nothing of this run's — read as directory
    listings, because a directory the loader failed to remove is empty and an
    inventory of files would not see it.

    The presence rows prove what a person finds after the install, on the
    lab's interactive desktop (guest\Invoke-PresenceAcceptance.ps1):

      user-nopath     per-user install as the signed-in user with the PATH
                      option off; the Start Menu folder holds exactly
                      TigerSetup Shell, TigerSetup Help (the PDF) and
                      TigerSetup Help (Markdown), and only the shell wears
                      TigerSetup's icon; TigerSetup Shell opens on the brief
                      help (coloured where the terminal says so), resolves
                      this installation and stays usable, and --help, build
                      --help and the absent help command answer as they
                      should in it; both help forms open, the PDF directly;
                      the installed help is the shipped bytes, its Markdown is
                      docs\TigerSetup-Help.md and its PDF was rendered from
                      it; uninstall leaves no Start Menu entry behind
      machine-nopath  the same for everyone, installed by the job account and
                      used by the signed-in standard user
      upgrade         -PreviousInstallerPath upgraded to this one: from a
                      release without the Start Menu folder, the upgrade adds
                      it; from one with another layout, the upgrade renames,
                      retargets and retires links until the folder holds
                      exactly this release's (a previous build of this same
                      version is reinstalled with the PATH choice stated, as
                      a same-version rerun with nothing to change is a
                      no-op); the same version again keeps them; uninstall
                      removes them

    The WinGet rows take the finished manifest set (-ManifestDirectory, after
    `tiger-setup winget finalize`):

      winget-user, winget-machine
                      TigerWinLab's WinGet scenario on the selected installer
                      entry: the manifest set's consistency and `winget
                      validate`, a hash-mismatch probe, `winget install
                      --manifest`, the installation, the declared command,
                      the uninstall and the cleanup
      moderator       what a WinGet moderator does, as the signed-in standard
                      user: `winget install --manifest`, `winget list`, Start >
                      type "TigerSetup Shell" > Enter, the shell checks above,
                      TigerSetup Help (the PDF) and the Markdown, `winget
                      uninstall`, and nothing left

    .EXAMPLE
    pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath artifacts\tigersetup\TigerSetup-0.12.0-Setup.exe
    pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath artifacts\tigersetup\TigerSetup-0.12.0-Setup.exe `
        -Rows winget-user,winget-machine,moderator -ManifestDirectory artifacts\tigersetup\winget
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [string] $BuilderPath,
    # The rows to run, each from the baseline. The default is every row that
    # needs neither a previous release nor a WinGet manifest set.
    [string[]] $Rows = @('user', 'machine', 'user-nopath', 'machine-nopath'),
    # The release the upgrade row starts from.
    [string] $PreviousInstallerPath,
    # The finished WinGet manifest set for this installer (winget rows).
    [string] $ManifestDirectory,
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    [string] $SessionId,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    [int] $JobTimeoutMinutes = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) {
    throw "The builder '$BuilderPath' does not exist; run cargo build --release first, or pass -BuilderPath."
}
$BuilderPath = (Resolve-Path -LiteralPath $BuilderPath).Path
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$knownRows = @('user', 'machine', 'user-nopath', 'machine-nopath', 'upgrade', 'winget-user', 'winget-machine', 'moderator')
foreach ($row in $Rows) {
    if ($row -notin $knownRows) { throw "Unknown row '$row'; the rows are $($knownRows -join ', ')." }
    $rowScope = $(if ($row -match 'machine') { 'machine' } else { 'user' })
    if ($rowScope -notin @($facts.scopes)) { throw "'$InstallerPath' allows scopes $($facts.scopes -join ', '); it has no '$rowScope' scope." }
}
if (@($Rows | Where-Object { $_ -in @('winget-user', 'winget-machine', 'moderator') }).Count -gt 0) {
    if ([string]::IsNullOrWhiteSpace($ManifestDirectory) -or -not (Test-Path -LiteralPath $ManifestDirectory -PathType Container)) {
        throw 'The WinGet rows need -ManifestDirectory: the finished manifest set for this installer (tiger-setup winget prepare, then finalize).'
    }
    $ManifestDirectory = (Resolve-Path -LiteralPath $ManifestDirectory).Path
}
if ($Rows -contains 'upgrade') {
    if ([string]::IsNullOrWhiteSpace($PreviousInstallerPath)) { throw 'The upgrade row needs -PreviousInstallerPath: the release it upgrades from.' }
    $PreviousInstallerPath = (Resolve-Path -LiteralPath $PreviousInstallerPath).Path
}

# The shipped help, taken out of the installer on the host: the bytes every
# installed copy must equal, the Markdown that must be the repository's
# docs\TigerSetup-Help.md, and the PDF that must name that Markdown as the
# document it was rendered from. The rows compare what the guest installed
# with this.
$shipped = @{}
$helpSource = Join-Path $repoRoot 'docs\TigerSetup-Help.md'
$exportZip = Join-Path ([System.IO.Path]::GetTempPath()) ('TigerSetupSelf-' + [Guid]::NewGuid().ToString('N') + '.zip')
try {
    $null = & $BuilderPath inspect $InstallerPath --output-zip $exportZip 2>&1
    if ($LASTEXITCODE -ne 0) { throw "tiger-setup inspect --output-zip failed ($LASTEXITCODE)." }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($exportZip)
    try {
        foreach ($entry in $zip.Entries) {
            $stream = $entry.Open()
            try {
                $memory = [System.IO.MemoryStream]::new()
                $stream.CopyTo($memory)
                $bytes = $memory.ToArray()
            }
            finally { $stream.Dispose() }
            $shipped[$entry.FullName.Replace('\', '/')] = [pscustomobject]@{
                sha256 = [System.Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
                title = $(if ($entry.FullName -like '*.pdf') { $m = [regex]::Match([System.Text.Encoding]::Latin1.GetString($bytes), '/Title\s*\(([^)]*)\)'); if ($m.Success) { $m.Groups[1].Value } else { $null } } else { $null })
            }
        }
    }
    finally { $zip.Dispose() }
}
finally { Remove-Item -LiteralPath $exportZip -Force -ErrorAction SilentlyContinue }
$helpSourceSha256 = (Get-FileHash -LiteralPath $helpSource -Algorithm SHA256).Hash.ToLowerInvariant()

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\self-installer-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
$guestInstaller = Join-Path $GuestStageRoot ([System.IO.Path]::GetFileName($InstallerPath))
$guestLogRoot = Join-Path $GuestStageRoot 'self-installer'

function Get-Evidence {
    param([object] $JobRun)
    if ($null -eq $JobRun -or $JobRun.status -ne 'OK' -or $null -eq $JobRun.result) { return $null }
    Get-Member2 $JobRun.result 'result'
}
function Get-Command2 {
    param([object] $Evidence, [string] $Name)
    if ($null -eq $Evidence) { return $null }
    @(@(Get-Member2 $Evidence 'commands') | Where-Object { $null -ne $_ -and $_.name -eq $Name }) | Select-Object -First 1
}
function Get-Inventory {
    param([object] $Evidence, [string] $Requested)
    if ($null -eq $Evidence) { return $null }
    @(@(Get-Member2 $Evidence 'inventory') | Where-Object { $null -ne $_ -and $_.requested -eq $Requested }) | Select-Object -First 1
}
function Get-RegistryRecord {
    param([object] $Evidence, [string] $Key)
    if ($null -eq $Evidence) { return $null }
    @(@(Get-Member2 $Evidence 'registry') | Where-Object { $null -ne $_ -and $_.requested -eq $Key }) | Select-Object -First 1
}
function Get-Codes {
    param([object] $Document)
    @(@(Get-Member2 $Document 'findings') | Where-Object { $null -ne $_ } | ForEach-Object { [string] (Get-Member2 $_ 'code') })
}
# The guest command that lists what is under the loader's roots: every entry
# under `%TEMP%\TigerSetup`, and the `TigerSetup-*` entries under
# `%SystemRoot%\Temp` (which holds other things too). One full path per line;
# nothing printed is the clean state.
$loaderResidueCommand = @(
    "Get-ChildItem -LiteralPath (Join-Path `$env:TEMP 'TigerSetup') -Force -ErrorAction SilentlyContinue | ForEach-Object { `$_.FullName }",
    "Get-ChildItem -LiteralPath (Join-Path `$env:SystemRoot 'Temp') -Filter 'TigerSetup-*' -Force -ErrorAction SilentlyContinue | ForEach-Object { `$_.FullName }",
    # A standard user may not list %SystemRoot%\Temp, where only an elevated
    # run extracts; the listing answers, and a refused read is not a failure.
    "exit 0"
) -join '; '
function New-LoaderResidueCommand {
    @{ name = 'loader-residue'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-NonInteractive', '-Command', $loaderResidueCommand); timeoutSeconds = 60 }
}
function Get-LoaderResidue {
    <# The lines the residue command printed: the entries the loader left. #>
    param([object] $Evidence)
    $command = Get-Command2 -Evidence $Evidence -Name 'loader-residue'
    if ($null -eq $command) { return @('(the residue command did not run)') }
    if ((Get-Member2 $command 'exitCode') -ne 0) { return @("(the residue command exited $(Get-Member2 $command 'exitCode'): $(Get-Member2 $command 'stderr'))") }
    @(([string] (Get-Member2 $command 'stdout')) -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}
function New-SetupCommand {
    <# A Setup.exe command with --json; a mutating one writes its own log too. #>
    param([string] $Name, [string[]] $Arguments, [int] $TimeoutSeconds = 600)
    $tail = @('--json')
    if ($Arguments[0] -in @('install', 'repair', 'uninstall')) { $tail += @('--log', "$guestLogRoot\$Name.log") }
    @{ name = $Name; executable = $guestInstaller; arguments = @($Arguments + $tail); timeoutSeconds = $TimeoutSeconds }
}

function Invoke-ScopeRow {
    param([string] $Scope)

    $checks = [System.Collections.Generic.List[object]]::new()
    function Add-Check {
        param([string] $Name, [string] $Code, [bool] $Pass, [string] $Message = '')
        $checks.Add((New-TigerSetupCheck -Name $Name -Code $Code -Status $(if ($Pass) { 'PASS' } else { 'FAIL' }) -Message $Message))
    }
    function Add-CommandCheck {
        <# The command ran, exited 0 and printed the outcome the step expects. #>
        param([object] $Evidence, [string] $Step, [string] $Name, [string] $Outcome)
        $command = Get-Command2 -Evidence $Evidence -Name $Name
        if ($null -eq $command) {
            Add-Check -Name "$Step/$Name ran" -Code "self.$Step.$Name.ran" -Pass $false -Message 'no record of the command'
            return $null
        }
        $exit = Get-Member2 $command 'exitCode'
        Add-Check -Name "$Step/$Name exit 0" -Code "self.$Step.$Name.exit" -Pass ($exit -eq 0) -Message "exit $exit after $($command.durationSeconds)s; $($command.stderr)"
        $document = Get-Member2 $command 'json'
        if (-not [string]::IsNullOrWhiteSpace($Outcome)) {
            $actual = [string] (Get-Member2 $document 'outcome')
            Add-Check -Name "$Step/$Name outcome $Outcome" -Code "self.$Step.$Name.outcome" -Pass ($actual -eq $Outcome) -Message "outcome=$actual code=$([string] (Get-Member2 $document 'code'))"
        }
        $document
    }
    function Add-VersionCheck {
        <# An installed executable's --version names the package's version. #>
        param([object] $Evidence, [string] $Step, [string] $Name)
        $command = Get-Command2 -Evidence $Evidence -Name $Name
        $stdout = $(if ($null -ne $command) { ([string] (Get-Member2 $command 'stdout')).Trim() } else { '' })
        $exit = $(if ($null -ne $command) { Get-Member2 $command 'exitCode' } else { $null })
        Add-Check -Name "$Step/$Name reports $($facts.version)" -Code "self.$Step.$Name.version" `
            -Pass ($exit -eq 0 -and $stdout -match ('(^|\s)' + [regex]::Escape($facts.version) + '(\s|$)')) -Message "exit $exit; stdout: $stdout"
    }
    function Add-VersionInfoChecks {
        <#
            The VERSIONINFO of every installed executable, as the guest's
            PowerShell reads it: `name|ProductVersion|FileVersion` per line.
            The engine has no --version of its own — the loader runs it, and
            its identity is its VERSIONINFO and the hash the installer states.
        #>
        param([object] $Evidence, [string] $Step, [string] $Name)
        $command = Get-Command2 -Evidence $Evidence -Name $Name
        $stdout = $(if ($null -ne $command) { [string] (Get-Member2 $command 'stdout') } else { '' })
        $lines = @($stdout -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
        $fileVersion = $facts.version + '.0'
        foreach ($file in @($facts.files | Where-Object { $_ -like '*.exe' })) {
            $line = @($lines | Where-Object { $_ -like "$file|*" }) | Select-Object -First 1
            $parts = @($(if ($null -ne $line) { $line -split '\|' } else { @() }))
            Add-Check -Name "$Step/$file VERSIONINFO $($facts.version) / $fileVersion" -Code "self.$Step.versioninfo" `
                -Pass ($parts.Count -eq 3 -and $parts[1] -eq $facts.version -and $parts[2] -eq $fileVersion) -Message $(if ($null -ne $line) { $line } else { "no line for $file in: $stdout" })
        }
    }
    function Add-ResidueChecks {
        <# After an uninstall, nothing of the product remains where it was. #>
        param([object] $Evidence, [string] $Step)
        $root = Get-Inventory -Evidence $Evidence -Requested $installRoot
        Add-Check -Name "$Step/install root gone" -Code "self.$Step.root.absent" -Pass ($null -ne $root -and -not [bool] $root.exists) -Message "$installRoot exists=$(if ($null -ne $root) { $root.exists } else { 'unread' }) files=$(if ($null -ne $root) { $root.fileCount } else { '?' })"
        $state = Get-Inventory -Evidence $Evidence -Requested $stateDir
        Add-Check -Name "$Step/state directory gone" -Code "self.$Step.state.absent" -Pass ($null -ne $state -and -not [bool] $state.exists) -Message "$stateDir exists=$(if ($null -ne $state) { $state.exists } else { 'unread' }) files=$(if ($null -ne $state) { $state.fileCount } else { '?' })"
        $registration = Get-RegistryRecord -Evidence $Evidence -Key $registrationKey
        Add-Check -Name "$Step/registration gone" -Code "self.$Step.registration.absent" -Pass ($null -ne $registration -and -not [bool] $registration.exists) -Message $registrationKey
        $pathValue = Get-PathEntries -Evidence $Evidence
        Add-Check -Name "$Step/PATH entry gone" -Code "self.$Step.path.absent" -Pass ($null -ne $pathValue -and @($pathValue | Where-Object { $_ -eq $installRootExpanded }).Count -eq 0) -Message ($pathValue -join ';')
    }
    function Get-PathEntries {
        param([object] $Evidence)
        $values = Get-Member2 $Evidence 'pathValues'
        if ($null -eq $values) { return $null }
        $block = Get-Member2 $values $Scope
        if ($null -eq $block) { return $null }
        @(@(Get-Member2 $block 'entries') | ForEach-Object { ([string] $_).TrimEnd('\') })
    }
    function Invoke-Step {
        param([string] $Step, [hashtable] $Request, [string[]] $PayloadFiles = @(), [switch] $FromBaseline)
        $policy = Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline
        $run = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $Request -PayloadFiles $PayloadFiles `
            -Name "ts-self-$Scope-$Step" @policy -ResultPath (Join-Path $ResultsRoot "runs\$Scope-$Step.json") -OutputRoot $labOutputRoot `
            -TimeoutMinutes $JobTimeoutMinutes
        Add-Check -Name "$Step/lab job" -Code "self.$Step.lab" -Pass ($run.status -eq 'OK') -Message "status $($run.status) (exit $($run.exitCode)) after $($run.durationSeconds)s"
        Get-Evidence $run
    }

    $installRoot = Get-TigerSetupInstallRoot -Facts $facts -Scope $Scope
    $stateDir = $(if ($Scope -eq 'machine') { "%ProgramData%\TigerSetup\$($facts.id)" } else { "%LOCALAPPDATA%\TigerSetup\$($facts.id)" })
    $registrationKey = $(if ($Scope -eq 'machine') { 'HKLM' } else { 'HKCU' }) + "\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($facts.registrationKey)"
    $expectedPath = @(Get-TigerSetupExpectedPathEntries -Facts $facts -Options @{} -InstallRoot $installRoot)
    $versionInfoCommand = "Get-ChildItem -LiteralPath '$installRoot' -Filter *.exe | ForEach-Object { `$v = `$_.VersionInfo; `$_.Name + '|' + `$v.ProductVersion + '|' + `$v.FileVersion }"
    $reads = @{
        inventory = @($installRoot, $stateDir)
        registry = @($registrationKey)
        pathValues = $true
    }
    $installRootExpanded = $null

    Write-Host ''; Write-Host "### $Scope / install"
    $installed = Invoke-Step -Step 'install' -FromBaseline -PayloadFiles @($InstallerPath) -Request ($reads + @{
            stage = @(@{ source = [System.IO.Path]::GetFileName($InstallerPath); destination = $guestInstaller })
            commands = @(
                @{ name = 'mkdir'; executable = 'cmd.exe'; arguments = @('/c', 'mkdir', $guestLogRoot); timeoutSeconds = 30 },
                (New-SetupCommand -Name 'install' -Arguments @('install', '--quiet', '--scope', $Scope)),
                @{ name = 'builder-version'; executable = "$installRoot\tiger-setup.exe"; arguments = @('--version'); timeoutSeconds = 60 },
                @{ name = 'versioninfo'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-NonInteractive', '-Command', $versionInfoCommand); timeoutSeconds = 60 },
                (New-SetupCommand -Name 'verify' -Arguments @('verify', '--scope', $Scope)),
                (New-SetupCommand -Name 'inspect' -Arguments @('inspect', '--scope', $Scope)),
                (New-LoaderResidueCommand)
            )
            runAs = 'job'
            inventoryHashes = $true
            logs = @("$guestLogRoot\install.log")
        })
    $environment = $(if ($null -ne $installed) { Get-Member2 $installed 'environment' } else { $null })
    $document = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'install' -Outcome 'installed'
    $outcomeInstallation = Get-Member2 $document 'installation'
    Add-Check -Name "install/scope $Scope" -Code 'self.install.scope' -Pass ([string] (Get-Member2 $outcomeInstallation 'scope') -eq $Scope) -Message "scope=$([string] (Get-Member2 $outcomeInstallation 'scope'))"
    Add-VersionCheck -Evidence $installed -Step 'install' -Name 'builder-version'
    Add-VersionInfoChecks -Evidence $installed -Step 'install' -Name 'versioninfo'
    $verifyDocument = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'verify' -Outcome ''
    Add-Check -Name 'install/verify ok' -Code 'self.install.verify.status' -Pass ([string] (Get-Member2 $verifyDocument 'status') -eq 'ok') -Message ("findings: " + ((Get-Codes $verifyDocument) -join ', '))
    $inspectDocument = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'inspect' -Outcome ''
    $installation = Get-Member2 $inspectDocument 'installation'
    Add-Check -Name "install/inspect reports $($facts.version) in $Scope scope" -Code 'self.install.inspect.identity' `
        -Pass ([string] (Get-Member2 $installation 'version') -eq $facts.version -and [string] (Get-Member2 $installation 'scope') -eq $Scope) `
        -Message "version=$([string] (Get-Member2 $installation 'version')) scope=$([string] (Get-Member2 $installation 'scope'))"
    $root = Get-Inventory -Evidence $installed -Requested $installRoot
    if ($null -ne $root) { $installRootExpanded = ([string] $root.path).TrimEnd('\') }
    $installedFiles = @($(if ($null -ne $root) { @($root.files) | ForEach-Object { ([string] $_).Replace('\', '/') } } else { @() }))
    foreach ($file in @($facts.files)) {
        Add-Check -Name "install/$file installed" -Code 'self.install.file' -Pass ($installedFiles -contains $file) -Message ($installedFiles -join ', ')
    }
    Add-Check -Name 'install/the install root holds the package files only' -Code 'self.install.root.exact' `
        -Pass ($null -ne $root -and [bool] $root.exists -and [int] $root.fileCount -eq @($facts.files).Count) -Message "files: $($installedFiles -join ', ')"
    $state = Get-Inventory -Evidence $installed -Requested $stateDir
    $stateFiles = @($(if ($null -ne $state) { @($state.files) | ForEach-Object { [string] $_ } } else { @() }))
    Add-Check -Name 'install/state database present' -Code 'self.install.state.db' -Pass ($stateFiles -contains 'state.db') -Message ($stateFiles -join ', ')
    Add-Check -Name 'install/uninstaller present' -Code 'self.install.state.uninstaller' -Pass (@($stateFiles | Where-Object { $_ -like '*.exe' }).Count -ge 1) -Message ($stateFiles -join ', ')
    $registration = Get-RegistryRecord -Evidence $installed -Key $registrationKey
    $values = $(if ($null -ne $registration) { Get-Member2 $registration 'values' } else { $null })
    Add-Check -Name "install/registered as $($facts.version)" -Code 'self.install.registration' `
        -Pass ($null -ne $registration -and [bool] $registration.exists -and [string] (Get-Member2 $values 'DisplayVersion') -eq $facts.version) `
        -Message "$registrationKey exists=$(if ($null -ne $registration) { $registration.exists } else { 'unread' }) DisplayVersion=$([string] (Get-Member2 $values 'DisplayVersion'))"
    $pathValue = Get-PathEntries -Evidence $installed
    foreach ($entry in $expectedPath) {
        $expanded = $(if ($null -ne $installRootExpanded -and $entry -eq $installRoot) { $installRootExpanded } else { $entry })
        Add-Check -Name "install/PATH holds $entry once" -Code 'self.install.path' `
            -Pass ($null -ne $pathValue -and @($pathValue | Where-Object { $_ -eq $expanded }).Count -eq 1) -Message ($pathValue -join ';')
    }
    $residue = @(Get-LoaderResidue -Evidence $installed)
    Add-Check -Name 'install/loader extraction cleaned' -Code 'self.install.loader.clean' -Pass ($residue.Count -eq 0) -Message ($residue -join ', ')

    Write-Host ''; Write-Host "### $Scope / uninstall"
    $uninstalled = Invoke-Step -Step 'uninstall' -Request ($reads + @{
            commands = @((New-SetupCommand -Name 'uninstall' -Arguments @('uninstall', '--quiet', '--scope', $Scope)), (New-LoaderResidueCommand))
            runAs = 'job'
            logs = @("$guestLogRoot\uninstall.log")
        })
    $null = Add-CommandCheck -Evidence $uninstalled -Step 'uninstall' -Name 'uninstall' -Outcome 'uninstalled'
    $log = $(if ($null -ne $uninstalled) { Get-Member2 $uninstalled 'logs' } else { $null })
    $logLines = @($(if ($null -ne $log) { @($log.PSObject.Properties | ForEach-Object { @($_.Value) }) } else { @() }) | ForEach-Object { [string] $_ })
    Add-Check -Name 'uninstall/the log records the state directory removal' -Code 'self.uninstall.log.state' `
        -Pass (@($logLines | Where-Object { $_ -like '*state_directory_removed*' }).Count -ge 1) -Message "$($logLines.Count) log line(s)"
    Add-ResidueChecks -Evidence $uninstalled -Step 'uninstall'
    $residue = @(Get-LoaderResidue -Evidence $uninstalled)
    Add-Check -Name 'uninstall/loader extraction cleaned' -Code 'self.uninstall.loader.clean' -Pass ($residue.Count -eq 0) -Message ($residue -join ', ')

    Write-TigerSetupRowResult -Row $Scope -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Scope.json") `
        -Environment $environment -Evidence @{ installer = $InstallerPath; package = $facts.id; version = $facts.version; engineSha256 = $facts.engineSha256; loaderSha256 = $facts.loaderSha256 }
}

# ---------------------------------------------------------------------------
# Presence, upgrade and WinGet rows
# ---------------------------------------------------------------------------

# The Start Menu entries the package declares: one folder, a shell and two
# forms of help, recognized by what they open. The names are what a person
# reads, so they are checked too: the PDF is the primary "TigerSetup Help".
$startMenuShortcuts = @($facts.shortcuts | Where-Object { $null -ne $_ -and [string] $_.location -eq 'start-menu' })
$shortcutNames = @{
    shell = [string] (@($startMenuShortcuts | Where-Object { [string] $_.target -eq 'tiger-setup.exe' -and [string] $_.arguments -eq 'shell' }) | Select-Object -First 1).name
    help = [string] (@($startMenuShortcuts | Where-Object { [string] $_.target -like '*.pdf' }) | Select-Object -First 1).name
    markdown = [string] (@($startMenuShortcuts | Where-Object { [string] $_.target -like '*.md' }) | Select-Object -First 1).name
}
$expectedShortcutNames = @('TigerSetup Shell', 'TigerSetup Help', 'TigerSetup Help (Markdown)')
$startMenuSubfolder = [string] (@($startMenuShortcuts) | Select-Object -First 1).folder

function Get-StartMenuFolder {
    param([string] $Scope)
    $base = $(if ($Scope -eq 'machine') { '%ProgramData%\Microsoft\Windows\Start Menu\Programs' } else { '%APPDATA%\Microsoft\Windows\Start Menu\Programs' })
    "$base\$startMenuSubfolder"
}

function Get-HelpChecks {
    <#
        The installed help is the shipped help, the shipped Markdown is the
        repository's docs\TigerSetup-Help.md, and the shipped PDF names that
        Markdown as the document it was rendered from.
    #>
    param([object] $Root, [string] $Step)
    $checks = [System.Collections.Generic.List[object]]::new()
    $installed = @{}
    if ($null -ne $Root -and $null -ne $Root.PSObject.Properties['hashes']) {
        foreach ($entry in @($Root.hashes)) { $installed[[string] $entry.path] = [string] $entry.sha256 }
    }
    foreach ($path in @('help/TigerSetup-Help.md', 'help/TigerSetup-Help.pdf')) {
        $expected = $(if ($shipped.ContainsKey($path)) { $shipped[$path].sha256 } else { $null })
        $actual = $(if ($installed.ContainsKey($path)) { $installed[$path] } else { $null })
        $checks.Add((New-TigerSetupCheck -Name "$Step/$path is the shipped file" -Code "self.$Step.help.shipped" `
                    -Status $(if ($null -ne $expected -and $actual -eq $expected) { 'PASS' } else { 'FAIL' }) -Message "installed $actual; shipped $expected"))
    }
    $markdown = $(if ($shipped.ContainsKey('help/TigerSetup-Help.md')) { $shipped['help/TigerSetup-Help.md'].sha256 } else { $null })
    $checks.Add((New-TigerSetupCheck -Name "$Step/the Markdown help is docs\TigerSetup-Help.md" -Code "self.$Step.help.source" `
                -Status $(if ($markdown -eq $helpSourceSha256) { 'PASS' } else { 'FAIL' }) -Message "shipped $markdown; source $helpSourceSha256"))
    $title = $(if ($shipped.ContainsKey('help/TigerSetup-Help.pdf')) { $shipped['help/TigerSetup-Help.pdf'].title } else { $null })
    $checks.Add((New-TigerSetupCheck -Name "$Step/the PDF help was rendered from TigerSetup-Help.md" -Code "self.$Step.help.rendered" `
                -Status $(if ($title -eq 'TigerSetup-Help.md') { 'PASS' } else { 'FAIL' }) -Message "PDF title: $title"))
    $checks.ToArray()
}

function Get-ShortcutChecks {
    <#
        The Start Menu entries are exactly TigerSetup Shell, TigerSetup Help
        (the PDF) and TigerSetup Help (Markdown); each exists and opens what it
        declares; only the shell wears TigerSetup's icon, and each help shows
        its document's own icon — its icon location is empty or the document.
    #>
    param([object] $Evidence, [string] $Step, [string] $Folder, [string] $InstallRootExpanded)
    $checks = [System.Collections.Generic.List[object]]::new()
    $declared = @($startMenuShortcuts | ForEach-Object { [string] $_.name })
    $roles = @($shortcutNames.shell, $shortcutNames.help, $shortcutNames.markdown)
    $checks.Add((New-TigerSetupCheck -Name "$Step/Start Menu entries: $($expectedShortcutNames -join ', ')" -Code "self.$Step.shortcut.names" `
                -Status $(if ($declared.Count -eq 3 -and (@(Compare-Object $roles $expectedShortcutNames -SyncWindow 0)).Count -eq 0) { 'PASS' } else { 'FAIL' }) `
                -Message "shell='$($roles[0])' pdf='$($roles[1])' markdown='$($roles[2])'; declared: $($declared -join ', ')"))
    $records = @(@(Get-Member2 $Evidence 'shortcuts') | Where-Object { $null -ne $_ })
    $exe = $InstallRootExpanded + '\tiger-setup.exe'
    foreach ($shortcut in $startMenuShortcuts) {
        $requested = "$Folder\$($shortcut.name).lnk"
        $record = @($records | Where-Object { [string] $_.requested -eq $requested }) | Select-Object -First 1
        $expectedTarget = $InstallRootExpanded + '\' + ([string] $shortcut.target).Replace('/', '\')
        $ok = $null -ne $record -and [bool] $record.exists -and [string] $record.target -ieq $expectedTarget -and [string] $record.arguments -eq [string] $shortcut.arguments
        $checks.Add((New-TigerSetupCheck -Name "$Step/Start Menu: $($shortcut.name)" -Code "self.$Step.shortcut" -Status $(if ($ok) { 'PASS' } else { 'FAIL' }) `
                    -Message $(if ($null -ne $record) { "exists=$($record.exists) target=$($record.target) arguments=$($record.arguments) icon=$($record.icon)" } else { "no record of $requested" })))
        if ($null -ne $record -and [bool] $record.exists) {
            $iconFile = ([string] $record.icon -replace ',-?\d+$', '').Trim()
            $isShell = [string] $shortcut.name -eq $shortcutNames.shell
            $iconOk = $(if ($isShell) { $iconFile -ieq $exe -or ($iconFile -eq '' -and $expectedTarget -ieq $exe) } else { $iconFile -eq '' -or $iconFile -ieq $expectedTarget })
            $checks.Add((New-TigerSetupCheck -Name "$Step/$($shortcut.name): $(if ($isShell) { "TigerSetup's icon" } else { "its document's icon, not TigerSetup's" })" -Code "self.$Step.shortcut.icon" `
                        -Status $(if ($iconOk) { 'PASS' } else { 'FAIL' }) -Message "icon location '$($record.icon)'"))
        }
    }
    $checks.ToArray()
}

function Get-FolderExactCheck {
    <# The Start Menu folder holds the declared shortcuts and nothing else. #>
    param([object] $Evidence, [string] $Step, [string] $Folder)
    $record = Get-Inventory -Evidence $Evidence -Requested $Folder
    $expected = @($startMenuShortcuts | ForEach-Object { "$($_.name).lnk" } | Sort-Object)
    $actual = @($(if ($null -ne $record) { @($record.files) }) | Sort-Object)
    New-TigerSetupCheck -Name "$Step/the Start Menu folder holds exactly the shortcuts" -Code "self.$Step.folder" `
        -Status $(if ($null -ne $record -and [bool] $record.exists -and (@(Compare-Object $expected $actual)).Count -eq 0) { 'PASS' } else { 'FAIL' }) `
        -Message "$Folder files=$($actual -join ', ')"
}

function Get-AbsenceChecks {
    <# After an uninstall: no install root, no state, no registration, no Start Menu folder, no shortcut. #>
    param([object] $Evidence, [string] $Step, [string] $InstallRoot, [string] $StateDir, [string] $RegistrationKey, [string] $Folder)
    $checks = [System.Collections.Generic.List[object]]::new()
    foreach ($pair in @(@('install root', $InstallRoot), @('state directory', $StateDir), @('Start Menu folder', $Folder))) {
        $record = Get-Inventory -Evidence $Evidence -Requested $pair[1]
        $checks.Add((New-TigerSetupCheck -Name "$Step/$($pair[0]) gone" -Code "self.$Step.absent" -Status $(if ($null -ne $record -and -not [bool] $record.exists) { 'PASS' } else { 'FAIL' }) `
                    -Message "$($pair[1]) exists=$(if ($null -ne $record) { $record.exists } else { 'unread' })"))
    }
    $registration = Get-RegistryRecord -Evidence $Evidence -Key $RegistrationKey
    $checks.Add((New-TigerSetupCheck -Name "$Step/registration gone" -Code "self.$Step.registration.absent" -Status $(if ($null -ne $registration -and -not [bool] $registration.exists) { 'PASS' } else { 'FAIL' }) -Message $RegistrationKey))
    $stale = @(@(Get-Member2 $Evidence 'shortcuts') | Where-Object { $null -ne $_ -and [bool] $_.exists })
    $checks.Add((New-TigerSetupCheck -Name "$Step/no Start Menu entry left" -Code "self.$Step.shortcut.absent" -Status $(if ($stale.Count -eq 0) { 'PASS' } else { 'FAIL' }) `
                -Message $(if ($stale.Count -gt 0) { "still there: $(@($stale | ForEach-Object { $_.path }) -join ', ')" } else { 'none' })))
    $checks.ToArray()
}

function Invoke-ChainedStep {
    param([string] $Row, [string] $Step, [hashtable] $Request, [string[]] $PayloadFiles = @(), [switch] $FromBaseline, [System.Collections.Generic.List[object]] $Checks)
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline
    $run = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $Request -PayloadFiles $PayloadFiles `
        -Name "ts-self-$Row-$Step" @policy -ResultPath (Join-Path $ResultsRoot "runs\$Row-$Step.json") -OutputRoot $labOutputRoot `
        -TimeoutMinutes $JobTimeoutMinutes
    $Checks.Add((New-TigerSetupCheck -Name "$Step/lab job" -Code "self.$Step.lab" -Status $(if ($run.status -eq 'OK') { 'PASS' } else { 'FAIL' }) -Message "status $($run.status) (exit $($run.exitCode)) after $($run.durationSeconds)s"))
    [pscustomobject]@{ run = $run; evidence = (Get-Evidence $run) }
}

function Add-SetupOutcomeCheck {
    param([System.Collections.Generic.List[object]] $Checks, [object] $Evidence, [string] $Step, [string] $Name, [string] $Outcome)
    $command = Get-Command2 -Evidence $Evidence -Name $Name
    $exit = $(if ($null -ne $command) { Get-Member2 $command 'exitCode' } else { $null })
    $document = $(if ($null -ne $command) { Get-Member2 $command 'json' } else { $null })
    $actual = [string] (Get-Member2 $document 'outcome')
    $Checks.Add((New-TigerSetupCheck -Name "$Step/$Name $Outcome" -Code "self.$Step.$Name" -Status $(if ($exit -eq 0 -and $actual -eq $Outcome) { 'PASS' } else { 'FAIL' }) `
                -Message "exit $exit outcome=$actual code=$([string] (Get-Member2 $document 'code')) $(if ($null -ne $command) { $command.stderr })"))
    $document
}

function New-LogDirectoryCommands {
    <# The log directory, made by the job account and writable by the signed-in user. #>
    @(
        @{ name = 'mkdir'; executable = 'cmd.exe'; arguments = @('/c', 'mkdir', $guestLogRoot); timeoutSeconds = 30; runAs = 'job' },
        @{ name = 'grant'; executable = 'icacls.exe'; arguments = @($guestLogRoot, '/grant', '*S-1-5-32-545:(OI)(CI)M'); timeoutSeconds = 30; runAs = 'job' }
    )
}

function Invoke-PresenceRow {
    <# user-nopath and machine-nopath: install with PATH off, the desktop, uninstall. #>
    param([string] $Row, [string] $Scope)
    $checks = [System.Collections.Generic.List[object]]::new()
    $runAs = $(if ($Scope -eq 'user') { 'interactiveUser' } else { 'job' })
    $installRoot = Get-TigerSetupInstallRoot -Facts $facts -Scope $Scope
    $stateDir = $(if ($Scope -eq 'machine') { "%ProgramData%\TigerSetup\$($facts.id)" } else { "%LOCALAPPDATA%\TigerSetup\$($facts.id)" })
    $registrationKey = $(if ($Scope -eq 'machine') { 'HKLM' } else { 'HKCU' }) + "\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($facts.registrationKey)"
    $folder = Get-StartMenuFolder -Scope $Scope
    $links = @($startMenuShortcuts | ForEach-Object { "$folder\$($_.name).lnk" })
    $reads = @{ inventory = @($installRoot, $stateDir, $folder); registry = @($registrationKey); pathValues = $true; shortcuts = $links; runAs = $runAs }

    Write-Host ''; Write-Host "### $Row / install"
    $install = Invoke-ChainedStep -Row $Row -Step 'install' -FromBaseline -PayloadFiles @($InstallerPath) -Checks $checks -Request ($reads + @{
            stage = @(@{ source = [System.IO.Path]::GetFileName($InstallerPath); destination = $guestInstaller })
            commands = @(New-LogDirectoryCommands) + @(
                (New-SetupCommand -Name 'install' -Arguments @('install', '--quiet', '--scope', $Scope, '--option', 'path', 'off')),
                (New-SetupCommand -Name 'verify' -Arguments @('verify', '--scope', $Scope)),
                (New-LoaderResidueCommand))
            inventoryHashes = $true
        })
    $evidence = $install.evidence
    $environment = $(if ($null -ne $evidence) { Get-Member2 $evidence 'environment' } else { $null })
    $null = Add-SetupOutcomeCheck -Checks $checks -Evidence $evidence -Step 'install' -Name 'install' -Outcome 'installed'
    $verify = Get-Member2 (Get-Command2 -Evidence $evidence -Name 'verify') 'json'
    $checks.Add((New-TigerSetupCheck -Name 'install/verify ok' -Code 'self.install.verify' -Status $(if ([string] (Get-Member2 $verify 'status') -eq 'ok') { 'PASS' } else { 'FAIL' }) -Message ((Get-Codes $verify) -join ', ')))
    $root = Get-Inventory -Evidence $evidence -Requested $installRoot
    $rootExpanded = $(if ($null -ne $root) { ([string] $root.path).TrimEnd('\') } else { $installRoot })
    foreach ($check in Get-HelpChecks -Root $root -Step 'install') { $checks.Add($check) }
    foreach ($check in Get-ShortcutChecks -Evidence $evidence -Step 'install' -Folder $folder -InstallRootExpanded $rootExpanded) { $checks.Add($check) }
    $checks.Add((Get-FolderExactCheck -Evidence $evidence -Step 'install' -Folder $folder))
    $pathValues = Get-Member2 $evidence 'pathValues'
    $holders = @(foreach ($which in @('machine', 'user')) {
            $block = Get-Member2 $pathValues $which
            if (@(@(Get-Member2 $block 'entries') | Where-Object { ([string] $_).TrimEnd('\') -ieq $rootExpanded }).Count -gt 0) { $which }
        })
    $checks.Add((New-TigerSetupCheck -Name 'install/PATH option off: no PATH entry' -Code 'self.install.path.off' -Status $(if ($null -ne $pathValues -and $holders.Count -eq 0) { 'PASS' } else { 'FAIL' }) `
                -Message $(if ($holders.Count -gt 0) { "the $($holders -join ' and ') PATH holds $rootExpanded" } else { 'neither PATH holds the install root' })))

    Write-Host ''; Write-Host "### $Row / desktop"
    $policy = Get-TigerSetupRowStepPolicy
    $desktop = Invoke-TigerSetupPresenceAcceptance -LabRoot $labRoot -Baseline $Baseline @policy -Name "ts-self-$Row-desktop" `
        -Request @{ version = $facts.version; scope = $Scope; installRoot = $installRoot; startMenuFolder = $folder; shortcuts = $shortcutNames; launch = 'link'; pathOption = $false } `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-desktop.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'desktop' -LabRun $desktop) { $checks.Add($check) }

    Write-Host ''; Write-Host "### $Row / uninstall"
    $uninstall = Invoke-ChainedStep -Row $Row -Step 'uninstall' -Checks $checks -Request ($reads + @{
            commands = @((New-SetupCommand -Name 'uninstall' -Arguments @('uninstall', '--quiet', '--scope', $Scope)), (New-LoaderResidueCommand))
        })
    $null = Add-SetupOutcomeCheck -Checks $checks -Evidence $uninstall.evidence -Step 'uninstall' -Name 'uninstall' -Outcome 'uninstalled'
    foreach ($check in Get-AbsenceChecks -Evidence $uninstall.evidence -Step 'uninstall' -InstallRoot $installRoot -StateDir $stateDir -RegistrationKey $registrationKey -Folder $folder) { $checks.Add($check) }
    $residue = @(Get-LoaderResidue -Evidence $uninstall.evidence)
    $checks.Add((New-TigerSetupCheck -Name 'uninstall/loader extraction cleaned' -Code 'self.uninstall.loader.clean' -Status $(if ($residue.Count -eq 0) { 'PASS' } else { 'FAIL' }) -Message ($residue -join ', ')))

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $environment `
        -Evidence @{ installer = $InstallerPath; package = $facts.id; version = $facts.version; engineSha256 = $facts.engineSha256; desktop = $desktop.resultPath }
}

function Invoke-UpgradeRow {
    <# The previous release, then this one over it, then this one again, then the uninstall. #>
    param([string] $Row)
    $checks = [System.Collections.Generic.List[object]]::new()
    $Scope = 'user'
    $installRoot = Get-TigerSetupInstallRoot -Facts $facts -Scope $Scope
    $stateDir = "%LOCALAPPDATA%\TigerSetup\$($facts.id)"
    $registrationKey = "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($facts.registrationKey)"
    $folder = Get-StartMenuFolder -Scope $Scope
    $previous = Join-Path $GuestStageRoot ([System.IO.Path]::GetFileName($PreviousInstallerPath))
    $previousFacts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $PreviousInstallerPath
    $previousVersion = [string] $previousFacts.version
    # The previous release's Start Menu entries, in this release's folder: a
    # release without them proves the upgrade adds the folder, one with a
    # different layout proves the upgrade renames, retargets and retires links.
    $previousLinks = @(@($previousFacts.shortcuts) | Where-Object { $null -ne $_ -and [string] $_.location -eq 'start-menu' -and [string] $_.folder -eq $startMenuSubfolder } | ForEach-Object { "$folder\$($_.name).lnk" })
    $links = @($startMenuShortcuts | ForEach-Object { "$folder\$($_.name).lnk" })
    $retired = @($previousLinks | Where-Object { $links -notcontains $_ })
    $reads = @{ inventory = @($installRoot, $stateDir, $folder); registry = @($registrationKey); pathValues = $true; shortcuts = @($links + $retired); runAs = 'interactiveUser'; inventoryHashes = $true }

    Write-Host ''; Write-Host "### $Row / previous $previousVersion"
    $first = Invoke-ChainedStep -Row $Row -Step 'previous' -FromBaseline -PayloadFiles @($PreviousInstallerPath, $InstallerPath) -Checks $checks -Request ($reads + @{
            stage = @(
                @{ source = [System.IO.Path]::GetFileName($PreviousInstallerPath); destination = $previous },
                @{ source = [System.IO.Path]::GetFileName($InstallerPath); destination = $guestInstaller })
            commands = @(New-LogDirectoryCommands) + @(@{ name = 'install'; executable = $previous; arguments = @('install', '--quiet', '--scope', $Scope, '--option', 'path', 'off', '--json', '--log', "$guestLogRoot\previous.log"); timeoutSeconds = 600 })
        })
    $null = Add-SetupOutcomeCheck -Checks $checks -Evidence $first.evidence -Step 'previous' -Name 'install' -Outcome 'installed'
    $environment = $(if ($null -ne $first.evidence) { Get-Member2 $first.evidence 'environment' } else { $null })
    $folderBefore = Get-Inventory -Evidence $first.evidence -Requested $folder
    $before = @($(if ($null -ne $folderBefore -and [bool] $folderBefore.exists) { @($folderBefore.files) }) | Sort-Object)
    $expectedBefore = @($previousLinks | ForEach-Object { Split-Path -Leaf $_ } | Sort-Object)
    $checks.Add((New-TigerSetupCheck -Name "previous/$previousVersion Start Menu: $(if ($expectedBefore.Count -gt 0) { $expectedBefore -join ', ' } else { 'no folder' })" -Code 'self.previous.folder' `
                -Status $(if ((@(Compare-Object $expectedBefore $before)).Count -eq 0 -and ($expectedBefore.Count -gt 0 -or $null -eq $folderBefore -or -not [bool] $folderBefore.exists)) { 'PASS' } else { 'FAIL' }) `
                -Message "the premise; found: $(if ($before.Count -gt 0) { $before -join ', ' } else { 'no folder' }); retired by this release: $(@($retired | ForEach-Object { Split-Path -Leaf $_ }) -join ', ')"))

    # A same-version rerun with nothing to change is a no-op by design
    # (`already_installed`). A previous build of this same version is
    # therefore reinstalled with the recorded PATH choice stated, which makes
    # the run reconcile the installation with this build.
    $reconcile = $(if ($previousVersion -eq $facts.version) { @('--option', 'path', 'off') } else { @() })
    foreach ($step in @('upgrade', 'again')) {
        Write-Host ''; Write-Host "### $Row / $step"
        $stepArguments = @('install', '--quiet', '--scope', $Scope) + $(if ($step -eq 'upgrade') { $reconcile } else { @() })
        $run = Invoke-ChainedStep -Row $Row -Step $step -Checks $checks -Request ($reads + @{
                commands = @(
                    (New-SetupCommand -Name 'install' -Arguments $stepArguments),
                    (New-SetupCommand -Name 'verify' -Arguments @('verify', '--scope', $Scope)),
                    (New-SetupCommand -Name 'inspect' -Arguments @('inspect', '--scope', $Scope)))
            })
        $null = Add-SetupOutcomeCheck -Checks $checks -Evidence $run.evidence -Step $step -Name 'install' -Outcome 'installed'
        $inspect = Get-Member2 (Get-Command2 -Evidence $run.evidence -Name 'inspect') 'json'
        $installed = Get-Member2 $inspect 'installation'
        $checks.Add((New-TigerSetupCheck -Name "$step/installed version $($facts.version)" -Code "self.$step.version" -Status $(if ([string] (Get-Member2 $installed 'version') -eq $facts.version) { 'PASS' } else { 'FAIL' }) -Message "version=$([string] (Get-Member2 $installed 'version'))"))
        $verify = Get-Member2 (Get-Command2 -Evidence $run.evidence -Name 'verify') 'json'
        $checks.Add((New-TigerSetupCheck -Name "$step/verify ok" -Code "self.$step.verify" -Status $(if ([string] (Get-Member2 $verify 'status') -eq 'ok') { 'PASS' } else { 'FAIL' }) -Message ((Get-Codes $verify) -join ', ')))
        $root = Get-Inventory -Evidence $run.evidence -Requested $installRoot
        $rootExpanded = $(if ($null -ne $root) { ([string] $root.path).TrimEnd('\') } else { $installRoot })
        foreach ($check in Get-HelpChecks -Root $root -Step $step) { $checks.Add($check) }
        foreach ($check in Get-ShortcutChecks -Evidence $run.evidence -Step $step -Folder $folder -InstallRootExpanded $rootExpanded) { $checks.Add($check) }
        $checks.Add((Get-FolderExactCheck -Evidence $run.evidence -Step $step -Folder $folder))
        foreach ($link in $retired) {
            $record = @(@(Get-Member2 $run.evidence 'shortcuts') | Where-Object { $null -ne $_ -and [string] $_.requested -eq $link }) | Select-Object -First 1
            $checks.Add((New-TigerSetupCheck -Name "$step/$previousVersion's $(Split-Path -Leaf $link) is gone" -Code "self.$step.shortcut.retired" `
                        -Status $(if ($null -ne $record -and -not [bool] $record.exists) { 'PASS' } else { 'FAIL' }) -Message "exists=$(if ($null -ne $record) { $record.exists } else { 'unread' })"))
        }
        # The recorded choice survives: PATH stayed off through the upgrade.
        $pathValues = Get-Member2 $run.evidence 'pathValues'
        $held = @(@(Get-Member2 (Get-Member2 $pathValues 'user') 'entries') | Where-Object { ([string] $_).TrimEnd('\') -ieq $rootExpanded }).Count -gt 0
        $checks.Add((New-TigerSetupCheck -Name "$step/the PATH option stays off" -Code "self.$step.path.off" -Status $(if ($null -ne $pathValues -and -not $held) { 'PASS' } else { 'FAIL' }) -Message "user PATH holds the root: $held"))
    }

    Write-Host ''; Write-Host "### $Row / uninstall"
    $uninstall = Invoke-ChainedStep -Row $Row -Step 'uninstall' -Checks $checks -Request ($reads + @{
            commands = @((New-SetupCommand -Name 'uninstall' -Arguments @('uninstall', '--quiet', '--scope', $Scope)))
        })
    $null = Add-SetupOutcomeCheck -Checks $checks -Evidence $uninstall.evidence -Step 'uninstall' -Name 'uninstall' -Outcome 'uninstalled'
    foreach ($check in Get-AbsenceChecks -Evidence $uninstall.evidence -Step 'uninstall' -InstallRoot $installRoot -StateDir $stateDir -RegistrationKey $registrationKey -Folder $folder) { $checks.Add($check) }

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $environment `
        -Evidence @{ installer = $InstallerPath; previous = $PreviousInstallerPath; previousVersion = $previousVersion; version = $facts.version }
}

function Get-ManifestInstallerUrl {
    $installerManifest = @(Get-ChildItem -LiteralPath $ManifestDirectory -Filter '*.installer.yaml')[0].FullName
    $line = @(Get-Content -LiteralPath $installerManifest | Where-Object { $_ -match '^\s*-?\s*InstallerUrl:\s*(\S+)' }) | Select-Object -First 1
    if ($null -eq $line -or $line -notmatch 'InstallerUrl:\s*(\S+)') { throw "$installerManifest names no InstallerUrl." }
    $Matches[1].Trim("'")
}

function Invoke-WinGetScenarioRow {
    <# winget-user and winget-machine: TigerWinLab's WinGet scenario on one installer entry. #>
    param([string] $Row, [string] $Scope)
    $checks = [System.Collections.Generic.List[object]]::new()
    $specPath = New-TigerSetupWinGetSpec -Name "ts-self-$Row" -Facts $facts -InstallerPath $InstallerPath -ManifestDirectory $ManifestDirectory `
        -ExpectedUrl (Get-ManifestInstallerUrl) -Identifier $facts.id -Scope $Scope `
        -ExpectedFiles @($facts.files | ForEach-Object { ([string] $_).Replace('/', '\') }) -MinimumFileCount @($facts.files).Count `
        -Commands @('tiger-setup') -Smoke @([ordered]@{ name = 'version'; command = 'tiger-setup'; arguments = @('--version'); expectedExitCode = 0; expectedOutputPattern = [regex]::Escape($facts.version) }) `
        -OutputPath (Join-Path $ResultsRoot "specs\$Row-winget.json")
    Write-Host ''; Write-Host "### $Row / winget scenario"
    $scenario = Invoke-TigerWinLabEntryPoint -LabRoot $labRoot -EntryPoint 'Invoke-TigerWinLabWinGetScenario.ps1' `
        -Parameters (@{ SpecPath = $specPath; Baseline = $Baseline } + (Get-TigerSetupRowStepPolicy -FromBaseline)) `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-winget.json") -OutputRoot $labOutputRoot -TimeoutMinutes 45
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'winget' -LabRun $scenario) { $checks.Add($check) }
    $environment = $(if ($null -ne $scenario.result -and $null -ne $scenario.result.PSObject.Properties['environment']) { $scenario.result.environment } else { $null })
    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $environment `
        -Evidence @{ installer = $InstallerPath; manifests = $ManifestDirectory; scenario = $scenario.resultPath }
}

function Invoke-ModeratorRow {
    <# What a WinGet moderator does, as the signed-in standard user, from a clean machine. #>
    param([string] $Row)
    $checks = [System.Collections.Generic.List[object]]::new()
    $Scope = 'user'
    $installRoot = Get-TigerSetupInstallRoot -Facts $facts -Scope $Scope
    Write-Host ''; Write-Host "### $Row / desktop"
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline
    $desktop = Invoke-TigerSetupPresenceAcceptance -LabRoot $labRoot -Baseline $Baseline @policy -Name "ts-self-$Row" `
        -Request @{ version = $facts.version; scope = $Scope; installRoot = $installRoot; startMenuFolder = (Get-StartMenuFolder -Scope $Scope); shortcuts = $shortcutNames; launch = 'start-menu'; pathOption = (Test-TigerSetupOptionEnabled -Facts $facts -Option 'path' -Options @{}) } `
        -WinGet @{ InstallerPath = $InstallerPath; ManifestDirectory = $ManifestDirectory; Identifier = $facts.id; Name = $facts.name; ProductCode = $facts.registrationKey; Scope = $Scope; Uninstall = $true } `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-desktop.json") -OutputRoot $labOutputRoot -TimeoutMinutes 30
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'moderator' -LabRun $desktop) { $checks.Add($check) }
    $environment = $(if ($null -ne $desktop.result -and $null -ne $desktop.result.PSObject.Properties['environment']) { $desktop.result.environment } else { $null })
    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $environment `
        -Evidence @{ installer = $InstallerPath; manifests = $ManifestDirectory; desktop = $desktop.resultPath }
}

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-self-installer-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
Write-Host "Installer $([System.IO.Path]::GetFileName($InstallerPath)) ($($facts.id) $($facts.version), engine $($facts.engineSha256.Substring(0, 16))..., loader $($facts.loaderSha256.Substring(0, 16))...)"
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description "TigerSetup self-installer rows: $($Rows -join ', ')" `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$results = [System.Collections.Generic.List[object]]::new()
try {
    foreach ($row in $Rows) {
        $started = [DateTimeOffset]::Now
        try {
            $result = switch ($row) {
                'user-nopath' { Invoke-PresenceRow -Row $row -Scope 'user' }
                'machine-nopath' { Invoke-PresenceRow -Row $row -Scope 'machine' }
                'upgrade' { Invoke-UpgradeRow -Row $row }
                'winget-user' { Invoke-WinGetScenarioRow -Row $row -Scope 'user' }
                'winget-machine' { Invoke-WinGetScenarioRow -Row $row -Scope 'machine' }
                'moderator' { Invoke-ModeratorRow -Row $row }
                default { Invoke-ScopeRow -Scope $row }
            }
        }
        catch {
            $result = [pscustomobject]@{ row = $row; status = 'ERROR'; pass = 0; warn = 0; fail = 1; message = $_.Exception.Message; statement = [string] $_.InvocationInfo.PositionMessage; stack = [string] $_.ScriptStackTrace }
            Write-Host "ERROR $row : $($_.Exception.Message)"
        }
        $minutes = [Math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1)
        # A finished row carries its counts nested; a row that threw carries them flat.
        $counts = $(if ($null -ne $result.PSObject.Properties['counts']) { $result.counts } else { $result })
        $results.Add([pscustomobject]@{
                row = $row; status = $result.status; pass = $counts.pass; warn = $counts.warn; fail = $counts.fail; minutes = $minutes
                message = $(if ($null -ne $result.PSObject.Properties['message']) { $result.message } else { $null })
                statement = $(if ($null -ne $result.PSObject.Properties['statement']) { $result.statement } else { $null })
                stack = $(if ($null -ne $result.PSObject.Properties['stack']) { $result.stack } else { $null })
            })
    }
}
finally {
    $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId -ResultPath (Join-Path $ResultsRoot 'session-close.json')
}

$results.ToArray() | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $ResultsRoot 'summary.json') -Encoding utf8
Write-Host ''
foreach ($entry in $results) { Write-Host ("{0,-15} {1,-6} pass={2} warn={3} fail={4} {5} min" -f $entry.row, $entry.status, $entry.pass, $entry.warn, $entry.fail, $entry.minutes) }
if (@($results | Where-Object { $_.status -in @('FAIL', 'ERROR') }).Count -gt 0) { exit 1 }
exit 0
