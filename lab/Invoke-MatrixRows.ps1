#Requires -Version 7.0
<#
    .SYNOPSIS
    Runs the acceptance matrix of TigerSetup-Validation.md §5.2 for a
    generated installer against TigerWinLab.

    .DESCRIPTION
    Every row is one TigerWinLab invocation or a chain of them on one baseline:
    the installer scenario (silent lifecycle, and the wizard phases where the
    row is interactive), the recovery scenario, the WinGet scenario, and plain
    jobs for preparation, inspection and the rows the scenarios do not shape.
    Expectations are derived from the installer itself through tiger-setup
    inspect; what is product-specific (smoke commands, the settings file, the
    legacy installer, the vendor URLs that prepare a runtime) comes from the
    package's lab-matrix.json.

    Rows run in series; the lab has one lease. A missing or unreadable lab
    result is a failing check, never a pass. Results land under
    lab\results\<run>\<row>.json with a summary.json.

    .EXAMPLE
    pwsh -File lab\Invoke-MatrixRows.ps1 -InstallerPath artifacts\TigerMarkView\TigerMarkView-0.8.2-Setup.exe `
        -PreviousInstallerPath artifacts\TigerMarkView\TigerMarkView-0.8.1-Setup.exe `
        -LegacyInstallerPath C:\Projects\TigerMarkView\artifacts\installer\TigerMarkView-0.8.1-win-x64-setup.exe `
        -ManifestDirectory artifacts\TigerMarkView\winget `
        -Rows checkpoint
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [string] $PreviousInstallerPath,
    [string] $LegacyInstallerPath,
    [string] $ManifestDirectory,
    [string] $MatrixPath,
    [string[]] $Rows = @('all'),
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    [string] $BuilderPath,
    # Names this run's lab sessions. One session per baseline is derived from
    # it, because a session protects every VM it touches and the baselines are
    # alternatives rather than a fleet: see the session block below.
    [string] $SessionId,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    [int] $HoldSeconds = 90,
    [ValidateRange(1, 600)] [int] $InterruptAfterSeconds = 20,
    [int] $ScenarioTimeoutMinutes = 45,
    [int] $JobTimeoutMinutes = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

$LabRecoveryStageRoot = 'C:\TigerWinLab\recovery'
$Win11 = 'TigerWinLab-Win11-Clean'
$Win10 = 'TigerWinLab-Win10-Clean'
$Server = 'TigerWinLab-Server2019-Clean'

# Row → baseline. The rows are TigerSetup-Validation.md §5.2's; M16 is the
# legacy migration row and S6 the interactive dependency-acquisition row.
$RowTable = [ordered]@{
    'M1' = $Win11; 'M2' = $Win11; 'M3' = $Win11; 'M4' = $Win11; 'M5a' = $Win11; 'M5b' = $Win11; 'M5c' = $Win11
    'M6' = $Win11; 'M7' = $Win11; 'M8' = $Win11; 'M9' = $Win11; 'M10' = $Win11; 'M11' = $Win11; 'M12' = $Win11
    'M13' = $Win11; 'M14' = $Win11; 'M15' = $Win11; 'M16' = $Win11; 'M17' = $Win11
    'W1' = $Win10; 'W2' = $Win10; 'W3' = $Win10; 'W4' = $Win10; 'W5' = $Win10
    'S1' = $Server; 'S2' = $Server; 'S3' = $Server; 'S4' = $Server; 'S5' = $Server; 'S6' = $Server
}
$CheckpointRows = @('M1', 'M2', 'M5a', 'W1')

# Row → the dependency state its scenario starts from, which is the row's
# premise in §5.2 and is asserted against the lab's own measurement at that
# step's start (Complete-Row). What "clean" contains is a fact about the
# baseline on the day: Windows 11 holds WebView2 inbox and Windows 10 22H2
# has held it since its September 2026 servicing, so on both the clean state
# is WebView2 only and .NET prepared is both; Server 2019 holds neither, so
# there the clean state is neither and .NET prepared is .NET only — the two
# WebView2-absent states are Server 2019's alone. The table is the premises,
# the switch below is the steps; a row whose measured state is not its
# premise fails, because it has proven nothing about the state it names.
$WebView2Only = @{ dotnet = $false; webview2 = $true }
$Both = @{ dotnet = $true; webview2 = $true }
$Neither = @{ dotnet = $false; webview2 = $false }
$DotNetOnly = @{ dotnet = $true; webview2 = $false }
$RowPremise = [ordered]@{
    'M1' = $WebView2Only; 'M2' = $Both; 'M3' = $Both; 'M4' = $WebView2Only; 'M5a' = $Both; 'M5b' = $Both; 'M5c' = $Both
    'M6' = $Both; 'M7' = $WebView2Only; 'M8' = $Both; 'M9' = $Both; 'M10' = $WebView2Only; 'M11' = $WebView2Only; 'M12' = $Both
    'M13' = $Both; 'M14' = $Both; 'M15' = $Both; 'M16' = $Both; 'M17' = $Both
    'W1' = $WebView2Only; 'W2' = $Both; 'W3' = $WebView2Only; 'W4' = $WebView2Only; 'W5' = $Both
    'S1' = $Neither; 'S2' = $Neither; 'S3' = $Neither; 'S4' = $DotNetOnly; 'S5' = $DotNetOnly; 'S6' = $DotNetOnly
}
# A row without a premise, or a premise without a row, is a driver defect
# and is refused here rather than found after the guest time was spent.
$rowsWithoutPremise = @($RowTable.Keys | Where-Object { -not $RowPremise.Contains($_) })
$premisesWithoutRow = @($RowPremise.Keys | Where-Object { -not $RowTable.Contains($_) })
if ($rowsWithoutPremise.Count -gt 0 -or $premisesWithoutRow.Count -gt 0) {
    throw "The row table and the premise table disagree: rows without a premise: $($rowsWithoutPremise -join ', '); premises without a row: $($premisesWithoutRow -join ', ')."
}

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Rows -contains 'all') { $Rows = @($RowTable.Keys) }
if ($Rows -contains 'checkpoint') { $Rows = $CheckpointRows }
$unknown = @($Rows | Where-Object { -not $RowTable.Contains($_) })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($RowTable.Keys -join ', ')." }

# ---------------------------------------------------------------------------
# Inputs
# ---------------------------------------------------------------------------

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($BuilderPath)) { $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) { throw "The builder '$BuilderPath' does not exist." }
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
# A row measures the engine inside the installer. Refusing here costs a second;
# discovering it from the results costs the whole matrix.
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath
$previous = $null
if (-not [string]::IsNullOrWhiteSpace($PreviousInstallerPath)) {
    $PreviousInstallerPath = (Resolve-Path -LiteralPath $PreviousInstallerPath).Path
    $previous = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $PreviousInstallerPath
    Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $previous -InstallerPath $PreviousInstallerPath
}
if ([string]::IsNullOrWhiteSpace($MatrixPath)) { $MatrixPath = Join-Path $repoRoot "packages\$($facts.name)\lab-matrix.json" }
$matrix = Get-Content -LiteralPath $MatrixPath -Raw | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\matrix-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
$installerFile = Split-Path -Leaf $InstallerPath
$stagedInstaller = Join-Path $GuestStageRoot $installerFile
$stagedPrevious = if ($null -ne $previous) { Join-Path $GuestStageRoot (Split-Path -Leaf $PreviousInstallerPath) } else { $null }

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

$expected = $matrix.expected
$smoke = @($expected.smoke | ForEach-Object { [ordered]@{ name = $_.name; path = $_.path; arguments = @($_.arguments); expectedExitCode = [int] $_.expectedExitCode; expectedOutputPattern = $_.expectedOutputPattern } })
$expectedFiles = @($expected.files | ForEach-Object { [string] $_ })
$minimumFileCount = [int] $expected.minimumFileCount
$versionFile = [string] $expected.versionFile
$dependencyPrep = @{}
foreach ($property in $matrix.dependencies.PSObject.Properties) {
    $d = $property.Value
    $dependencyPrep[$property.Name] = @{
        id = $property.Name; url = [string] $d.url; file = [string] $d.file; arguments = @($d.arguments); successExitCodes = @($d.successExitCodes)
        detect = @{ kind = [string] $d.detect.kind; path = [string] (Get-Member2 $d.detect 'path'); pattern = [string] (Get-Member2 $d.detect 'pattern'); keys = @(Get-Member2 $d.detect 'keys'); value = [string] (Get-Member2 $d.detect 'value') }
        presentWhenPathExists = [string] $d.presentWhenPathExists
    }
}
$dotnetId = 'Microsoft.DotNet.DesktopRuntime.10'
$webviewId = 'Microsoft.EdgeWebView2Runtime'

Write-Host "TigerWinLab: $labRoot"
Write-Host "Installer:   $InstallerPath ($($facts.name) $($facts.version), key $($facts.registrationKey))"
if ($null -ne $previous) { Write-Host "Previous:    $PreviousInstallerPath ($($previous.version))" }
Write-Host "Results:     $ResultsRoot"

# ---------------------------------------------------------------------------
# Building blocks
# ---------------------------------------------------------------------------

function Invoke-Prepare {
    <# Starts the row from the baseline and installs the named runtimes with their vendors' installers. #>
    param([string] $Row, [string] $Baseline, [string[]] $Ids)
    $dependencies = @($Ids | ForEach-Object { $dependencyPrep[$_] })
    Write-Host "  prepare: from the baseline, install $($Ids -join ', ')"
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline
    Invoke-TigerSetupPrepareDependencies -LabRoot $labRoot -Baseline $Baseline -Dependencies $dependencies -Name "ts-prep-$Row" `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-prepare.json") -OutputRoot $labOutputRoot @policy -TimeoutMinutes 30
}

function Add-PrepareChecks {
    param([System.Collections.Generic.List[object]] $Checks, [object] $Run)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'prepare' -LabRun $Run) { $Checks.Add($check) }
    foreach ($dependency in @(Get-Member2 (Get-Member2 $Run.result 'result') 'dependencies')) {
        $present = $null -ne $dependency.after -and [bool] $dependency.after.present
        $Checks.Add((New-TigerSetupCheck -Name "prepare/$($dependency.id) present" -Code 'prepare.dependency.present' -Status $(if ($present) { 'PASS' } else { 'FAIL' }) `
                    -Message "$($dependency.id): $(if ($present) { "present ($($dependency.after.version))" } else { "absent: $($dependency.error)" })."))
    }
}

