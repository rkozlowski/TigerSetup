<#
    .SYNOPSIS
    Runs the consolidated feature acceptance of TigerSetup-Validation.md §5.3
    against TigerWinLab: the whole optional-resource model and the custom
    lifecycle actions exercised through one lifecycle on the synthetic
    TigerSetupTestApp package.

    .DESCRIPTION
    Each row is a chain of guest jobs on one baseline. The `lifecycle-user` row
    is the acceptance lifecycle: fresh install with a pre-existing environment
    variable and the preflight action on → upgrade with no explicit choices →
    upgrade changing several choices → an upgrade that fails before its commit
    → a reinstall whose post-install action fails on purpose → the same change
    committed → a reinstall that converges → repair after owned resources and
    the action's cache were damaged → uninstall. After every step the row reads
    the machine as Windows reads it — the shell for shortcuts (target, working
    directory, AppUserModelID), the firewall service for rules, the registry
    for the environment, PATH, the association, the URL scheme, App Paths and
    the context-menu verbs, App Paths through a real shell lookup, the files
    the package's actions wrote and the programs the state directory keeps for
    its uninstall actions — beside the engine's own verify --json and
    inspect --json.

    `standard-user` runs the install as the signed-in standard user, where a
    firewall rule cannot be created and is reported skipped; `machine` runs the
    machine scope, where Send To has no shared folder and everything else lives
    in HKLM and the common folders; `win10` is the minimum functional coverage
    on the Windows 10 baseline.

    The rows need the two versions of packages/test-app built with the current
    release binaries (packages\test-app\Build-Package.ps1); installer file names
    must follow <name>-<version>-Setup.exe.

    .EXAMPLE
    pwsh -File lab\Invoke-FeatureRows.ps1 -InstallerPath artifacts\test-app\TigerSetupTestApp-1.0.0-Setup.exe `
        -UpgradeInstallerPath artifacts\test-app\TigerSetupTestApp-1.1.0-Setup.exe -Rows lifecycle-user
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $InstallerPath,
    [Parameter(Mandatory)] [string] $UpgradeInstallerPath,
    [string[]] $Rows = @('all'),
    [string] $Win11Baseline = 'TigerWinLab-Win11-Clean',
    [string] $Win10Baseline = 'TigerWinLab-Win10-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    # The lab session every operation of this run belongs to; one per baseline
    # is derived from it. Naming one joins an existing session.
    [string] $SessionId,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    # Used only to read what each installer declares, so that a row cannot
    # silently measure an engine older than the one just built.
    [string] $BuilderPath,
    [int] $JobTimeoutMinutes = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

$AllRows = @('lifecycle-user', 'standard-user', 'machine', 'win10')
# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($Rows -contains 'all') { $Rows = $AllRows }
$unknown = @($Rows | Where-Object { $AllRows -notcontains $_ })
if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', '). Known rows: $($AllRows -join ', ')." }

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

function Get-FirstMatch {
    <#
        The first element of a collection that satisfies the filter, or
        $null. Under Set-StrictMode `@()[0]` on an empty result throws, and a
        row must not fail on the absence of evidence it is about to report.
    #>
    param([object[]] $Items, [scriptblock] $Filter)
    $matches = @(@($Items) | Where-Object { $null -ne $_ } | Where-Object $Filter)
    if ($matches.Count -eq 0) { return $null }
    $matches[0]
}

function Get-InstallerVersion {
    param([string] $Path)
    if ((Split-Path -Leaf $Path) -match '-(\d+\.\d+\.\d+)-Setup\.exe$') { return $Matches[1] }
    throw "Cannot read a version from installer name '$Path'; expected <name>-<version>-Setup.exe."
}

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($BuilderPath)) { $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' }
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$UpgradeInstallerPath = (Resolve-Path -LiteralPath $UpgradeInstallerPath).Path
$versionA = Get-InstallerVersion $InstallerPath
$versionB = Get-InstallerVersion $UpgradeInstallerPath
$installerFileA = Split-Path -Leaf $InstallerPath
$installerFileB = Split-Path -Leaf $UpgradeInstallerPath
$stagedA = Join-Path $GuestStageRoot $installerFileA
$stagedB = Join-Path $GuestStageRoot $installerFileB
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) { throw "The builder $BuilderPath is missing; build the release binaries first." }
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
foreach ($installer in @($InstallerPath, $UpgradeInstallerPath)) {
    Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -InstallerPath $installer `
        -Facts (Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $installer)
}
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\features-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'

# The package's test identities, read once from the package itself so that a
# manifest change changes this specification with it.
$productName = [string] $facts.name
$productId = [string] $facts.id
$raw = $facts.raw
$association = Get-FirstMatch -Items @(Get-Member2 $raw 'file_associations') -Filter { $true }
$protocol = Get-FirstMatch -Items @(Get-Member2 $raw 'url_protocols') -Filter { $true }
$firewall = Get-FirstMatch -Items @(Get-Member2 $raw 'firewall_rules') -Filter { $true }
$environmentVariable = Get-FirstMatch -Items @(Get-Member2 $raw 'environment_variables') -Filter { $true }
$verbs = @(Get-Member2 $raw 'context_menu_verbs')
$dependency = Get-FirstMatch -Items @(Get-Member2 $raw 'dependencies') -Filter { $true }
foreach ($pair in @(@('file association', $association), @('URL protocol', $protocol), @('firewall rule', $firewall), @('environment variable', $environmentVariable), @('embedded dependency', $dependency))) {
    if ($null -eq $pair[1]) { throw "The package declares no $($pair[0]); these rows are written for the consolidated TigerSetupTestApp package." }
}
$progId = [string] $association.prog_id
$extension = [string] $association.extensions[0]
$scheme = [string] $protocol.scheme
$handlerProgId = [string] $(if ([string]::IsNullOrWhiteSpace([string] $protocol.prog_id)) { "$($productName -replace '\.', '').$scheme" } else { $protocol.prog_id })
$ruleName = [string] $firewall.name
$variableName = [string] $environmentVariable.name
$filesVerb = [string] (Get-Member2 (Get-FirstMatch -Items $verbs -Filter { [string] $_.target -eq 'files' }) 'verb')
$backgroundVerb = [string] (Get-Member2 (Get-FirstMatch -Items $verbs -Filter { [string] $_.target -eq 'directory-background' }) 'verb')
$prereqDirectory = '%ProgramData%\TigerSetupTestPrereq'
# The package's custom actions, read from the installer like everything else:
# the names by phase, and the packaged programs' hashes, which name the
# directories the state directory keeps the uninstall programs in.
$declaredActions = @(Get-Member2 $raw 'actions')
if ($declaredActions.Count -eq 0) { throw 'The package declares no custom action; these rows are written for the consolidated TigerSetupTestApp package.' }
function Get-ActionSha {
    param([string] $Name)
    $action = Get-FirstMatch -Items $declaredActions -Filter { [string] $_.name -eq $Name }
    if ($null -eq $action) { throw "The package declares no action '$Name'." }
    [string] (Get-Member2 (Get-Member2 $action 'packaged') 'sha256')
}
# Where the actions write their evidence in the guest: outside the install
# root and outside TigerSetup's state, writable by a standard user.
$actionsDirectory = 'C:\ProgramData\TigerSetupTestActions'
$actionRecordFile = "$actionsDirectory\record.txt"
$actionCacheFile = "$actionsDirectory\cache.txt"

Write-Host "TigerWinLab: $labRoot"
Write-Host "Installer A: $InstallerPath ($versionA)"
Write-Host "Installer B: $UpgradeInstallerPath ($versionB)"
Write-Host "Results:     $ResultsRoot"

# ---------------------------------------------------------------------------
# Where a scope keeps things, as the guest spells it
# ---------------------------------------------------------------------------

function Get-ScopeLocations {
    param([ValidateSet('user', 'machine')] [string] $Scope)
    if ($Scope -eq 'machine') {
        return [pscustomobject]@{
            hive = 'HKLM'
            installRoot = "%ProgramFiles%\$productName"
            stateDirectory = "%ProgramData%\TigerSetup\$productId"
            programs = '%ProgramData%\Microsoft\Windows\Start Menu\Programs'
            desktop = '%PUBLIC%\Desktop'
            startup = '%ProgramData%\Microsoft\Windows\Start Menu\Programs\StartUp'
            sendTo = $null
            environmentKey = 'HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
            pathSide = 'machine'
        }
    }
    [pscustomobject]@{
        hive = 'HKCU'
        installRoot = "%LOCALAPPDATA%\Programs\$productName"
        stateDirectory = "%LOCALAPPDATA%\TigerSetup\$productId"
        programs = '%APPDATA%\Microsoft\Windows\Start Menu\Programs'
        desktop = '%USERPROFILE%\Desktop'
        startup = '%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup'
        sendTo = '%APPDATA%\Microsoft\Windows\SendTo'
        environmentKey = 'HKCU\Environment'
        pathSide = 'user'
    }
}

