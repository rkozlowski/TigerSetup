#Requires -Version 7.0
<#
    .SYNOPSIS
    Runs the broad-corpus benchmark's installers (15 applications x 3
    technologies) through TigerWinLab's clean Windows 11 baseline, one row
    at a time: silent install into an explicit root, the installed payload
    verified file by file against the canonical inventory, the technology's
    bookkeeping measured, silent uninstall from the uninstaller Add/Remove
    Programs registered, the removal read from the machine — with the
    process lifecycle and the lab lifecycle of one row both complete before
    the next row begins.

    .DESCRIPTION
    Every package implements the one common contract in
    packages\broad\contract.md (install the canonical tree, register with
    Add/Remove Programs, remove all of it), so a row is the same in every
    technology:

      session opens
      install job     (EntryPolicy Baseline: the VM is restored to the clean
                       checkpoint) stages the installer, runs it silently
                       into C:\TigerSetupBenchmark\apps\<App>-<Tech>, then
                       reads the root (every file's SHA-256, after the timed
                       command), the Add/Remove Programs key and the
                       package-owned state the technology keeps outside the
                       root
      uninstall job   (EntryPolicy DontCare: the same VM, as the install
                       left it) runs the uninstaller the registration named,
                       silently, and reads the same evidence again
      session closes  the lab takes the VM back and normalizes it
      wait            until the lab reports the VM Available again

    Timing has the same boundary in every technology. An install's elapsed
    time is the installer process's own lifetime plus the exit of any
    process of the installer's own name it left behind (TigerSetup's loader
    waits for its engine, so that wait is nil; it is kept so a hand-off
    could never escape the clock). An uninstall's elapsed time is the
    uninstaller's lifetime plus what it hands off to — an NSIS uninstaller
    copies itself to %TEMP%\~nsuX.tmp\Au_.exe and exits at once while the
    copy does the work; Inno Setup's unins000.exe waits for its second
    phase itself; TigerSetup's state-directory uninstaller runs the engine
    and a helper deletes the state directory after it exits — and the
    removal of the install root and of the package-owned state outside it.
    The guest reader (lab\guest\Invoke-SetupCommands.ps1) implements both
    waits; each row records the process lifetime and the completion wait
    separately, and there is no artificial settle delay anywhere in the
    measured path.

    All commands run as the job account ("job": LabAdmin, session 0, an
    administrator with no interactive desktop): every installer invocation
    here is silent, in user scope, and no technology elevates.

    Rows run in corpus order with the technology order rotating with the
    application's index — the same rotation Build-Installers.ps1 used — so
    no technology always follows a fresh baseline restore or a long row.
    Every row records the installer's hash and the tool version it was
    built with (build.json beside the results root), so a runtime figure is
    tied to the exact bytes it measured.

    Two kinds of evidence come out of a campaign, and they are kept apart.
    -OutputJson is the **compact record** and is committed: one row per
    installer, carrying the installer it ran (hash, bytes, tool version),
    the canonical inventory that decided its payload verdict (path and that
    document's SHA-256), the counts and the full path lists of whatever did
    not match, the bookkeeping the technology wrote, the cleanup verdicts
    and the lab job ids. -ResultsRoot is the **raw evidence** — the lab job
    documents with every installed file's SHA-256, the session records, the
    VM states, the job folders and their logs — which a passing row
    duplicates from the canonical inventory and the compact record, and
    which is therefore transient and gitignored. It stays on the build
    machine for as long as it is useful; nothing the report or an audit
    needs is only there.

    .EXAMPLE
    pwsh -File benchmark\scripts\Invoke-BroadBenchmarkLab.ps1 -ResultsRoot benchmark\results\0.10.0-broad\lab -OutputJson benchmark\results\0.10.0-broad\lab-results.json -ArtifactsRoot benchmark\artifacts\0.10.0-broad
    pwsh -File benchmark\scripts\Invoke-BroadBenchmarkLab.ps1 -OnlyRows TigerKeyring-NSIS -ResultsRoot benchmark\results\0.10.0-broad\smoke\lab -OutputJson benchmark\results\0.10.0-broad\smoke\lab-results.json -ArtifactsRoot benchmark\artifacts\0.10.0-broad -BuildJson benchmark\results\0.10.0-broad\build.json -Fresh
#>
[CmdletBinding()]
param(
    [string] $TigerWinLabRoot,
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $ArtifactsRoot = (Join-Path $PSScriptRoot '..\artifacts\0.10.0-broad'),
    # Raw lab evidence: the job documents, session records, VM states, the
    # job folders and their logs. Transient and gitignored (results/*/lab/);
    # the campaign's committed record is -OutputJson.
    [string] $ResultsRoot = (Join-Path $PSScriptRoot '..\results\0.10.0-broad\lab'),
    # The campaign's compact, committed record: one row per installer, with
    # everything the report and an audit need.
    [string] $OutputJson = (Join-Path $PSScriptRoot '..\results\0.10.0-broad\lab-results.json'),
    [string] $CorpusJson = (Join-Path $PSScriptRoot '..\results\0.10.0-broad\corpus.json'),
    [string] $SessionPrefix = ('broad-' + (Get-Date -Format 'yyyyMMdd-HHmmss')),
    # The largest payload is a gigabyte and twelve thousand files; the job
    # timeout also covers staging the installer and hashing the tree.
    [int] $InstallTimeoutMinutes = 30,
    [int] $UninstallTimeoutMinutes = 15,
    [int] $NormalizeTimeoutMinutes = 10,
    [string[]] $OnlyRows,
    [string[]] $OnlyApps,
    [string[]] $OnlyTechnologies,
    # The build record the installers under -ArtifactsRoot came from; each
    # row records its installer's hash and tool version from it.
    [string] $BuildJson,
    # Start the results file over. Without it, rows this run measures replace
    # their earlier records in an existing results file and every other row
    # is kept, so a row re-measured after a fix joins the campaign it belongs
    # to (the record says when each row was measured).
    [switch] $Fresh
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\lab\TigerSetupLab.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'BenchmarkRow.psm1') -Force

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$labOutputRoot = Join-Path $ResultsRoot 'jobs'
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$null = New-Item -ItemType Directory -Path $labOutputRoot -Force
$OnlyRows = @($OnlyRows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$OnlyApps = @($OnlyApps | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$OnlyTechnologies = @($OnlyTechnologies | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ([string]::IsNullOrWhiteSpace($BuildJson)) { $BuildJson = Join-Path (Split-Path -Parent $ResultsRoot) 'build.json' }
$buildRecord = $null
if (Test-Path -LiteralPath $BuildJson -PathType Leaf) { $buildRecord = Get-Content -LiteralPath $BuildJson -Raw | ConvertFrom-Json }
else { Write-Warning "No build record at '$BuildJson'; rows will not carry their installer's build identity." }

$corpus = Get-Content -LiteralPath $CorpusJson -Raw | ConvertFrom-Json
$corpusRoot = Split-Path -Parent ([System.IO.Path]::GetFullPath($CorpusJson))
$canonical = @{}
$canonicalIdentity = @{}
$corpusApps = @{}
foreach ($app in @($corpus.apps)) {
    $corpusApps[[string] $app.app] = $app
    $inventoryPath = Join-Path $corpusRoot ([string] $app.inventory)
    $canonical[[string] $app.app] = Get-Content -LiteralPath $inventoryPath -Raw | ConvertFrom-Json
    # The row records which inventory decided its payload verdict, and that
    # document's own SHA-256, so a compact row is auditable on its own.
    $canonicalIdentity[[string] $app.app] = [ordered]@{
        path = [string] $app.inventory
        sha256 = (Get-FileHash -LiteralPath $inventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

# Guest paths are fixed and explicit: every technology installs where the
# command line says, and the evidence is looked for there afterwards.
$guestInstallerDir = 'C:\TigerSetupBenchmark\installers'
$guestInstallRoot = 'C:\TigerSetupBenchmark\apps'
$art = (Resolve-Path -LiteralPath $ArtifactsRoot).Path

# Technology-specific invocation and locations. {root} is the row's install
# root, {installer} the staged installer, {app} the application name. The
# uninstaller is not named here: the row reads it from the UninstallString
# the install registered, so what runs is what Add/Remove Programs would run.
$technologies = @{
    TigerSetup = @{
        install = 'install --quiet --scope user --install-root "{root}" --log "{root}.install.log"'
        uninstall = 'uninstall --quiet --log "{root}.uninstall.log"'
        # The loader waits for the engine it extracts, and the engine keeps
        # the file name it was started under; waiting on that name follows
        # the whole operation whatever process carries it.
        installWaitProcesses = @('{installerName}')
        uninstallWaitProcesses = @('{uninstallerName}')
        arpKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Benchmark.{app}'
        # What the technology keeps outside the root while the package is
        # installed, and must remove with it: the state directory (the
        # database, the log, the uninstaller copy).
        packageOwnedPaths = @('%LOCALAPPDATA%\TigerSetup\Benchmark.{app}')
        # Bookkeeping the technology is expected to put under the root.
        expectedExtras = @()
        # Where the technology's hand-off leaves its scratch; recorded, not judged.
        residuePaths = @()
        implementation = 'declarative manifest'
    }
    InnoSetup = @{
        install = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /DIR="{root}" /LOG="{root}.install.log"'
        uninstall = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="{root}.uninstall.log"'
        # unins000.exe's first phase waits for its second phase (the copy in
        # %TEMP%) itself, so its own lifetime is the uninstall's.
        installWaitProcesses = @()
        uninstallWaitProcesses = @()
        arpKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Benchmark.{app}_is1'
        packageOwnedPaths = @()
        expectedExtras = @('unins000.exe', 'unins000.dat')
        residuePaths = @()
        implementation = 'installer script'
    }
    NSIS = @{
        # /S is what makes an NSIS installer silent; /D= must be last and unquoted.
        install = '/S /D={root}'
        uninstall = '/S'
        # Uninstall.exe copies itself to %TEMP%\~nsuX.tmp\Au_.exe and exits;
        # the copy does the work and is what the clock must follow.
        installWaitProcesses = @()
        uninstallWaitProcesses = @('Au_')
        arpKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Benchmark.{app}'
        packageOwnedPaths = @()
        expectedExtras = @('Uninstall.exe')
        # The first uninstaller copy on a fresh profile is ~nsuA.tmp; NSIS
        # marks it for deletion at the next reboot.
        residuePaths = @('%TEMP%\~nsuA.tmp')
        implementation = 'installer script'
    }
}
$rotation = @(@('TigerSetup', 'InnoSetup', 'NSIS'), @('InnoSetup', 'NSIS', 'TigerSetup'), @('NSIS', 'TigerSetup', 'InnoSetup'))

function Get-BuildOf {
    param([string] $App, [string] $Tech)
    if ($null -eq $buildRecord) { return $null }
    @($buildRecord.builds | Where-Object { $_.app -eq $App -and $_.technology -eq $Tech }) | Select-Object -First 1
}

function Expand-RowTemplate {
    param([string] $Template, [object] $Row)
    $Template.Replace('{root}', $Row.installRoot).Replace('{installer}', $Row.guestInstaller).Replace('{app}', $Row.app).Replace('{installerName}', $Row.installerName)
}

function New-Row {
    param([string] $App, [string] $Tech)
    $t = $technologies[$Tech]
    $built = Get-BuildOf -App $App -Tech $Tech
    $row = [ordered]@{
        row = "$App-$Tech"
        app = $App
        tech = $Tech
        scope = 'user'
        localFile = (Join-Path $art "$($App.ToLowerInvariant())\$App-$Tech.exe")
        guestInstaller = "$guestInstallerDir\$App-$Tech.exe"
        installerName = "$App-$Tech"
        installRoot = "$guestInstallRoot\$App-$Tech"
        toolVersion = $(if ($null -ne $built) { [string] (Get-Prop $built 'toolVersion') } else { '' })
        installerSha256 = $(if ($null -ne $built) { [string] (Get-Prop $built 'installerSha256') } else { '' })
        installerBytes = $(if ($null -ne $built) { Get-Prop $built 'installerBytes' } else { $null })
        implementation = $t.implementation
    }
    $row.installArgs = Expand-RowTemplate $t.install $row
    $row.uninstallArgs = Expand-RowTemplate $t.uninstall $row
    $row.installWaitProcesses = @($t.installWaitProcesses | ForEach-Object { Expand-RowTemplate $_ $row })
    $row.uninstallWaitProcessTemplates = @($t.uninstallWaitProcesses)
    $row.arpKey = Expand-RowTemplate $t.arpKey $row
    $row.packageOwnedPaths = @($t.packageOwnedPaths | ForEach-Object { Expand-RowTemplate $_ $row })
    $row.expectedExtras = @($t.expectedExtras)
    $row.residuePaths = @($t.residuePaths)
    [pscustomobject] $row
}

$order = [System.Collections.Generic.List[object]]::new()
$index = 0
foreach ($app in @($corpus.apps)) {
    $order.Add(@([string] $app.app, $rotation[$index % 3]))
    $index++
}
$rows = @(foreach ($pair in $order) { foreach ($tech in $pair[1]) { New-Row -App $pair[0] -Tech $tech } })
if ($OnlyApps.Count -gt 0) {
    $unknownApp = @($OnlyApps | Where-Object { $_ -notin $corpusApps.Keys })
    if ($unknownApp.Count -gt 0) { throw "Unknown application(s): $($unknownApp -join ', ')." }
    $rows = @($rows | Where-Object { $_.app -in $OnlyApps })
}
if ($OnlyTechnologies.Count -gt 0) {
    $unknownTech = @($OnlyTechnologies | Where-Object { $_ -notin $technologies.Keys })
    if ($unknownTech.Count -gt 0) { throw "Unknown technology(ies): $($unknownTech -join ', ')." }
    $rows = @($rows | Where-Object { $_.tech -in $OnlyTechnologies })
}
if ($OnlyRows.Count -gt 0) {
    $rows = @($rows | Where-Object { $_.row -in $OnlyRows })
    $unknown = @($OnlyRows | Where-Object { $_ -notin @($rows | ForEach-Object { $_.row }) })
    if ($unknown.Count -gt 0) { throw "Unknown row(s): $($unknown -join ', ')." }
}
foreach ($row in $rows) {
    if (-not (Test-Path -LiteralPath $row.localFile -PathType Leaf)) { throw "Installer '$($row.localFile)' does not exist; run Build-Installers.ps1 first." }
}

function Get-RegisteredUninstaller {
    <#
        The executable Add/Remove Programs would run, from the
        UninstallString the install registered: its first token, unquoted.
        $null when the key or the value is absent.
    #>
    param([object] $Evidence, [string] $ArpKey)
    $text = Get-RegistryValue $Evidence $ArpKey 'UninstallString'
    if ([string]::IsNullOrWhiteSpace($text)) { return $null }
    $argv = @(Split-CommandLine $text)
    if ($argv.Count -eq 0) { return $null }
    [string] $argv[0]
}

function Get-InventorySummary {
    <# A guest inventory record as exists/files/bytes, for the package-owned state and residue lists. #>
    param([object] $Evidence, [string] $Path)
    $record = Get-Record $Evidence 'inventory' 'requested' $Path
    [ordered]@{
        path = $Path
        exists = $(if ($null -ne $record) { [bool] (Get-Prop $record 'exists') } else { $null })
        files = $(if ($null -ne $record) { Get-Prop $record 'fileCount' } else { $null })
        bytes = $(if ($null -ne $record) { Get-Prop $record 'totalBytes' } else { $null })
    }
}

$allResults = [System.Collections.Generic.List[object]]::new()
$campaignStarted = [DateTimeOffset]::Now
$previous = $null
$labEnvironment = $null
if (-not $Fresh -and (Test-Path -LiteralPath $OutputJson -PathType Leaf)) {
    $previous = Get-Content -LiteralPath $OutputJson -Raw | ConvertFrom-Json
    foreach ($kept in @($previous.rows | Where-Object { $null -ne $_ -and $_.row -notin @($rows | ForEach-Object { $_.row }) })) { $allResults.Add($kept) }
    if ($allResults.Count -gt 0) { Write-Host "Keeping $($allResults.Count) row(s) already in $OutputJson; this run's rows replace theirs." }
    $campaignStarted = [DateTimeOffset] $previous.startedAt
    $labEnvironment = Get-Prop $previous 'labEnvironment'
}
Write-Host "Broad benchmark campaign on $Baseline ($($rows.Count) row(s)); results under $ResultsRoot"

foreach ($row in $rows) {
    Write-Host ""
    Write-Host "=== $($row.row) ($($row.scope) scope, $($row.implementation)) ==="
    $rowStarted = [DateTimeOffset]::Now
    $sessionId = "$SessionPrefix-$($row.row.ToLowerInvariant())"
    $installRun = $null
    $uninstallRun = $null
    $uninstaller = $null
    $uninstallSkipped = ''
    $null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $sessionId -Description "TigerSetup broad benchmark row $($row.row)" `
        -ResultPath (Join-Path $ResultsRoot "$($row.row)-session-open.json")
    try {
        # --- install job (fresh from the baseline) ---
        # Inno Setup's /LOG= and TigerSetup's --log need their target
        # directory to exist before the installer starts; only the
        # installer's own destination directory is guaranteed by `stage`, so
        # the parent of installRoot is created explicitly first (untimed).
        $mkdirCommand = @{
            name = 'mkdir'
            executable = 'cmd.exe'
            arguments = @('/c', 'mkdir', (Split-Path -Parent $row.installRoot))
            timeoutSeconds = 30
        }
        $installCommand = @{
            name = 'install'
            executable = $row.guestInstaller
            arguments = @(Split-CommandLine $row.installArgs)
            timeoutSeconds = ($InstallTimeoutMinutes * 60)
            waitForProcesses = @($row.installWaitProcesses)
        }
        $installRequest = @{
            stage = @(@{ source = (Split-Path -Leaf $row.localFile); destination = $row.guestInstaller })
            commands = @($mkdirCommand, $installCommand)
            runAs = 'job'
            inventory = @(@($row.installRoot) + @($row.packageOwnedPaths))
            # Every installed file's SHA-256, read after the timed command,
            # so the installed payload is compared with the canonical one
            # file by file rather than by count and size alone.
            inventoryHashes = $true
            logs = @("$($row.installRoot).install.log")
            registry = @($row.arpKey)
        }
        $installPolicy = Get-TigerSetupRowStepPolicy -FromBaseline
        $installRun = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $installRequest `
            -PayloadFiles @($row.localFile) -Name ("$($row.row)-i".ToLowerInvariant()) @installPolicy `
            -ResultPath (Join-Path $ResultsRoot "$($row.row)-install.json") -OutputRoot $labOutputRoot -TimeoutMinutes ($InstallTimeoutMinutes + 15)

        if (Test-JobOk $installRun) {
            if ($null -eq $labEnvironment) { $labEnvironment = Get-Prop $installRun.result 'environment' }
            $uninstaller = Get-RegisteredUninstaller -Evidence (Get-Evidence $installRun) -ArpKey $row.arpKey
            if ($null -eq $uninstaller) {
                $uninstallSkipped = "the install registered no UninstallString under $($row.arpKey)"
                Write-Warning "  $uninstallSkipped; uninstall not attempted"
            }
            else {
                # --- uninstall job (the same VM, as the install left it) ---
                # The uninstaller is the one Add/Remove Programs registered;
                # any process of its own name it leaves behind is followed.
                $uninstallerName = [System.IO.Path]::GetFileNameWithoutExtension($uninstaller)
                $uninstallCommand = @{
                    name = 'uninstall'
                    executable = $uninstaller
                    arguments = @(Split-CommandLine $row.uninstallArgs)
                    timeoutSeconds = ($UninstallTimeoutMinutes * 60)
                    waitForProcesses = @($row.uninstallWaitProcessTemplates | ForEach-Object { $_.Replace('{uninstallerName}', $uninstallerName) })
                    waitForAbsentPaths = @(@($row.installRoot) + @($row.packageOwnedPaths))
                }
                $uninstallRequest = @{
                    commands = @($uninstallCommand)
                    runAs = 'job'
                    logs = @("$($row.installRoot).uninstall.log")
                    inventory = @(@($row.installRoot) + @($row.packageOwnedPaths) + @($row.residuePaths))
                    registry = @($row.arpKey)
                }
                $uninstallPolicy = Get-TigerSetupRowStepPolicy
                $uninstallRun = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $uninstallRequest `
                    -Name ("$($row.row)-u".ToLowerInvariant()) @uninstallPolicy `
                    -ResultPath (Join-Path $ResultsRoot "$($row.row)-uninstall.json") -OutputRoot $labOutputRoot -TimeoutMinutes ($UninstallTimeoutMinutes + 10)
            }
        }
        else {
            Write-Warning "  install job did not complete: $(Get-JobFailureReason $installRun)"
        }
    }
    finally {
        # The row's session ends whatever happened, and the lab takes the VM
        # back; the next row waits for it to be Available again.
        $null = Exit-TigerSetupLabSession -LabRoot $labRoot -SessionId $sessionId -ResultPath (Join-Path $ResultsRoot "$($row.row)-session-close.json")
    }
    $vm = Wait-TigerSetupLabVmAvailable -LabRoot $labRoot -Baseline $Baseline -TimeoutMinutes $NormalizeTimeoutMinutes `
        -ResultPath (Join-Path $ResultsRoot "$($row.row)-vm-state.json")
    $rowFinished = [DateTimeOffset]::Now

    # --- the row's record ---
    $installEvidence = Get-Evidence $installRun
    $uninstallEvidence = Get-Evidence $uninstallRun
    $installSummary = Get-CommandSummary $installEvidence 'install'
    $uninstallSummary = Get-CommandSummary $uninstallEvidence 'uninstall'
    $installEngine = $null; $uninstallEngine = $null
    if ($row.tech -eq 'TigerSetup') {
        $installEngine = Get-EngineSpan -JobRun $installRun -LogLeaf "$(Split-Path -Leaf $row.installRoot).install.log" -Command (Get-Record $installEvidence 'commands' 'name' 'install')
        $uninstallEngine = Get-EngineSpan -JobRun $uninstallRun -LogLeaf "$(Split-Path -Leaf $row.installRoot).uninstall.log" -Command (Get-Record $uninstallEvidence 'commands' 'name' 'uninstall')
    }
    $corpusApp = $corpusApps[$row.app]
    $installInventory = Get-Record $installEvidence 'inventory' 'requested' $row.installRoot
    $uninstallInventory = Get-Record $uninstallEvidence 'inventory' 'requested' $row.installRoot
    $installArp = Get-Record $installEvidence 'registry' 'requested' $row.arpKey
    $uninstallArp = Get-Record $uninstallEvidence 'registry' 'requested' $row.arpKey
    $expectedFiles = [int] $corpusApp.packageFiles
    $expectedBytes = [long] $corpusApp.packageBytes
    $installedFiles = Get-Prop $installInventory 'fileCount'
    $installedBytes = Get-Prop $installInventory 'totalBytes'
    $payload = Compare-InstalledPayload -Inventory $installInventory -Canonical $canonical[$row.app] -Exclude @(Get-Prop $corpusApp 'packageExcludes')
    $unexpectedExtras = @($payload.extras | Where-Object { $_ -notin $row.expectedExtras })
    $ownedAfterInstall = @($row.packageOwnedPaths | ForEach-Object { Get-InventorySummary $installEvidence $_ })
    $ownedAfterUninstall = @($row.packageOwnedPaths | ForEach-Object { Get-InventorySummary $uninstallEvidence $_ })
    $residueAfterUninstall = @($row.residuePaths | ForEach-Object { Get-InventorySummary $uninstallEvidence $_ })
    $arpDisplayName = [string] (Get-RegistryValue $installEvidence $row.arpKey 'DisplayName')
    $arpDisplayVersion = [string] (Get-RegistryValue $installEvidence $row.arpKey 'DisplayVersion')

    $summary = [ordered]@{
        row       = $row.row
        app       = $row.app
        tech      = $row.tech
        scope     = $row.scope
        implementation = $row.implementation
        toolVersion = $row.toolVersion
        installerSha256 = $row.installerSha256
        installerBytes = $row.installerBytes
        canonicalFiles = $expectedFiles
        canonicalBytes = $expectedBytes
        canonicalInventory = $canonicalIdentity[$row.app]
        session   = $sessionId
        startedAt = $rowStarted.ToString('o')
        finishedAt = $rowFinished.ToString('o')
        rowSeconds = [math]::Round(($rowFinished - $rowStarted).TotalSeconds, 1)
        vmStateAfter = [string] (Get-Prop $vm 'state')
        install   = $installSummary + [ordered]@{
            # The lab job that produced this step: the only link from the
            # committed compact record to the raw evidence under $ResultsRoot.
            jobId = [string] (Get-Prop (Get-Prop $installRun 'result') 'jobId')
            jobStatus = [string] (Get-Prop $installRun 'status')
            jobSeconds = (Get-Prop $installRun 'durationSeconds')
            installedFiles = $installedFiles
            installedBytes = $installedBytes
            # The canonical payload file by file (when the guest returned hashes):
            # exact means every canonical file is present with its size and
            # SHA-256; extras are what the technology added under the root.
            payloadVerified = $payload.verified
            payloadExact = $payload.exact
            payloadExpected = $payload.expected
            payloadMatched = $payload.matched
            payloadMissing = @($payload.missing)
            payloadDiffering = @($payload.differing)
            payloadExtras = @($payload.extras)
            payloadExtraBytes = $payload.extraBytes
            payloadUnexpectedExtras = @($unexpectedExtras)
            arpPresent = [bool] (Get-Prop $installArp 'exists')
            arpDisplayName = $arpDisplayName
            arpDisplayVersion = $arpDisplayVersion
            arpUninstallString = [string] (Get-RegistryValue $installEvidence $row.arpKey 'UninstallString')
            # Registered means: the key exists, DisplayName names the application
            # (Inno Setup's default is "<AppName> <AppVersion>", its own convention),
            # DisplayVersion is the package version, and an uninstaller is named.
            arpRegistered = ([bool] (Get-Prop $installArp 'exists') -and ($arpDisplayName -eq $row.app -or $arpDisplayName -like "$($row.app) *") -and $arpDisplayVersion -eq [string] $corpusApp.packageVersion -and $null -ne $uninstaller)
            # What the technology keeps outside the root while installed.
            packageOwnedState = @($ownedAfterInstall)
            engine = $installEngine
        }
        uninstall = $uninstallSummary + [ordered]@{
            jobId = [string] (Get-Prop (Get-Prop $uninstallRun 'result') 'jobId')
            jobStatus = [string] (Get-Prop $uninstallRun 'status')
            jobSeconds = (Get-Prop $uninstallRun 'durationSeconds')
            uninstaller = $uninstaller
            skipped = $uninstallSkipped
            rootRemoved = ($null -ne $uninstallInventory -and -not [bool] (Get-Prop $uninstallInventory 'exists'))
            remainingFiles = (Get-Prop $uninstallInventory 'fileCount')
            arpRemoved = ($null -ne $uninstallArp -and -not [bool] (Get-Prop $uninstallArp 'exists'))
            packageOwnedState = @($ownedAfterUninstall)
            packageOwnedStateRemoved = (@($ownedAfterUninstall | Where-Object { $_.exists -ne $false }).Count -eq 0)
            # Scratch the technology's hand-off may leave (NSIS's ~nsuA.tmp); recorded, not judged.
            residue = @($residueAfterUninstall)
            engine = $uninstallEngine
        }
    }
    $summary.success = (
        (Test-JobOk $installRun) -and (Test-JobOk $uninstallRun) -and
        $installSummary.exitCode -eq 0 -and $uninstallSummary.exitCode -eq 0 -and
        [bool] $installSummary.completionSatisfied -and [bool] $uninstallSummary.completionSatisfied -and
        [bool] $summary.install.arpRegistered -and
        [bool] $payload.verified -and [bool] $payload.exact -and $unexpectedExtras.Count -eq 0 -and
        $summary.uninstall.rootRemoved -and $summary.uninstall.arpRemoved -and $summary.uninstall.packageOwnedStateRemoved
    )
    $allResults.Add([pscustomobject] $summary)
    # The campaign's row order is the matrix's, whatever order rows were measured in.
    $matrixOrder = @($order | ForEach-Object { $app = $_[0]; $_[1] | ForEach-Object { "$app-$_" } })
    $ordered = @($allResults | Sort-Object { [array]::IndexOf($matrixOrder, [string] $_.row) })

    Write-Host ("  install   exit={0} {1}s ({2}s process + {3}s completion) files={4}/{5} bytes={6}/{7} ARP={8}" -f $installSummary.exitCode, $installSummary.durationSeconds, $installSummary.processSeconds, $installSummary.completionWaitSeconds, $installedFiles, $expectedFiles, $installedBytes, $expectedBytes, $summary.install.arpRegistered)
    if ($null -ne $installEngine) { Write-Host ("    engine span {0}s ({1}s before {2}, {3}s after {4})" -f $installEngine.spanSeconds, $installEngine.beforeSeconds, $installEngine.firstEvent, $installEngine.afterSeconds, $installEngine.lastEvent) }
    if ($payload.verified) { Write-Host ("    payload {0}: {1}/{2} canonical files matched, {3} missing, {4} differing, {5} extra ({6}; {7} bytes)" -f $(if ($payload.exact) { 'exact' } else { 'NOT EXACT' }), $payload.matched, $payload.expected, @($payload.missing).Count, @($payload.differing).Count, @($payload.extras).Count, (@($payload.extras | Select-Object -First 5) -join ', '), $payload.extraBytes) }
    foreach ($owned in $ownedAfterInstall) { Write-Host ("    owned state {0}: exists={1} files={2} bytes={3}" -f $owned.path, $owned.exists, $owned.files, $owned.bytes) }
    Write-Host ("  uninstall exit={0} {1}s ({2}s process + {3}s completion) via {4}; root removed={5} ARP removed={6} owned state removed={7}" -f $uninstallSummary.exitCode, $uninstallSummary.durationSeconds, $uninstallSummary.processSeconds, $uninstallSummary.completionWaitSeconds, $uninstaller, $summary.uninstall.rootRemoved, $summary.uninstall.arpRemoved, $summary.uninstall.packageOwnedStateRemoved)
    if ($null -ne $uninstallEngine) { Write-Host ("    engine span {0}s ({1}s before {2}, {3}s after {4})" -f $uninstallEngine.spanSeconds, $uninstallEngine.beforeSeconds, $uninstallEngine.firstEvent, $uninstallEngine.afterSeconds, $uninstallEngine.lastEvent) }
    foreach ($left in $residueAfterUninstall) { Write-Host ("    residue {0}: exists={1} files={2} bytes={3}" -f $left.path, $left.exists, $left.files, $left.bytes) }
    Write-Host ("  row {0}s; VM {1}; success={2}" -f $summary.rowSeconds, $summary.vmStateAfter, $summary.success)

    # The results file is rewritten after every row, so an interrupted
    # campaign still leaves what it measured.
    $record = [ordered]@{
        baseline = $Baseline
        startedAt = $campaignStarted.ToString('o')
        updatedAt = [DateTimeOffset]::Now.ToString('o')
        contract = 'packages/broad/contract.md: install the canonical payload into the named root in user scope, register with Add/Remove Programs, uninstall from the registered uninstaller, remove everything'
        timing = 'install: the installer process lifetime, plus the exit of any process of the installer''s own name it left running (TigerSetup; the loader''s wait for its engine makes that none); uninstall: the registered uninstaller''s process lifetime plus the exit of the processes it hands off to (NSIS: Au_.exe; TigerSetup: any process of the uninstaller''s own name) and the removal of the install root and of the package-owned state outside it (TigerSetup: the state directory); no artificial delay'
        builder = $(if ($null -ne $buildRecord) { [string] (Get-Prop $buildRecord 'builder') } else { '' })
        engineSha256 = $(if ($null -ne $buildRecord) { [string] (Get-Prop $buildRecord 'engineSha256') } else { '' })
        labEnvironment = $labEnvironment
        rows = @($ordered)
    }
    $record | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $OutputJson -Encoding utf8
}

$campaignFinished = [DateTimeOffset]::Now
Write-Host ""
Write-Host ("Campaign: {0} row(s) in {1:N1} min; {2} succeeded; wrote {3}" -f $allResults.Count, ($campaignFinished - $campaignStarted).TotalMinutes, @($allResults | Where-Object { $_.success }).Count, $OutputJson)
if ($null -ne $previous) { Write-Host "  ($($rows.Count) row(s) measured in this run; the campaign started $($campaignStarted.ToString('o')))" }
if (@($allResults | Where-Object { -not $_.success }).Count -gt 0) { exit 1 }
exit 0