function Invoke-InstallerScenarioRow {
    param(
        [string] $Row, [string] $Baseline, [ValidateSet('user', 'machine')] [string] $Scope,
        [hashtable] $Options = @{}, [string[]] $ExtraInstallArguments = @(),
        [switch] $WithUpgrade, [hashtable] $Interactive, [string] $NetworkState = 'online',
        [string] $Language, [int] $ScalePercent = 0, [switch] $FromBaseline
    )
    $specArguments = @{
        Name = "ts-$Row".ToLowerInvariant(); Facts = $facts; InstallerPath = $InstallerPath; Scope = $Scope; Options = $Options
        ExtraInstallArguments = $ExtraInstallArguments; ExpectedFiles = $expectedFiles; MinimumFileCount = $minimumFileCount
        VersionFile = $versionFile; Smoke = $smoke; OutputPath = (Join-Path $ResultsRoot "specs\$Row-installer.json")
    }
    if ($WithUpgrade) {
        if ($null -eq $previous) { throw "Row $Row needs -PreviousInstallerPath." }
        $specArguments.UpgradeFromPath = $PreviousInstallerPath
        $specArguments.UpgradeFromVersion = $previous.version
    }
    if ($null -ne $Interactive) { $specArguments.Interactive = $Interactive }
    $specPath = New-TigerSetupInstallerSpec @specArguments
    $parameters = @{ SpecPath = $specPath; Baseline = $Baseline; NetworkState = $NetworkState } + (Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline)
    if (-not [string]::IsNullOrWhiteSpace($Language)) { $parameters.Language = $Language }
    if ($ScalePercent -gt 0) { $parameters.ScalePercent = $ScalePercent }
    Write-Host "  installer scenario: $Scope scope, $NetworkState$(if ($Interactive) { ', interactive' })$(if ($Language) { ", $Language" })$(if ($ScalePercent) { " @ $ScalePercent %" })"
    Invoke-TigerWinLabEntryPoint -LabRoot $labRoot -EntryPoint 'Invoke-TigerWinLabInstallerScenario.ps1' -Parameters $parameters `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-installer.json") -OutputRoot $labOutputRoot -TimeoutMinutes $ScenarioTimeoutMinutes
}

function Invoke-GuestJob {
    param(
        [string] $Row, [string] $Suffix, [string] $Baseline, [hashtable] $Request, [string[]] $PayloadFiles = @(),
        [switch] $FromBaseline, [string] $NetworkState, [string] $Language, [int] $ScalePercent = 0, [int] $TimeoutMinutes = $JobTimeoutMinutes
    )
    $arguments = @{
        LabRoot = $labRoot; Baseline = $Baseline; Request = $Request; PayloadFiles = $PayloadFiles; Name = "ts-$Row-$Suffix".ToLowerInvariant()
        ResultPath = (Join-Path $ResultsRoot "runs\$Row-$Suffix.json"); OutputRoot = $labOutputRoot; TimeoutMinutes = $TimeoutMinutes
    } + (Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline)
    if (-not [string]::IsNullOrWhiteSpace($NetworkState)) { $arguments.NetworkState = $NetworkState }
    if (-not [string]::IsNullOrWhiteSpace($Language)) { $arguments.Language = $Language }
    if ($ScalePercent -gt 0) { $arguments.ScalePercent = $ScalePercent }
    Write-Host "  job: $Suffix"
    Invoke-TigerSetupGuestCommands @arguments
}

function Get-JsonOf {
    param([object] $Run, [string] $CommandName)
    $command = Get-TigerSetupCommandResult -JobRun $Run -CommandName $CommandName
    if ($null -eq $command) { return $null }
    Get-Member2 $command 'json'
}

function Get-Inventory {
    param([object] $Run, [string] $Requested)
    $records = Get-Member2 (Get-Member2 $Run.result 'result') 'inventory'
    @($records | Where-Object { (Get-Member2 $_ 'requested') -eq $Requested }) | Select-Object -First 1
}

function Get-RegistryRecord {
    param([object] $Run, [string] $Requested)
    $records = Get-Member2 (Get-Member2 $Run.result 'result') 'registry'
    @($records | Where-Object { (Get-Member2 $_ 'requested') -eq $Requested }) | Select-Object -First 1
}

function Get-JobLog {
    param([object] $Run, [string] $Path)
    $record = Get-Member2 (Get-Member2 $Run.result 'result') 'logs'
    if ($null -eq $record) { return @() }
    $property = $record.PSObject.Properties | Where-Object { $_.Name -like "*$([System.IO.Path]::GetFileName($Path))" } | Select-Object -First 1
    if ($null -eq $property -or $null -eq $property.Value) { return @() }
    @($property.Value | ForEach-Object { [string] $_ })
}

function Test-LogHasCode {
    param([string[]] $Lines, [string] $Code)
    @($Lines | Where-Object { $_ -match ('\[' + [regex]::Escape($Code) + '\]') }).Count -gt 0
}

function Add-Check {
    param([System.Collections.Generic.List[object]] $Checks, [string] $Name, [string] $Code, [bool] $Condition, [string] $Message, [string] $Failure)
    $Checks.Add((New-TigerSetupCheck -Name $Name -Code $Code -Status $(if ($Condition) { 'PASS' } else { 'FAIL' }) -Message $(if ($Condition -or [string]::IsNullOrWhiteSpace($Failure)) { $Message } else { $Failure })))
}

function Add-EngineCommandCheck {
    <# One Setup.exe command's exit code and outcome code. #>
    param([System.Collections.Generic.List[object]] $Checks, [object] $Run, [string] $CommandName, [int[]] $ExitCodes, [string] $ExpectedCode, [string] $Prefix)
    $command = Get-TigerSetupCommandResult -JobRun $Run -CommandName $CommandName
    if ($null -eq $command) {
        $Checks.Add((New-TigerSetupCheck -Name "$Prefix/$CommandName ran" -Code "$Prefix.$CommandName.ran" -Status FAIL -Message "No record of the '$CommandName' command."))
        return $null
    }
    $json = Get-Member2 $command 'json'
    $code = [string] (Get-Member2 $json 'code')
    Add-Check $Checks "$Prefix/$CommandName exit" "$Prefix.$CommandName.exit" ($ExitCodes -contains $command.exitCode) "'$CommandName' exited $($command.exitCode) with code '$code'." "'$CommandName' exited $($command.exitCode) (expected $($ExitCodes -join ' or ')) with code '$code'. $($command.stderr)".Trim()
    if (-not [string]::IsNullOrWhiteSpace($ExpectedCode)) {
        Add-Check $Checks "$Prefix/$CommandName code" "$Prefix.$CommandName.code" ($code -eq $ExpectedCode) "The outcome code is '$code'." "The outcome code is '$code'; expected '$ExpectedCode'."
    }
    $json
}

function Get-StateDirectory {
    param([string] $Scope)
    if ($Scope -eq 'machine') { "%ProgramData%\TigerSetup\$($facts.id)" } else { "%LOCALAPPDATA%\TigerSetup\$($facts.id)" }
}

function Get-RegistrationPath {
    param([string] $Scope)
    "$(if ($Scope -eq 'machine') { 'HKLM' } else { 'HKCU' })\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$($facts.registrationKey)"
}

function Get-InstallRootOf {
    param([string] $Scope)
    Get-TigerSetupInstallRoot -Facts $facts -Scope $Scope
}

function Get-FaultSequence {
    <#
        The journal sequence of a large file's install operation, from the
        deterministic plan order: the root, then every directory shallowest
        first, then the files in the payload's stream order, which is the
        order `inspect` lists them in. A recovery row confirms it against the
        interrupted run's log.
    #>
    param([object] $Facts, [string] $PreferredFile)
    $directories = @($Facts.raw.directories)
    $files = @($Facts.files)
    $index = [Array]::IndexOf($files, $PreferredFile)
    if ($index -lt 0) {
        # The largest file the package declares.
        $largest = @($Facts.raw.files | Sort-Object -Property size -Descending | Select-Object -First 1)[0]
        $index = [Array]::IndexOf($files, [string] $largest.path)
    }
    [pscustomobject]@{ sequence = 1 + $directories.Count + $index + 1; target = $files[$index].Replace('/', '\') }
}

function New-GuestScriptCommand {
    <#
        A guest command that runs a PowerShell script, encoded so that it
        arrives intact whichever way the guest starts it. A command that runs
        in the signed-in user's session is started through Start-Process
        -ArgumentList, which joins the arguments with spaces and quotes
        nothing, so a -Command script would arrive re-split and re-parsed with
        its quoting eaten. -EncodedCommand is a single base64 token with
        neither space, quote nor newline in it; Windows PowerShell 5.1 in the
        guest decodes it as UTF-16LE.
    #>
    param([string] $Name, [string] $Script, [int] $TimeoutSeconds = 120)
    @{
        name = $Name
        executable = 'powershell.exe'
        arguments = @('-NoProfile', '-NonInteractive', '-EncodedCommand', [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($Script)))
        timeoutSeconds = $TimeoutSeconds
    }
}

function New-ReadRequest {
    <# verify/inspect plus inventories and registry reads for a scope. #>
    param([string] $Scope, [string] $Engine = $stagedInstaller, [string[]] $Logs = @(), [switch] $PathValues)
    @{
        commands = @(
            @{ name = 'verify'; executable = $Engine; arguments = @('verify', '--json', '--scope', $Scope); timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $Engine; arguments = @('inspect', '--json', '--scope', $Scope); timeoutSeconds = 300 }
        )
        logs = @($Logs)
        inventory = @((Get-InstallRootOf $Scope), (Get-StateDirectory $Scope))
        registry = @((Get-RegistrationPath $Scope))
        pathValues = [bool] $PathValues
    }
}

function Add-ReadChecks {
    <# Checks the verify/inspect documents of a read job for an installed version or for absence. #>
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $Run, [string[]] $ExpectedVersions, [string] $Scope)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix $Prefix -LabRun $Run) { $Checks.Add($check) }
    $verify = Get-JsonOf $Run 'verify'
    $inspect = Get-JsonOf $Run 'inspect'
    $installation = Get-Member2 $inspect 'installation'
    $version = [string] (Get-Member2 $installation 'version')
    $status = [string] (Get-Member2 $verify 'status')
    Add-Check $Checks "$Prefix/no open transaction" "$Prefix.transaction.closed" ($null -eq (Get-Member2 $inspect 'transaction')) 'No transaction is open.' "A transaction is still open: $((Get-Member2 $inspect 'transaction') | ConvertTo-Json -Compress)"
    $registration = Get-RegistryRecord $Run (Get-RegistrationPath $Scope)
    $registered = $null -ne $registration -and [bool] $registration.exists
    if ($ExpectedVersions.Count -eq 0) {
        Add-Check $Checks "$Prefix/verify reports absence" "$Prefix.verify.absent" ($status -eq 'not_installed') "verify status '$status'." "verify status '$status'; expected not_installed."
        Add-Check $Checks "$Prefix/registration absent" "$Prefix.registration.absent" (-not $registered) 'No Add/Remove Programs entry.' "The registration $($facts.registrationKey) still exists."
        $root = Get-Inventory $Run (Get-InstallRootOf $Scope)
        Add-Check $Checks "$Prefix/install root gone" "$Prefix.root.absent" ($null -ne $root -and -not [bool] $root.exists) 'The install root is gone.' "The install root still holds $(Get-Member2 $root 'fileCount') file(s)."
    }
    else {
        Add-Check $Checks "$Prefix/verify ok" "$Prefix.verify.ok" ($status -eq 'ok') "verify status '$status'." "verify status '$status'; findings: $(@(Get-Member2 $verify 'findings') | ConvertTo-Json -Compress -Depth 4)"
        Add-Check $Checks "$Prefix/installed version" "$Prefix.installation.version" ($ExpectedVersions -contains $version) "Installed version '$version'." "Installed version '$version'; expected one of $($ExpectedVersions -join ', ')."
        Add-Check $Checks "$Prefix/registration present" "$Prefix.registration.present" $registered "Registered as $($facts.registrationKey) (DisplayVersion $(Get-Member2 (Get-Member2 $registration 'values') 'DisplayVersion'))." "No registration $($facts.registrationKey)."
    }
    [pscustomobject]@{ verify = $verify; inspect = $inspect; version = $version }
}

function Complete-Row {
    param([string] $Row, [System.Collections.Generic.List[object]] $Checks, [object] $Environment, [hashtable] $Evidence = @{})
    # The environment is the one the row's scenario step reported at its
    # start, so it is the state the row's premise is about.
    $Checks.Add((Test-TigerSetupDependencyPremise -Environment $Environment -Premise $RowPremise[$Row]))
    Write-TigerSetupRowResult -Row $Row -Checks $Checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") -Environment $Environment -Evidence $Evidence
}

function Get-Env {
    param([object] $Run)
    Get-Member2 $Run.result 'environment'
}

function New-InteractiveSpec {
    <#
        The lab drives a wizard by clicking controls whose stripped label
        matches a pattern: "prefer" controls are clicked first on every page
        (a choice to settle), then the first matching "advance" control. The
        Launch check box is unchecked on the last page so no application is
        left running behind the row; a machine-scope wizard is answered by the
        lab's elevated session.
    #>
    param([string] $Language = 'en-US', [string[]] $Prefer = @(), [switch] $WithUpgrade)
    $polish = $Language -like 'pl*'
    $advance = if ($polish) { @('^Dalej$', '^Zainstaluj$', '^Zakończ$', '^Tak$', '^OK$') } else { @('^Next$', '^Install$', '^Finish$', '^Yes$', '^OK$') }
    $launch = if ($polish) { '^Uruchom' } else { '^Launch' }
    # The licence page starts on "I do not accept" and keeps the forward
    # button disabled until somebody accepts — deliberately, and the same as
    # the installer TigerSetup replaces. A driver that only knows how to
    # advance therefore waits on that page until the row's timeout, so
    # accepting is part of every interactive specification rather than
    # something each row remembers to ask for.
    $accept = if ($polish) { '^Akceptuję' } else { '^I accept' }
    $name = [regex]::Escape($facts.name)
    $installTitle = if ($polish) { "^Instalator — $name" } else { "^$name Setup" }
    $uninstallTitle = if ($polish) { "^Dezinstalator — $name" } else { "^$name Uninstall" }
    $spec = @{
        enabled = $true
        arguments = @('--lang', $Language)
        install = @{ windowTitlePattern = $installTitle; advanceButtons = $advance; preferControls = @($Prefer + @($accept, $launch)); maxSteps = 12 }
        uninstall = @{ windowTitlePattern = $uninstallTitle; advanceButtons = $advance; preferControls = @(); maxSteps = 6 }
    }
    # The upgrade wizard is the same wizard answered a second time, so it is
    # offered the same choices and needs the same answers. Leaving the row's own
    # preferences off it let the scope page default to "for me only", and an
    # elevated administrator answering the default installed a second, per-user
    # copy beside the machine one the row had just made.
    if ($WithUpgrade) { $spec.upgrade = @{ windowTitlePattern = $installTitle; advanceButtons = $advance; preferControls = @($Prefer + @($accept, $launch)); maxSteps = 12 } }
    $spec
}

# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------

function Invoke-SilentLifecycleRow {
    <# M1/M2/M3/W1/W2/W5/S1/S5: the installer scenario's silent lifecycle, with the dependency state the row names. #>
    param([string] $Row, [string] $Baseline, [string] $Scope, [string[]] $Prepare = @(), [string] $NetworkState = 'online', [switch] $WithUpgrade, [string[]] $PreservedDependencies = @())
    $checks = [System.Collections.Generic.List[object]]::new()
    $fromBaseline = $true
    if ($Prepare.Count -gt 0) {
        # Not $prepare: PowerShell variable names are case-insensitive, so that
        # would assign the run object to the [string[]] $Prepare parameter and
        # silently stringify it.
        $prepareRun = Invoke-Prepare -Row $Row -Baseline $Baseline -Ids $Prepare
        Add-PrepareChecks -Checks $checks -Run $prepareRun
        $fromBaseline = $false
    }
    $scenario = Invoke-InstallerScenarioRow -Row $Row -Baseline $Baseline -Scope $Scope -WithUpgrade:$WithUpgrade -NetworkState $NetworkState -FromBaseline:$fromBaseline
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'scenario' -LabRun $scenario) { $checks.Add($check) }
    $evidence = @{ results = @($scenario.resultPath) }
    if ($PreservedDependencies.Count -gt 0) {
        # A shared dependency the installer acquired is still present after the uninstall.
        $request = @{ commands = @(); inventory = @($PreservedDependencies | ForEach-Object { $dependencyPrep[$_].presentWhenPathExists }); registry = @($PreservedDependencies | ForEach-Object { $dependencyPrep[$_].detect.keys } | Where-Object { $_ }) }
        $after = Invoke-GuestJob -Row $Row -Suffix 'dependencies-after' -Baseline $Baseline -Request $request -NetworkState 'online'
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'after' -LabRun $after) { $checks.Add($check) }
        foreach ($id in $PreservedDependencies) {
            $inventory = Get-Inventory $after $dependencyPrep[$id].presentWhenPathExists
            $present = $null -ne $inventory -and [bool] $inventory.exists
            if (-not $present) {
                foreach ($key in @($dependencyPrep[$id].detect.keys)) { $record = Get-RegistryRecord $after $key; if ($null -ne $record -and [bool] $record.exists) { $present = $true } }
            }
            Add-Check $checks "after/$id preserved" 'after.dependency.preserved' $present "$id is still present after the uninstall." "$id is gone after the uninstall."
        }
        $evidence.results += $after.resultPath
    }
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $scenario) -Evidence $evidence
}

function Invoke-OfflineFailureRow {
    <# M4/W3/S2/S4: offline with a missing dependency, the silent install fails cleanly and leaves nothing. #>
    param([string] $Row, [string] $Baseline, [string[]] $Prepare = @())
    $checks = [System.Collections.Generic.List[object]]::new()
    $fromBaseline = $true
    if ($Prepare.Count -gt 0) { Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids $Prepare); $fromBaseline = $false }
    $log = Join-Path $GuestStageRoot 'offline-install.log'
    $request = @{
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(@{ name = 'install'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $log); timeoutSeconds = 900 })
        logs = @($log)
        inventory = @((Get-InstallRootOf 'machine'), (Get-StateDirectory 'machine'))
        registry = @((Get-RegistrationPath 'machine'))
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'offline' -Baseline $Baseline -Request $request -PayloadFiles @($InstallerPath) -FromBaseline:$fromBaseline -NetworkState 'offline'
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'offline' -LabRun $run) { $checks.Add($check) }
    $outcome = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'install' -ExitCodes @(3) -ExpectedCode 'dependency_unacquirable' -Prefix 'offline'
    $root = Get-Inventory $run (Get-InstallRootOf 'machine')
    $state = Get-Inventory $run (Get-StateDirectory 'machine')
    $registration = Get-RegistryRecord $run (Get-RegistrationPath 'machine')
    Add-Check $checks 'offline/no install root' 'offline.root.absent' ($null -ne $root -and -not [bool] $root.exists) 'No install root was created.' 'An install root exists.'
    # A clean failure claims nothing: no state database, no owned resources.
    # The run's own log is in the state directory by then, which is where a
    # failed install's diagnostics belong, so the directory itself may exist.
    $stateFiles = @(if ($null -ne $state -and [bool] $state.exists) { $state.files } else { @() })
    $claimed = @($stateFiles | Where-Object { $_ -notlike 'logs\*' })
    Add-Check $checks 'offline/nothing claimed' 'offline.state.absent' ($claimed.Count -eq 0) 'The state directory holds nothing but the log of the failed run.' "The failed run left $($claimed -join ', ') in the state directory." 
    Add-Check $checks 'offline/no registration' 'offline.registration.absent' ($null -eq $registration -or -not [bool] $registration.exists) 'Nothing was registered.' 'A registration exists.'
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $run) -Evidence @{ outcome = $outcome; log = (Get-JobLog $run $log); results = @($run.resultPath) }
}

function Invoke-RecoveryRow {
    <# M5a/M5b/M5c: an interruption during a machine-scope install or upgrade, then the engine's recovery and verify. #>
    param([string] $Row, [string] $Baseline, [string] $Method, [string] $Point, [switch] $Upgrade)
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $target = $facts
    $installerHostPath = $InstallerPath
    $expectedVersions = @($facts.version)
    if ($Upgrade) {
        if ($null -eq $previous) { throw "Row $Row needs -PreviousInstallerPath." }
        $installLog = Join-Path $GuestStageRoot 'install-previous.log'
        $request = @{
            stage = @(@{ source = (Split-Path -Leaf $PreviousInstallerPath); destination = $stagedPrevious })
            commands = @(@{ name = 'install'; executable = $stagedPrevious; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $installLog); timeoutSeconds = 900 })
            logs = @($installLog)
        }
        $baselineRun = Invoke-GuestJob -Row $Row -Suffix 'install-previous' -Baseline $Baseline -Request $request -PayloadFiles @($PreviousInstallerPath)
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'previous' -LabRun $baselineRun) { $checks.Add($check) }
        $null = Add-EngineCommandCheck -Checks $checks -Run $baselineRun -CommandName 'install' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'previous'
        $expectedVersions = @($previous.version, $facts.version)
    }
    $fault = Get-FaultSequence -Facts $target -PreferredFile 'libSkiaSharp.dll'
    $faultText = if ($Point -match '^(before_commit|after_commit_before_cleanup)$') { "${Point}:hold:$HoldSeconds" } else { "$Point@$($fault.sequence):hold:$HoldSeconds" }
    $specPath = New-TigerSetupRecoverySpec -Name "ts-$Row".ToLowerInvariant() -DisplayName $facts.name -InstallRoot (Get-InstallRootOf 'machine') `
        -InstallerPath $installerHostPath -InstallerArguments @('install', '--quiet', '--scope', 'machine', '--fault', $faultText) -Method $Method `
        -AfterSeconds $InterruptAfterSeconds -StatePaths @((Get-StateDirectory 'machine')) -RegistrationKeyName $facts.registrationKey `
        -RecoveryArguments @('install', '--quiet', '--scope', 'machine') -RecoveryTimeoutMinutes 15 -ExpectInstallRootExists $true `
        -OutputPath (Join-Path $ResultsRoot "specs\$Row-recovery.json")
    Write-Host "  recovery scenario: $Method after ${InterruptAfterSeconds}s, fault '$faultText' (target $($fault.target))"
    $recovery = Invoke-TigerWinLabEntryPoint -LabRoot $labRoot -EntryPoint 'Invoke-TigerWinLabRecoveryScenario.ps1' `
        -Parameters (@{ SpecPath = $specPath; Baseline = $Baseline } + (Get-TigerSetupRowStepPolicy)) `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-recovery.json") -OutputRoot $labOutputRoot -TimeoutMinutes $ScenarioTimeoutMinutes
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'recovery' -LabRun $recovery) { $checks.Add($check) }
    $engine = Join-Path $LabRecoveryStageRoot $installerFile
    $recoveryLog = Join-Path $LabRecoveryStageRoot 'recovery-run.log'
    $interruptedLog = Join-Path $LabRecoveryStageRoot 'interrupted-install.log'
    $read = Invoke-GuestJob -Row $Row -Suffix 'read' -Baseline $Baseline -Request (New-ReadRequest -Scope 'machine' -Engine $engine -Logs @($recoveryLog, $interruptedLog))
    $state = Add-ReadChecks -Checks $checks -Prefix 'read' -Run $read -ExpectedVersions $expectedVersions -Scope 'machine'
    $recoveryLines = Get-JobLog $read $recoveryLog
    $interruptedLines = Get-JobLog $read $interruptedLog
    $faultLine = @($interruptedLines | Where-Object { $_ -match '\[fault_injected\]' }) | Select-Object -First 1
    # A per-operation fault point names the resource it fired on; a
    # transaction-level one such as `before_commit` has none, so what the
    # line must carry there is the point itself. The same condition that
    # chose the fault text chooses what to assert about it.
    $expected = if ($Point -match '^(before_commit|after_commit_before_cleanup)$') { "point=$Point" } else { "target=$($fault.target)" }
    $faultNamesTarget = $null -ne $faultLine -and $faultLine -like "*$expected*"
    $checks.Add((New-TigerSetupCheck -Name 'read/fault landed on the expected file' -Code 'read.fault.target' -Status $(if ($faultNamesTarget) { 'PASS' } elseif ($null -eq $faultLine) { 'WARN' } else { 'FAIL' }) `
                -Message $(if ($null -eq $faultLine) { 'The interrupted log has no fault line (a power cut may have taken the tail).' } else { "Expected '$expected'. The fault line: $faultLine" })))
    Add-Check $checks 'read/recovery ran' 'read.log.recovery' ((Test-LogHasCode $recoveryLines 'recovery_started') -or (Test-LogHasCode $recoveryLines 'already_installed')) 'The recovery run reconciled the journal.' 'The recovery log records neither a recovery nor an already-installed outcome.'
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $recovery) -Evidence @{ recoveryLog = $recoveryLines; interruptedLog = $interruptedLines; engine = $state; fault = $fault; results = @($recovery.resultPath, $read.resultPath) }
}