function New-EvidenceRequest {
    <#
        The evidence every step collects for a scope: the engine's logs,
        the install root and the folders, the registry keys of every
        integration, both PATH values, the firewall rule, the shortcuts and
        the environment variable.
    #>
    param([object] $Locations, [string[]] $Logs = @())
    $hive = $Locations.hive
    $shortcuts = @(
        "$($Locations.programs)\$productName.lnk",
        "$($Locations.programs)\$productName Documentation.url",
        "$($Locations.desktop)\$productName.lnk",
        "$($Locations.startup)\$productName Agent.lnk"
    )
    if ($null -ne $Locations.sendTo) { $shortcuts += "$($Locations.sendTo)\$productName.lnk" }
    @{
        logs = @($Logs) + @($actionRecordFile, $actionCacheFile)
        inventory = @($Locations.installRoot, $Locations.stateDirectory, $prereqDirectory, $actionsDirectory)
        registry = @(
            "$hive\Software\Classes\$progId\shell\open\command",
            "$hive\Software\Classes\$extension",
            "$hive\Software\Classes\$extension\OpenWithProgids",
            "$hive\Software\Classes\$scheme",
            "$hive\Software\Classes\$scheme\shell\open\command",
            "$hive\Software\Classes\$handlerProgId",
            "$hive\Software\Classes\*\shell\$filesVerb",
            "$hive\Software\Classes\*\shell\$filesVerb\command",
            "$hive\Software\Classes\Directory\Background\shell\$backgroundVerb\command",
            "$hive\Software\Microsoft\Windows\CurrentVersion\App Paths\$productName.exe",
            "$hive\Software\IT Tiger\$productName\Capabilities",
            "$hive\Software\IT Tiger\$productName\Capabilities\FileAssociations",
            "$hive\Software\IT Tiger\$productName\Capabilities\URLAssociations",
            "$hive\Software\RegisteredApplications",
            "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\UserChoice"
        )
        pathValues = $true
        firewallRules = @($ruleName)
        shortcuts = $shortcuts
        environmentVariables = @($variableName)
    }
}

# ---------------------------------------------------------------------------
# Reading the evidence
# ---------------------------------------------------------------------------

function Get-Evidence {
    <# The guest's own result inside the lab's job document. #>
    param([object] $JobRun)
    Get-Member2 $JobRun.result 'result'
}


function Get-RegistryRecord {
    param([object] $Evidence, [string] $Key)
    Get-FirstMatch -Items @(Get-Member2 $Evidence 'registry') -Filter { [string] $_.requested -eq $Key }
}

function Get-RegistryValue {
    <# A value of a collected key, or $null when the key or the value is absent; '' names the default value. #>
    param([object] $Evidence, [string] $Key, [string] $Name)
    $record = Get-RegistryRecord -Evidence $Evidence -Key $Key
    if ($null -eq $record -or -not [bool] $record.exists) { return $null }
    $values = Get-Member2 $record 'values'
    $property = $(if ($Name -eq '') { '(default)' } else { $Name })
    Get-Member2 $values $property
}

function Test-RegistryKeyExists {
    param([object] $Evidence, [string] $Key)
    $record = Get-RegistryRecord -Evidence $Evidence -Key $Key
    $null -ne $record -and [bool] $record.exists
}

function Get-ShortcutRecord {
    param([object] $Evidence, [string] $Path)
    Get-FirstMatch -Items @(Get-Member2 $Evidence 'shortcuts') -Filter { [string] $_.requested -eq $Path }
}

function Get-FirewallRules {
    param([object] $Evidence)
    $record = Get-FirstMatch -Items @(Get-Member2 $Evidence 'firewallRules') -Filter { [string] $_.requested -eq $ruleName }
    if ($null -eq $record) { return , @() }
    , @(Get-Member2 $record 'rules')
}

function Get-EnvironmentRecord {
    param([object] $Evidence)
    Get-FirstMatch -Items @(Get-Member2 $Evidence 'environmentVariables') -Filter { [string] $_.name -eq $variableName }
}

function Get-InventoryRecord {
    param([object] $Evidence, [string] $Path)
    Get-FirstMatch -Items @(Get-Member2 $Evidence 'inventory') -Filter { [string] $_.requested -eq $Path }
}

function Test-InventoryHasFile {
    param([object] $Evidence, [string] $Path, [string] $Relative)
    $record = Get-InventoryRecord -Evidence $Evidence -Path $Path
    $null -ne $record -and [bool] $record.exists -and (@($record.files) -contains $Relative)
}

function Get-LogLines {
    param([object] $JobRun, [string] $Path)
    $record = Get-Member2 (Get-Evidence $JobRun) 'logs'
    if ($null -eq $record) { return , @() }
    $lines = Get-Member2 $record $Path
    if ($null -eq $lines) { return , @() }
    , @($lines | ForEach-Object { [string] $_ })
}

function Test-LogHasCode {
    param([string[]] $Lines, [string] $Code)
    @($Lines | Where-Object { $_ -match ('\[' + [regex]::Escape($Code) + '\]') }).Count -gt 0
}

