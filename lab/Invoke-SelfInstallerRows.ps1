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

    .EXAMPLE
    pwsh -File lab\Invoke-SelfInstallerRows.ps1 -InstallerPath artifacts\tigersetup\TigerSetup-0.11.0-Setup.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [string] $BuilderPath,
    # The scopes to run, each a row of its own from the baseline.
    [string[]] $Rows = @('user', 'machine'),
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
foreach ($row in $Rows) {
    if ($row -notin @('user', 'machine')) { throw "Unknown row '$row'; the rows are user and machine." }
    if ($row -notin @($facts.scopes)) { throw "'$InstallerPath' allows scopes $($facts.scopes -join ', '); it has no '$row' scope." }
}

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
    "Get-ChildItem -LiteralPath (Join-Path `$env:SystemRoot 'Temp') -Filter 'TigerSetup-*' -Force -ErrorAction SilentlyContinue | ForEach-Object { `$_.FullName }"
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
        foreach ($file in @($facts.files)) {
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
            $result = Invoke-ScopeRow -Scope $row
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
foreach ($entry in $results) { Write-Host ("{0,-8} {1,-6} pass={2} warn={3} fail={4} {5} min" -f $entry.row, $entry.status, $entry.pass, $entry.warn, $entry.fail, $entry.minutes) }
if (@($results | Where-Object { $_.status -in @('FAIL', 'ERROR') }).Count -gt 0) { exit 1 }
exit 0