function Invoke-RunningApplicationRow {
    <#
        M6: upgrade while the application runs; Restart Manager closes it.

        The application and the elevated upgrade both run on the signed-in
        user's desktop, because that is where a person runs both and because it
        is the only place the requirement can be evidenced. The Restart Manager
        lists a holder from its open file handles, which crosses sessions, but
        it closes a GUI application by messaging its windows, which does not: an
        application started from the job's session 0 is asked and never answers,
        and the row then measures the arrangement rather than the product.

        Every check here says where its evidence came from — which session each
        command ran in, whether the application had a window at all, and what
        the Restart Manager decided it could ask of it — because
        `package_in_use` reads the same whether the application refused, was
        never asked, or had already gone.
    #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    if ($null -eq $previous) { throw "Row $Row needs -PreviousInstallerPath." }
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $installLog = Join-Path $GuestStageRoot 'install-previous.log'
    $upgradeLog = Join-Path $GuestStageRoot 'upgrade-running.log'
    $root = Get-InstallRootOf 'machine'
    $process = [string] $expected.runningProcess
    $request = @{
        stage = @(
            @{ source = (Split-Path -Leaf $PreviousInstallerPath); destination = $stagedPrevious },
            @{ source = $installerFile; destination = $stagedInstaller })
        commands = @(
            @{ name = 'install'; executable = $stagedPrevious; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $installLog); timeoutSeconds = 900 },
            # The count alone would not say whether the application has a window
            # to be closed, which is the precondition the row exists to exercise;
            # MainWindowHandle is readable only from the application's own
            # session, which is where this runs. A cold VM's first start of a
            # framework application takes what it takes, so the launch waits
            # for the window, bounded, and records how long it took rather
            # than reading a fixed pause as the application's state.
            @{ name = 'launch'; runAs = 'interactiveUser'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "Start-Process -FilePath '$root\$process.exe'; `$started = Get-Date; `$deadline = `$started.AddSeconds(90); `$p = @(); do { Start-Sleep -Milliseconds 500; `$p = @(Get-Process -Name '$process' -ErrorAction SilentlyContinue | ForEach-Object { `$_.Refresh(); `$_ }) } while ((Get-Date) -lt `$deadline -and (`$p.Count -eq 0 -or `$p[0].MainWindowHandle -eq 0)); Start-Sleep -Seconds 3; `$p = @(Get-Process -Name '$process' -ErrorAction SilentlyContinue); [pscustomobject]@{ count = `$p.Count; window = [int64] `$(if (`$p.Count) { `$p[0].MainWindowHandle } else { 0 }); title = `$(if (`$p.Count) { `$p[0].MainWindowTitle } else { '' }); waitedSeconds = [math]::Round(((Get-Date) - `$started).TotalSeconds, 1) } | ConvertTo-Json -Compress"); timeoutSeconds = 180 },
            @{ name = 'upgrade'; runAs = 'elevatedUser'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $upgradeLog); timeoutSeconds = 900 },
            @{ name = 'running-after'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "Start-Sleep -Seconds 5; (Get-Process -Name '$process' -ErrorAction SilentlyContinue | Measure-Object).Count"); timeoutSeconds = 120 },
            @{ name = 'stop'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "Get-Process -Name '$process' -ErrorAction SilentlyContinue | Stop-Process -Force; 'stopped'"); timeoutSeconds = 120 },
            @{ name = 'verify'; executable = $stagedInstaller; arguments = @('verify', '--json', '--scope', 'machine'); timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedInstaller; arguments = @('inspect', '--json', '--scope', 'machine'); timeoutSeconds = 300 })
        logs = @($installLog, $upgradeLog)
        inventory = @($root, (Get-StateDirectory 'machine'))
        registry = @((Get-RegistrationPath 'machine'))
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'running' -Baseline $Baseline -Request $request -PayloadFiles @($PreviousInstallerPath, $InstallerPath) -TimeoutMinutes 40
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'running' -LabRun $run) { $checks.Add($check) }
    $null = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'install' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'running'
    $launch = Get-TigerSetupCommandResult -JobRun $run -CommandName 'launch'
    $launchState = Get-JsonOf $run 'launch'
    $launched = [int] (Get-Member2 $launchState 'count') -gt 0
    Add-Check $checks 'running/application launched' 'running.launched' $launched "$process was running before the upgrade (its window seen after $(Get-Member2 $launchState 'waitedSeconds') s)." "$process did not start: $(Get-Member2 $launch 'stderr')"
    # A GUI application is closed by messaging its windows, so an application
    # with no window is not evidence for or against quiescence either way.
    $windowed = [int64] (Get-Member2 $launchState 'window') -ne 0
    Add-Check $checks 'running/application presented a window' 'running.window' $windowed `
        "$process had a top-level window ('$(Get-Member2 $launchState 'title')') for the Restart Manager to close." `
        "$process was running with no top-level window, so there was nothing for a graceful shutdown to reach."
    # The row is only evidence about closing a window while the application had
    # one to close and the upgrade could reach it, so where each of the two ran
    # is a check rather than an assumption.
    $upgradeCommand = Get-TigerSetupCommandResult -JobRun $run -CommandName 'upgrade'
    Add-Check $checks 'running/application ran on the desktop' 'running.session.application' `
        ([string] (Get-Member2 $launch 'session') -eq 'interactiveUser') `
        "$process ran in the signed-in user's session as $(Get-Member2 $launch 'runAs')." `
        "$process ran in the '$(Get-Member2 $launch 'session')' session, where it has no window to be closed."
    Add-Check $checks 'running/upgrade ran elevated on that desktop' 'running.session.upgrade' `
        ([string] (Get-Member2 $upgradeCommand 'session') -eq 'elevatedUser') `
        "The upgrade ran elevated on the same desktop as $(Get-Member2 $upgradeCommand 'runAs')." `
        "The upgrade ran in the '$(Get-Member2 $upgradeCommand 'session')' session, which cannot message the application's windows."
    $null = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'upgrade' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'running'
    $upgradeLines = Get-JobLog $run $upgradeLog
    Add-Check $checks 'running/restart manager saw the application' 'running.restart_manager.listed' (Test-LogHasCode $upgradeLines 'restart_manager_holders') 'The upgrade log names the application as holding files.' 'The upgrade log records no Restart Manager holder.'
    # The Restart Manager's own classification of the holder. "Still running
    # after the grace period" reads identically for an application that refused
    # to close and one Windows never found a window to ask, and only the first
    # of those is a product question.
    $holderText = [string] (@($upgradeLines | Where-Object { $_ -match '\[restart_manager_holders\]' }) | Select-Object -First 1)
    Add-Check $checks 'running/restart manager could ask it to close' 'running.restart_manager.kind' `
        ($holderText -match 'main window|other window') `
        "The Restart Manager listed the holder as a windowed application: $holderText" `
        "The Restart Manager found no window to ask: $holderText"
    Add-Check $checks 'running/restart manager shut it down' 'running.restart_manager.shutdown' (Test-LogHasCode $upgradeLines 'restart_manager_shutdown') 'The application was shut down before the files were replaced.' 'The upgrade log records no shutdown.'
    $after = Get-TigerSetupCommandResult -JobRun $run -CommandName 'running-after'
    $restarted = $null -ne $after -and ([string] $after.stdout).Trim() -match '^[1-9]'
    $checks.Add((New-TigerSetupCheck -Name 'running/application restarted' -Code 'running.restart_manager.restart' -Status $(if ($restarted) { 'PASS' } else { 'WARN' }) `
                -Message $(if ($restarted) { "$process was running again after the upgrade." } else { "$process was not running after the upgrade: an application comes back only when it registered with RegisterApplicationRestart." })))
    $state = Add-ReadChecks -Checks $checks -Prefix 'running' -Run $run -ExpectedVersions @($facts.version) -Scope 'machine'
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $run) -Evidence @{ upgradeLog = $upgradeLines; engine = $state; results = @($run.resultPath) }
}

