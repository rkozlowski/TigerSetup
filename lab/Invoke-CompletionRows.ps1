#Requires -Version 7.0
<#
    .SYNOPSIS
    TigerWinLab proof of PowerShell completion on a clean Windows 11: the
    installing user's profiles gain a block that a real Windows PowerShell 5.1
    and a real PowerShell 7 load, every other account's profiles stay as they
    were, and the user's own profile content comes back byte for byte.

    .DESCRIPTION
    One lab session, four chains of jobs, each read with the generic guest
    reader (guest\Invoke-SetupCommands.ps1) and the completion probe
    (guest\Invoke-CompletionProbe.ps1), which reports the profiles of the
    account it runs as and asks a new PowerShell session — profiles included —
    for the completions of `TigerSetupTestAction al`:

      user        on the standard user's desktop: their Windows PowerShell
                  profile is seeded first (UTF-8 with a BOM, CRLF, Polish
                  text); a quiet user-scope install into a non-ASCII folder
                  adds a block to both editions' profiles; completion answers
                  `alpha` in Windows PowerShell 5.1 started with
                  -ExecutionPolicy RemoteSigned and in PowerShell 7, while
                  5.1 under the machine's default policy agrees with what the
                  install reported about it (`windows_powershell_profiles_not_run`);
                  a reinstall changes nothing, PATH off takes the blocks out
                  and leaves the seeded profile exactly as it was, PATH on
                  puts them back, and the uninstall restores the seed byte
                  for byte and removes the PowerShell 7 profile and folder it
                  created
      machine     on the same desktop, a machine-scope install by an
                  administrator elevated beside the signed-in standard user
                  (`completion_installer_not_desktop_user`) and a repair from
                  the job account (`completion_no_desktop_user`) activate
                  nothing: neither account's profiles change
      admin       from a fresh baseline with an administrator signed in, a
                  machine-scope install elevated on that administrator's own
                  desktop activates their profiles; completion answers `alpha`
                  in both editions as that administrator unelevated; the
                  uninstall restores their seeded profile byte for byte
      upgrade-recovery
                  from a fresh baseline, the standard user's seeded profiles
                  through an interrupted transaction and an upgrade: an
                  install crashed after the blocks were written
                  (`--fault before_commit:crash`) and then uninstalled gives
                  the profiles back byte for byte; the same crash followed by
                  an install converges forward with one block each; an upgrade
                  to the next version leaves both profiles byte for byte as
                  they were and completion still answers; the uninstall
                  restores the seed

    -Rows chooses `profiles` (the first three chains, which share their
    preparation) or `upgrade-recovery`; both run by default.

    PowerShell 7 is not part of the baseline (TigerWinLab installs none), so the
    row brings its own: the pinned win-x64 zip, downloaded once into
    artifacts\lab-tools\ and checked against its published SHA-256, then
    unpacked in the guest.

    The default installers are the fixture of packages\test-completion and the
    same fixture as 1.1.0, built on the fly with the release builder. The
    process-level tests
    (crates\tigersetup-setup\tests\completion.rs) prove the same lifecycle,
    rollback, crash recovery and the security refusals against relocated
    folders; this row is the real profiles, the real accounts and the real
    shells.

    .EXAMPLE
    pwsh -File lab\Invoke-CompletionRows.ps1
    pwsh -File lab\Invoke-CompletionRows.ps1 -Rows upgrade-recovery
#>
[CmdletBinding()]
param(
    [string] $InstallerPath,
    # The next version of the same package, for the upgrade-recovery chain.
    [string] $UpgradeInstallerPath,
    [string[]] $Rows = @('all'),
    [string] $BuilderPath,
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $TigerWinLabRoot,
    [string] $ResultsRoot,
    [string] $SessionId,
    [string] $GuestStageRoot = 'C:\TigerSetupLab',
    [int] $JobTimeoutMinutes = 20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TigerSetupLab.psm1') -Force

# The PowerShell 7 the row stages: a pinned release and its published digest.
$pwshVersion = '7.6.7'
$pwshZipName = "PowerShell-$pwshVersion-win-x64.zip"
$pwshZipUrl = "https://github.com/PowerShell/PowerShell/releases/download/v$pwshVersion/$pwshZipName"
$pwshZipSha256 = 'd2f952ebdbf24386cb8923553aa58e41a846db35a6155ea04c58142a4b16dbb7'

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($BuilderPath)) {
    $BuilderPath = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe'
}
if (-not (Test-Path -LiteralPath $BuilderPath -PathType Leaf)) {
    throw "The builder '$BuilderPath' does not exist; run cargo build --release first, or pass -BuilderPath."
}
$BuilderPath = (Resolve-Path -LiteralPath $BuilderPath).Path

