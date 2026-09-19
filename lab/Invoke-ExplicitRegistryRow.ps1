#Requires -Version 7.0
<#
    .SYNOPSIS
    Focused TigerWinLab proof of a `[[registry]]` value at an explicit
    location outside the scope's Software root, on the real machine hive of a
    clean Windows 11: written over what Windows had, kept and repaired,
    preserved where an administrator changed it, and put back when the
    product goes.

    .DESCRIPTION
    One lab session, one VM, five chained jobs, each read with the generic
    guest reader (guest\Invoke-SetupCommands.ps1) so the registry is read
    exactly as Windows holds it:

      baseline    what the clean VM holds at every explicit location the
                  package declares, before anything ran — the state an
                  uninstall must give back
      install     a quiet machine-scope install; every explicit value holds
                  the package's data, `verify --json` is clean, and
                  `inspect --json` owns the values at their explicit paths
      reconcile   a reconciling reinstall (an explicit option makes it one)
                  converges without a finding; then an administrator turns
                  the setting back off (`reg.exe add`), a second reinstall
                  preserves their value and says so
                  (`registry_value_modified_preserved`), and a repair puts the
                  package's value back
      uninstall   a quiet uninstall; every pre-existing value holds what the
                  baseline held, every created value and the keys TigerSetup
                  created are gone, and the Windows keys above them stay

    The checks derive from what the installer declares (`tiger-setup inspect
    --json`): every `registry_values[]` entry whose `root` is not `software`
    is an explicit location, and the package must be machine-only, which the
    builder already enforces for an `HKLM` root. The default installer is the
    fixture of packages\test-explicit-registry, built on the fly with the
    release builder; any machine-only package that declares explicit values
    can be passed instead.

    The process-level tests (crates\tigersetup-setup\tests\explicit_registry.rs)
    prove the same lifecycle, rollback and the per-option gate against
    relocated registry roots; this row is the real hive.

    .EXAMPLE
    pwsh -File lab\Invoke-ExplicitRegistryRow.ps1
    pwsh -File lab\Invoke-ExplicitRegistryRow.ps1 -InstallerPath benchmark\artifacts\qbittorrent\qBittorrent-TigerSetup.exe
#>
[CmdletBinding()]
param(
    [string] $InstallerPath,
    [string] $BuilderPath,
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

# The default installer is the fixture, built here so it always carries the
# engine beside the builder; a caller's installer is checked for the same.
if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
    $manifest = Join-Path $repoRoot 'packages\test-explicit-registry\TigerSetup.toml'
    $outputDir = Join-Path $repoRoot 'artifacts\test-explicit-registry'
    $null = New-Item -ItemType Directory -Path $outputDir -Force
    $built = & $BuilderPath build $manifest --output $outputDir --fast 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "Building the fixture failed: $built" }
    $InstallerPath = Join-Path $outputDir 'TigerSetupTestExplicitRegistry-1.0.0-Setup.exe'
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath
if ((@($facts.scopes) -join ',') -ne 'machine') {
    throw "'$InstallerPath' allows scopes $($facts.scopes -join ', '); this row is written for a machine-only package."
}

function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}

# The explicit values the package declares, with the hive path the guest
# reader takes and the data the engine will have written (the templates a
# value may carry are expanded the way the engine expands them).
$explicit = @(@(Get-Member2 $facts.raw 'registry_values') | Where-Object {
        $null -ne $_ -and [string] (Get-Member2 $_ 'root') -ne 'software'
    } | ForEach-Object {
        $option = Get-Member2 (Get-Member2 $_ 'when') 'option'
        [pscustomobject][ordered]@{
            key = "$([string] $_.root)\$([string] $_.key)"
            name = [string] $_.name
            kind = [string] $_.kind
            data = ([string] $_.data).Replace('%VERSION%', $facts.version)
            option = $(if ($null -ne $option) { [string] $option } else { '' })
        }
    })