function Get-PathEntries {
    param([object] $Evidence, [string] $Side)
    $values = Get-Member2 $Evidence 'pathValues'
    $record = Get-Member2 $values $Side
    if ($null -eq $record) { return , @() }
    , @(@(Get-Member2 $record 'entries') | ForEach-Object { ([string] $_).Trim().TrimEnd('\').ToLowerInvariant() })
}

function Test-LabRunUsable {
    param([object] $LabRun)
    $null -ne $LabRun -and $LabRun.status -eq 'OK'
}

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

function Add-Check {
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [string] $Name, [string] $Code, [bool] $Pass, [string] $Message)
    $Checks.Add((New-TigerSetupCheck -Name "$Prefix/$Name" -Code "$Prefix.$Code" -Status $(if ($Pass) { 'PASS' } else { 'FAIL' }) -Message $Message))
}

function Add-CommandCheck {
    <# One Setup.exe command's exit code and outcome code, from its record. #>
    param(
        [System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $JobRun, [string] $CommandName,
        [int[]] $ExitCodes, [string] $ExpectedCode = '', [string] $ExpectedOutcome = ''
    )
    $command = Get-TigerSetupCommandResult -JobRun $JobRun -CommandName $CommandName
    if ($null -eq $command) {
        Add-Check $Checks $Prefix "$CommandName ran" "$CommandName.ran" $false "The job produced no record of '$CommandName'."
        return $null
    }
    $json = Get-Member2 $command 'json'
    $code = [string] (Get-Member2 $json 'code')
    $outcome = [string] (Get-Member2 $json 'outcome')
    $exit = Get-Member2 $command 'exitCode'
    $pass = ($null -ne $exit) -and ($ExitCodes -contains [int] $exit)
    if ($ExpectedCode -ne '' -and $code -ne $ExpectedCode) { $pass = $false }
    if ($ExpectedOutcome -ne '' -and $outcome -ne $ExpectedOutcome) { $pass = $false }
    Add-Check $Checks $Prefix "$CommandName exit" "$CommandName.exit" $pass `
        "'$CommandName' exited $exit with code '$code' and outcome '$outcome' (expected exit $($ExitCodes -join '/')$(if ($ExpectedCode) { ", code $ExpectedCode" })$(if ($ExpectedOutcome) { ", outcome $ExpectedOutcome" })). $([string] (Get-Member2 $command 'stderr'))".Trim()
    $json
}

function Add-FindingCheck {
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $Outcome, [string] $Code, [bool] $Expected = $true)
    $codes = @(@(Get-Member2 $Outcome 'findings') | ForEach-Object { [string] (Get-Member2 $_ 'code') })
    $present = $codes -contains $Code
    Add-Check $Checks $Prefix "finding $Code $(if ($Expected) { 'reported' } else { 'absent' })" "finding.$Code" ($present -eq $Expected) `
        "The outcome $(if ($present) { 'reports' } else { 'does not report' }) '$Code'; findings: $($codes -join ', ')."
}

function Add-ReadChecks {
    <# verify --json and inspect --json of a step: status, version, options. #>
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $JobRun, [string] $ExpectedVersion, [hashtable] $ExpectedOptions = @{})
    $verify = Get-Member2 (Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'verify') 'json'
    $inspect = Get-Member2 (Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'inspect') 'json'
    $status = [string] (Get-Member2 $verify 'status')
    if ($ExpectedVersion -eq '') {
        Add-Check $Checks $Prefix 'verify reports absence' 'verify.absent' ($status -eq 'not_installed') "verify --json status '$status'."
        Add-Check $Checks $Prefix 'no installation recorded' 'inspect.absent' ($null -eq (Get-Member2 $inspect 'installation')) "inspect --json installation: $(Get-Member2 $inspect 'installation' | ConvertTo-Json -Compress -Depth 3)."
        return
    }
    Add-Check $Checks $Prefix 'verify ok' 'verify.ok' ($status -eq 'ok') "verify --json status '$status'; findings: $(@(Get-Member2 $verify 'findings') | ConvertTo-Json -Compress -Depth 4)."
    $installed = [string] (Get-Member2 (Get-Member2 $inspect 'installation') 'version')
    Add-Check $Checks $Prefix "installed version $ExpectedVersion" 'inspect.version' ($installed -eq $ExpectedVersion) "inspect --json reports version '$installed'."
    Add-Check $Checks $Prefix 'no open transaction' 'inspect.transaction' ($null -eq (Get-Member2 $inspect 'transaction')) 'inspect --json reports no open transaction.'
    $options = Get-Member2 (Get-Member2 $inspect 'owned') 'options'
    foreach ($name in $ExpectedOptions.Keys) {
        $actual = Get-Member2 $options $name
        $expected = $ExpectedOptions[$name]
        $same = $(if ($expected -is [bool]) { ($actual -is [bool]) -and ($actual -eq $expected) } else { [string] $actual -eq [string] $expected })
        Add-Check $Checks $Prefix "option $name = $expected" "option.$name" $same "inspect --json records option '$name' as $(ConvertTo-Json $actual -Compress) (type-preserving); expected $(ConvertTo-Json $expected -Compress)."
    }
}

function Add-ResourceChecks {
    <#
        The machine as Windows reads it, against the selection in effect.
        $Selected: path-mode, and the booleans; $Elevated: whether the run
        could write the firewall.
    #>
    param(
        [System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $JobRun,
        [object] $Locations, [hashtable] $Selected, [bool] $Elevated = $true, [bool] $ShellProbe = $true, [string] $ExpectedVariableAfterRemoval = $null
    )
    $evidence = Get-Evidence $JobRun
    $root = [string] (Get-InventoryRecord -Evidence $evidence -Path $Locations.installRoot).path
    $rootLower = $root.TrimEnd('\').ToLowerInvariant()
    $exe = "$root\bin\$productName.exe"
    $command = "`"$exe`" `"%1`""
    $hive = $Locations.hive

    # PATH, as the environment block holds it.
    $entries = Get-PathEntries -Evidence $evidence -Side $Locations.pathSide
    $mode = [string] $Selected['path-mode']
    $binOn = $entries -contains "$rootLower\bin"
    $toolsOn = $entries -contains "$rootLower\tools"
    Add-Check $Checks $Prefix "PATH for mode $mode" 'path.mode' (($binOn -eq ($mode -ne 'none')) -and ($toolsOn -eq ($mode -eq 'tools'))) "$($Locations.pathSide) PATH has bin=$binOn tools=$toolsOn for mode '$mode'."

    # Components.
    $extras = Test-InventoryHasFile -Evidence $evidence -Path $Locations.installRoot -Relative 'extras\notes.txt'
    Add-Check $Checks $Prefix "extras component $(if ($Selected['extras']) { 'installed' } else { 'absent' })" 'component.extras' ($extras -eq [bool] $Selected['extras']) "extras\notes.txt present=$extras."
    $tools = Test-InventoryHasFile -Evidence $evidence -Path $Locations.installRoot -Relative 'tools\tsta-tool.exe'
    Add-Check $Checks $Prefix "tools component $(if ($mode -eq 'tools') { 'installed' } else { 'absent' })" 'component.tools' ($tools -eq ($mode -eq 'tools')) "tools\tsta-tool.exe present=$tools."

    # Shortcuts in the real known folders.
    $startMenu = Get-ShortcutRecord -Evidence $evidence -Path "$($Locations.programs)\$productName.lnk"
    $startMenuOk = $null -ne $startMenu -and [bool] $startMenu.exists -and ([string] $startMenu.target).ToLowerInvariant() -eq $exe.ToLowerInvariant()
    Add-Check $Checks $Prefix 'Start Menu link targets the application' 'shortcut.startmenu' $startMenuOk "$($Locations.programs)\$productName.lnk -> $(Get-Member2 $startMenu 'target')."
    $workingOk = $null -ne $startMenu -and ([string] (Get-Member2 $startMenu 'workingDirectory')).TrimEnd('\').ToLowerInvariant() -eq "$rootLower\data"
    Add-Check $Checks $Prefix 'Start Menu link working directory' 'shortcut.workingdirectory' $workingOk "Working directory: $(Get-Member2 $startMenu 'workingDirectory'); expected $root\data."
    $aumidOk = $null -ne $startMenu -and [string] (Get-Member2 $startMenu 'appUserModelId') -eq 'ITTiger.TigerSetupTestApp'
    Add-Check $Checks $Prefix 'Start Menu link AppUserModelID' 'shortcut.aumid' $aumidOk "AppUserModelID: '$(Get-Member2 $startMenu 'appUserModelId')' as the shell reads it."
    $documentation = Get-ShortcutRecord -Evidence $evidence -Path "$($Locations.programs)\$productName Documentation.url"
    Add-Check $Checks $Prefix 'documentation URL shortcut' 'shortcut.url' ($null -ne $documentation -and [bool] $documentation.exists -and [string] (Get-Member2 $documentation 'url') -eq 'https://ittiger.example/tigersetup-test-app') "URL: $(Get-Member2 $documentation 'url')."
    $startup = Get-ShortcutRecord -Evidence $evidence -Path "$($Locations.startup)\$productName Agent.lnk"
    $startupExists = $null -ne $startup -and [bool] $startup.exists
    Add-Check $Checks $Prefix "Startup link $(if ($Selected['startup']) { 'present' } else { 'absent' })" 'shortcut.startup' ($startupExists -eq [bool] $Selected['startup'] -and (-not $startupExists -or [string] (Get-Member2 $startup 'arguments') -eq '--agent')) "Startup link present=$startupExists arguments='$(Get-Member2 $startup 'arguments')'."
    $desktop = Get-ShortcutRecord -Evidence $evidence -Path "$($Locations.desktop)\$productName.lnk"
    Add-Check $Checks $Prefix "desktop link $(if ($Selected['desktop-shortcut']) { 'present' } else { 'absent' })" 'shortcut.desktop' (($null -ne $desktop -and [bool] $desktop.exists) -eq [bool] $Selected['desktop-shortcut']) "Desktop link present=$($null -ne $desktop -and [bool] $desktop.exists)."
    if ($null -ne $Locations.sendTo) {
        $sendTo = Get-ShortcutRecord -Evidence $evidence -Path "$($Locations.sendTo)\$productName.lnk"
        Add-Check $Checks $Prefix "Send To link $(if ($Selected['send-to']) { 'present' } else { 'absent' })" 'shortcut.sendto' (($null -ne $sendTo -and [bool] $sendTo.exists) -eq [bool] $Selected['send-to']) "Send To link present=$($null -ne $sendTo -and [bool] $sendTo.exists)."
    }

    # Environment variable, as the registry holds it.
    $variable = Get-EnvironmentRecord -Evidence $evidence
    $side = Get-Member2 $variable $Locations.pathSide
    $rawValue = [string] (Get-Member2 $side 'raw')
    if ($Selected['environment']) {
        Add-Check $Checks $Prefix 'environment variable set to the install root' 'environment.set' ($rawValue.TrimEnd('\').ToLowerInvariant() -eq $rootLower -and [string] (Get-Member2 $side 'kind') -eq 'ExpandString') "$variableName = '$rawValue' ($(Get-Member2 $side 'kind')) in the $($Locations.pathSide) environment."
    }
    elseif ($null -ne $ExpectedVariableAfterRemoval) {
        Add-Check $Checks $Prefix 'environment variable restored' 'environment.restored' ($rawValue -eq $ExpectedVariableAfterRemoval) "$variableName = '$rawValue'; expected the pre-installation value '$ExpectedVariableAfterRemoval'."
    }
    else {
        Add-Check $Checks $Prefix 'environment variable absent' 'environment.absent' ([string]::IsNullOrEmpty($rawValue)) "$variableName = '$rawValue'."
    }

    # File association: a handler, never the default.
    $associationCommand = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\$progId\shell\open\command" -Name ''
    $openWith = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\$extension\OpenWithProgids" -Name $progId
    if ($Selected['file-association']) {
        Add-Check $Checks $Prefix 'ProgID opens with the application' 'association.command' ([string] $associationCommand -eq $command) "$progId\shell\open\command = '$associationCommand'."
        Add-Check $Checks $Prefix 'extension lists the ProgID in Open With' 'association.openwith' ($null -ne $openWith) "$extension\OpenWithProgids\$progId present=$($null -ne $openWith)."
        Add-Check $Checks $Prefix 'capability registration names the extension' 'association.capability' ([string] (Get-RegistryValue -Evidence $evidence -Key "$hive\Software\IT Tiger\$productName\Capabilities\FileAssociations" -Name $extension) -eq $progId) 'Capabilities\FileAssociations maps the extension to the ProgID.'
    }
    else {
        Add-Check $Checks $Prefix 'association absent' 'association.absent' ($null -eq $associationCommand -and $null -eq $openWith) "ProgID command='$associationCommand' OpenWithProgids present=$($null -ne $openWith)."
    }
    $extensionDefault = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\$extension" -Name ''
    Add-Check $Checks $Prefix 'the default handler was not taken over' 'association.default.untouched' ([string]::IsNullOrEmpty([string] $extensionDefault) -and -not (Test-RegistryKeyExists -Evidence $evidence -Key "HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\UserChoice")) "$extension (default)='$extensionDefault'; no UserChoice was written."

    # URL protocol resolves to the application.
    $schemeCommand = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\$scheme\shell\open\command" -Name ''
    $urlProtocol = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\$scheme" -Name 'URL Protocol'
    if ($Selected['url-protocol']) {
        Add-Check $Checks $Prefix "$scheme`: resolves to the application" 'protocol.command' ([string] $schemeCommand -eq $command -and $null -ne $urlProtocol) "$scheme\shell\open\command = '$schemeCommand'; URL Protocol present=$($null -ne $urlProtocol)."
        Add-Check $Checks $Prefix 'handler ProgID registered' 'protocol.handler' (Test-RegistryKeyExists -Evidence $evidence -Key "$hive\Software\Classes\$handlerProgId") "$handlerProgId present."
        Add-Check $Checks $Prefix 'capability registration names the scheme' 'protocol.capability' ([string] (Get-RegistryValue -Evidence $evidence -Key "$hive\Software\IT Tiger\$productName\Capabilities\URLAssociations" -Name $scheme) -eq $handlerProgId) 'Capabilities\URLAssociations maps the scheme to the handler.'
    }
    else {
        Add-Check $Checks $Prefix 'protocol absent' 'protocol.absent' ($null -eq $schemeCommand -and -not (Test-RegistryKeyExists -Evidence $evidence -Key "$hive\Software\Classes\$handlerProgId")) "$scheme command='$schemeCommand'."
    }
    $registered = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\RegisteredApplications" -Name $productName
    $capabilitiesWanted = [bool] $Selected['file-association'] -or [bool] $Selected['url-protocol']
    Add-Check $Checks $Prefix "RegisteredApplications entry $(if ($capabilitiesWanted) { 'present' } else { 'absent' })" 'capabilities.registered' (($null -ne $registered) -eq $capabilitiesWanted) "RegisteredApplications\$productName = '$registered'."

    # App Paths: the registration, and a real shell lookup by bare name.
    $appPath = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Microsoft\Windows\CurrentVersion\App Paths\$productName.exe" -Name ''
    Add-Check $Checks $Prefix 'App Paths names the executable' 'apppaths.registered' (([string] $appPath).ToLowerInvariant() -eq $exe.ToLowerInvariant()) "App Paths\$productName.exe = '$appPath'."
    if ($ShellProbe) {
        $probe = Get-Member2 (Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'shell-probe') 'json'
        Add-Check $Checks $Prefix 'shell resolves the bare executable name' 'apppaths.resolves' ($null -ne $probe -and [bool] (Get-Member2 $probe 'resolved')) "ShellExecute('$productName.exe') -> Win32 error $(Get-Member2 $probe 'errorCode') ($(Get-Member2 $probe 'message'))."
    }

    # Classic context-menu verbs.
    $filesCommand = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\*\shell\$filesVerb\command" -Name ''
    $backgroundCommand = Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\Directory\Background\shell\$backgroundVerb\command" -Name ''
    if ($Selected['context-menu']) {
        Add-Check $Checks $Prefix 'files verb registered' 'contextmenu.files' ([string] $filesCommand -eq $command -and [string] (Get-RegistryValue -Evidence $evidence -Key "$hive\Software\Classes\*\shell\$filesVerb" -Name '') -eq "Open with $productName") "*\shell\$filesVerb\command = '$filesCommand'."
        Add-Check $Checks $Prefix 'directory background verb registered' 'contextmenu.background' ([string] $backgroundCommand -eq "`"$exe`" `"%V`"") "Directory\Background\shell\$backgroundVerb\command = '$backgroundCommand'."
    }
    else {
        Add-Check $Checks $Prefix 'verbs absent' 'contextmenu.absent' ($null -eq $filesCommand -and $null -eq $backgroundCommand) "files='$filesCommand' background='$backgroundCommand'."
    }

    # Firewall rule, as the firewall service answers.
    $rules = Get-FirewallRules -Evidence $evidence
    if ($Selected['firewall'] -and $Elevated) {
        $rule = Get-FirstMatch -Items @($rules) -Filter { $true }
        $ruleOk = $rules.Count -eq 1 -and $null -ne $rule -and ([string] $rule.program).ToLowerInvariant() -eq $exe.ToLowerInvariant() -and [string] $rule.direction -eq 'inbound' -and [string] $rule.action -eq 'allow' -and [string] $rule.protocol -eq 'TCP' -and [string] $rule.localPort -eq '47110' -and [bool] $rule.enabled -and [string] $rule.group -eq "TigerSetup: $productName"
        Add-Check $Checks $Prefix 'firewall rule exists as declared' 'firewall.rule' $ruleOk "$($rules.Count) rule(s) named '$ruleName': $($rules | ConvertTo-Json -Compress -Depth 3)."
    }
    else {
        Add-Check $Checks $Prefix 'firewall rule absent' 'firewall.absent' ($rules.Count -eq 0) "$($rules.Count) rule(s) named '$ruleName'."
    }

    # The embedded prerequisite is on the machine.
    Add-Check $Checks $Prefix 'prerequisite installed by the embedded installer' 'prereq.present' (Test-InventoryHasFile -Evidence $evidence -Path $prereqDirectory -Relative '1.0.0\prereq.txt') "$prereqDirectory\1.0.0\prereq.txt present."
}

function Add-ActionChecks {
    <#
        What the package's custom actions did in one step, from three kinds
        of evidence: the outcome document's actions[] (name, status, operation
        and phase per action, in order), the files the programs wrote (the
        markers and the cache, as the guest's file system holds them), and
        the programs the state directory keeps for the uninstall actions.
        $Actions is the expected actions[] as name=status pairs, in order;
        $CacheVersion the version the cache must be for, or '' for no cache;
        $Markers the marker files that must exist, and $AbsentMarkers those
        that must not.
    #>
    param(
        [System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $JobRun, [object] $Outcome, [object] $Locations,
        [string[]] $Actions = @(), [string] $CacheVersion = '', [string[]] $Markers = @(), [string[]] $AbsentMarkers = @(), [string] $Operation = '', [bool] $StoredPrograms = $true
    )
    $evidence = Get-Evidence $JobRun
    # The outcome omits `actions` when the run executed none, so the member
    # reads as $null, which `@()` would turn into one null element.
    $executed = @(@(Get-Member2 $Outcome 'actions') | Where-Object { $null -ne $_ })
    $reported = @($executed | ForEach-Object { "$([string] (Get-Member2 $_ 'name'))=$([string] (Get-Member2 $_ 'status'))" })
    Add-Check $Checks $Prefix "actions run: $($Actions -join ', ')" 'actions.run' ((@($reported) -join '|') -eq (@($Actions) -join '|')) "The outcome reports actions [$($reported -join ', ')]; expected [$($Actions -join ', ')]."
    if ($Operation -ne '') {
        $operations = @($executed | ForEach-Object { [string] (Get-Member2 $_ 'operation') } | Select-Object -Unique)
        Add-Check $Checks $Prefix "actions ran on $Operation" 'actions.operation' ($Actions.Count -eq 0 -or ($operations.Count -eq 1 -and $operations[0] -eq $Operation)) "The actions report operation(s) [$($operations -join ', ')]."
    }
    $cacheLines = Get-LogLines -JobRun $JobRun -Path $actionCacheFile
    if ($CacheVersion -ne '') {
        Add-Check $Checks $Prefix "cache built for $CacheVersion" 'actions.cache' ($cacheLines.Count -gt 0 -and $cacheLines[0] -eq "TigerSetupTestApp cache for $CacheVersion") "$actionCacheFile holds [$($cacheLines -join ' / ')]."
    }
    else {
        Add-Check $Checks $Prefix 'cache absent' 'actions.cache.absent' ($cacheLines.Count -eq 0) "$actionCacheFile holds [$($cacheLines -join ' / ')]."
    }
    foreach ($marker in $Markers) {
        Add-Check $Checks $Prefix "marker $marker present" "actions.marker.$($marker -replace '[^A-Za-z0-9]+', '.')" (Test-InventoryHasFile -Evidence $evidence -Path $actionsDirectory -Relative $marker) "$actionsDirectory\$marker."
    }
    foreach ($marker in $AbsentMarkers) {
        Add-Check $Checks $Prefix "marker $marker absent" "actions.nomarker.$($marker -replace '[^A-Za-z0-9]+', '.')" (-not (Test-InventoryHasFile -Evidence $evidence -Path $actionsDirectory -Relative $marker)) "$actionsDirectory\$marker."
    }
    if ($StoredPrograms) {
        foreach ($pair in @(@('clear-cache', 'clear-cache.cmd'), @('farewell', 'TigerSetupTestAction.exe'))) {
            $relative = "actions\$(Get-ActionSha $pair[0])\$($pair[1])"
            Add-Check $Checks $Prefix "stored program of $($pair[0])" "actions.stored.$($pair[0])" (Test-InventoryHasFile -Evidence $evidence -Path $Locations.stateDirectory -Relative $relative) "$($Locations.stateDirectory)\$relative."
        }
    }
}

function Add-AbsenceChecks {
    <# After an uninstall: every owned resource gone, the pre-existing variable back, the prerequisite kept. #>
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [object] $JobRun, [object] $Locations, [string] $ExpectedVariable)
    $evidence = Get-Evidence $JobRun
    $hive = $Locations.hive
    $rootRecord = Get-InventoryRecord -Evidence $evidence -Path $Locations.installRoot
    Add-Check $Checks $Prefix 'install root removed' 'root.removed' ($null -ne $rootRecord -and -not [bool] $rootRecord.exists) "$($Locations.installRoot) exists=$(Get-Member2 $rootRecord 'exists')."
    $stateRecord = Get-InventoryRecord -Evidence $evidence -Path $Locations.stateDirectory
    Add-Check $Checks $Prefix 'state directory removed' 'state.removed' ($null -ne $stateRecord -and -not [bool] $stateRecord.exists) "$($Locations.stateDirectory) exists=$(Get-Member2 $stateRecord 'exists')."
    foreach ($key in @("$hive\Software\Classes\$progId\shell\open\command", "$hive\Software\Classes\$extension\OpenWithProgids", "$hive\Software\Classes\$scheme", "$hive\Software\Classes\$handlerProgId", "$hive\Software\Classes\*\shell\$filesVerb", "$hive\Software\Classes\Directory\Background\shell\$backgroundVerb\command", "$hive\Software\Microsoft\Windows\CurrentVersion\App Paths\$productName.exe", "$hive\Software\IT Tiger\$productName\Capabilities")) {
        Add-Check $Checks $Prefix "removed $key" ("removed." + ($key -replace '[^A-Za-z0-9]+', '.').ToLowerInvariant()) (-not (Test-RegistryKeyExists -Evidence $evidence -Key $key)) "$key exists=$(Test-RegistryKeyExists -Evidence $evidence -Key $key)."
    }
    Add-Check $Checks $Prefix 'RegisteredApplications entry removed' 'removed.registeredapplications' ($null -eq (Get-RegistryValue -Evidence $evidence -Key "$hive\Software\RegisteredApplications" -Name $productName)) 'RegisteredApplications no longer names the product.'
    $probe = Get-Member2 (Get-TigerSetupCommandResult -JobRun $JobRun -CommandName 'shell-probe') 'json'
    if ($null -ne $probe) {
        Add-Check $Checks $Prefix 'shell no longer resolves the bare executable name' 'removed.apppaths.resolves' (-not [bool] (Get-Member2 $probe 'resolved')) "ShellExecute('$productName.exe') -> Win32 error $(Get-Member2 $probe 'errorCode') ($(Get-Member2 $probe 'message'))."
    }
    foreach ($shortcut in @(Get-Member2 $evidence 'shortcuts')) {
        if ($null -eq $shortcut) { continue }
        Add-Check $Checks $Prefix "removed $($shortcut.requested)" ("removed.shortcut." + (Split-Path -Leaf ([string] $shortcut.requested)) -replace '[^A-Za-z0-9]+', '.') (-not [bool] $shortcut.exists) "$($shortcut.requested) exists=$($shortcut.exists)."
    }
    $rules = Get-FirewallRules -Evidence $evidence
    Add-Check $Checks $Prefix 'firewall rule removed' 'removed.firewall' ($rules.Count -eq 0) "$($rules.Count) rule(s) named '$ruleName' remain."
    $variable = Get-EnvironmentRecord -Evidence $evidence
    $rawValue = [string] (Get-Member2 (Get-Member2 $variable $Locations.pathSide) 'raw')
    Add-Check $Checks $Prefix 'environment variable restored or removed' 'environment.final' ($rawValue -eq $ExpectedVariable) "$variableName = '$rawValue'; expected '$ExpectedVariable'."
    $entries = Get-PathEntries -Evidence $evidence -Side $Locations.pathSide
    Add-Check $Checks $Prefix 'nothing of the product on PATH' 'removed.path' (@($entries | Where-Object { $_ -like "*\$($productName.ToLowerInvariant())\*" }).Count -eq 0) "$($Locations.pathSide) PATH: $($entries -join ';')."
    Add-Check $Checks $Prefix 'prerequisite outlives the product' 'prereq.kept' (Test-InventoryHasFile -Evidence $evidence -Path $prereqDirectory -Relative '1.0.0\prereq.txt') 'The embedded prerequisite is a requirement, not a resource, and stays.'
}

# ---------------------------------------------------------------------------
# Steps
# ---------------------------------------------------------------------------

function Invoke-Step {
    param(
        [string] $Row, [string] $Baseline, [string] $Suffix, [object] $Locations, [string] $RunAs,
        [object[]] $Commands, [string[]] $Logs = @(), [string[]] $PayloadFiles = @(), [switch] $FromBaseline, [object[]] $Stage = @()
    )
    $request = New-EvidenceRequest -Locations $Locations -Logs $Logs
    $request.commands = @($Commands)
    $request.runAs = $RunAs
    if ($Stage.Count -gt 0) { $request.stage = @($Stage) }
    Write-Host "  $Suffix"
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline
    Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $request -PayloadFiles $PayloadFiles -Name ("ts-$Suffix".ToLowerInvariant()) @policy `
        -ResultPath (Join-Path $ResultsRoot "runs\$Row-$Suffix.json") -OutputRoot $labOutputRoot -TimeoutMinutes $JobTimeoutMinutes
}

function New-SetupCommand {
    param([string] $Name, [string] $Executable, [string[]] $Arguments, [int] $Timeout = 600)
    @{ name = $Name; executable = $Executable; arguments = @($Arguments); timeoutSeconds = $Timeout }
}

function New-ReadCommands {
    <#
        verify and inspect after every step, plus — where -ProbeRunAs names a
        session — the shell probe: ShellExecute of the bare executable name,
        which is what App Paths is for. It runs as its own command rather than
        as a collector so that it runs in the session whose registry the
        installer wrote, and only where Windows consults that registry at all:
        an elevated process ignores HKCU\...\App Paths (a per-user hive must
        not redirect an administrator), so a per-user registration is proved
        from an unelevated session and a machine one from any.
    #>
    param([string] $Executable, [string] $Scope, [string] $ProbeRunAs = '')
    $commands = @(
        (New-SetupCommand -Name 'verify' -Executable $Executable -Arguments @('verify', '--json', '--scope', $Scope) -Timeout 300),
        (New-SetupCommand -Name 'inspect' -Executable $Executable -Arguments @('inspect', '--json', '--scope', $Scope) -Timeout 300)
    )
    if (-not [string]::IsNullOrWhiteSpace($ProbeRunAs)) {
        $commands += @(New-ShellProbeCommand -Name "$productName.exe" -RunAs $ProbeRunAs)
    }
    $commands
}

function New-ShellProbeCommand {
    <#
        A Windows PowerShell command that asks the shell to start a bare
        executable name and prints one JSON object: resolved, errorCode,
        message. A name the shell found but could not run (the synthetic
        executable is not a Win32 program: 193 or 216) was resolved; error 2
        means the shell never found it. A program the shell could run is
        killed at once.
    #>
    param([string] $Name, [string] $RunAs)
    $script = @"
`$n = '$Name'; `$r = [ordered]@{ name = `$n; resolved = `$false; errorCode = 0; message = '' }
try {
    `$p = New-Object System.Diagnostics.ProcessStartInfo; `$p.FileName = `$n; `$p.UseShellExecute = `$true; `$p.WindowStyle = 'Hidden'
    `$s = [System.Diagnostics.Process]::Start(`$p); if (`$s) { try { `$s.Kill() } catch { } }
    `$r.resolved = `$true; `$r.message = 'started'
}
catch {
    `$i = `$_.Exception
    while (`$i -and -not (`$i -is [System.ComponentModel.Win32Exception]) -and `$i.InnerException) { `$i = `$i.InnerException }
    `$c = `$(if (`$i -is [System.ComponentModel.Win32Exception]) { [int] `$i.NativeErrorCode } else { -1 })
    `$r.errorCode = `$c; `$r.message = [string] `$i.Message; `$r.resolved = (`$c -ne 2 -and `$c -ne -1)
}
[pscustomobject] `$r | ConvertTo-Json -Compress
"@
    $encoded = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($script))
    @{ name = 'shell-probe'; runAs = $RunAs; executable = 'powershell.exe'; arguments = @('-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded); timeoutSeconds = 120 }
}

function ConvertTo-OptionArguments {
    param([hashtable] $Options)
    $arguments = @()
    foreach ($name in @($Options.Keys | Sort-Object)) {
        $value = $Options[$name]
        $arguments += @('--option', $name, $(if ($value -is [bool]) { $(if ($value) { 'on' } else { 'off' }) } else { [string] $value }))
    }
    $arguments
}

$defaults = @{ 'path-mode' = 'command'; 'desktop-shortcut' = $false; 'startup' = $true; 'send-to' = $false; 'file-association' = $true; 'url-protocol' = $true; 'context-menu' = $true; 'environment' = $true; 'firewall' = $true; 'extras' = $false; 'preflight' = $false; 'fail-action' = $false }

function Merge-Selection {
    param([hashtable] $Base, [hashtable] $Changes)
    $merged = @{}
    foreach ($key in $Base.Keys) { $merged[$key] = $Base[$key] }
    foreach ($key in $Changes.Keys) { $merged[$key] = $Changes[$key] }
    $merged
}

function Get-AppliedSequence {
    <# The journal sequence of the applied operation of `Kind` on `Target`, from the engine log, or -1. #>
    param([string[]] $Lines, [string] $Kind, [string] $Target)
    $needle = "kind=$Kind target=$Target"
    foreach ($line in $Lines) {
        if ($line -match '\[operation_applied\] sequence=(\d+) ' -and $line.EndsWith($needle)) { return [int] $Matches[1] }
    }
    -1
}

function Add-UninstallOrderChecks {
    <#
        The uninstall log proves the phase order: the pre-uninstall action is
        the first operation, and the post-uninstall action follows the
        removal of the install root.
    #>
    param([System.Collections.Generic.List[object]] $Checks, [string] $Prefix, [string[]] $Lines)
    $first = Get-AppliedSequence -Lines $Lines -Kind 'run_action' -Target 'clear-cache'
    $last = Get-AppliedSequence -Lines $Lines -Kind 'run_action' -Target 'farewell'
    $root = Get-AppliedSequence -Lines $Lines -Kind 'remove_directory' -Target ''
    Add-Check $Checks $Prefix 'pre-uninstall action is the first operation' 'actions.order.pre' ($first -eq 1) "clear-cache applied as operation $first."
    Add-Check $Checks $Prefix 'post-uninstall action follows the removal of the install root' 'actions.order.post' ($root -gt 0 -and $last -gt $root) "farewell applied as operation $last, the install root removed as operation $root."
}

# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------

function Invoke-LifecycleRow {
    param([string] $Row, [string] $Baseline, [string] $Scope, [string] $RunAs, [bool] $Elevated, [switch] $Short)

    $checks = [System.Collections.Generic.List[object]]::new()
    $locations = Get-ScopeLocations -Scope $Scope
    $previousVariable = 'C:\Elsewhere'
    $scopeArguments = @('--scope', $Scope)
    $evidenceSets = [ordered]@{}
    # Where the shell probe answers (New-ReadCommands): a machine registration
    # from any session, a per-user one only from an unelevated session.
    $probeRunAs = $(if ($Scope -eq 'machine' -or -not $Elevated) { $RunAs } else { '' })

    # The machine row selects Send To, which machine scope has no folder for:
    # the run must report shortcut_location_unavailable and install the rest.
    # The full lifecycle turns the preflight action on, so that the
    # pre-install phase is exercised on the install and the upgrade.
    $initialChanges = $(if ($Scope -eq 'machine') { @{ 'send-to' = $true } } else { @{} })
    if (-not $Short) { $initialChanges['preflight'] = $true }
    $initial = Merge-Selection -Base $defaults -Changes $initialChanges
    # `@(...)` around the conditional: an `if` unrolls its one-element result
    # to a string, and a string + an array is string concatenation.
    $preflightMarkers = @($(if ($Short) { @() } else { 'preflight.txt' }))
    $preflightActions = @($(if ($Short) { @() } else { 'preflight=completed' }))

    # 1. Fresh install, over a pre-existing environment variable.
    $installLog = Join-Path $GuestStageRoot 'install-a.log'
    $seed = New-SetupCommand -Name 'seed-variable' -Executable 'reg.exe' -Arguments @('add', $locations.environmentKey, '/v', $variableName, '/t', 'REG_SZ', '/d', $previousVariable, '/f') -Timeout 60
    $install = New-SetupCommand -Name 'install' -Executable $stagedA -Arguments (@('install', '--quiet', '--json', '--log', $installLog) + $scopeArguments + (ConvertTo-OptionArguments $initialChanges))
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'install' -Locations $locations -RunAs $RunAs -FromBaseline `
        -Stage @(@{ source = $installerFileA; destination = $stagedA }, @{ source = $installerFileB; destination = $stagedB }) -PayloadFiles @($InstallerPath, $UpgradeInstallerPath) `
        -Commands (@($seed, $install) + (New-ReadCommands -Executable $stagedA -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($installLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'install' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'install' -JobRun $step -CommandName 'install' -ExitCodes @(0) -ExpectedCode 'ok' -ExpectedOutcome 'installed'
    Add-ReadChecks -Checks $checks -Prefix 'install' -JobRun $step -ExpectedVersion $versionA -ExpectedOptions $initial
    Add-ResourceChecks -Checks $checks -Prefix 'install' -JobRun $step -Locations $locations -Selected $initial -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    $lines = Get-LogLines -JobRun $step -Path $installLog
    foreach ($code in @('dependency_extracted', 'dependency_installed', 'dependency_verified')) {
        Add-Check $checks 'install' "log records $code" "log.$code" (Test-LogHasCode $lines $code) "The install log $(if (Test-LogHasCode $lines $code) { 'records' } else { 'does not record' }) [$code]."
    }
    Add-Check $checks 'install' 'log records no download' 'log.no_download' (-not (Test-LogHasCode $lines 'dependency_downloading')) 'The embedded prerequisite was not downloaded.'
    $dependency = Get-FirstMatch -Items @(Get-Member2 $outcome 'dependencies') -Filter { $true }
    Add-Check $checks 'install' 'outcome records the prerequisite as installed from the payload' 'dependency.installed' ($null -ne $dependency -and [string] (Get-Member2 $dependency 'status') -eq 'installed' -and ([string] (Get-Member2 $dependency 'url')).StartsWith('payload:')) "dependencies[0]: $($dependency | ConvertTo-Json -Compress -Depth 3)."
    if (-not $Elevated) { Add-FindingCheck -Checks $checks -Prefix 'install' -Outcome $outcome -Code 'firewall_rule_skipped_unelevated' }
    if ($Scope -eq 'machine') { Add-FindingCheck -Checks $checks -Prefix 'install' -Outcome $outcome -Code 'shortcut_location_unavailable' }
    Add-ActionChecks -Checks $checks -Prefix 'install' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('build-cache=completed')) -CacheVersion $versionA -Markers $preflightMarkers -AbsentMarkers @('pre-uninstall.txt', 'post-uninstall.txt', 'failed-action.txt') -Operation 'install'
    Add-Check $checks 'install' 'log records the stored uninstall programs' 'log.action_program_stored' (Test-LogHasCode $lines 'action_program_stored') 'The install log records [action_program_stored].'
    $evidenceSets['install'] = Get-Evidence $step

    if ($Short) {
        # The short lifecycle: upgrade, then uninstall.
        $upgradeLog = Join-Path $GuestStageRoot 'upgrade-b.log'
        $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'upgrade' -Locations $locations -RunAs $RunAs `
            -Commands (@((New-SetupCommand -Name 'upgrade' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $upgradeLog) + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($upgradeLog)
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'upgrade' -LabRun $step) { $checks.Add($check) }
        if (Test-LabRunUsable $step) {
            $outcome = Add-CommandCheck -Checks $checks -Prefix 'upgrade' -JobRun $step -CommandName 'upgrade' -ExitCodes @(0) -ExpectedCode 'ok' -ExpectedOutcome 'installed'
            Add-ReadChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $initial
            Add-ResourceChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -Locations $locations -Selected $initial -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
            Add-ActionChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -Outcome $outcome -Locations $locations -Actions @('build-cache=completed') -CacheVersion $versionB -Operation 'upgrade'
            $lines = Get-LogLines -JobRun $step -Path $upgradeLog
            Add-Check $checks 'upgrade' 'prerequisite detected, not extracted again' 'log.dependency_detected' ((Test-LogHasCode $lines 'dependency_detected') -and -not (Test-LogHasCode $lines 'dependency_extracted')) 'The upgrade detected the prerequisite and extracted nothing.'
        }
        $uninstallLog = Join-Path $GuestStageRoot 'uninstall.log'
        $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'uninstall' -Locations $locations -RunAs $RunAs `
            -Commands (@((New-SetupCommand -Name 'uninstall' -Executable $stagedB -Arguments (@('uninstall', '--quiet', '--json', '--log', $uninstallLog) + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($uninstallLog)
        foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'uninstall' -LabRun $step) { $checks.Add($check) }
        if (Test-LabRunUsable $step) {
            $outcome = Add-CommandCheck -Checks $checks -Prefix 'uninstall' -JobRun $step -CommandName 'uninstall' -ExitCodes @(0) -ExpectedOutcome 'uninstalled'
            Add-ReadChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -ExpectedVersion ''
            Add-AbsenceChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -Locations $locations -ExpectedVariable $previousVariable
            Add-ActionChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -Outcome $outcome -Locations $locations -Actions @('clear-cache=completed', 'farewell=completed') -CacheVersion '' -Markers @('pre-uninstall.txt', 'post-uninstall.txt') -Operation 'uninstall' -StoredPrograms $false
            Add-UninstallOrderChecks -Checks $checks -Prefix 'uninstall' -Lines (Get-LogLines -JobRun $step -Path $uninstallLog)
            $evidenceSets['uninstall'] = Get-Evidence $step
        }
        return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
            -Environment (Get-Member2 $step.result 'environment') -Evidence @{ steps = $evidenceSets }
    }

    # 2. Upgrade with no explicit choices: everything is remembered.
    $upgradeLog = Join-Path $GuestStageRoot 'upgrade-b.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'upgrade' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'upgrade' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $upgradeLog) + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($upgradeLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'upgrade' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'upgrade' -JobRun $step -CommandName 'upgrade' -ExitCodes @(0) -ExpectedCode 'ok' -ExpectedOutcome 'installed'
    Add-ReadChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $initial
    Add-ResourceChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -Locations $locations -Selected $initial -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    Add-ActionChecks -Checks $checks -Prefix 'upgrade' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('build-cache=completed')) -CacheVersion $versionB -Markers $preflightMarkers -Operation 'upgrade'
    $lines = Get-LogLines -JobRun $step -Path $upgradeLog
    Add-Check $checks 'upgrade' 'prerequisite detected, not extracted again' 'log.dependency_detected' ((Test-LogHasCode $lines 'dependency_detected') -and -not (Test-LogHasCode $lines 'dependency_extracted')) 'The upgrade detected the prerequisite and extracted nothing.'

    # 3. Change several choices: new resources appear, deselected ones go,
    #    the rest stay.
    $changes = @{ 'extras' = $true; 'path-mode' = 'tools'; 'send-to' = $true; 'startup' = $false; 'file-association' = $false; 'firewall' = $false }
    $changed = Merge-Selection -Base $initial -Changes $changes
    $changeLog = Join-Path $GuestStageRoot 'change.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'change' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'change' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $changeLog) + $scopeArguments + (ConvertTo-OptionArguments $changes)))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($changeLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'change' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'change' -JobRun $step -CommandName 'change' -ExitCodes @(0) -ExpectedCode 'ok' -ExpectedOutcome 'installed'
    Add-Check $checks 'change' 'the changed run is a reinstall' 'transaction.reinstall' ([string] (Get-Member2 (Get-Member2 $outcome 'transaction') 'kind') -eq 'reinstall') "transaction kind: $(Get-Member2 (Get-Member2 $outcome 'transaction') 'kind')."
    Add-ReadChecks -Checks $checks -Prefix 'change' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $changed
    Add-ResourceChecks -Checks $checks -Prefix 'change' -JobRun $step -Locations $locations -Selected $changed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    Add-ActionChecks -Checks $checks -Prefix 'change' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('build-cache=completed')) -CacheVersion $versionB -Operation 'reinstall'
    $evidenceSets['change'] = Get-Evidence $step

    # 4. An upgrade that fails before its commit: the previous choices and
    #    resources remain.
    $failingChanges = @{ 'firewall' = $true; 'extras' = $false; 'path-mode' = 'none'; 'startup' = $true }
    $failLog = Join-Path $GuestStageRoot 'failed-change.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'failed-change' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'failed-change' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $failLog) + $scopeArguments + (ConvertTo-OptionArguments $failingChanges) + @('--fault', 'before_commit:fail')))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($failLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'failed-change' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'failed-change' -JobRun $step -CommandName 'failed-change' -ExitCodes @(1) -ExpectedOutcome 'rolled_back'
    Add-ReadChecks -Checks $checks -Prefix 'failed-change' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $changed
    Add-ResourceChecks -Checks $checks -Prefix 'failed-change' -JobRun $step -Locations $locations -Selected $changed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    $lines = Get-LogLines -JobRun $step -Path $failLog
    Add-Check $checks 'failed-change' 'the run rolled back' 'log.rolled_back' (Test-LogHasCode $lines 'transaction_rolled_back') 'The log records [transaction_rolled_back].'
    # The actions had run before the fault; the rollback says so rather than
    # pretending to undo them.
    Add-ActionChecks -Checks $checks -Prefix 'failed-change' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('build-cache=completed')) -CacheVersion $versionB -Operation 'reinstall'
    Add-FindingCheck -Checks $checks -Prefix 'failed-change' -Outcome $outcome -Code 'action_not_reverted'

    # 4b. A reinstall whose post-install action fails on purpose: the run
    #     rolls back what TigerSetup owns, the committed choices and
    #     resources stay, and the program's own marker stays where the
    #     program wrote it.
    $failingAction = @{ 'fail-action' = $true; 'extras' = $false }
    $failActionLog = Join-Path $GuestStageRoot 'failed-action.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'failed-action' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'failed-action' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $failActionLog) + $scopeArguments + (ConvertTo-OptionArguments $failingAction)))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($failActionLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'failed-action' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'failed-action' -JobRun $step -CommandName 'failed-action' -ExitCodes @(1) -ExpectedCode 'action_failed' -ExpectedOutcome 'rolled_back'
    Add-ReadChecks -Checks $checks -Prefix 'failed-action' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $changed
    Add-ResourceChecks -Checks $checks -Prefix 'failed-action' -JobRun $step -Locations $locations -Selected $changed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    Add-ActionChecks -Checks $checks -Prefix 'failed-action' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('fail-on-purpose=failed')) -CacheVersion $versionB -Markers @('failed-action.txt') -Operation 'reinstall'
    Add-FindingCheck -Checks $checks -Prefix 'failed-action' -Outcome $outcome -Code 'action_not_reverted'
    $failed = Get-FirstMatch -Items @(Get-Member2 $outcome 'actions') -Filter { [string] $_.name -eq 'fail-on-purpose' }
    Add-Check $checks 'failed-action' 'the failing action is named with its exit code' 'actions.failed.exit' ($null -ne $failed -and [int] (Get-Member2 $failed 'exit_code') -eq 3 -and [string] (Get-Member2 $failed 'code') -eq 'action_failed') "actions[fail-on-purpose]: $($failed | ConvertTo-Json -Compress -Depth 3)."
    $lines = Get-LogLines -JobRun $step -Path $failActionLog
    Add-Check $checks 'failed-action' 'the run rolled back' 'log.rolled_back' ((Test-LogHasCode $lines 'action_failed') -and (Test-LogHasCode $lines 'transaction_rolled_back')) 'The log records [action_failed] and [transaction_rolled_back].'

    # 5. The same change, committed.
    $committed = Merge-Selection -Base $changed -Changes $failingChanges
    $commitLog = Join-Path $GuestStageRoot 'commit-change.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'commit-change' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'commit-change' -Executable $stagedB -Arguments (@('install', '--quiet', '--json', '--log', $commitLog) + $scopeArguments + (ConvertTo-OptionArguments $failingChanges)))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($commitLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'commit-change' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'commit-change' -JobRun $step -CommandName 'commit-change' -ExitCodes @(0) -ExpectedCode 'ok' -ExpectedOutcome 'installed'
    Add-ReadChecks -Checks $checks -Prefix 'commit-change' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $committed
    Add-ResourceChecks -Checks $checks -Prefix 'commit-change' -JobRun $step -Locations $locations -Selected $committed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    Add-ActionChecks -Checks $checks -Prefix 'commit-change' -JobRun $step -Outcome $outcome -Locations $locations -Actions ($preflightActions + @('build-cache=completed')) -CacheVersion $versionB -Operation 'reinstall'

    # 6. A reinstall that names nothing converges without losing a choice.
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'reinstall' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'reinstall' -Executable $stagedB -Arguments (@('install', '--quiet', '--json') + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs))
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'reinstall' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'reinstall' -JobRun $step -CommandName 'reinstall' -ExitCodes @(0) -ExpectedCode 'already_installed'
    Add-ReadChecks -Checks $checks -Prefix 'reinstall' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $committed
    Add-ResourceChecks -Checks $checks -Prefix 'reinstall' -JobRun $step -Locations $locations -Selected $committed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    Add-ActionChecks -Checks $checks -Prefix 'reinstall' -JobRun $step -Outcome $outcome -Locations $locations -Actions @() -CacheVersion $versionB

    # 7. Damage owned resources; verify says so; repair converges.
    $repairLog = Join-Path $GuestStageRoot 'repair.log'
    $hive = $locations.hive
    $damage = @(
        (New-SetupCommand -Name 'damage-variable' -Executable 'reg.exe' -Arguments @('delete', $locations.environmentKey, '/v', $variableName, '/f') -Timeout 60),
        (New-SetupCommand -Name 'damage-protocol' -Executable 'reg.exe' -Arguments @('delete', "$hive\Software\Classes\$scheme\shell\open\command", '/ve', '/f') -Timeout 60),
        (New-SetupCommand -Name 'damage-shortcut' -Executable 'cmd.exe' -Arguments @('/c', 'del', '/f', '/q', "$($locations.programs)\$productName Documentation.url") -Timeout 60),
        (New-SetupCommand -Name 'damage-cache' -Executable 'cmd.exe' -Arguments @('/c', 'del', '/f', '/q', $actionCacheFile) -Timeout 60)
    )
    if ($Elevated) {
        $damage += (New-SetupCommand -Name 'damage-firewall' -Executable 'powershell.exe' -Arguments @('-NoProfile', '-NonInteractive', '-Command', "Remove-NetFirewallRule -DisplayName '$ruleName'") -Timeout 120)
    }
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'repair' -Locations $locations -RunAs $RunAs `
        -Commands ($damage + @((New-SetupCommand -Name 'verify-damaged' -Executable $stagedB -Arguments (@('verify', '--json') + $scopeArguments) -Timeout 300), (New-SetupCommand -Name 'repair' -Executable $stagedB -Arguments (@('repair', '--quiet', '--json', '--log', $repairLog) + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($repairLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'repair' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $damaged = Get-Member2 (Get-TigerSetupCommandResult -JobRun $step -CommandName 'verify-damaged') 'json'
    $damagedCodes = @(@(Get-Member2 $damaged 'findings') | ForEach-Object { [string] (Get-Member2 $_ 'code') })
    Add-Check $checks 'repair' 'verify sees the damage' 'verify.damaged' ([string] (Get-Member2 $damaged 'status') -eq 'failed') "verify --json after the damage: status '$(Get-Member2 $damaged 'status')', findings $($damagedCodes -join ', ')."
    foreach ($code in @('environment_variable_missing', 'registry_value_missing', 'shortcut_missing') + $(if ($Elevated) { @('firewall_rule_missing') } else { @() })) {
        Add-Check $checks 'repair' "verify reports $code" "verify.$code" ($damagedCodes -contains $code) "verify findings: $($damagedCodes -join ', ')."
    }
    $repairOutcome = Add-CommandCheck -Checks $checks -Prefix 'repair' -JobRun $step -CommandName 'repair' -ExitCodes @(0) -ExpectedCode 'ok'
    Add-Check $checks 'repair' 'the run is a repair' 'transaction.repair' ([string] (Get-Member2 (Get-Member2 $repairOutcome 'transaction') 'kind') -eq 'repair') "transaction kind: $(Get-Member2 (Get-Member2 $repairOutcome 'transaction') 'kind')."
    Add-ReadChecks -Checks $checks -Prefix 'repair' -JobRun $step -ExpectedVersion $versionB -ExpectedOptions $committed
    Add-ResourceChecks -Checks $checks -Prefix 'repair' -JobRun $step -Locations $locations -Selected $committed -Elevated $Elevated -ShellProbe ($probeRunAs -ne '')
    # Only the action that opted into repair runs, and it rebuilds the cache.
    Add-ActionChecks -Checks $checks -Prefix 'repair' -JobRun $step -Outcome $repairOutcome -Locations $locations -Actions @('build-cache=completed') -CacheVersion $versionB -Operation 'repair'

    # 8. Uninstall: owned resources gone, the pre-existing variable back.
    $uninstallLog = Join-Path $GuestStageRoot 'uninstall.log'
    $step = Invoke-Step -Row $Row -Baseline $Baseline -Suffix 'uninstall' -Locations $locations -RunAs $RunAs `
        -Commands (@((New-SetupCommand -Name 'uninstall' -Executable $stagedB -Arguments (@('uninstall', '--quiet', '--json', '--log', $uninstallLog) + $scopeArguments))) + (New-ReadCommands -Executable $stagedB -Scope $Scope -ProbeRunAs $probeRunAs)) -Logs @($uninstallLog)
    foreach ($check in ConvertTo-TigerSetupFlattenedChecks -Prefix 'uninstall' -LabRun $step) { $checks.Add($check) }
    if (-not (Test-LabRunUsable $step)) { return Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") }
    $outcome = Add-CommandCheck -Checks $checks -Prefix 'uninstall' -JobRun $step -CommandName 'uninstall' -ExitCodes @(0) -ExpectedOutcome 'uninstalled'
    Add-ReadChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -ExpectedVersion ''
    Add-AbsenceChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -Locations $locations -ExpectedVariable $previousVariable
    Add-ActionChecks -Checks $checks -Prefix 'uninstall' -JobRun $step -Outcome $outcome -Locations $locations -Actions @('clear-cache=completed', 'farewell=completed') -CacheVersion '' -Markers @('pre-uninstall.txt', 'post-uninstall.txt') -Operation 'uninstall' -StoredPrograms $false
    Add-UninstallOrderChecks -Checks $checks -Prefix 'uninstall' -Lines (Get-LogLines -JobRun $step -Path $uninstallLog)
    $evidenceSets['uninstall'] = Get-Evidence $step

    Write-TigerSetupRowResult -Row $Row -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot "$Row.json") `
        -Environment (Get-Member2 $step.result 'environment') -Evidence @{ steps = $evidenceSets }
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-features-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
$rowBaseline = @{ 'lifecycle-user' = $Win11Baseline; 'standard-user' = $Win11Baseline; 'machine' = $Win11Baseline; 'win10' = $Win10Baseline }
$summary = [System.Collections.Generic.List[object]]::new()
$openSession = $null
try {
foreach ($baseline in @($Rows | ForEach-Object { $rowBaseline[$_] } | Select-Object -Unique)) {
    # One session per baseline: a session protects every VM it touches, so one
    # spanning two baselines would hold two VMs (lab/README.md).
    $baselineRows = @($Rows | Where-Object { $rowBaseline[$_] -eq $baseline })
    $openSession = "$SessionId-$(($baseline -replace '[^A-Za-z0-9]+', '-').ToLowerInvariant())"
    $null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $openSession `
        -Description "TigerSetup feature rows on $baseline`: $($baselineRows -join ', ')" `
        -ResultPath (Join-Path $ResultsRoot "session-open-$baseline.json")
    Write-Host "Lab session $openSession ($baseline)"
    try {
        foreach ($row in $baselineRows) {
            Write-Host ""
            Write-Host "### $row"
            $started = [DateTimeOffset]::Now
            try {
                $result = switch ($row) {
                    'lifecycle-user' { Invoke-LifecycleRow -Row $row -Baseline $baseline -Scope 'user' -RunAs 'job' -Elevated $true }
                    'standard-user' { Invoke-LifecycleRow -Row $row -Baseline $baseline -Scope 'user' -RunAs 'interactiveUser' -Elevated $false -Short }
                    'machine' { Invoke-LifecycleRow -Row $row -Baseline $baseline -Scope 'machine' -RunAs 'job' -Elevated $true -Short }
                    'win10' { Invoke-LifecycleRow -Row $row -Baseline $baseline -Scope 'user' -RunAs 'job' -Elevated $true -Short }
                }
                $summary.Add([pscustomobject]@{ row = $row; baseline = $baseline; status = $result.status; pass = $result.counts.pass; warn = $result.counts.warn; fail = $result.counts.fail; minutes = [math]::Round(([DateTimeOffset]::Now - $started).TotalMinutes, 1) })
            }
            catch {
                Write-Host "   ERROR $($_.Exception.Message)"
                $where = "$($_.InvocationInfo.ScriptName):$($_.InvocationInfo.ScriptLineNumber)"
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
        $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $openSession -ResultPath (Join-Path $ResultsRoot "session-close-$baseline.json")
        $openSession = $null
    }
}
}
finally {
    if ($null -ne $openSession) {
        $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $openSession -ResultPath (Join-Path $ResultsRoot 'session-close-final.json')
    }
}

Write-Host ""
Write-Host "Summary ($ResultsRoot)"
$summary | Format-Table -AutoSize | Out-String | Write-Host
[System.IO.File]::WriteAllText((Join-Path $ResultsRoot 'summary.json'), ($summary | ConvertTo-Json -Depth 4), [System.Text.UTF8Encoding]::new($false))
if (@($summary | Where-Object { $_.status -in @('FAIL', 'ERROR') }).Count -gt 0) { exit 1 }
exit 0