function Invoke-InteractiveRow {
    <# M7/M8/M9/M10/M14/W4/S3: the installer scenario with its wizard phases at a language and scale. #>
    param([string] $Row, [string] $Baseline, [string] $Scope, [string] $Language, [int] $ScalePercent, [string[]] $Prepare = @(), [string[]] $Prefer = @(), [hashtable] $Options = @{}, [switch] $WithUpgrade, [string] $NetworkState = 'online')
    $checks = [System.Collections.Generic.List[object]]::new()
    $fromBaseline = $true
    if ($Prepare.Count -gt 0) { Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids $Prepare); $fromBaseline = $false }
    $interactive = New-InteractiveSpec -Language $Language -Prefer $Prefer -WithUpgrade:$WithUpgrade
    $scenario = Invoke-InstallerScenarioRow -Row $Row -Baseline $Baseline -Scope $Scope -Options $Options -WithUpgrade:$WithUpgrade -Interactive $interactive `
        -NetworkState $NetworkState -Language $Language -ScalePercent $ScalePercent -FromBaseline:$fromBaseline
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'scenario' -LabRun $scenario) { $checks.Add($check) }
    $env = Get-Env $scenario
    $interactiveEnv = Get-Member2 $env 'interactive'
    # The lab names the session's display language `uiLanguage`. Not knowing it
    # is not the same as knowing it is wrong: an unknown language is a gap in
    # the evidence, a different one is a failure.
    $measured = [string] (Get-Member2 $interactiveEnv 'uiLanguage')
    $checks.Add((New-TigerSetupCheck -Name 'scenario/language measured' -Code 'scenario.env.language' `
                -Status $(if ($measured -eq $Language) { 'PASS' } elseif ([string]::IsNullOrWhiteSpace($measured)) { 'WARN' } else { 'FAIL' }) `
                -Message $(if ([string]::IsNullOrWhiteSpace($measured)) { "The lab reported no interactive language for this scenario; $Language was requested." } else { "The session's language is '$measured'." })))
    Add-Check $checks 'scenario/scale measured' 'scenario.env.scale' ([int] (Get-Member2 $interactiveEnv 'scalePercent') -eq $ScalePercent) "The session runs at $ScalePercent %." "The session runs at $(Get-Member2 $interactiveEnv 'scalePercent') %; expected $ScalePercent."
    Complete-Row -Row $Row -Checks $checks -Environment $env -Evidence @{ results = @($scenario.resultPath); screenshots = (Join-Path $labOutputRoot ([string] (Get-Member2 $scenario.result 'jobId'))) }
}

function Invoke-DependencyFailureRow {
    <# M11: the dependency installer fails → clean failure; the product fails after the dependency was acquired → rolled back, dependency remains. #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    $log1 = Join-Path $GuestStageRoot 'dependency-fail.log'
    $log2 = Join-Path $GuestStageRoot 'product-fail.log'
    $fault = Get-FaultSequence -Facts $facts -PreferredFile 'libSkiaSharp.dll'
    $request = @{
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(
            @{ name = 'dependency-fail'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $log1, '--fault', 'before_dependency_install@1:fail'); timeoutSeconds = 1200 },
            @{ name = 'inspect-1'; executable = $stagedInstaller; arguments = @('inspect', '--json', '--scope', 'machine'); timeoutSeconds = 300 },
            @{ name = 'product-fail'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $log2, '--fault', "after_applied@$($fault.sequence):fail"); timeoutSeconds = 1200 },
            @{ name = 'verify'; executable = $stagedInstaller; arguments = @('verify', '--json', '--scope', 'machine'); timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedInstaller; arguments = @('inspect', '--json', '--scope', 'machine'); timeoutSeconds = 300 })
        logs = @($log1, $log2)
        inventory = @((Get-InstallRootOf 'machine'), (Get-StateDirectory 'machine'), $dependencyPrep[$dotnetId].presentWhenPathExists)
        registry = @((Get-RegistrationPath 'machine'))
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'faults' -Baseline $Baseline -Request $request -PayloadFiles @($InstallerPath) -FromBaseline -TimeoutMinutes 45
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'faults' -LabRun $run) { $checks.Add($check) }
    $first = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'dependency-fail' -ExitCodes @(3) -ExpectedCode 'dependency_install_failed' -Prefix 'faults'
    $inspect1 = Get-JsonOf $run 'inspect-1'
    Add-Check $checks 'faults/nothing installed after the dependency failure' 'faults.dependency.nothing_installed' ($null -eq (Get-Member2 $inspect1 'installation')) 'No installation exists after the dependency failure.' 'An installation exists after the dependency failure.'
    $second = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'product-fail' -ExitCodes @(1) -ExpectedCode 'fault_injected' -Prefix 'faults'
    Add-Check $checks 'faults/product rolled back' 'faults.product.rolled_back' ([string] (Get-Member2 $second 'outcome') -eq 'rolled_back') 'The product transaction rolled back.' "The outcome is '$(Get-Member2 $second 'outcome')'."
    $dependencies = @(Get-Member2 $second 'dependencies')
    $acquired = @($dependencies | Where-Object { [string] $_.id -eq $dotnetId -and [string] $_.action -in @('installed', 'reboot_required') }).Count -gt 0
    Add-Check $checks 'faults/dependency acquired before the product failed' 'faults.dependency.acquired' $acquired "$dotnetId was installed by the run." "The run did not install $dotnetId (dependencies: $($dependencies | ConvertTo-Json -Compress -Depth 3))."
    $dotnet = Get-Inventory $run $dependencyPrep[$dotnetId].presentWhenPathExists
    Add-Check $checks 'faults/dependency preserved after rollback' 'faults.dependency.preserved' ($null -ne $dotnet -and [bool] $dotnet.exists) "$dotnetId is still present after the rollback." "$dotnetId is gone after the rollback."
    $null = Add-ReadChecks -Checks $checks -Prefix 'faults' -Run $run -ExpectedVersions @() -Scope 'machine'
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $run) -Evidence @{ first = $first; second = $second; log1 = (Get-JobLog $run $log1); log2 = (Get-JobLog $run $log2); results = @($run.resultPath) }
}

function Invoke-ModifiedFileRow {
    <# M12: a modified owned file is preserved and reported; settings outside the root survive the uninstall. #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $root = Get-InstallRootOf 'user'
    $settings = [string] $expected.settingsFile
    $modified = "$root\Docs\HELP.md"
    $installLog = Join-Path $GuestStageRoot 'install-user.log'
    $uninstallLog = Join-Path $GuestStageRoot 'uninstall-user.log'
    $request = @{
        runAs = 'interactiveUser'
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(
            @{ name = 'install'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'user', '--json', '--log', $installLog); timeoutSeconds = 900 },
            @{ name = 'modify'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "Add-Content -LiteralPath '$modified' -Value 'edited by the user'; New-Item -ItemType Directory -Force -Path (Split-Path -Parent '$settings') | Out-Null; Set-Content -LiteralPath '$settings' -Value '{ ""theme"": ""dark"" }'; 'done'"); timeoutSeconds = 120 },
            @{ name = 'uninstall'; executable = $stagedInstaller; arguments = @('uninstall', '--quiet', '--scope', 'user', '--json', '--log', $uninstallLog); timeoutSeconds = 900 },
            @{ name = 'verify'; executable = $stagedInstaller; arguments = @('verify', '--json', '--scope', 'user'); timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedInstaller; arguments = @('inspect', '--json', '--scope', 'user'); timeoutSeconds = 300 },
            @{ name = 'residue'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "[pscustomobject]@{ modified = (Test-Path -LiteralPath '$modified'); settings = (Test-Path -LiteralPath '$settings'); files = @(Get-ChildItem -LiteralPath '$root' -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object { `$_.FullName }) } | ConvertTo-Json -Compress"); timeoutSeconds = 120 })
        logs = @($installLog, $uninstallLog)
        inventory = @($root, (Get-StateDirectory 'user'))
        registry = @((Get-RegistrationPath 'user'))
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'modified' -Baseline $Baseline -Request $request -PayloadFiles @($InstallerPath) -TimeoutMinutes 40
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'modified' -LabRun $run) { $checks.Add($check) }
    $null = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'install' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'modified'
    $uninstall = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'uninstall' -ExitCodes @(0) -Prefix 'modified'
    $findings = @(Get-Member2 $uninstall 'findings')
    $reported = @($findings | Where-Object { (Get-Member2 $_ 'code') -eq 'file_modified_preserved' -and ([string] (Get-Member2 $_ 'path')) -like '*HELP.md' }).Count -gt 0
    Add-Check $checks 'modified/file reported as preserved' 'modified.reported' $reported 'The uninstall reported the modified file as file_modified_preserved.' "The uninstall findings: $($findings | ConvertTo-Json -Compress -Depth 3)"
    $residue = Get-JsonOf $run 'residue'
    Add-Check $checks 'modified/file still present' 'modified.preserved' ([bool] (Get-Member2 $residue 'modified')) 'The modified owned file survived the uninstall.' 'The modified owned file was removed.'
    Add-Check $checks 'modified/settings survive' 'modified.settings' ([bool] (Get-Member2 $residue 'settings')) 'The settings file outside the install root survived.' 'The settings file was removed.'
    $files = @(Get-Member2 $residue 'files')
    Add-Check $checks 'modified/everything else is gone' 'modified.others_gone' ($files.Count -eq 1) "Only the preserved file remains under the install root." "$($files.Count) file(s) remain under the install root: $(@($files | Select-Object -First 5) -join ', ')"
    $verify = Get-JsonOf $run 'verify'
    Add-Check $checks 'modified/verify reports absence' 'modified.verify.absent' ([string] (Get-Member2 $verify 'status') -eq 'not_installed') 'verify reports not_installed.' "verify status '$(Get-Member2 $verify 'status')'."
    $registration = Get-RegistryRecord $run (Get-RegistrationPath 'user')
    Add-Check $checks 'modified/registration gone' 'modified.registration.absent' ($null -eq $registration -or -not [bool] $registration.exists) 'The registration is gone.' 'The registration remains.'
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $run) -Evidence @{ uninstall = $uninstall; residue = $residue; results = @($run.resultPath) }
}

