#Requires -Version 7.0
<#
    .SYNOPSIS
    Runs the benchmark's 12 real-application installers (4 apps x 3
    technologies) through TigerWinLab's clean Windows 11 baseline, one row at
    a time: silent install, the functional contract read from the machine,
    the installed payload measured, silent uninstall, the removal read from
    the machine — with the process lifecycle and the lab lifecycle of one row
    both complete before the next row begins.

    .DESCRIPTION
    A row is one lab session on one VM:

      session opens
      install job     (EntryPolicy Baseline: the VM is restored to the clean
                       checkpoint) stages the installer, runs it silently,
                       then reads the install root, the registry keys, the
                       shortcuts, the firewall rules and PATH the app's
                       contract names
      uninstall job   (EntryPolicy DontCare: the same VM, as the install
                       left it) runs the uninstaller silently and reads the
                       same evidence again
      session closes  the lab takes the VM back and normalizes it
      wait            until the lab reports the VM Available again

    and only then the next row. Rows never share a VM or overlap, so no
    row's leftover state, and no lab lifecycle still in flight, can touch
    another's measurement.

    Timing has the same boundary in every technology. An install's elapsed
    time is the installer process's own lifetime: Inno Setup's /VERYSILENT,
    NSIS's /S and TigerSetup's --quiet all finish their work in the process
    that was started. An uninstall's elapsed time is the uninstaller's
    lifetime plus what it hands off: an NSIS uninstaller copies itself to
    %TEMP%\~nsuX.tmp\Au_.exe and exits at once while the copy does the
    work, so the clock runs until that copy has exited; Inno Setup's
    unins000.exe first phase waits for its second phase itself; TigerSetup's
    engine removes everything in-process. In every technology the clock also
    runs until the install root is gone, which the uninstaller's own last
    step makes true within milliseconds of its exit. The guest reader
    (lab\guest\Invoke-SetupCommands.ps1) implements both waits; each row's
    result records the process lifetime and the completion wait separately,
    and there is no artificial settle delay anywhere in the measured path.

    All commands run as the job account ("job": LabAdmin, session 0, an
    administrator with no interactive desktop) because every installer
    invocation here is silent/unattended -- this benchmark is about
    packaging and install/uninstall outcomes, not the UAC/desktop dance,
    which TigerSetup's own elevation acceptance suite covers.

    The functional contract of each application (packages\<app>\contract.md)
    is read as Windows holds it, in a technology-neutral way: a file type is
    linked to its ProgID whether the installer set the extension's default
    (Inno Setup, NSIS) or registered a handler under OpenWithProgids
    (TigerSetup); a context-menu verb counts wherever Windows consults it for
    those files; a URL scheme counts with the command under the scheme class
    or under a handler ProgID the scheme names. Every probe records the
    mechanism it found, so the report can say how each technology did it.

    A later campaign of one technology alone -- a new TigerSetup against the
    first campaign's Inno Setup and NSIS rows -- runs with -OnlyTechnologies
    and its own -ArtifactsRoot, -ResultsRoot and -OutputJson, and the same
    rows, contracts, timing boundaries and lifecycle; every row records the
    installer's hash and the tool version it was built with (from the
    build.json beside the results root), so a runtime figure is tied to the
    exact bytes and engine it measured. TigerSetup's installer is a loader
    that extracts its engine and waits for it, so the installer process's
    lifetime already spans the whole operation; the row still waits for any
    process of the installer's own name (the engine keeps the package's file
    name) so a hand-off, should one ever appear, would show as completion
    wait rather than be missed.

    -OutputJson is the campaign's compact record and is what gets committed:
    a row carries the installer it ran, the canonical inventory that decided
    its payload verdict (path and that document's SHA-256), the counts and
    the full path lists of whatever did not match, the contract probes, the
    cleanup verdicts and the lab job ids. -ResultsRoot is the raw lab
    evidence a passing row duplicates — job documents with every installed
    file's hash, session records, VM states, job folders — and a later
    campaign's is gitignored (benchmark/README.md, *What a campaign
    commits*). The first campaign's results/lab/ predates that split.

    .EXAMPLE
    pwsh -File benchmark\scripts\Invoke-BenchmarkLab.ps1
    pwsh -File benchmark\scripts\Invoke-BenchmarkLab.ps1 -OnlyRows WinMerge-NSIS -ResultsRoot benchmark\results\smoke
    pwsh -File benchmark\scripts\Invoke-BenchmarkLab.ps1 -OnlyTechnologies TigerSetup -ArtifactsRoot benchmark\artifacts\0.9.0 -ResultsRoot benchmark\results\0.9.0\lab -OutputJson benchmark\results\0.9.0\lab-results.json