# `pwsh -File` hands a comma-joined list to a [string[]] parameter as one string.
$Rows = @($Rows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$knownRows = @('profiles', 'upgrade-recovery')
if ($Rows -contains 'all') { $Rows = $knownRows }
foreach ($row in $Rows) {
    if ($row -notin $knownRows) { throw "Unknown row '$row'; the rows are $($knownRows -join ', ')." }
}
$withUpgrade = $Rows -contains 'upgrade-recovery'

$buildPackage = Join-Path $repoRoot 'packages\test-completion\Build-Package.ps1'
if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
    & $buildPackage -ReleaseDirectory (Split-Path -Parent $BuilderPath) -SkipBuild -Fast
    $InstallerPath = Join-Path $repoRoot 'artifacts\test-completion\TigerSetupTestCompletion-1.0.0-Setup.exe'
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
$facts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $InstallerPath
Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $facts -InstallerPath $InstallerPath
$upgradeFacts = $null
if ($withUpgrade) {
    if ([string]::IsNullOrWhiteSpace($UpgradeInstallerPath)) {
        & $buildPackage -ReleaseDirectory (Split-Path -Parent $BuilderPath) -SkipBuild -Fast -Version '1.1.0'
        $UpgradeInstallerPath = Join-Path $repoRoot 'artifacts\test-completion\TigerSetupTestCompletion-1.1.0-Setup.exe'
    }
    $UpgradeInstallerPath = (Resolve-Path -LiteralPath $UpgradeInstallerPath).Path
    $upgradeFacts = Get-TigerSetupPackageFacts -BuilderPath $BuilderPath -InstallerPath $UpgradeInstallerPath
    Assert-TigerSetupEngineIsCurrent -BuilderPath $BuilderPath -Facts $upgradeFacts -InstallerPath $UpgradeInstallerPath
    if ($upgradeFacts.id -ne $facts.id -or [version] $upgradeFacts.version -le [version] $facts.version) {
        throw "'$UpgradeInstallerPath' ($($upgradeFacts.id) $($upgradeFacts.version)) is not a later version of $($facts.id) $($facts.version)."
    }
}

$toolsRoot = Join-Path $repoRoot 'artifacts\lab-tools'
$pwshZip = Join-Path $toolsRoot $pwshZipName
if (-not (Test-Path -LiteralPath $pwshZip -PathType Leaf)) {
    $null = New-Item -ItemType Directory -Path $toolsRoot -Force
    Write-Host "Downloading $pwshZipUrl"
    Invoke-WebRequest -Uri $pwshZipUrl -OutFile "$pwshZip.partial"
    Move-Item -LiteralPath "$pwshZip.partial" -Destination $pwshZip -Force
}
$actualSha256 = (Get-FileHash -LiteralPath $pwshZip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $pwshZipSha256) {
    throw "$pwshZip has SHA-256 $actualSha256, not the published $pwshZipSha256; delete it and run again."
}

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
if ([string]::IsNullOrWhiteSpace($ResultsRoot)) {
    $ResultsRoot = Join-Path $PSScriptRoot ('results\completion-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss'))
}
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$labOutputRoot = Join-Path $ResultsRoot 'lab'
$probePath = Join-Path $PSScriptRoot 'guest\Invoke-CompletionProbe.ps1'
$guestInstaller = Join-Path $GuestStageRoot ([System.IO.Path]::GetFileName($InstallerPath))
$guestUpgradeInstaller = $(if ($withUpgrade) { Join-Path $GuestStageRoot ([System.IO.Path]::GetFileName($UpgradeInstallerPath)) } else { '' })
$guestProbe = Join-Path $GuestStageRoot 'Invoke-CompletionProbe.ps1'
$guestZip = Join-Path $GuestStageRoot $pwshZipName
$guestPwshRoot = Join-Path $GuestStageRoot 'pwsh'
$guestPwsh = Join-Path $guestPwshRoot 'pwsh.exe'
$guestLogRoot = Join-Path $GuestStageRoot 'completion'
# A per-user folder whose name is not ASCII: the block must name it in a form
# every edition reads the same whatever the profile's encoding.
# No space in it: the lab's desktop sessions pass a command's arguments on
# without quoting them, so one with a space arrives as two.
$userInstallRoot = '%LOCALAPPDATA%\Programs\Zażółć-Tiger'
# What the user had before anything was installed: UTF-8 with a BOM, CRLF
# line ends, and text no ANSI code page holds in full.
$seedBytes = [byte[]] (@(0xEF, 0xBB, 0xBF) + [System.Text.Encoding]::UTF8.GetBytes("Set-Alias ll Get-ChildItem`r`n# zażółć gęślą jaźń`r`n"))
$seedBase64 = [Convert]::ToBase64String($seedBytes)
$completed = 'alpha'