function Invoke-WinGetRow {
    <# M13: the WinGet scenario on the prepared manifests. #>
    param([string] $Row, [string] $Baseline)
    if ([string]::IsNullOrWhiteSpace($ManifestDirectory)) { throw "Row $Row needs -ManifestDirectory." }
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $identifier = [string] $matrix.winget.identifier
    $url = ([string] $matrix.winget.releaseUrlTemplate).Replace('{version}', $facts.version).Replace('{file}', $installerFile)
    # `prepare` leaves the installer URL unresolved on purpose: it is not known
    # until the bytes are published. `finalize` writes the published URL and
    # the hash of those exact bytes, and it is the step the row is here to
    # exercise — the manifest the scenario reads must be a finished one.
    $finalize = & $BuilderPath winget finalize $ManifestDirectory --url $url --installer $InstallerPath 2>&1 | Out-String
    Add-Check $checks 'winget/manifest finalized' 'winget.manifest.finalized' ($LASTEXITCODE -eq 0) "The manifest set names the published URL and the hash of the exact published bytes." "tiger-setup winget finalize failed: $finalize"
    $dependencies = @()
    foreach ($id in @($dotnetId, $webviewId)) {
        $d = $dependencyPrep[$id]
        $dependencies += [ordered]@{ packageIdentifier = $id; presentWhenPathExists = $d.presentWhenPathExists; installer = [ordered]@{ url = $d.url; arguments = @($d.arguments) } }
    }
    $wingetSmoke = @($expected.smoke | ForEach-Object { [ordered]@{ name = $_.name; command = [string] $matrix.winget.commands[0]; arguments = @($_.arguments); expectedExitCode = [int] $_.expectedExitCode; expectedOutputPattern = $_.expectedOutputPattern } })
    $specPath = New-TigerSetupWinGetSpec -Name "ts-$Row".ToLowerInvariant() -Facts $facts -InstallerPath $InstallerPath -ManifestDirectory $ManifestDirectory -ExpectedUrl $url `
        -Identifier $identifier -ExpectedFiles $expectedFiles -MinimumFileCount $minimumFileCount -VersionFile $versionFile -Commands @($matrix.winget.commands) `
        -Dependencies $dependencies -Smoke $wingetSmoke -OutputPath (Join-Path $ResultsRoot "specs\$Row-winget.json")
    Write-Host '  winget scenario'
    $scenario = Invoke-TigerWinLabEntryPoint -LabRoot $labRoot -EntryPoint 'Invoke-TigerWinLabWinGetScenario.ps1' `
        -Parameters (@{ SpecPath = $specPath; Baseline = $Baseline } + (Get-TigerSetupRowStepPolicy)) `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-winget.json") -OutputRoot $labOutputRoot -TimeoutMinutes 60
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'winget' -LabRun $scenario) { $checks.Add($check) }
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $scenario) -Evidence @{ results = @($scenario.resultPath) }
}