#>
[CmdletBinding()]
param(
    [string] $TigerWinLabRoot,
    [string] $Baseline = 'TigerWinLab-Win11-Clean',
    [string] $ArtifactsRoot = (Join-Path $PSScriptRoot '..\artifacts'),
    [string] $ResultsRoot = (Join-Path $PSScriptRoot '..\results\lab'),
    [string] $OutputJson = (Join-Path $PSScriptRoot '..\results\lab-results.json'),
    [string] $SessionPrefix = ('benchmark-' + (Get-Date -Format 'yyyyMMdd-HHmmss')),
    [int] $InstallTimeoutMinutes = 20,
    [int] $UninstallTimeoutMinutes = 10,
    [int] $NormalizeTimeoutMinutes = 10,
    [string[]] $OnlyRows,
    [string[]] $OnlyTechnologies,
    # The build record the installers under -ArtifactsRoot came from; each
    # row records its installer's hash and tool version from it.
    [string] $BuildJson,
    # Start the results file over. Without it, rows this run measures replace
    # their earlier records in an existing results file and every other row
    # is kept, so a row re-measured after a definition fix joins the campaign
    # it belongs to (the record says when each row was measured).
    [switch] $Fresh
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\lab\TigerSetupLab.psm1') -Force
# The evidence reading every benchmark driver shares (Get-Prop, Split-CommandLine,
# Test-JobOk, Compare-InstalledPayload, Get-EngineSpan, Get-CommandSummary, ...).
Import-Module (Join-Path $PSScriptRoot 'BenchmarkRow.psm1') -Force

$labRoot = Get-TigerSetupLabRoot -TigerWinLabRoot $TigerWinLabRoot
$ResultsRoot = [System.IO.Path]::GetFullPath($ResultsRoot)
$labOutputRoot = Join-Path $ResultsRoot 'jobs'
$null = New-Item -ItemType Directory -Path $ResultsRoot -Force
$null = New-Item -ItemType Directory -Path $labOutputRoot -Force
$OnlyRows = @($OnlyRows | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$OnlyTechnologies = @($OnlyTechnologies | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ([string]::IsNullOrWhiteSpace($BuildJson)) { $BuildJson = Join-Path (Split-Path -Parent $ResultsRoot) 'build.json' }
$buildRecord = $null
if (Test-Path -LiteralPath $BuildJson -PathType Leaf) { $buildRecord = Get-Content -LiteralPath $BuildJson -Raw | ConvertFrom-Json }
else { Write-Warning "No build record at '$BuildJson'; rows will not carry their installer's build identity." }

# ---------------------------------------------------------------------------
# The matrix. Guest paths are fixed and explicit (not %LOCALAPPDATA%) so a
# "user" scope row is just as easy to inventory as a "machine" one -- the
# job account (LabAdmin) is a real Windows account either way, and the scope
# difference is what the three technologies are asked to do, not where the
# evidence is looked for afterwards.
# ---------------------------------------------------------------------------

$guestInstallerDir = 'C:\TigerSetupBenchmark\installers'
$guestInstallRoot = 'C:\TigerSetupBenchmark\apps'
$art = (Resolve-Path -LiteralPath $ArtifactsRoot).Path

# The canonical payloads, for the installed-payload control measurement.
$canonical = @{}
$canonicalIdentity = @{}
foreach ($app in 'ShareX', 'WinMerge', 'qBittorrent', 'VLC') {
    $inventoryPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\results\canonical-$app.json")).Path
    $canonical[$app] = Get-Content -LiteralPath $inventoryPath -Raw | ConvertFrom-Json
    # The row records which inventory decided its payload verdict, and that
    # document's own SHA-256, so a compact row is auditable on its own.
    $canonicalIdentity[$app] = [ordered]@{
        path = "results/canonical-$app.json"
        sha256 = (Get-FileHash -LiteralPath $inventoryPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

# Technology-specific invocation and locations. {root} is the row's install
# root, {installer} the staged installer.
$technologies = @{
    TigerSetup = @{
        install = { param($scope) "install --quiet --scope $scope --install-root ""{root}"" --log ""{root}.install.log""" }
        uninstallCommand = '{installer}'
        uninstall = { param($scope) "uninstall --quiet --scope $scope --log ""{root}.uninstall.log""" }
        # The loader waits for the engine it extracts, and the engine keeps
        # the installer's file name; waiting on that name follows the whole
        # operation whatever process carries it.
        installWaitProcesses = @('{installerName}')
        uninstallWaitProcesses = @('{installerName}')
        arpKey = { param($app, $appId, $name) "Benchmark.$app" }
        # TigerSetup puts the Start Menu link directly in Programs.
        startMenu = { param($app, $name, $group) "$name.lnk" }
        implementation = 'declarative manifest'
    }
    InnoSetup = @{
        install = { param($scope) "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- $(if ($scope -eq 'user') { '/CURRENTUSER ' })/DIR=""{root}"" /LOG=""{root}.install.log""" }
        uninstallCommand = '{root}\unins000.exe'
        uninstall = { param($scope) '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="{root}.uninstall.log"' }
        # unins000.exe's first phase waits for its second phase (the copy in
        # %TEMP%) itself, so its own lifetime is the uninstall's.
        installWaitProcesses = @()
        uninstallWaitProcesses = @()
        arpKey = { param($app, $appId, $name) "$($appId)_is1" }
        startMenu = { param($app, $name, $group) "$group\$name.lnk" }
        implementation = 'installer script'
    }
    NSIS = @{
        # /S is what makes an NSIS installer silent; the MultiUser packages
        # take /CurrentUser too. /D= must be last and unquoted.
        install = { param($scope) "/S $(if ($scope -eq 'user') { '/CurrentUser ' })/D={root}" }
        uninstallCommand = '{root}\Uninstall.exe'
        uninstall = { param($scope) "/S$(if ($scope -eq 'user') { ' /CurrentUser' })" }
        # Uninstall.exe copies itself to %TEMP%\~nsuX.tmp\Au_.exe and exits;
        # the copy does the work and is what the clock must follow.
        installWaitProcesses = @()
        uninstallWaitProcesses = @('Au_')
        arpKey = { param($app, $appId, $name) $name }
        startMenu = { param($app, $name, $group) "$group\$name.lnk" }
        implementation = 'installer script'
    }
}

# Per-application contract: scope, identities, and the probes the row reads.
# Every registry key a probe needs is listed under `registry`, in the hive
# the scope writes ({hive} is HKCU or HKLM). Probe kinds:
#   extension   .ext linked to a ProgID (default value, or OpenWithProgids)
#   progid      a ProgID class whose shell\open\command names the executable
#   verb        a context-menu verb: any of the listed classes' shell\<verb>\command
#   scheme      a URL scheme: URL Protocol on the scheme class and a command under it or under the handler ProgID
#   apppath     an App Paths entry naming the executable
#   shellext    a COM shell extension: the ContextMenuHandlers entry and the CLSID's InProcServer32 under the install root
#   setting     a plain value that must hold `data` after install; recorded after uninstall
#   firewall    a firewall rule of that name for the executable
#   absent-link a shortcut that must NOT exist (an option that defaults off)
$applications = @{
    ShareX = @{
        scope = 'user'; name = 'ShareX'; exe = 'ShareX.exe'; group = 'ShareX'
        appId = '{2B7B6B10-6E29-4B7C-9B36-7C0B1E9A2A01}'
        firewallRules = @()
        probes = @(
            @{ feature = 'Desktop shortcut (off by default)'; kind = 'absent-link'; path = '%USERPROFILE%\Desktop\ShareX.lnk' }
            @{ feature = 'Send To entry (off by default)'; kind = 'absent-link'; path = '%APPDATA%\Microsoft\Windows\SendTo\ShareX.lnk' }
            @{ feature = 'Start at sign-in (off by default)'; kind = 'absent-link'; path = '%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ShareX.lnk' }
            @{ feature = 'Explorer context menu (off by default)'; kind = 'absent-key'; key = '{hive}\Software\Classes\*\shell\ShareX' }
        )
    }
    WinMerge = @{
        scope = 'user'; name = 'WinMerge'; exe = 'WinMergeU.exe'; group = 'WinMerge'
        appId = '{9E2E6F2C-6C6A-4D64-9F02-9C9C6F6B7E02}'
        firewallRules = @()
        probes = @(
            @{ feature = '.WinMerge file association'; kind = 'extension'; extension = '.WinMerge'; progId = 'WinMerge.Project.File' }
            @{ feature = 'WinMerge.Project.File ProgID'; kind = 'progid'; progId = 'WinMerge.Project.File' }
            @{ feature = 'App Paths entry'; kind = 'apppath' }
            @{ feature = 'Explorer context menu (COM shell extension)'; kind = 'shellext'; clsid = '{4E716236-AA30-4C65-B225-D68BBA81E9C2}'; handler = 'WinMerge'; dll = 'ShellExtensionX64.dll' }
            @{ feature = 'Add to PATH (off by default)'; kind = 'absent-path' }
            @{ feature = 'Desktop shortcut (off by default)'; kind = 'absent-link'; path = '%USERPROFILE%\Desktop\WinMerge.lnk' }
        )
    }
    qBittorrent = @{
        scope = 'machine'; name = 'qBittorrent'; exe = 'qbittorrent.exe'; group = 'qBittorrent'
        appId = '{6C6A6E01-9E2E-4D64-9F02-1B2C3D4E5F01}'
        firewallRules = @('qBittorrent')
        probes = @(
            @{ feature = '.torrent file association'; kind = 'extension'; extension = '.torrent'; progId = 'qBittorrent.File.Torrent' }
            @{ feature = 'qBittorrent.File.Torrent ProgID'; kind = 'progid'; progId = 'qBittorrent.File.Torrent' }
            @{ feature = 'magnet: URL protocol'; kind = 'scheme'; scheme = 'magnet'; progId = 'qBittorrent.Url.Magnet' }
            @{ feature = 'Firewall rule'; kind = 'firewall'; rule = 'qBittorrent' }
            @{ feature = 'LongPathsEnabled = 1'; kind = 'setting'; key = 'HKLM\SYSTEM\CurrentControlSet\Control\FileSystem'; name = 'LongPathsEnabled'; data = '1' }
            @{ feature = 'Desktop shortcut (off by default)'; kind = 'absent-link'; path = '%PUBLIC%\Desktop\qBittorrent.lnk' }
            @{ feature = 'Startup shortcut (off by default)'; kind = 'absent-link'; path = '%PROGRAMDATA%\Microsoft\Windows\Start Menu\Programs\Startup\qBittorrent.lnk' }
        )
    }
    VLC = @{
        scope = 'machine'; name = 'VLC media player'; exe = 'vlc.exe'; group = 'VideoLAN'
        appId = '{7A8B9C01-3D4E-4F50-9A1B-2C3D4E5F6A01}'
        firewallRules = @()
        probes = @(
            @{ feature = 'App Paths entry'; kind = 'apppath' }
        ) + @(foreach ($ext in 'mp3', 'flac', 'wav', 'mp4', 'mkv', 'avi') {
                @{ feature = ".$ext file association"; kind = 'extension'; extension = ".$ext"; progId = "VLC.$ext" }
                @{ feature = "VLC.$ext ProgID"; kind = 'progid'; progId = "VLC.$ext" }
                @{ feature = "Play with VLC on .$ext"; kind = 'verb'; verb = 'PlayWithVLC'; classes = @("VLC.$ext", "SystemFileAssociations\.$ext") }
            }) + @(
            @{ feature = 'Play with VLC on a folder background'; kind = 'verb'; verb = 'PlayWithVLC'; classes = @('Directory\Background') }
            @{ feature = 'Desktop shortcut (off by default)'; kind = 'absent-link'; path = '%PUBLIC%\Desktop\VLC media player.lnk' }
        )
    }
}

# Technology order rotates between applications, so no technology always
# runs first or last on the host's disk cache.
$order = @(
    @('ShareX', @('TigerSetup', 'InnoSetup', 'NSIS')),
    @('WinMerge', @('InnoSetup', 'NSIS', 'TigerSetup')),
    @('qBittorrent', @('NSIS', 'TigerSetup', 'InnoSetup')),
    @('VLC', @('TigerSetup', 'InnoSetup', 'NSIS'))
)

function Get-RegistryKeysFor {
    <# Every registry key the app's probes read, plus the ARP key. #>
    param([hashtable] $App, [string] $Hive, [string] $ArpKey)
    $keys = [System.Collections.Generic.List[string]]::new()
    $keys.Add($ArpKey)
    $keys.Add("$Hive\Software\Microsoft\Windows\CurrentVersion\App Paths\$($App.exe)")
    foreach ($probe in $App.probes) {
        switch ($probe.kind) {
            'extension' {
                $keys.Add("$Hive\Software\Classes\$($probe.extension)")
                $keys.Add("$Hive\Software\Classes\$($probe.extension)\OpenWithProgids")
            }
            'progid' { $keys.Add("$Hive\Software\Classes\$($probe.progId)\shell\open\command") }
            'verb' { foreach ($class in $probe.classes) { $keys.Add("$Hive\Software\Classes\$class\shell\$($probe.verb)\command") } }
            'scheme' {
                $keys.Add("$Hive\Software\Classes\$($probe.scheme)")
                $keys.Add("$Hive\Software\Classes\$($probe.scheme)\shell\open\command")
                $keys.Add("$Hive\Software\Classes\$($probe.progId)\shell\open\command")
            }
            'shellext' {
                # regsvr32 runs elevated as the job account in every row, so
                # DllRegisterServer writes HKCR = HKLM\Software\Classes.
                $keys.Add("HKLM\Software\Classes\*\ShellEx\ContextMenuHandlers\$($probe.handler)")
                $keys.Add("HKLM\Software\Classes\CLSID\$($probe.clsid)\InProcServer32")
            }
            'setting' { $keys.Add($probe.key) }
            'absent-key' { $keys.Add(($probe.key -replace '\{hive\}', $Hive)) }
        }
    }
    @($keys | Sort-Object -Unique)
}

function Get-BuildOf {
    param([string] $App, [string] $Tech)
    if ($null -eq $buildRecord) { return $null }
    @($buildRecord.builds | Where-Object { $_.app -eq $App -and $_.technology -eq $Tech }) | Select-Object -First 1
}

function New-Row {
    param([string] $App, [string] $Tech)
    $a = $applications[$App]
    $t = $technologies[$Tech]
    $installerName = "$App-$Tech"
    $built = Get-BuildOf -App $App -Tech $Tech
    $hive = $(if ($a.scope -eq 'machine') { 'HKLM' } else { 'HKCU' })
    $arpName = & $t.arpKey $App $a.appId $a.name
    $arpKey = "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\$arpName"
    $programs = $(if ($a.scope -eq 'machine') { '%PROGRAMDATA%\Microsoft\Windows\Start Menu\Programs' } else { '%APPDATA%\Microsoft\Windows\Start Menu\Programs' })
    $link = & $t.startMenu $App $a.name $a.group
    [pscustomobject][ordered]@{
        row = "$App-$Tech"
        app = $App
        tech = $Tech
        scope = $a.scope
        hive = $hive
        localFile = (Join-Path $art "$($App.ToLowerInvariant())\$App-$Tech.exe")
        guestInstaller = "$guestInstallerDir\$App-$Tech.exe"
        installRoot = "$guestInstallRoot\$App-$Tech"
        installArgs = (& $t.install $a.scope)
        uninstallCommand = $t.uninstallCommand
        uninstallArgs = (& $t.uninstall $a.scope)
        installWaitProcesses = @($t.installWaitProcesses | ForEach-Object { $_.Replace('{installerName}', $installerName) })
        uninstallWaitProcesses = @($t.uninstallWaitProcesses | ForEach-Object { $_.Replace('{installerName}', $installerName) })
        toolVersion = $(if ($null -ne $built) { [string] (Get-Prop $built 'toolVersion') } else { '' })
        installerSha256 = $(if ($null -ne $built) { [string] (Get-Prop $built 'installerSha256') } else { '' })
        installerBytes = $(if ($null -ne $built) { Get-Prop $built 'installerBytes' } else { $null })
        arpKey = $arpKey
        startMenuLink = "$programs\$link"
        registryKeys = @(Get-RegistryKeysFor -App $a -Hive $hive -ArpKey $arpKey)
        firewallRules = @($a.firewallRules)
        shortcuts = @(@("$programs\$link") + @($a.probes | Where-Object { $_.kind -eq 'absent-link' } | ForEach-Object { $_.path }))
        exe = $a.exe
        probes = @($a.probes)
        implementation = $t.implementation
    }
}

$rows = @(foreach ($pair in $order) { foreach ($tech in $pair[1]) { New-Row -App $pair[0] -Tech $tech } })
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

function Expand-RowTemplate {
    param([string] $Template, [object] $Row)
    $Template.Replace('{root}', $Row.installRoot).Replace('{installer}', $Row.guestInstaller)
}

function Test-Probe {
    <#
        One contract probe against the evidence of a step. Returns the
        feature, whether the contract holds, and the mechanism found -- after
        install `present` must be $true; after uninstall a resource of the
        app must be gone ($false), and a `setting` is recorded as observed.
    #>
    param([object] $Row, [hashtable] $Probe, [object] $Evidence, [string] $Step)
    $hive = $Row.hive
    $classes = "$hive\Software\Classes"
    $root = $Row.installRoot
    $exe = $Row.exe
    $result = [ordered]@{ feature = $Probe.feature; kind = $Probe.kind; present = $false; mechanism = ''; detail = '' }
    switch ($Probe.kind) {
        'extension' {
            $default = Get-RegistryValue $Evidence "$classes\$($Probe.extension)" ''
            $handlers = @(Get-ValueNames $Evidence "$classes\$($Probe.extension)\OpenWithProgids")
            if ($default -eq $Probe.progId) { $result.present = $true; $result.mechanism = 'extension default = ProgID' }
            elseif ($handlers -contains $Probe.progId) { $result.present = $true; $result.mechanism = 'OpenWithProgids handler (never the default)' }
            $result.detail = "default=$(if ($null -eq $default) { '<absent>' } else { $default }); handlers=$($handlers -join ',')"
        }
        'progid' {
            $command = Get-RegistryValue $Evidence "$classes\$($Probe.progId)\shell\open\command" ''
            $result.present = ($null -ne $command) -and ($command -like "*$root\$exe*")
            $result.mechanism = 'ProgID class with shell\open\command'
            $result.detail = "command=$(if ($null -eq $command) { '<absent>' } else { $command })"
        }
        'verb' {
            foreach ($class in $Probe.classes) {
                $command = Get-RegistryValue $Evidence "$classes\$class\shell\$($Probe.verb)\command" ''
                if ($null -ne $command -and $command -like "*$root\$exe*") {
                    $result.present = $true
                    $result.mechanism = "verb under Classes\$class"
                    $result.detail = "command=$command"
                    break
                }
            }
            if (-not $result.present) { $result.detail = 'no command under ' + (($Probe.classes | ForEach-Object { "Classes\$_\shell\$($Probe.verb)" }) -join ' or ') }
        }
        'scheme' {
            $protocol = Get-ValueNames $Evidence "$classes\$($Probe.scheme)"
            $direct = Get-RegistryValue $Evidence "$classes\$($Probe.scheme)\shell\open\command" ''
            $handler = Get-RegistryValue $Evidence "$classes\$($Probe.progId)\shell\open\command" ''
            $isProtocol = $protocol -contains 'URL Protocol'
            if ($isProtocol -and $null -ne $direct -and $direct -like "*$root\$exe*") { $result.present = $true; $result.mechanism = 'command under the scheme class' }
            elseif ($isProtocol -and $null -ne $handler -and $handler -like "*$root\$exe*") { $result.present = $true; $result.mechanism = 'handler ProgID registered for the scheme (never the default)' }
            elseif ($null -ne $handler -and $handler -like "*$root\$exe*") { $result.present = $true; $result.mechanism = 'handler ProgID (scheme class left to its owner)' }
            $result.detail = "URL Protocol=$isProtocol; scheme command=$(if ($null -eq $direct) { '<absent>' } else { $direct }); handler command=$(if ($null -eq $handler) { '<absent>' } else { $handler })"
        }
        'apppath' {
            $target = Get-RegistryValue $Evidence "$hive\Software\Microsoft\Windows\CurrentVersion\App Paths\$exe" ''
            $result.present = ($null -ne $target) -and ($target -like "*$root\$exe*")
            $result.mechanism = 'App Paths entry'
            $result.detail = "target=$(if ($null -eq $target) { '<absent>' } else { $target })"
        }
        'shellext' {
            $handler = Get-RegistryValue $Evidence "HKLM\Software\Classes\*\ShellEx\ContextMenuHandlers\$($Probe.handler)" ''
            $server = Get-RegistryValue $Evidence "HKLM\Software\Classes\CLSID\$($Probe.clsid)\InProcServer32" ''
            $result.present = ($handler -eq $Probe.clsid) -and ($null -ne $server) -and ($server -like "*$root\$($Probe.dll)*")
            $result.mechanism = 'regsvr32 (DllRegisterServer, machine-wide HKCR)'
            $result.detail = "handler=$(if ($null -eq $handler) { '<absent>' } else { $handler }); server=$(if ($null -eq $server) { '<absent>' } else { $server })"
        }
        'setting' {
            $data = Get-RegistryValue $Evidence $Probe.key $Probe.name
            $result.present = ($data -eq $Probe.data)
            $result.mechanism = 'registry value'
            $result.detail = "$($Probe.key)\$($Probe.name)=$(if ($null -eq $data) { '<absent>' } else { $data })"
        }
        'firewall' {
            $record = Get-Record $Evidence 'firewallRules' 'requested' $Probe.rule
            $rules = @(if ($null -ne $record) { Get-Prop $record 'rules' } else { @() })
            $match = @($rules | Where-Object { [string] $_.program -like "*$root\$exe*" -and [string] $_.direction -eq 'inbound' -and [string] $_.action -eq 'allow' -and [bool] $_.enabled })
            $result.present = $match.Count -gt 0
            $result.mechanism = 'Windows Firewall rule'
            $result.detail = "rules named '$($Probe.rule)': $($rules.Count); matching: $($match.Count)"
        }
        'absent-link' {
            $record = Get-Record $Evidence 'shortcuts' 'requested' $Probe.path
            $exists = ($null -ne $record) -and [bool] (Get-Prop $record 'exists')
            # Present means "the contract holds": the link is absent.
            $result.present = -not $exists
            $result.mechanism = 'shortcut absent'
            $result.detail = "$($Probe.path) exists=$exists"
        }
        'absent-key' {
            $key = $Probe.key -replace '\{hive\}', $hive
            $exists = Test-KeyExists $Evidence $key
            $result.present = -not $exists
            $result.mechanism = 'key absent'
            $result.detail = "$key exists=$exists"
        }
        'absent-path' {
            $values = Get-Prop $Evidence 'pathValues'
            $entries = @()
            if ($null -ne $values) {
                $side = $(if ($Row.scope -eq 'machine') { Get-Prop $values 'machine' } else { Get-Prop $values 'user' })
                $entries = @(Get-Prop $side 'entries')
            }
            $onPath = @($entries | Where-Object { $_.TrimEnd('\') -eq $root }).Count -gt 0
            $result.present = -not $onPath
            $result.mechanism = 'PATH untouched'
            $result.detail = "install root on $($Row.scope) PATH=$onPath"
        }
    }
    [pscustomobject] $result
}

$allResults = [System.Collections.Generic.List[object]]::new()
$campaignStarted = [DateTimeOffset]::Now
$previous = $null
if (-not $Fresh -and (Test-Path -LiteralPath $OutputJson -PathType Leaf)) {
    $previous = Get-Content -LiteralPath $OutputJson -Raw | ConvertFrom-Json
    foreach ($kept in @($previous.rows | Where-Object { $null -ne $_ -and $_.row -notin @($rows | ForEach-Object { $_.row }) })) { $allResults.Add($kept) }
    if ($allResults.Count -gt 0) { Write-Host "Keeping $($allResults.Count) row(s) already in $OutputJson; this run's rows replace theirs." }
    $campaignStarted = [DateTimeOffset] $previous.startedAt
}
Write-Host "Benchmark campaign on $Baseline ($($rows.Count) row(s)); results under $ResultsRoot"

foreach ($row in $rows) {
    Write-Host ""
    Write-Host "=== $($row.row) ($($row.scope) scope, $($row.implementation)) ==="
    $rowStarted = [DateTimeOffset]::Now
    $sessionId = "$SessionPrefix-$($row.row.ToLowerInvariant())"
    $installRun = $null
    $uninstallRun = $null
    $null = Enter-TigerSetupLabSession -LabRoot $labRoot -SessionId $sessionId -Description "TigerSetup benchmark row $($row.row)" `
        -ResultPath (Join-Path $ResultsRoot "$($row.row)-session-open.json")
    try {
        # --- install job (fresh from the baseline) ---
        # Inno Setup's /LOG= (and TigerSetup's --log) need their target
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
            arguments = @(Split-CommandLine (Expand-RowTemplate $row.installArgs $row))
            timeoutSeconds = ($InstallTimeoutMinutes * 60)
            waitForProcesses = @($row.installWaitProcesses)
        }
        $installRequest = @{
            stage = @(@{ source = (Split-Path -Leaf $row.localFile); destination = $row.guestInstaller })
            commands = @($mkdirCommand, $installCommand)
            runAs = 'job'
            inventory = @($row.installRoot)
            # Every installed file's SHA-256, read after the timed command,
            # so the installed payload is compared with the canonical one
            # file by file rather than by count and size alone.
            inventoryHashes = $true
            # The installer's own log, where the technology writes one
            # (TigerSetup --log, Inno Setup /LOG=; NSIS has none): brought
            # back as evidence, and for TigerSetup read for the engine's own
            # span inside the process lifetime.
            logs = @("$($row.installRoot).install.log")
            registry = @($row.registryKeys)
            firewallRules = @($row.firewallRules)
            pathValues = $true
            # The guest expands %APPDATA%/%PROGRAMDATA%/%PUBLIC%/%USERPROFILE%
            # itself, in the job account's own environment -- passed
            # through unexpanded on purpose.
            shortcuts = @($row.shortcuts)
        }
        $installPolicy = Get-TigerSetupRowStepPolicy -FromBaseline
        $installRun = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $installRequest `
            -PayloadFiles @($row.localFile) -Name ("$($row.row)-i".ToLowerInvariant()) @installPolicy `
            -ResultPath (Join-Path $ResultsRoot "$($row.row)-install.json") -OutputRoot $labOutputRoot -TimeoutMinutes ($InstallTimeoutMinutes + 10)

        if (Test-JobOk $installRun) {
            # --- uninstall job (the same VM, as the install left it) ---
            $uninstallCommand = @{
                name = 'uninstall'
                executable = (Expand-RowTemplate $row.uninstallCommand $row)
                arguments = @(Split-CommandLine (Expand-RowTemplate $row.uninstallArgs $row))
                timeoutSeconds = ($UninstallTimeoutMinutes * 60)
                waitForProcesses = @($row.uninstallWaitProcesses)
                waitForAbsentPaths = @($row.installRoot)
            }
            $uninstallRequest = @{
                commands = @($uninstallCommand)
                runAs = 'job'
                logs = @("$($row.installRoot).uninstall.log")
                inventory = @($row.installRoot)
                registry = @($row.registryKeys)
                firewallRules = @($row.firewallRules)
                pathValues = $true
                shortcuts = @($row.shortcuts)
            }
            $uninstallPolicy = Get-TigerSetupRowStepPolicy
            $uninstallRun = Invoke-TigerSetupGuestCommands -LabRoot $labRoot -Baseline $Baseline -Request $uninstallRequest `
                -Name ("$($row.row)-u".ToLowerInvariant()) @uninstallPolicy `
                -ResultPath (Join-Path $ResultsRoot "$($row.row)-uninstall.json") -OutputRoot $labOutputRoot -TimeoutMinutes ($UninstallTimeoutMinutes + 10)
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
    $installInventory = Get-Record $installEvidence 'inventory' 'requested' $row.installRoot
    $uninstallInventory = Get-Record $uninstallEvidence 'inventory' 'requested' $row.installRoot
    $installArp = Get-Record $installEvidence 'registry' 'requested' $row.arpKey
    $uninstallArp = Get-Record $uninstallEvidence 'registry' 'requested' $row.arpKey
    $installLink = Get-Record $installEvidence 'shortcuts' 'requested' $row.startMenuLink
    $uninstallLink = Get-Record $uninstallEvidence 'shortcuts' 'requested' $row.startMenuLink
    $linkTarget = [string] (Get-Prop $installLink 'target')
    $expectedFiles = [int] $canonical[$row.app].fileCount
    $expectedBytes = [long] $canonical[$row.app].totalBytes
    $installedFiles = Get-Prop $installInventory 'fileCount'
    $installedBytes = Get-Prop $installInventory 'totalBytes'
    $payload = Compare-InstalledPayload -Inventory $installInventory -Canonical $canonical[$row.app]

    $contractInstall = @(foreach ($probe in $row.probes) { Test-Probe -Row $row -Probe $probe -Evidence $installEvidence -Step 'install' })
    $contractUninstall = @(foreach ($probe in $row.probes) { Test-Probe -Row $row -Probe $probe -Evidence $uninstallEvidence -Step 'uninstall' })
    # After uninstall, the app's own resources must be gone; a setting is an
    # observation; an "absent" probe must still hold.
    $removalOk = @($contractUninstall | Where-Object {
            ($_.kind -in @('absent-link', 'absent-key', 'absent-path') -and -not $_.present) -or
            ($_.kind -notin @('absent-link', 'absent-key', 'absent-path', 'setting') -and $_.present)
        }).Count -eq 0

    $summary = [ordered]@{
        row       = $row.row
        app       = $row.app
        tech      = $row.tech
        scope     = $row.scope
        implementation = $row.implementation
        toolVersion = $row.toolVersion
        installerSha256 = $row.installerSha256
        installerBytes = $row.installerBytes
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
            canonicalFiles = $expectedFiles
            canonicalBytes = $expectedBytes
            canonicalInventory = $canonicalIdentity[$row.app]
            # The install root holds the canonical payload plus the technology's own
            # bookkeeping (Inno's unins000.*, NSIS's Uninstall.exe, an installer log
            # written beside the root by the harness is outside it).
            extraFiles = $(if ($null -ne $installedFiles) { [int] $installedFiles - $expectedFiles } else { $null })
            extraBytes = $(if ($null -ne $installedBytes) { [long] $installedBytes - $expectedBytes } else { $null })
            # The canonical payload file by file (when the guest returned hashes):
            # exact means every canonical file is present with its size and
            # SHA-256; extras are what the technology added under the root.
            payloadVerified = $payload.verified
            payloadExact = $payload.exact
            payloadMatched = $payload.matched
            payloadMissing = @($payload.missing)
            payloadDiffering = @($payload.differing)
            payloadExtras = @($payload.extras)
            arpPresent = [bool] (Get-Prop $installArp 'exists')
            arpDisplayName = [string] (Get-RegistryValue $installEvidence $row.arpKey 'DisplayName')
            arpDisplayVersion = [string] (Get-RegistryValue $installEvidence $row.arpKey 'DisplayVersion')
            startMenuLinkPresent = [bool] (Get-Prop $installLink 'exists')
            startMenuLinkTarget = $linkTarget
            startMenuLinkOk = ($linkTarget -like "$($row.installRoot)\$($row.exe)")
            contract = @($contractInstall)
            engine = $installEngine
        }
        uninstall = $uninstallSummary + [ordered]@{
            jobId = [string] (Get-Prop (Get-Prop $uninstallRun 'result') 'jobId')
            jobStatus = [string] (Get-Prop $uninstallRun 'status')
            jobSeconds = (Get-Prop $uninstallRun 'durationSeconds')
            rootRemoved = -not [bool] (Get-Prop $uninstallInventory 'exists')
            remainingFiles = (Get-Prop $uninstallInventory 'fileCount')
            arpRemoved = -not [bool] (Get-Prop $uninstallArp 'exists')
            startMenuLinkRemoved = -not [bool] (Get-Prop $uninstallLink 'exists')
            contract = @($contractUninstall)
            engine = $uninstallEngine
        }
    }
    $summary.success = (
        (Test-JobOk $installRun) -and (Test-JobOk $uninstallRun) -and
        $installSummary.exitCode -eq 0 -and $uninstallSummary.exitCode -eq 0 -and
        [bool] $uninstallSummary.completionSatisfied -and
        $summary.install.arpPresent -and $summary.install.startMenuLinkOk -and
        ($installedFiles -ge $expectedFiles) -and
        (-not $payload.verified -or [bool] $payload.exact) -and
        @($contractInstall | Where-Object { -not $_.present }).Count -eq 0 -and
        $summary.uninstall.rootRemoved -and $summary.uninstall.arpRemoved -and $summary.uninstall.startMenuLinkRemoved -and $removalOk
    )
    $allResults.Add([pscustomobject] $summary)
    # The campaign's row order is the matrix's, whatever order rows were measured in.
    $matrixOrder = @($order | ForEach-Object { $app = $_[0]; $_[1] | ForEach-Object { "$app-$_" } })
    $ordered = @($allResults | Sort-Object { [array]::IndexOf($matrixOrder, [string] $_.row) })

    Write-Host ("  install   exit={0} {1}s ({2}s process) files={3}/{4} bytes={5}/{6} ARP={7} link={8}" -f $installSummary.exitCode, $installSummary.durationSeconds, $installSummary.processSeconds, $installedFiles, $expectedFiles, $installedBytes, $expectedBytes, $summary.install.arpPresent, $summary.install.startMenuLinkOk)
    if ($null -ne $installEngine) { Write-Host ("    engine span {0}s ({1}s before {2}, {3}s after {4})" -f $installEngine.spanSeconds, $installEngine.beforeSeconds, $installEngine.firstEvent, $installEngine.afterSeconds, $installEngine.lastEvent) }
    if ($payload.verified) { Write-Host ("    payload {0}: {1} canonical files matched, {2} missing, {3} differing, {4} extra ({5})" -f $(if ($payload.exact) { 'exact' } else { 'NOT EXACT' }), $payload.matched, @($payload.missing).Count, @($payload.differing).Count, @($payload.extras).Count, (@($payload.extras | Select-Object -First 5) -join ', ')) }
    foreach ($probe in $contractInstall) { Write-Host ("    {0} {1}: {2}" -f $(if ($probe.present) { 'ok  ' } else { 'MISS' }), $probe.feature, $probe.mechanism) }
    Write-Host ("  uninstall exit={0} {1}s ({2}s process + {3}s completion) root removed={4} ARP removed={5} link removed={6} removal ok={7}" -f $uninstallSummary.exitCode, $uninstallSummary.durationSeconds, $uninstallSummary.processSeconds, $uninstallSummary.completionWaitSeconds, $summary.uninstall.rootRemoved, $summary.uninstall.arpRemoved, $summary.uninstall.startMenuLinkRemoved, $removalOk)
    foreach ($probe in $contractUninstall) {
        if ($probe.kind -eq 'setting') { Write-Host ("    after: {0} -> {1}" -f $probe.feature, $probe.detail) }
        elseif ($probe.kind -notlike 'absent-*' -and $probe.present) { Write-Host ("    LEFT {0}: {1}" -f $probe.feature, $probe.detail) }
    }
    Write-Host ("  row {0}s; VM {1}; success={2}" -f $summary.rowSeconds, $summary.vmStateAfter, $summary.success)

    # The results file is rewritten after every row, so an interrupted
    # campaign still leaves what it measured.
    $record = [ordered]@{
        baseline = $Baseline
        startedAt = $campaignStarted.ToString('o')
        updatedAt = [DateTimeOffset]::Now.ToString('o')
        timing = 'install: the installer process lifetime, plus the exit of any process of the installer''s own name it left running (TigerSetup; the loader''s wait for its engine makes that none); uninstall: the uninstaller process lifetime plus the exit of the processes it hands off to (NSIS: Au_.exe; TigerSetup: any process of the installer''s own name) and the removal of the install root; no artificial delay'
        builder = $(if ($null -ne $buildRecord) { [string] (Get-Prop $buildRecord 'builder') } else { '' })
        engineSha256 = $(if ($null -ne $buildRecord) { [string] (Get-Prop $buildRecord 'engineSha256') } else { '' })
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