$checks = [System.Collections.Generic.List[object]]::new()
function Add-Check {
    param([string] $Name, [string] $Code, [bool] $Pass, [string] $Message = '')
    $checks.Add((New-TigerSetupCheck -Name $Name -Code $Code -Status $(if ($Pass) { 'PASS' } else { 'FAIL' }) -Message $Message))
}
function Get-Member2 {
    param([object] $Object, [string] $Name)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $null }
    $Object.$Name
}
function Get-Command2 {
    param([object] $Evidence, [string] $Name)
    @(@(Get-Member2 $Evidence 'commands') | Where-Object { $null -ne $_ -and $_.name -eq $Name }) | Select-Object -First 1
}
function Get-Codes {
    param([object] $Document)
    @(@(Get-Member2 $Document 'findings') | Where-Object { $null -ne $_ } | ForEach-Object { [string] (Get-Member2 $_ 'code') })
}
function Add-CommandCheck {
    <# The command ran and exited 0; a Setup.exe command also printed the outcome the step expects. #>
    param([object] $Evidence, [string] $Step, [string] $Name, [string] $Outcome = '')
    $command = Get-Command2 $Evidence $Name
    if ($null -eq $command) {
        Add-Check -Name "$Step/$Name ran" -Code "completion.$Step.$Name.ran" -Pass $false -Message 'no record of the command'
        return $null
    }
    $exit = Get-Member2 $command 'exitCode'
    Add-Check -Name "$Step/$Name exit 0" -Code "completion.$Step.$Name.exit" -Pass ($exit -eq 0) `
        -Message "exit $exit after $($command.durationSeconds)s as $($command.runAs); $($command.error) $($command.stderr)"
    $document = Get-Member2 $command 'json'
    if (-not [string]::IsNullOrWhiteSpace($Outcome)) {
        $actual = [string] (Get-Member2 $document 'outcome')
        Add-Check -Name "$Step/$Name outcome $Outcome" -Code "completion.$Step.$Name.outcome" -Pass ($actual -eq $Outcome) `
            -Message "outcome=$actual code=$([string] (Get-Member2 $document 'code')) findings: $((Get-Codes $document) -join ', ')"
    }
    $document
}
function New-SetupCommand {
    <# A Setup.exe command with --json; a mutating one writes its own log too. #>
    param([string] $Name, [string[]] $Arguments, [string] $RunAs, [int] $TimeoutSeconds = 600, [string] $Executable = $guestInstaller)
    $tail = @('--json')
    if ($Arguments[0] -in @('install', 'repair', 'uninstall')) { $tail += @('--log', "$guestLogRoot\$Name.log") }
    @{ name = $Name; executable = $Executable; arguments = @($Arguments + $tail); runAs = $RunAs; timeoutSeconds = $TimeoutSeconds }
}
function New-ProbeCommand {
    <# The completion probe in the session -RunAs names. #>
    param([string] $Name, [string] $RunAs, [string[]] $Arguments)
    @{
        name = $Name
        executable = 'powershell.exe'
        arguments = @('-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $guestProbe) + $Arguments
        runAs = $RunAs
        timeoutSeconds = 180
    }
}
function New-CompletionProbes {
    <# The three completion questions: 5.1 allowed to run scripts, 5.1 under the machine's policy, PowerShell 7. #>
    param([string] $Prefix, [string] $RunAs)
    @(
        (New-ProbeCommand -Name "$Prefix-complete-51" -RunAs $RunAs -Arguments @('-Mode', 'Complete', '-Shell', 'powershell.exe', '-Policy', 'RemoteSigned')),
        (New-ProbeCommand -Name "$Prefix-complete-51-default" -RunAs $RunAs -Arguments @('-Mode', 'Complete', '-Shell', 'powershell.exe')),
        (New-ProbeCommand -Name "$Prefix-complete-7" -RunAs $RunAs -Arguments @('-Mode', 'Complete', '-Shell', $guestPwsh))
    )
}
function Get-Profile {
    <# One edition's profile as a probe reported it. #>
    param([object] $Probe, [string] $Edition)
    @(@(Get-Member2 $Probe 'profiles') | Where-Object { $null -ne $_ -and $_.edition -eq $Edition }) | Select-Object -First 1
}
function Get-ProfileText {
    param([object] $Entry)
    if ($null -eq $Entry -or -not [bool] $Entry.exists) { return '' }
    $bytes = [Convert]::FromBase64String([string] $Entry.base64)
    $offset = $(if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { 3 } else { 0 })
    [System.Text.Encoding]::UTF8.GetString($bytes, $offset, $bytes.Length - $offset)
}
function Add-BlockChecks {
    <#
        Both profiles of the probe's account hold the product's block — the
        seeded Windows PowerShell profile still beginning with what the user
        had — or, with -Absent, neither does.
    #>
    param([string] $Step, [object] $Probe, [switch] $Absent, [string] $Seed = '')
    $marker = "# >>> TigerSetup completion: TigerSetupTestAction ($($facts.id)) >>>"
    foreach ($edition in @('WindowsPowerShell', 'PowerShell')) {
        $entry = Get-Profile $Probe $edition
        $text = Get-ProfileText $entry
        $has = $text.Contains($marker)
        if ($Absent) {
            Add-Check -Name "$Step/$edition profile of $($Probe.user) has no block" -Code "completion.$Step.block.absent" -Pass (-not $has) -Message (Get-Member2 $entry 'path')
        }
        else {
            Add-Check -Name "$Step/$edition profile of $($Probe.user) has the block" -Code "completion.$Step.block" -Pass $has -Message "$(Get-Member2 $entry 'path'): $text"
        }
    }
    if (-not $Absent -and $Seed -ne '') {
        $entry = Get-Profile $Probe 'WindowsPowerShell'
        $bytes = $(if ($null -ne $entry -and [bool] $entry.exists) { [Convert]::FromBase64String([string] $entry.base64) } else { [byte[]] @() })
        $prefix = [Convert]::FromBase64String($Seed)
        $kept = ($bytes.Length -ge $prefix.Length) -and ([Convert]::ToBase64String([byte[]] $bytes[0..($prefix.Length - 1)]) -eq $Seed)
        Add-Check -Name "$Step/the user's own profile content is kept as it was" -Code "completion.$Step.seed.kept" -Pass $kept -Message "$($bytes.Length) bytes"
    }
}
function Add-RestoredChecks {
    <# The account's profiles are exactly what they were before anything ran. #>
    param([string] $Step, [object] $Probe, [object] $Before)
    foreach ($edition in @('WindowsPowerShell', 'PowerShell')) {
        $now = Get-Profile $Probe $edition
        $then = Get-Profile $Before $edition
        $same = ($null -ne $now -and $null -ne $then) -and ([bool] $now.exists -eq [bool] $then.exists) -and `
            ([string] $now.base64 -eq [string] $then.base64) -and ([bool] $now.directoryExists -eq [bool] $then.directoryExists)
        Add-Check -Name "$Step/$edition profile of $(Get-Member2 $Probe 'user') is as it was" -Code "completion.$Step.restored" -Pass $same `
            -Message "file $(Get-Member2 $then 'exists')->$(Get-Member2 $now 'exists'), folder $(Get-Member2 $then 'directoryExists')->$(Get-Member2 $now 'directoryExists')"
    }
}
function Add-CompletionChecks {
    <#
        Completion answers in 5.1 allowed to run scripts and in PowerShell 7;
        5.1 under the machine's own policy answers exactly when the install
        did not report that the policy runs no profile.
    #>
    param([string] $Step, [object] $Evidence, [string] $Prefix, [string[]] $InstallCodes, [switch] $AllowingScriptsOnly)
    foreach ($name in @("$Prefix-complete-51", "$Prefix-complete-7")) {
        $probe = Add-CommandCheck -Evidence $Evidence -Step $Step -Name $name
        $found = @(Get-Member2 $probe 'matches')
        Add-Check -Name "$Step/$name offers $completed" -Code "completion.$Step.complete" -Pass ($found -contains $completed) `
            -Message "matches [$($found -join ', ')]; stderr $(Get-Member2 $probe 'stderr')"
    }
    # The machine's own policy is the install's finding to report; a run that
    # did not install afresh (an upgrade, a recovery) need not repeat it.
    if ($AllowingScriptsOnly) { return }
    $probe = Add-CommandCheck -Evidence $Evidence -Step $Step -Name "$Prefix-complete-51-default"
    $found = @(Get-Member2 $probe 'matches')
    $blocked = $InstallCodes -contains 'windows_powershell_profiles_not_run'
    Add-Check -Name "$Step/5.1 under the default policy $(if ($blocked) { 'runs no profile, as reported' } else { 'loads the block' })" -Code "completion.$Step.default-policy" `
        -Pass (($found -contains $completed) -ne $blocked) -Message "reported not run: $blocked; matches [$($found -join ', ')]"
}
function Invoke-Step {
    param([string] $Step, [hashtable] $Request, [string[]] $PayloadFiles = @(), [switch] $FromBaseline, [string] $InteractiveKind = '')
    $policy = Get-TigerSetupRowStepPolicy -FromBaseline:$FromBaseline
    $run = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $Request -PayloadFiles $PayloadFiles `
        -Name "ts-completion-$Step" @policy -ResultPath (Join-Path $ResultsRoot "runs\$Step.json") -OutputRoot $labOutputRoot `
        -InteractiveKind $InteractiveKind -TimeoutMinutes $JobTimeoutMinutes
    Add-Check -Name "$Step/lab job" -Code "completion.$Step.lab" -Pass ($run.status -eq 'OK') -Message "status $($run.status) (exit $($run.exitCode)) after $($run.durationSeconds)s"
    if ($null -eq $run -or $run.status -ne 'OK' -or $null -eq $run.result) { return $null }
    Get-Member2 $run.result 'result'
}
function Get-ProbeJson {
    param([object] $Evidence, [string] $Step, [string] $Name)
    Add-CommandCheck -Evidence $Evidence -Step $Step -Name $Name
}
function New-PrepareRequest {
    <# Stage the installers, the probe and PowerShell 7, unpack it, and seed the signed-in user's profile. #>
    $stage = @(
        @{ source = [System.IO.Path]::GetFileName($InstallerPath); destination = $guestInstaller },
        @{ source = 'Invoke-CompletionProbe.ps1'; destination = $guestProbe },
        @{ source = $pwshZipName; destination = $guestZip }
    )
    if ($withUpgrade) { $stage += @{ source = [System.IO.Path]::GetFileName($UpgradeInstallerPath); destination = $guestUpgradeInstaller } }
    @{
        stage = $stage
        commands = @(
            @{ name = 'mkdir'; executable = 'cmd.exe'; arguments = @('/c', 'mkdir', $guestLogRoot, '&&', 'mkdir', $guestPwshRoot); runAs = 'job'; timeoutSeconds = 30 },
            @{ name = 'unpack-pwsh'; executable = 'tar.exe'; arguments = @('-xf', $guestZip, '-C', $guestPwshRoot); runAs = 'job'; timeoutSeconds = 300 },
            @{ name = 'pwsh-version'; executable = $guestPwsh; arguments = @('-NoLogo', '-NoProfile', '-Command', '$PSVersionTable.PSVersion.ToString()'); runAs = 'job'; timeoutSeconds = 120 },
            (New-ProbeCommand -Name 'seed' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Seed', '-SeedBase64', $seedBase64)),
            (New-ProbeCommand -Name 'elevated-profiles' -RunAs 'elevatedUser' -Arguments @('-Mode', 'Profiles'))
        )
    }
}

if ([string]::IsNullOrWhiteSpace($SessionId)) {
    $SessionId = 'tigersetup-completion-' + [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
}
Write-Host "Installer $([System.IO.Path]::GetFileName($InstallerPath)) ($($facts.id) $($facts.version)), PowerShell $pwshVersion"
$null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId `
    -Description 'TigerSetup PowerShell completion rows' `
    -ResultPath (Join-Path $ResultsRoot 'session-open.json')
Write-Host "Lab session $SessionId"

$environment = $null
$payload = @($InstallerPath, $probePath, $pwshZip)
if ($withUpgrade) { $payload += $UpgradeInstallerPath }
try {
    if ($Rows -contains 'profiles') {
        # ===== user: the standard user's own installation =====
        Write-Host ''; Write-Host '### user-prepare'
        $prepared = Invoke-Step -Step 'user-prepare' -FromBaseline -PayloadFiles $payload -Request (New-PrepareRequest)
        $environment = Get-Member2 $prepared 'environment'
        $null = Add-CommandCheck -Evidence $prepared -Step 'user-prepare' -Name 'unpack-pwsh'
        $version = Get-Command2 $prepared 'pwsh-version'
        Add-Check -Name "user-prepare/PowerShell $pwshVersion runs" -Code 'completion.user-prepare.pwsh' -Pass ([string] (Get-Member2 $version 'stdout')).Trim().StartsWith($pwshVersion) -Message ([string] (Get-Member2 $version 'stdout'))
        $userSeeded = Get-ProbeJson $prepared 'user-prepare' 'seed'
        $adminBefore = Get-ProbeJson $prepared 'user-prepare' 'elevated-profiles'

        Write-Host ''; Write-Host '### user-install'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'install' -RunAs 'interactiveUser' -Arguments @('install', '--quiet', '--scope', 'user', '--install-root', $userInstallRoot)),
                (New-SetupCommand -Name 'verify' -RunAs 'interactiveUser' -Arguments @('verify', '--scope', 'user')),
                (New-SetupCommand -Name 'inspect' -RunAs 'interactiveUser' -Arguments @('inspect', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            ) + (New-CompletionProbes -Prefix 'user' -RunAs 'interactiveUser')
        }
        $installed = Invoke-Step -Step 'user-install' -Request $request
        $document = Add-CommandCheck -Evidence $installed -Step 'user-install' -Name 'install' -Outcome 'installed'
        $installCodes = @(Get-Codes $document)
        $verify = Add-CommandCheck -Evidence $installed -Step 'user-install' -Name 'verify'
        Add-Check -Name 'user-install/verify ok with both blocks checked' -Code 'completion.user-install.verify' `
            -Pass ([string] (Get-Member2 $verify 'status') -eq 'ok' -and (Get-Member2 (Get-Member2 $verify 'counts') 'profile_blocks_ok') -eq 2) `
            -Message "status $([string] (Get-Member2 $verify 'status')); counts $((Get-Member2 $verify 'counts') | ConvertTo-Json -Compress)"
        $inspect = Add-CommandCheck -Evidence $installed -Step 'user-install' -Name 'inspect'
        $owned = @(@(Get-Member2 (Get-Member2 $inspect 'owned') 'profile_blocks') | Where-Object { $null -ne $_ })
        Add-Check -Name 'user-install/inspect owns two blocks of the installing user' -Code 'completion.user-install.owned' `
            -Pass ($owned.Count -eq 2 -and @($owned | Where-Object { $_.user_sid -ne $userSeeded.sid }).Count -eq 0) -Message (($owned | ConvertTo-Json -Compress -Depth 3))
        $profiles = Get-ProbeJson $installed 'user-install' 'profiles'
        Add-BlockChecks -Step 'user-install' -Probe $profiles -Seed $seedBase64
        Add-CompletionChecks -Step 'user-install' -Evidence $installed -Prefix 'user' -InstallCodes $installCodes
        $afterInstall = $profiles

        Write-Host ''; Write-Host '### user-change'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'reinstall' -RunAs 'interactiveUser' -Arguments @('install', '--quiet', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles-reinstalled' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles')),
                (New-SetupCommand -Name 'path-off' -RunAs 'interactiveUser' -Arguments @('install', '--quiet', '--scope', 'user', '--option', 'path', 'off')),
                (New-ProbeCommand -Name 'profiles-path-off' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles')),
                (New-SetupCommand -Name 'path-on' -RunAs 'interactiveUser' -Arguments @('install', '--quiet', '--scope', 'user', '--option', 'path', 'on')),
                (New-ProbeCommand -Name 'profiles-path-on' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            )
        }
        $changed = Invoke-Step -Step 'user-change' -Request $request
        $null = Add-CommandCheck -Evidence $changed -Step 'user-change' -Name 'reinstall' -Outcome 'installed'
        Add-RestoredChecks -Step 'user-change/reinstall' -Probe (Get-ProbeJson $changed 'user-change' 'profiles-reinstalled') -Before $afterInstall
        $null = Add-CommandCheck -Evidence $changed -Step 'user-change' -Name 'path-off' -Outcome 'installed'
        Add-RestoredChecks -Step 'user-change/path-off' -Probe (Get-ProbeJson $changed 'user-change' 'profiles-path-off') -Before $userSeeded
        $null = Add-CommandCheck -Evidence $changed -Step 'user-change' -Name 'path-on' -Outcome 'installed'
        Add-BlockChecks -Step 'user-change/path-on' -Probe (Get-ProbeJson $changed 'user-change' 'profiles-path-on') -Seed $seedBase64

        Write-Host ''; Write-Host '### user-uninstall'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'uninstall' -RunAs 'interactiveUser' -Arguments @('uninstall', '--quiet', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            )
        }
        $uninstalled = Invoke-Step -Step 'user-uninstall' -Request $request
        $null = Add-CommandCheck -Evidence $uninstalled -Step 'user-uninstall' -Name 'uninstall' -Outcome 'uninstalled'
        Add-RestoredChecks -Step 'user-uninstall' -Probe (Get-ProbeJson $uninstalled 'user-uninstall' 'profiles') -Before $userSeeded

        # ===== machine: installed by an account that is not the desktop's =====
        Write-Host ''; Write-Host '### machine-other-user'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'install' -RunAs 'elevatedUser' -Arguments @('install', '--quiet', '--scope', 'machine')),
                (New-SetupCommand -Name 'repair' -RunAs 'job' -Arguments @('repair', '--quiet', '--scope', 'machine')),
                (New-SetupCommand -Name 'inspect' -RunAs 'job' -Arguments @('inspect', '--scope', 'machine')),
                (New-ProbeCommand -Name 'user-profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles')),
                (New-ProbeCommand -Name 'admin-profiles' -RunAs 'elevatedUser' -Arguments @('-Mode', 'Profiles')),
                (New-SetupCommand -Name 'uninstall' -RunAs 'job' -Arguments @('uninstall', '--quiet', '--scope', 'machine'))
            )
        }
        $other = Invoke-Step -Step 'machine-other-user' -Request $request
        $document = Add-CommandCheck -Evidence $other -Step 'machine-other-user' -Name 'install' -Outcome 'installed'
        Add-Check -Name 'machine-other-user/an elevated install beside the signed-in user activates nothing' -Code 'completion.machine.elevated.unreached' `
            -Pass (@(Get-Codes $document) -contains 'completion_installer_not_desktop_user') -Message ((Get-Codes $document) -join ', ')
        $document = Add-CommandCheck -Evidence $other -Step 'machine-other-user' -Name 'repair' -Outcome 'installed'
        Add-Check -Name 'machine-other-user/a repair with no desktop activates nothing' -Code 'completion.machine.job.unreached' `
            -Pass (@(Get-Codes $document) -contains 'completion_no_desktop_user') -Message ((Get-Codes $document) -join ', ')
        $inspect = Add-CommandCheck -Evidence $other -Step 'machine-other-user' -Name 'inspect'
        Add-Check -Name 'machine-other-user/inspect owns no block' -Code 'completion.machine.owned' `
            -Pass ($null -ne $inspect -and @(@(Get-Member2 (Get-Member2 $inspect 'owned') 'profile_blocks') | Where-Object { $null -ne $_ }).Count -eq 0)
        Add-RestoredChecks -Step 'machine-other-user/signed-in user' -Probe (Get-ProbeJson $other 'machine-other-user' 'user-profiles') -Before $userSeeded
        Add-RestoredChecks -Step 'machine-other-user/administrator' -Probe (Get-ProbeJson $other 'machine-other-user' 'admin-profiles') -Before $adminBefore
        $null = Add-CommandCheck -Evidence $other -Step 'machine-other-user' -Name 'uninstall' -Outcome 'uninstalled'

        # ===== admin: a machine-wide install by the administrator signed in =====
        Write-Host ''; Write-Host '### admin-prepare'
        $prepared = Invoke-Step -Step 'admin-prepare' -FromBaseline -InteractiveKind administrator -PayloadFiles $payload -Request (New-PrepareRequest)
        $null = Add-CommandCheck -Evidence $prepared -Step 'admin-prepare' -Name 'unpack-pwsh'
        $adminSeeded = Get-ProbeJson $prepared 'admin-prepare' 'seed'
        $elevated = Get-ProbeJson $prepared 'admin-prepare' 'elevated-profiles'
        Add-Check -Name 'admin-prepare/the elevated session is the signed-in administrator' -Code 'completion.admin-prepare.same-user' `
            -Pass ($null -ne $adminSeeded -and $null -ne $elevated -and $adminSeeded.sid -eq $elevated.sid) -Message "$(Get-Member2 $adminSeeded 'user') / $(Get-Member2 $elevated 'user')"

        Write-Host ''; Write-Host '### admin-install'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'install' -RunAs 'elevatedUser' -Arguments @('install', '--quiet', '--scope', 'machine')),
                (New-SetupCommand -Name 'verify' -RunAs 'elevatedUser' -Arguments @('verify', '--scope', 'machine')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            ) + (New-CompletionProbes -Prefix 'admin' -RunAs 'interactiveUser') + @(
                (New-SetupCommand -Name 'uninstall' -RunAs 'elevatedUser' -Arguments @('uninstall', '--quiet', '--scope', 'machine')),
                (New-ProbeCommand -Name 'profiles-after' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            )
        }
        $admin = Invoke-Step -Step 'admin-install' -InteractiveKind administrator -Request $request
        $document = Add-CommandCheck -Evidence $admin -Step 'admin-install' -Name 'install' -Outcome 'installed'
        $installCodes = @(Get-Codes $document)
        $verify = Add-CommandCheck -Evidence $admin -Step 'admin-install' -Name 'verify'
        Add-Check -Name 'admin-install/verify ok with both blocks checked' -Code 'completion.admin-install.verify' `
            -Pass ([string] (Get-Member2 $verify 'status') -eq 'ok' -and (Get-Member2 (Get-Member2 $verify 'counts') 'profile_blocks_ok') -eq 2) `
            -Message "status $([string] (Get-Member2 $verify 'status')); counts $((Get-Member2 $verify 'counts') | ConvertTo-Json -Compress)"
        Add-BlockChecks -Step 'admin-install' -Probe (Get-ProbeJson $admin 'admin-install' 'profiles') -Seed $seedBase64
        Add-CompletionChecks -Step 'admin-install' -Evidence $admin -Prefix 'admin' -InstallCodes $installCodes
        $null = Add-CommandCheck -Evidence $admin -Step 'admin-install' -Name 'uninstall' -Outcome 'uninstalled'
        Add-RestoredChecks -Step 'admin-install/uninstall' -Probe (Get-ProbeJson $admin 'admin-install' 'profiles-after') -Before $adminSeeded
    }

    if ($withUpgrade) {
        # ===== upgrade-recovery: an interrupted transaction and an upgrade, on the user's own profiles =====
        Write-Host ''; Write-Host '### recovery-prepare'
        $prepared = Invoke-Step -Step 'recovery-prepare' -FromBaseline -PayloadFiles $payload -Request (New-PrepareRequest)
        if ($null -eq $environment) { $environment = Get-Member2 $prepared 'environment' }
        $null = Add-CommandCheck -Evidence $prepared -Step 'recovery-prepare' -Name 'unpack-pwsh'
        $seeded = Get-ProbeJson $prepared 'recovery-prepare' 'seed'
        $crash = @('install', '--quiet', '--scope', 'user', '--fault', 'before_commit:crash')

        # A crash after the blocks were written, then an uninstall: the
        # interrupted install is taken back and the profiles are as they were.
        Write-Host ''; Write-Host '### crash-back'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'crash' -RunAs 'interactiveUser' -Arguments $crash),
                (New-ProbeCommand -Name 'profiles-crashed' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles')),
                (New-SetupCommand -Name 'uninstall' -RunAs 'interactiveUser' -Arguments @('uninstall', '--quiet', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            )
        }
        $back = Invoke-Step -Step 'crash-back' -Request $request
        $crashed = Get-Command2 $back 'crash'
        Add-Check -Name 'crash-back/the injected crash ended the install' -Code 'completion.crash-back.crashed' `
            -Pass ($null -ne $crashed -and (Get-Member2 $crashed 'exitCode') -ne 0) -Message "exit $(Get-Member2 $crashed 'exitCode')"
        Add-BlockChecks -Step 'crash-back/crashed' -Probe (Get-ProbeJson $back 'crash-back' 'profiles-crashed') -Seed $seedBase64
        $null = Add-CommandCheck -Evidence $back -Step 'crash-back' -Name 'uninstall'
        Add-RestoredChecks -Step 'crash-back/uninstall' -Probe (Get-ProbeJson $back 'crash-back' 'profiles') -Before $seeded

        # The same crash, then an install: the transaction completes forward.
        Write-Host ''; Write-Host '### crash-forward'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'crash' -RunAs 'interactiveUser' -Arguments $crash),
                (New-SetupCommand -Name 'install' -RunAs 'interactiveUser' -Arguments @('install', '--quiet', '--scope', 'user')),
                (New-SetupCommand -Name 'verify' -RunAs 'interactiveUser' -Arguments @('verify', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            ) + (New-CompletionProbes -Prefix 'forward' -RunAs 'interactiveUser')
        }
        $forward = Invoke-Step -Step 'crash-forward' -Request $request
        $crashed = Get-Command2 $forward 'crash'
        Add-Check -Name 'crash-forward/the injected crash ended the install' -Code 'completion.crash-forward.crashed' `
            -Pass ($null -ne $crashed -and (Get-Member2 $crashed 'exitCode') -ne 0) -Message "exit $(Get-Member2 $crashed 'exitCode')"
        $document = Add-CommandCheck -Evidence $forward -Step 'crash-forward' -Name 'install' -Outcome 'installed'
        $direction = [string] (Get-Member2 (Get-Member2 $document 'recovery') 'direction')
        Add-Check -Name 'crash-forward/the install completed the interrupted transaction forward' -Code 'completion.crash-forward.recovery' `
            -Pass ($direction -eq 'forward') -Message "recovery direction '$direction'"
        $verify = Add-CommandCheck -Evidence $forward -Step 'crash-forward' -Name 'verify'
        Add-Check -Name 'crash-forward/verify ok with both blocks checked' -Code 'completion.crash-forward.verify' `
            -Pass ([string] (Get-Member2 $verify 'status') -eq 'ok' -and (Get-Member2 (Get-Member2 $verify 'counts') 'profile_blocks_ok') -eq 2) `
            -Message "status $([string] (Get-Member2 $verify 'status')); counts $((Get-Member2 $verify 'counts') | ConvertTo-Json -Compress)"
        $profiles = Get-ProbeJson $forward 'crash-forward' 'profiles'
        Add-BlockChecks -Step 'crash-forward' -Probe $profiles -Seed $seedBase64
        $marker = "# >>> TigerSetup completion: TigerSetupTestAction ($($facts.id)) >>>"
        foreach ($edition in @('WindowsPowerShell', 'PowerShell')) {
            $count = ([regex]::Matches((Get-ProfileText (Get-Profile $profiles $edition)), [regex]::Escape($marker))).Count
            Add-Check -Name "crash-forward/$edition profile holds the block once" -Code 'completion.crash-forward.block.once' -Pass ($count -eq 1) -Message "$count"
        }
        Add-CompletionChecks -Step 'crash-forward' -Evidence $forward -Prefix 'forward' -AllowingScriptsOnly

        # The upgrade: the next version over it, the profiles untouched.
        Write-Host ''; Write-Host '### upgrade'
        $request = @{
            commands = @(
                (New-SetupCommand -Name 'upgrade' -RunAs 'interactiveUser' -Executable $guestUpgradeInstaller -Arguments @('install', '--quiet', '--scope', 'user')),
                (New-SetupCommand -Name 'verify' -RunAs 'interactiveUser' -Executable $guestUpgradeInstaller -Arguments @('verify', '--scope', 'user')),
                (New-SetupCommand -Name 'inspect' -RunAs 'interactiveUser' -Executable $guestUpgradeInstaller -Arguments @('inspect', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            ) + (New-CompletionProbes -Prefix 'upgraded' -RunAs 'interactiveUser') + @(
                (New-SetupCommand -Name 'uninstall' -RunAs 'interactiveUser' -Executable $guestUpgradeInstaller -Arguments @('uninstall', '--quiet', '--scope', 'user')),
                (New-ProbeCommand -Name 'profiles-after' -RunAs 'interactiveUser' -Arguments @('-Mode', 'Profiles'))
            )
        }
        $upgraded = Invoke-Step -Step 'upgrade' -Request $request
        $null = Add-CommandCheck -Evidence $upgraded -Step 'upgrade' -Name 'upgrade' -Outcome 'installed'
        $verify = Add-CommandCheck -Evidence $upgraded -Step 'upgrade' -Name 'verify'
        Add-Check -Name 'upgrade/verify ok with both blocks checked' -Code 'completion.upgrade.verify' `
            -Pass ([string] (Get-Member2 $verify 'status') -eq 'ok' -and (Get-Member2 (Get-Member2 $verify 'counts') 'profile_blocks_ok') -eq 2) `
            -Message "status $([string] (Get-Member2 $verify 'status')); counts $((Get-Member2 $verify 'counts') | ConvertTo-Json -Compress)"
        $inspect = Add-CommandCheck -Evidence $upgraded -Step 'upgrade' -Name 'inspect'
        $installedVersion = [string] (Get-Member2 (Get-Member2 $inspect 'installation') 'version')
        Add-Check -Name "upgrade/the installation is $($upgradeFacts.version)" -Code 'completion.upgrade.version' `
            -Pass ($installedVersion -eq [string] $upgradeFacts.version) -Message "inspect reports '$installedVersion'"
        Add-RestoredChecks -Step 'upgrade' -Probe (Get-ProbeJson $upgraded 'upgrade' 'profiles') -Before $profiles
        Add-CompletionChecks -Step 'upgrade' -Evidence $upgraded -Prefix 'upgraded' -AllowingScriptsOnly
        $null = Add-CommandCheck -Evidence $upgraded -Step 'upgrade' -Name 'uninstall' -Outcome 'uninstalled'
        Add-RestoredChecks -Step 'upgrade/uninstall' -Probe (Get-ProbeJson $upgraded 'upgrade' 'profiles-after') -Before $seeded
    }
}
finally {
    $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $SessionId -ResultPath (Join-Path $ResultsRoot 'session-close.json')
}

$result = Write-TigerSetupRowResult -Row 'completion' -Checks $checks.ToArray() -OutputPath (Join-Path $ResultsRoot 'completion.json') `
    -Environment $environment -Evidence @{ installer = $InstallerPath; package = $facts.id; version = $facts.version; engineSha256 = $facts.engineSha256; pwsh = $pwshVersion }
if ($result.status -eq 'FAIL') { exit 1 }
exit 0