function Invoke-PathVectorsRow {
    <# M15: the machine PATH seeded with a lookalike and an empty segment; install, reinstall and uninstall leave them alone and add exactly one entry. #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $root = (Get-InstallRootOf 'machine')
    $lookalike = "$root\"
    $seed = "Set-StrictMode -Off; `$key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment', `$true); `$path = `$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames'); `$key.SetValue('Path', (`$path.TrimEnd(';') + ';' + [Environment]::ExpandEnvironmentVariables('$lookalike') + ';C:\Seeded\Entry;;'), 'ExpandString'); 'seeded'"
    $log1 = Join-Path $GuestStageRoot 'path-install.log'
    $log2 = Join-Path $GuestStageRoot 'path-reinstall.log'
    $log3 = Join-Path $GuestStageRoot 'path-uninstall.log'
    $readPath = "Set-StrictMode -Off; `$key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment'); [pscustomobject]@{ kind = [string] `$key.GetValueKind('Path'); raw = `$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } | ConvertTo-Json -Compress"
    $request = @{
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(
            @{ name = 'seed'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', $seed); timeoutSeconds = 60 },
            @{ name = 'path-0'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', $readPath); timeoutSeconds = 60 },
            @{ name = 'install'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $log1); timeoutSeconds = 900 },
            @{ name = 'path-1'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', $readPath); timeoutSeconds = 60 },
            @{ name = 'reinstall'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--option', 'path', 'on', '--json', '--log', $log2); timeoutSeconds = 900 },
            @{ name = 'path-2'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', $readPath); timeoutSeconds = 60 },
            @{ name = 'uninstall'; executable = $stagedInstaller; arguments = @('uninstall', '--quiet', '--scope', 'machine', '--json', '--log', $log3); timeoutSeconds = 900 },
            @{ name = 'path-3'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', $readPath); timeoutSeconds = 60 })
        logs = @($log1, $log2, $log3)
        inventory = @($root)
        registry = @((Get-RegistrationPath 'machine'))
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'path' -Baseline $Baseline -Request $request -PayloadFiles @($InstallerPath) -TimeoutMinutes 45
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'path' -LabRun $run) { $checks.Add($check) }
    foreach ($name in @('install', 'reinstall', 'uninstall')) { $null = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName $name -ExitCodes @(0) -Prefix 'path' }
    # An array, not a hashtable keyed by number: the row result is JSON, and
    # a dictionary with non-string keys cannot be written as JSON at all.
    $paths = @(0..3 | ForEach-Object { Get-JsonOf $run "path-$_" })
    $entriesOf = { param($p) @(([string] (Get-Member2 $p 'raw')) -split ';') }
    $expandedRoot = $null
    $env = Get-Env $run
    # The lookalike keeps its trailing backslash; TigerSetup's own entry is the root without one.
    foreach ($step in 1..3) {
        $entries = & $entriesOf $paths[$step]
        $lookalikeCount = @($entries | Where-Object { $_ -like '*\' + $facts.name + '\' }).Count
        $ownCount = @($entries | Where-Object { $_ -like '*\' + $facts.name }).Count
        $emptyTrailing = ($entries.Count -ge 2 -and $entries[-1] -eq '' -and $entries[-2] -eq '')
        $seededKept = $entries -contains 'C:\Seeded\Entry'
        # None, at any step. An equivalent entry was seeded before the
        # install, and TigerSetup neither duplicates it nor claims it
        # (`TigerSetup-Design.md` §5.6) — so the entry that serves the product
        # is the one that was already there, and TigerSetup adds nothing of
        # its own to add, keep or remove.
        Add-Check $checks "path/step $step adds no second entry" "path.$step.own" ($ownCount -eq 0) "TigerSetup added no entry of its own after step $step; the equivalent entry that pre-existed serves the product." "$ownCount TigerSetup entry(ies) after step $step; a pre-existing equivalent entry must be neither duplicated nor claimed. PATH: $(Get-Member2 $paths[$step] 'raw')"
        Add-Check $checks "path/step $step lookalike untouched" "path.$step.lookalike" ($lookalikeCount -eq 1) 'The pre-existing lookalike is neither duplicated nor removed.' "$lookalikeCount lookalike entries after step $step."
        Add-Check $checks "path/step $step empty segment survives" "path.$step.empty_segment" $emptyTrailing 'The trailing empty segment survives.' "The trailing empty segment is gone: $(Get-Member2 $paths[$step] 'raw')"
        Add-Check $checks "path/step $step seeded entry survives" "path.$step.seeded" $seededKept 'The seeded entry survives.' 'The seeded entry is gone.'
        Add-Check $checks "path/step $step value type" "path.$step.kind" ([string] (Get-Member2 $paths[$step] 'kind') -eq 'ExpandString') 'Path is REG_EXPAND_SZ.' "Path is $(Get-Member2 $paths[$step] 'kind')."
    }
    Complete-Row -Row $Row -Checks $checks -Environment $env -Evidence @{ paths = $paths; results = @($run.resultPath) }
}

function Invoke-LegacyMigrationRow {
    <# M16: an Inno-installed previous version is migrated uninstall-first; settings survive; one registration remains. #>
    param([string] $Row, [string] $Baseline)
    if ([string]::IsNullOrWhiteSpace($LegacyInstallerPath)) { throw "Row $Row needs -LegacyInstallerPath." }
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $legacy = $matrix.legacy
    $legacyFile = Split-Path -Leaf $LegacyInstallerPath
    $stagedLegacy = Join-Path $GuestStageRoot $legacyFile
    $legacyKey = "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$($legacy.registrationKey)"
    $settings = [string] $expected.settingsFile
    $migrateLog = Join-Path $GuestStageRoot 'migrate.log'
    $root = Get-InstallRootOf 'machine'
    $request = @{
        stage = @(@{ source = $legacyFile; destination = $stagedLegacy }, @{ source = $installerFile; destination = $stagedInstaller })
        commands = @(
            @{ name = 'legacy-install'; executable = $stagedLegacy; arguments = @(@($legacy.installArguments) + @('/ALLUSERS')); timeoutSeconds = 900 },
            @{ name = 'seed-settings'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "New-Item -ItemType Directory -Force -Path (Split-Path -Parent '$settings') | Out-Null; Set-Content -LiteralPath '$settings' -Value '{ ""theme"": ""dark"" }'; 'seeded'"); timeoutSeconds = 60 },
            @{ name = 'migrate'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $migrateLog); timeoutSeconds = 900 },
            @{ name = 'verify'; executable = $stagedInstaller; arguments = @('verify', '--json', '--scope', 'machine'); timeoutSeconds = 300 },
            @{ name = 'inspect'; executable = $stagedInstaller; arguments = @('inspect', '--json', '--scope', 'machine'); timeoutSeconds = 300 },
            @{ name = 'settings'; executable = 'powershell.exe'; arguments = @('-NoProfile', '-Command', "Test-Path -LiteralPath '$settings'"); timeoutSeconds = 60 })
        logs = @($migrateLog)
        inventory = @($root, (Get-StateDirectory 'machine'))
        registry = @($legacyKey, (Get-RegistrationPath 'machine'))
        pathValues = $true
    }
    $run = Invoke-GuestJob -Row $Row -Suffix 'migrate' -Baseline $Baseline -Request $request -PayloadFiles @($LegacyInstallerPath, $InstallerPath) -TimeoutMinutes 45
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'migrate' -LabRun $run) { $checks.Add($check) }
    $legacyRun = Get-TigerSetupCommandResult -JobRun $run -CommandName 'legacy-install'
    Add-Check $checks 'migrate/legacy install' 'migrate.legacy.install' ($null -ne $legacyRun -and $legacyRun.exitCode -eq 0) "The legacy installer exited $($legacyRun.exitCode)." "The legacy installer exited $(Get-Member2 $legacyRun 'exitCode')."
    $outcome = Add-EngineCommandCheck -Checks $checks -Run $run -CommandName 'migrate' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'migrate'
    $lines = Get-JobLog $run $migrateLog
    Add-Check $checks 'migrate/legacy uninstall ran' 'migrate.legacy.uninstalled' (Test-LogHasCode $lines 'legacy_uninstalled') 'The log records the legacy uninstall.' 'The log does not record legacy_uninstalled.'
    $legacyRecord = Get-RegistryRecord $run $legacyKey
    Add-Check $checks 'migrate/legacy registration gone' 'migrate.legacy.registration_gone' ($null -eq $legacyRecord -or -not [bool] $legacyRecord.exists) 'The Inno registration is gone.' 'The Inno registration remains.'
    $state = Add-ReadChecks -Checks $checks -Prefix 'migrate' -Run $run -ExpectedVersions @($facts.version) -Scope 'machine'
    $settingsRun = Get-TigerSetupCommandResult -JobRun $run -CommandName 'settings'
    Add-Check $checks 'migrate/settings survive' 'migrate.settings' ($null -ne $settingsRun -and ([string] $settingsRun.stdout).Trim() -eq 'True') 'The settings file survived the migration.' 'The settings file is gone.'
    $pathValues = Get-Member2 (Get-Member2 $run.result 'result') 'pathValues'
    $entries = @(Get-Member2 (Get-Member2 $pathValues 'machine') 'entries')
    $own = @($entries | Where-Object { $_ -like '*\' + $facts.name -or $_ -like '*\' + $facts.name + '\' }).Count
    Add-Check $checks 'migrate/exactly one PATH entry' 'migrate.path.single' ($own -eq 1) "One PATH entry for the install root." "$own PATH entries name the install root: $(Get-Member2 (Get-Member2 $pathValues 'machine') 'raw')"
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $run) -Evidence @{ outcome = $outcome; migrateLog = $lines; engine = $state; results = @($run.resultPath) }
}

function Invoke-StateDirectoryOwnershipRow {
    <#
        M17: a standard user creates the machine-scope state directory under
        %ProgramData% before any install — where the creator owns what it
        creates, and an owner keeps WRITE_DAC however the list reads — and the
        elevated install takes that ownership away. The three jobs are three,
        not one, because a job runs either as its administrator account or in
        the signed-in user's session: seeding the directory and probing it
        afterwards prove nothing unless they run as the standard user.
    #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $stateDirectory = Get-StateDirectory 'machine'
    $installLog = Join-Path $GuestStageRoot 'ownership-install.log'

    # The placeholder is expanded inside the guest: an encoded script is one
    # opaque token to the job's own path expansion.
    $seedScript = @"
`$ErrorActionPreference = 'Stop'
`$dir = [Environment]::ExpandEnvironmentVariables('$stateDirectory')
`$created = `$false
if (-not (Test-Path -LiteralPath `$dir)) { `$null = New-Item -ItemType Directory -Path `$dir -Force; `$created = `$true }
`$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
`$principal = New-Object System.Security.Principal.WindowsPrincipal(`$identity)
[pscustomobject]@{
    path = `$dir
    created = `$created
    user = `$identity.Name
    userSid = `$identity.User.Value
    elevated = `$principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
    owner = (Get-Acl -LiteralPath `$dir).GetOwner([System.Security.Principal.SecurityIdentifier]).Value
} | ConvertTo-Json -Compress
"@

    # Effective access is read from the token that has it: the standard user's
    # own SID plus every group SID in its token, matched against the list by
    # SID rather than by a display name a localized guest would spell
    # differently.
    $tamperScript = @"
`$ErrorActionPreference = 'Stop'
`$dir = [Environment]::ExpandEnvironmentVariables('$stateDirectory')
`$uninstaller = Join-Path `$dir 'uninstall.exe'
`$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
`$sids = @(`$identity.User.Value) + @(`$identity.Groups | ForEach-Object { `$_.Value })
`$acl = Get-Acl -LiteralPath `$dir
`$allow = 0
`$deny = 0
`$entries = @()
foreach (`$rule in `$acl.GetAccessRules(`$true, `$true, [System.Security.Principal.SecurityIdentifier])) {
    `$sid = `$rule.IdentityReference.Value
    `$entries += ('{0} {1} 0x{2:x}' -f `$sid, `$rule.AccessControlType, [int] `$rule.FileSystemRights)
    if (`$sids -notcontains `$sid) { continue }
    if (`$rule.AccessControlType -eq 'Allow') { `$allow = `$allow -bor [int] `$rule.FileSystemRights }
    else { `$deny = `$deny -bor [int] `$rule.FileSystemRights }
}
`$hashBefore = ''
`$hashAfter = ''
if (Test-Path -LiteralPath `$uninstaller -PathType Leaf) { `$hashBefore = (Get-FileHash -LiteralPath `$uninstaller -Algorithm SHA256).Hash }
`$probe = Join-Path `$dir 'ownership-probe.txt'
`$createAttempted = `$false
`$createSucceeded = `$false
`$createError = ''
try {
    `$createAttempted = `$true
    `$stream = [System.IO.File]::Open(`$probe, 'CreateNew', 'Write', 'None')
    `$stream.Dispose()
    `$createSucceeded = `$true
}
catch { `$createError = `$_.Exception.GetType().FullName + ': ' + `$_.Exception.Message }
`$replaceAttempted = `$false
`$replaceSucceeded = `$false
`$replaceError = ''
if (`$hashBefore -ne '') {
    try {
        `$replaceAttempted = `$true
        `$stream = [System.IO.File]::Open(`$uninstaller, 'Open', 'Write', 'None')
        `$bytes = [byte[]] (1..16)
        `$stream.Write(`$bytes, 0, `$bytes.Length)
        `$stream.Dispose()
        `$replaceSucceeded = `$true
    }
    catch { `$replaceError = `$_.Exception.GetType().FullName + ': ' + `$_.Exception.Message }
    `$hashAfter = (Get-FileHash -LiteralPath `$uninstaller -Algorithm SHA256).Hash
}
[pscustomobject]@{
    path = `$dir
    user = `$identity.Name
    userSid = `$identity.User.Value
    owner = `$acl.GetOwner([System.Security.Principal.SecurityIdentifier]).Value
    effective = (`$allow -band (-bnot `$deny))
    entries = `$entries
    uninstallerExists = (`$hashBefore -ne '')
    hashBefore = `$hashBefore
    hashAfter = `$hashAfter
    createAttempted = `$createAttempted
    createSucceeded = `$createSucceeded
    createError = `$createError
    createResidue = (Test-Path -LiteralPath `$probe)
    replaceAttempted = `$replaceAttempted
    replaceSucceeded = `$replaceSucceeded
    replaceError = `$replaceError
} | ConvertTo-Json -Compress
"@

    $seedRun = Invoke-GuestJob -Row $Row -Suffix 'seed' -Baseline $Baseline -Request @{
        runAs = 'interactiveUser'
        commands = @((New-GuestScriptCommand -Name 'seed' -Script $seedScript -TimeoutSeconds 180))
    }
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'seed' -LabRun $seedRun) { $checks.Add($check) }
    $seed = Get-JsonOf $seedRun 'seed'
    $seedUser = [string] (Get-Member2 $seed 'user')
    $seedOwner = [string] (Get-Member2 $seed 'owner')
    $seededByUser = $null -ne $seed -and [bool] (Get-Member2 $seed 'created') -and -not [bool] (Get-Member2 $seed 'elevated') -and $seedOwner -eq [string] (Get-Member2 $seed 'userSid')
    Add-Check $checks 'ownership/the standard user created the state directory first' 'ownership.seeded' $seededByUser `
        "$seedUser created $(Get-Member2 $seed 'path') unelevated before any install and owned it (owner SID $seedOwner)." `
        "The state directory was not left owned by an unelevated standard user before the install: $($seed | ConvertTo-Json -Compress)"

    $installRun = Invoke-GuestJob -Row $Row -Suffix 'install' -Baseline $Baseline -TimeoutMinutes 45 -PayloadFiles @($InstallerPath) -Request @{
        stage = @(@{ source = $installerFile; destination = $stagedInstaller })
        commands = @(@{ name = 'install'; executable = $stagedInstaller; arguments = @('install', '--quiet', '--scope', 'machine', '--json', '--log', $installLog); timeoutSeconds = 900 })
        logs = @($installLog)
        inventory = @((Get-InstallRootOf 'machine'), $stateDirectory)
        registry = @((Get-RegistrationPath 'machine'))
    }
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'install' -LabRun $installRun) { $checks.Add($check) }
    $outcome = Add-EngineCommandCheck -Checks $checks -Run $installRun -CommandName 'install' -ExitCodes @(0) -ExpectedCode 'ok' -Prefix 'ownership'
    $lines = Get-JobLog $installRun $installLog
    $protectionEvents = @($lines | Where-Object { $_ -match 'state_directory_' })
    Add-Check $checks 'ownership/the install claimed the directory' 'ownership.claimed' (Test-LogHasCode $lines 'state_directory_ownership_claimed') `
        'The install log records state_directory_ownership_claimed for the state directory.' `
        "The install log does not record state_directory_ownership_claimed. The state-directory events it holds: $(if ($protectionEvents.Count -gt 0) { $protectionEvents -join ' | ' } else { 'none' })"

    $tamperRun = Invoke-GuestJob -Row $Row -Suffix 'tamper' -Baseline $Baseline -Request @{
        runAs = 'interactiveUser'
        commands = @((New-GuestScriptCommand -Name 'tamper' -Script $tamperScript -TimeoutSeconds 300))
    }
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'tamper' -LabRun $tamperRun) { $checks.Add($check) }
    $tamperCommand = Get-TigerSetupCommandResult -JobRun $tamperRun -CommandName 'tamper'
    $tamper = Get-Member2 $tamperCommand 'json'
    Add-Check $checks 'ownership/the standard user reported the directory' 'ownership.probe.reported' ($null -ne $tamper) `
        "$(Get-Member2 $tamper 'user') read $(Get-Member2 $tamper 'path') and reported what it found." `
        "The standard user's probe reported nothing readable. stdout: $(Get-Member2 $tamperCommand 'stdout') stderr: $(Get-Member2 $tamperCommand 'stderr') error: $(Get-Member2 $tamperCommand 'error')"

    $owner = [string] (Get-Member2 $tamper 'owner')
    $ownerName = switch ($owner) {
        'S-1-5-32-544' { 'the Administrators group' }
        'S-1-5-18' { 'SYSTEM' }
        default { 'neither the Administrators group nor SYSTEM' }
    }
    Add-Check $checks 'ownership/owned by Administrators afterwards' 'ownership.owner' (@('S-1-5-32-544', 'S-1-5-18') -contains $owner) `
        "The state directory is owned by $ownerName ($owner) after the install; the standard user owned it before ($seedOwner)." `
        "The state directory is owned by $owner — $ownerName. Expected S-1-5-32-544 (Administrators) or S-1-5-18 (SYSTEM); it was $seedOwner before the install."

    # Read and execute is FILE_GENERIC_READ | FILE_GENERIC_EXECUTE; every bit
    # that would let its holder change the directory, its contents, its list
    # or its owner must be absent.
    $readAndExecute = 0x1200A9
    $writeRights = 0x00002 -bor 0x00004 -bor 0x00010 -bor 0x00040 -bor 0x00100 -bor 0x10000 -bor 0x40000 -bor 0x80000
    $effective = [int] (Get-Member2 $tamper 'effective')
    $readOnly = $null -ne $tamper -and (($effective -band $readAndExecute) -eq $readAndExecute) -and (($effective -band $writeRights) -eq 0)
    Add-Check $checks 'ownership/read and execute only for the standard user' 'ownership.effective_access' $readOnly `
        ("Effective access for $(Get-Member2 $tamper 'user') is 0x{0:x}: read and execute, with no write, delete, change-permissions or take-ownership right." -f $effective) `
        ("Effective access for $(Get-Member2 $tamper 'user') is 0x{0:x}; expected read and execute only (0x{1:x}). The list: {2}" -f $effective, $readAndExecute, (@(Get-Member2 $tamper 'entries') -join ' | '))

    $createAttempted = [bool] (Get-Member2 $tamper 'createAttempted')
    $createDenied = $createAttempted -and -not [bool] (Get-Member2 $tamper 'createSucceeded') -and -not [bool] (Get-Member2 $tamper 'createResidue')
    Add-Check $checks 'ownership/the standard user cannot write into the directory' 'ownership.write_denied' $createDenied `
        "Creating a file in the state directory as $(Get-Member2 $tamper 'user') failed and left nothing behind: $(Get-Member2 $tamper 'createError')" `
        $(if ($createAttempted) { "Creating a file in the state directory as $(Get-Member2 $tamper 'user') succeeded=$(Get-Member2 $tamper 'createSucceeded'), residue=$(Get-Member2 $tamper 'createResidue'), error='$(Get-Member2 $tamper 'createError')'." } else { 'The write was never attempted, so nothing about it is known.' })

    $replaceAttempted = [bool] (Get-Member2 $tamper 'replaceAttempted')
    $replaceDenied = $replaceAttempted -and -not [bool] (Get-Member2 $tamper 'replaceSucceeded')
    Add-Check $checks 'ownership/the standard user cannot replace uninstall.exe' 'ownership.replace_denied' $replaceDenied `
        "Opening uninstall.exe for writing as $(Get-Member2 $tamper 'user') failed: $(Get-Member2 $tamper 'replaceError')" `
        $(if ($replaceAttempted) { "Opening uninstall.exe for writing as $(Get-Member2 $tamper 'user') succeeded — the binary an elevated Add/Remove Programs uninstall runs is writable by a standard user." } else { "The replacement was never attempted: uninstall.exe was $(if ([bool] (Get-Member2 $tamper 'uninstallerExists')) { 'present' } else { 'absent' }) in $(Get-Member2 $tamper 'path')." })

    $hashBefore = [string] (Get-Member2 $tamper 'hashBefore')
    $hashAfter = [string] (Get-Member2 $tamper 'hashAfter')
    Add-Check $checks 'ownership/uninstall.exe is unchanged' 'ownership.uninstaller_intact' ($hashBefore -ne '' -and $hashAfter -eq $hashBefore) `
        "uninstall.exe is byte-identical after the attempt (SHA-256 $hashBefore)." `
        $(if ($hashBefore -eq '') { "uninstall.exe was not found in $(Get-Member2 $tamper 'path'), so nothing was hashed." } else { "uninstall.exe changed from SHA-256 $hashBefore to $hashAfter." })

    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $installRun) -Evidence @{
        seed = $seed; outcome = $outcome; installLog = $lines; tamper = $tamper
        results = @($seedRun.resultPath, $installRun.resultPath, $tamperRun.resultPath)
    }
}

function Invoke-InteractiveDependencyRow {
    <#
        S6: the wizard acquires a dependency without elevation (WebView2 per
        user on Server 2019, the one baseline without it, .NET prepared),
        driven page by page in the standard user's session with captures of
        every page including the dependency progress.

        The row must stay unelevated, and the wizard's first page is the one
        that decides that: Alt+M takes "Install for me only", where Alt+A would
        take "Install for all users" and put a UAC prompt on the secure desktop,
        which no capture can read. The licence page that follows is the one
        Alt+A accepts. Every other page advances with Enter, and the capture
        waits for each page to change rather than for a settle delay, because
        acquiring the dependency is minutes of one page.
    #>
    param([string] $Row, [string] $Baseline)
    $checks = [System.Collections.Generic.List[object]]::new()
    Add-PrepareChecks -Checks $checks -Run (Invoke-Prepare -Row $Row -Baseline $Baseline -Ids @($dotnetId))
    $wizard = @{
        ExecutablePath = $InstallerPath; Arguments = @('--lang', 'en-US'); CaptureName = "$Row-install"
        TitlePattern = "^$([regex]::Escape($facts.name)) Setup"; MaxPages = 10; AdvanceKeys = @('Return')
        AdvanceByPage = @{ '1' = @(@('Menu', 'M'), @('Return')); '2' = @(@('Menu', 'A'), @('Return')) }
        PageTimeoutSeconds = 600
    }
    Write-Host '  wizard capture (dependency acquisition as the standard user)'
    # The wizard is the second step of the row: it continues the prepared VM
    # and preserves it for the read step that follows.
    $policy = Get-TigerSetupRowStepPolicy
    $capture = Invoke-TigerSetupWizardCapture -LabRoot $labRoot -Baseline $Baseline -Wizards @($wizard) -Language 'en-US' -ScalePercent 100 -Name "ts-$Row-wizard".ToLowerInvariant() `
        @policy -ResultPath (Join-Path $ResultsRoot "runs\$Row-wizard.json") -OutputRoot $labOutputRoot -TimeoutMinutes 40
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'wizard' -LabRun $capture) { $checks.Add($check) }
    $record = Get-Member2 $capture.result 'result'
    $pages = @(Get-Member2 (@(Get-Member2 $record 'wizards') | Select-Object -First 1) 'pages')
    Add-Check $checks 'wizard/pages captured' 'wizard.pages' ($pages.Count -ge 4) "$($pages.Count) page(s) captured." "Only $($pages.Count) page(s) captured."
    $read = Invoke-GuestJob -Row $Row -Suffix 'read' -Baseline $Baseline -Request (@{ runAs = 'interactiveUser' } + (New-ReadRequest -Scope 'user' -Engine (Join-Path 'C:\TigerSetupLab\ui' $installerFile)))
    $state = Add-ReadChecks -Checks $checks -Prefix 'read' -Run $read -ExpectedVersions @($facts.version) -Scope 'user'
    $inspect = $state.inspect
    $webview = @(Get-Member2 $inspect 'dependencies') | Where-Object { [string] $_.id -eq $webviewId } | Select-Object -First 1
    Add-Check $checks 'read/WebView2 present afterwards' 'read.dependency.webview2' ($null -ne $webview -and [string] (Get-Member2 $webview 'status') -eq 'present') "WebView2 is present ($(Get-Member2 $webview 'version'))." "WebView2 is not present after the wizard: $($webview | ConvertTo-Json -Compress)"
    Complete-Row -Row $Row -Checks $checks -Environment (Get-Env $capture) -Evidence @{ capture = $record; engine = $state; results = @($capture.resultPath, $read.resultPath) }
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-matrix-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}

<#
    One lab session per baseline, not one for the run.

    Every job of a row preserves the VM for the next job of the run, and the
    preserved VM stays running, reserved to the session, until the session ends
    — that is what makes a VM boot once for a whole baseline's rows instead of
    between them. It also means a session that has preserved two baselines is
    holding two running VMs, and the host runs a bounded number at a time: the
    matrix spans three, so a single session for the whole run could not start
    the third and every Server row would fail before it begins.

    Leaving a baseline therefore ends its session, which hands the VM to the
    lab's own maintenance — shutdown, baseline restored, Off — without this
    run waiting for it. The ids are derived from the run's own, so they stay
    stable and readable in the lab's diagnostics.
#>
$labSessionBaseline = $null
$labSessionId = $null

$summary = [System.Collections.Generic.List[object]]::new()
try {
foreach ($row in $Rows) {
    $baseline = $RowTable[$row]
    if ($baseline -ne $labSessionBaseline) {
        if ($null -ne $labSessionBaseline) {
            $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $labSessionId -ResultPath (Join-Path $ResultsRoot "session-close-$labSessionBaseline.json")
        }
        $labSessionBaseline = $baseline
        # Not $sessionId: PowerShell variable names are case-insensitive, so a
        # local differing from the -SessionId parameter only in case is that
        # parameter, and the run would name its sessions after nothing.
        $labSessionId = "$SessionId-$($baseline -replace '^TigerWinLab-', '')".ToLowerInvariant()
        $null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $labSessionId `
            -Description "TigerSetup acceptance matrix on $baseline" `
            -ResultPath (Join-Path $ResultsRoot "session-open-$baseline.json")
        Write-Host ''
        Write-Host "Lab session $labSessionId"
    }
    Write-Host ''
    Write-Host "### $row ($baseline)"
    $started = [DateTimeOffset]::Now
    try {
        $result = switch ($row) {
            'M1' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -WithUpgrade -PreservedDependencies @($dotnetId) }
            'M2' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'user' -Prepare @($dotnetId) -WithUpgrade }
            'M3' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -Prepare @($dotnetId) -NetworkState 'offline' }
            'M4' { Invoke-OfflineFailureRow -Row $row -Baseline $baseline }
            'M5a' { Invoke-RecoveryRow -Row $row -Baseline $baseline -Method 'powerOff' -Point 'after_write_before_flush' }
            'M5b' { Invoke-RecoveryRow -Row $row -Baseline $baseline -Method 'powerOff' -Point 'after_rename' -Upgrade }
            'M5c' { Invoke-RecoveryRow -Row $row -Baseline $baseline -Method 'reboot' -Point 'before_commit' -Upgrade }
            'M6' { Invoke-RunningApplicationRow -Row $row -Baseline $baseline }
            # The row the matrix asks for: machine scope chosen on the scope
            # page and the PATH option turned *off* in the wizard, so the
            # installation is expected to own no PATH entry at all. Every
            # preferred control is anchored, because an unanchored pattern
            # also matches the summary page's recital of the same choices.
            'M7' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'machine' -Language 'en-US' -ScalePercent 100 -Prefer @('^Install for all users', '^Add .+ to PATH') -Options @{ path = $false } -WithUpgrade }
            'M8' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'user' -Language 'pl-PL' -ScalePercent 150 -Prepare @($dotnetId) }
            'M9' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'user' -Language 'en-US' -ScalePercent 200 -Prepare @($dotnetId) }
            'M10' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'machine' -Language 'pl-PL' -ScalePercent 100 -Prefer @('^Instaluj dla wszystkich') }
            'M11' { Invoke-DependencyFailureRow -Row $row -Baseline $baseline }
            'M12' { Invoke-ModifiedFileRow -Row $row -Baseline $baseline }
            'M13' { Invoke-WinGetRow -Row $row -Baseline $baseline }
            'M14' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'machine' -Language 'en-US' -ScalePercent 125 -Prepare @($dotnetId) -Prefer @('^Install for all users') }
            'M15' { Invoke-PathVectorsRow -Row $row -Baseline $baseline }
            'M16' { Invoke-LegacyMigrationRow -Row $row -Baseline $baseline }
            'M17' { Invoke-StateDirectoryOwnershipRow -Row $row -Baseline $baseline }
            # Windows 10 22H2 holds WebView2 after servicing, so its rows start
            # from WebView2 only (clean) or both (.NET prepared), like Windows 11's.
            'W1' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -WithUpgrade -PreservedDependencies @($dotnetId, $webviewId) }
            'W2' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'user' -Prepare @($dotnetId) }
            'W3' { Invoke-OfflineFailureRow -Row $row -Baseline $baseline }
            'W4' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'machine' -Language 'en-US' -ScalePercent 100 -Prefer @('^Install for all users') }
            'W5' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -Prepare @($dotnetId) -NetworkState 'offline' }
            # Server 2019 holds neither runtime, so it alone starts from
            # neither (clean) or .NET only (.NET prepared): every row whose
            # premise is WebView2 absent runs here.
            'S1' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -WithUpgrade -PreservedDependencies @($dotnetId, $webviewId) }
            'S2' { Invoke-OfflineFailureRow -Row $row -Baseline $baseline }
            'S3' { Invoke-InteractiveRow -Row $row -Baseline $baseline -Scope 'machine' -Language 'en-US' -ScalePercent 100 -Prefer @('^Install for all users') }
            'S4' { Invoke-OfflineFailureRow -Row $row -Baseline $baseline -Prepare @($dotnetId) }
            'S5' { Invoke-SilentLifecycleRow -Row $row -Baseline $baseline -Scope 'user' -Prepare @($dotnetId) }
            'S6' { Invoke-InteractiveDependencyRow -Row $row -Baseline $baseline }
        }
        $summary.Add([pscustomobject]@{ row = $row; baseline = $baseline; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail; minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1) })
    }
    catch {
        # A row that throws is a defect in the row or in what it drove, and
        # re-running a row costs minutes of guest time, so the position and
        # the stack are recorded with the message rather than left to a
        # second run to discover.
        $where = "$($_.InvocationInfo.ScriptName):$($_.InvocationInfo.ScriptLineNumber)"
        Write-Host "   ERROR $($_.Exception.Message)"
        Write-Host "         at $where — $($_.InvocationInfo.Line.Trim())"
        $summary.Add([pscustomobject]@{
                row = $row; baseline = $baseline; status = 'ERROR'; pass = 0; warn = 0; fail = 1
                minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1)
                error = $_.Exception.Message; at = $where; statement = $_.InvocationInfo.Line.Trim()
                stack = @($_.ScriptStackTrace -split "`r?`n")
            })
    }
}

}
finally {
    # The open session ends with the run - on the failing paths too, because the
    # lab never expires one and a leaked session holds a VM until somebody ends
    # it by hand.
    if ($null -ne $labSessionId) {
        $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $labSessionId -ResultPath (Join-Path $ResultsRoot "session-close-$labSessionBaseline.json")
    }
}

Write-Host ''
Write-Host "Summary ($ResultsRoot)"
$summary | Format-Table -AutoSize | Out-String | Write-Host
[System.IO.File]::WriteAllText((Join-Path $ResultsRoot 'summary.json'), ($summary | ConvertTo-Json -Depth 4), [System.Text.UTF8Encoding]::new($false))
if (@($summary | Where-Object { $_.status -in @('FAIL', 'ERROR') }).Count -gt 0) { exit 1 }
exit 0