if ($explicit.Count -eq 0) { throw "'$InstallerPath' declares no registry value at an explicit location." }
if (@($explicit | Where-Object { $_.data -like '*%INSTALLROOT%*' }).Count -gt 0) {
    throw 'This row does not expand %INSTALLROOT% in an explicit value; declare the fixture without it.'
}
# The reconciling reinstall needs an explicit option; the setting the row
# turns off behind TigerSetup's back is the first DWORD it finds.
$firstOption = @($facts.options | Where-Object { $null -ne $_ } | Select-Object -First 1)
if ($firstOption.Count -eq 0) { throw "'$InstallerPath' declares no option; the reconciling reinstall needs one to name." }
$reconcileOption = [string] $firstOption[0].name
$reconcileValue = [string] $firstOption[0].default
if ($reconcileValue -eq 'True') { $reconcileValue = 'on' } elseif ($reconcileValue -eq 'False') { $reconcileValue = 'off' }
$setting = @($explicit | Where-Object { $_.kind -eq 'dword' } | Select-Object -First 1)
if ($setting.Count -eq 0) { throw "'$InstallerPath' declares no explicit DWORD; the administrator-change step needs one." }
$setting = $setting[0]
$keysToRead = @($explicit | ForEach-Object { $_.key } | Sort-Object -Unique)
# Every key TigerSetup may have created above an explicit value: the chain
# below the hive's top-level key, which the uninstall must remove again when
# it created it.
$chainKeys = @(@(foreach ($key in $keysToRead) {
            $parts = $key -split '\\'
            for ($depth = 3; $depth -lt $parts.Count; $depth++) { ($parts[0..($depth - 1)] -join '\') }
        }) | Sort-Object -Unique)

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\explicit-registry-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
$guestInstaller = Join-Path $GuestStageRoot ([System.IO.Path]::GetFileName($InstallerPath))
$guestLogRoot = Join-Path $GuestStageRoot 'explicit-registry'

$checks = [System.Collections.Generic.List[object]]::new()
function Add-Check {
    param([string] $Name, [string] $Code, [bool] $Pass, [string] $Message = '')
    $checks.Add((New-TigerSetupCheck -Name $Name -Code $Code -Status $(if ($Pass) { 'PASS' } else { 'FAIL' }) -Message $Message))
}
function Get-Evidence {
    param([object] $JobRun)
    if ($null -eq $JobRun -or $JobRun.status -ne 'OK' -or $null -eq $JobRun.result) { return $null }
    $record = Get-Member2 $JobRun.result 'result'
    $record
}
function Get-RegistryValue {
    <# The text the guest reader holds for a value, or $null when the key or the value is absent. #>
    param([object] $Evidence, [string] $Key, [string] $Name)
    if ($null -eq $Evidence) { return $null }
    $record = @(@(Get-Member2 $Evidence 'registry') | Where-Object { $null -ne $_ -and $_.requested -eq $Key }) | Select-Object -First 1
    if ($null -eq $record -or -not [bool] $record.exists) { return $null }
    $values = Get-Member2 $record 'values'
    $label = $(if ($Name -eq '') { '(default)' } else { $Name })
    if ($null -eq $values -or $null -eq $values.PSObject.Properties[$label]) { return $null }
    [string] $values.$label
}
function Test-RegistryKeyExists {
    param([object] $Evidence, [string] $Key)
    if ($null -eq $Evidence) { return $false }
    $record = @(@(Get-Member2 $Evidence 'registry') | Where-Object { $null -ne $_ -and $_.requested -eq $Key }) | Select-Object -First 1
    ($null -ne $record) -and [bool] $record.exists
}
function Get-CommandJson {
    param([object] $Evidence, [string] $Name)
    if ($null -eq $Evidence) { return $null }
    $command = @(@(Get-Member2 $Evidence 'commands') | Where-Object { $null -ne $_ -and $_.name -eq $Name }) | Select-Object -First 1
    if ($null -eq $command) { return $null }
    Get-Member2 $command 'json'
}
function Get-Codes {
    param([object] $Document)
    @(@(Get-Member2 $Document 'findings') | Where-Object { $null -ne $_ } | ForEach-Object { [string] (Get-Member2 $_ 'code') })
}
function Add-CommandCheck {
    <# The command ran, exited 0 and printed the outcome the step expects. #>
    param([object] $Evidence, [string] $Step, [string] $Name, [string] $Outcome)
    $command = @(@(Get-Member2 $Evidence 'commands') | Where-Object { $null -ne $_ -and $_.name -eq $Name }) | Select-Object -First 1
    if ($null -eq $command) {
        Add-Check -Name "$Step/$Name ran" -Code "explicit.$Step.$Name.ran" -Pass $false -Message 'no record of the command'
        return $null
    }
    $exit = Get-Member2 $command 'exitCode'
    Add-Check -Name "$Step/$Name exit 0" -Code "explicit.$Step.$Name.exit" -Pass ($exit -eq 0) -Message "exit $exit after $($command.durationSeconds)s; $($command.stderr)"
    $document = Get-Member2 $command 'json'
    if (-not [string]::IsNullOrWhiteSpace($Outcome)) {
        $actual = [string] (Get-Member2 $document 'outcome')
        Add-Check -Name "$Step/$Name outcome $Outcome" -Code "explicit.$Step.$Name.outcome" -Pass ($actual -eq $Outcome) -Message "outcome=$actual code=$([string] (Get-Member2 $document 'code'))"
    }
    $document
}
function New-SetupCommand {
    <# A Setup.exe command with --json; a mutating one writes its own log too. #>
    param([string] $Name, [string[]] $Arguments, [int] $TimeoutSeconds = 600)
    $tail = @('--json')
    if ($Arguments[0] -in @('install', 'repair', 'uninstall')) { $tail += @('--log', "$guestLogRoot\$Name.log") }
    @{ name = $Name; executable = $guestInstaller; arguments = @($Arguments + $tail); timeoutSeconds = $TimeoutSeconds }
}
function Invoke-Step {
    param([string] $Step, [hashtable] $Request, [string[]] $PayloadFiles = @(), [switch] $FromBaseline)
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline
    $run = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $Request -PayloadFiles $PayloadFiles `
        -Name "ts-xreg-$Step" @policy -ResultPath (Join-Path $ResultsRoot "runs\$Step.json") -OutputRoot $labOutputRoot `
        -TimeoutMinutes $JobTimeoutMinutes
    Add-Check -Name "$Step/lab job" -Code "explicit.$Step.lab" -Pass ($run.status -eq 'OK') -Message "status $($run.status) (exit $($run.exitCode)) after $($run.durationSeconds)s"
    Get-Evidence $run
}

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-explicit-registry-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
Write-Host "Installer $([System.IO.Path]::GetFileName($InstallerPath)) ($($facts.id) $($facts.version)), explicit values:"
foreach ($value in $explicit) { Write-Host "  $($value.key)\$($value.name) $($value.kind) = $($value.data)$(if ($value.option) { " (option $($value.option))" })" }
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description 'TigerSetup explicit registry location row' `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$environment = $null
try {
    # --- baseline: what the clean VM holds at every explicit location ---
    Write-Host ''; Write-Host '### baseline'
    $before = Invoke-Step -Step 'baseline' -FromBaseline -Request @{
        commands = @(@{ name = 'mkdir'; executable = 'cmd.exe'; arguments = @('/c', 'mkdir', $guestLogRoot); timeoutSeconds = 30 })
        runAs = 'job'
        registry = @($keysToRead + $chainKeys)
    }
    $priorValues = @{}
    foreach ($value in $explicit) {
        $priorValues["$($value.key)\$($value.name)"] = Get-RegistryValue -Evidence $before -Key $value.key -Name $value.name
        Write-Host ("  before: {0}\{1} = {2}" -f $value.key, $value.name, $(if ($null -eq $priorValues["$($value.key)\$($value.name)"]) { '<absent>' } else { $priorValues["$($value.key)\$($value.name)"] }))
    }
    $chainBefore = @{}
    foreach ($key in $chainKeys) { $chainBefore[$key] = Test-RegistryKeyExists -Evidence $before -Key $key }
    Add-Check -Name 'baseline/the setting pre-exists with other data' -Code 'explicit.baseline.setting.preexists' `
        -Pass ($null -ne $priorValues["$($setting.key)\$($setting.name)"] -and $priorValues["$($setting.key)\$($setting.name)"] -ne $setting.data) `
        -Message "$($setting.key)\$($setting.name) = $($priorValues["$($setting.key)\$($setting.name)"]) on the baseline; the row needs a value Windows already holds with other data"

    # --- install ---
    Write-Host ''; Write-Host '### install'
    $installed = Invoke-Step -Step 'install' -PayloadFiles @($InstallerPath) -Request @{
        stage = @(@{ source = [System.IO.Path]::GetFileName($InstallerPath); destination = $guestInstaller })
        commands = @(
            (New-SetupCommand -Name 'install' -Arguments @('install', '--quiet', '--scope', 'machine')),
            (New-SetupCommand -Name 'verify' -Arguments @('verify', '--scope', 'machine')),
            (New-SetupCommand -Name 'inspect' -Arguments @('inspect', '--scope', 'machine'))
        )
        runAs = 'job'
        registry = @($keysToRead + $chainKeys)
    }
    $environment = $(if ($null -ne $installed) { Get-Member2 $installed 'environment' } else { $null })
    $null = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'install' -Outcome 'installed'
    $verifyDocument = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'verify' -Outcome ''
    Add-Check -Name 'install/verify ok' -Code 'explicit.install.verify.status' -Pass ([string] (Get-Member2 $verifyDocument 'status') -eq 'ok') -Message ("findings: " + ((Get-Codes $verifyDocument) -join ', '))
    $inspectDocument = Add-CommandCheck -Evidence $installed -Step 'install' -Name 'inspect' -Outcome ''
    $ownedValues = @(@(Get-Member2 (Get-Member2 $inspectDocument 'owned') 'registry_values') | ForEach-Object { [string] $_ })
    foreach ($value in $explicit) {
        $actual = Get-RegistryValue -Evidence $installed -Key $value.key -Name $value.name
        Add-Check -Name "install/$($value.key)\$($value.name) = $($value.data)" -Code 'explicit.install.value' -Pass ($actual -eq $value.data) -Message "found $(if ($null -eq $actual) { '<absent>' } else { $actual })"
        Add-Check -Name "install/inspect owns $($value.key)\$($value.name)" -Code 'explicit.install.owned' -Pass ($ownedValues -contains "$($value.key)\$($value.name)") -Message ($ownedValues -join '; ')
    }
    foreach ($key in $chainKeys) {
        Add-Check -Name "install/key $key exists" -Code 'explicit.install.key' -Pass (Test-RegistryKeyExists -Evidence $installed -Key $key)
    }

    # --- reconcile: converge, then an administrator's change, then repair ---
    Write-Host ''; Write-Host '### reconcile'
    $reconciled = Invoke-Step -Step 'reconcile' -Request @{
        commands = @(
            (New-SetupCommand -Name 'reinstall' -Arguments @('install', '--quiet', '--scope', 'machine', '--option', $reconcileOption, $reconcileValue)),
            @{ name = 'admin-change'; executable = 'reg.exe'; arguments = @('add', $setting.key, '/v', $setting.name, '/t', 'REG_DWORD', '/d', [string] $priorValues["$($setting.key)\$($setting.name)"], '/f'); timeoutSeconds = 60 },
            (New-SetupCommand -Name 'reinstall-after-change' -Arguments @('install', '--quiet', '--scope', 'machine', '--option', $reconcileOption, $reconcileValue)),
            (New-SetupCommand -Name 'verify-after-change' -Arguments @('verify', '--scope', 'machine')),
            (New-SetupCommand -Name 'repair' -Arguments @('repair', '--quiet', '--scope', 'machine'))
        )
        runAs = 'job'
        registry = @($keysToRead)
    }
    $document = Add-CommandCheck -Evidence $reconciled -Step 'reconcile' -Name 'reinstall' -Outcome 'installed'
    Add-Check -Name 'reconcile/reinstall reconciled without a finding' -Code 'explicit.reconcile.reinstall.clean' `
        -Pass ($null -ne (Get-Member2 $document 'transaction') -and @(Get-Codes $document).Count -eq 0) -Message ("findings: " + ((Get-Codes $document) -join ', '))
    $document = Add-CommandCheck -Evidence $reconciled -Step 'reconcile' -Name 'reinstall-after-change' -Outcome 'installed'
    Add-Check -Name 'reconcile/the changed setting is preserved and reported' -Code 'explicit.reconcile.preserved' `
        -Pass (@(Get-Codes $document) -contains 'registry_value_modified_preserved') -Message ("findings: " + ((Get-Codes $document) -join ', '))
    $command = @(@(Get-Member2 $reconciled 'commands') | Where-Object { $null -ne $_ -and $_.name -eq 'verify-after-change' }) | Select-Object -First 1
    $verifyDocument = $(if ($null -ne $command) { Get-Member2 $command 'json' } else { $null })
    Add-Check -Name 'reconcile/verify reports the changed value' -Code 'explicit.reconcile.verify.modified' `
        -Pass (@(Get-Codes $verifyDocument) -contains 'registry_value_modified') -Message ("status $([string] (Get-Member2 $verifyDocument 'status')); findings: " + ((Get-Codes $verifyDocument) -join ', '))
    $document = Add-CommandCheck -Evidence $reconciled -Step 'reconcile' -Name 'repair' -Outcome 'installed'
    Add-Check -Name 'reconcile/repair ran as a repair' -Code 'explicit.reconcile.repair.kind' -Pass ([string] (Get-Member2 (Get-Member2 $document 'transaction') 'kind') -eq 'repair')
    $actual = Get-RegistryValue -Evidence $reconciled -Key $setting.key -Name $setting.name
    Add-Check -Name "reconcile/repair put $($setting.name) back to $($setting.data)" -Code 'explicit.reconcile.repair.value' -Pass ($actual -eq $setting.data) -Message "found $(if ($null -eq $actual) { '<absent>' } else { $actual })"

    # --- uninstall: the baseline comes back ---
    Write-Host ''; Write-Host '### uninstall'
    $uninstalled = Invoke-Step -Step 'uninstall' -Request @{
        commands = @((New-SetupCommand -Name 'uninstall' -Arguments @('uninstall', '--quiet', '--scope', 'machine')))
        runAs = 'job'
        registry = @($keysToRead + $chainKeys)
    }
    $null = Add-CommandCheck -Evidence $uninstalled -Step 'uninstall' -Name 'uninstall' -Outcome 'uninstalled'
    foreach ($value in $explicit) {
        $expected = $priorValues["$($value.key)\$($value.name)"]
        $actual = Get-RegistryValue -Evidence $uninstalled -Key $value.key -Name $value.name
        Add-Check -Name "uninstall/$($value.key)\$($value.name) back to $(if ($null -eq $expected) { '<absent>' } else { $expected })" -Code 'explicit.uninstall.value' `
            -Pass ($actual -eq $expected) -Message "found $(if ($null -eq $actual) { '<absent>' } else { $actual })"
    }
    foreach ($key in $chainKeys) {
        $exists = Test-RegistryKeyExists -Evidence $uninstalled -Key $key
        Add-Check -Name "uninstall/key $key $(if ($chainBefore[$key]) { 'stays' } else { 'is gone' })" -Code 'explicit.uninstall.key' -Pass ($exists -eq $chainBefore[$key])
    }
}
finally {
    $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId -ResultPath (Join-Path $ResultsRoot 'session-close.json')
}

$result = Write-TigerSetupRowResult -Row 'explicit-registry' -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot 'explicit-registry.json') `
    -Environment $environment -Evidence @{ installer = $InstallerPath; package = $facts.id; version = $facts.version; engineSha256 = $facts.engineSha256 }
if ($result.status -eq 'FAIL') { exit 1 }
exit 0
