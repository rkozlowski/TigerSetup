<#
    .SYNOPSIS
    Proves what a person finds after installing TigerSetup, on the lab's
    interactive desktop: the Start Menu's TigerSetup Shell opens a command
    prompt that shows the brief help (`tiger-setup --help-brief`), runs the
    installation it belongs to and stays usable; TigerSetup Help opens the PDF
    and TigerSetup Help (Markdown) the Markdown; only the shell wears
    TigerSetup's icon; nothing of the persistent PATH changes. Optionally the
    whole thing goes through WinGet as the signed-in user would use it.

    .DESCRIPTION
    A TigerWinLab guest job entry script, started with -Desktop: the lab hands
    over the signed-in standard user's session as $TigerWinLabDesktop. The job
    itself runs as the lab administrator in session 0; everything a person does
    runs in the user's session through the lab's agent. This script neither
    imports the lab's module nor copies its client.

    request.json:
      version          the product version the shell must report
      scope            user | machine — where the installation is
      installRoot      the install root, %LOCALAPPDATA% etc. meaning the
                       signed-in user's folders
      startMenuFolder  the Start Menu folder holding the shortcuts, likewise
      shortcuts        { shell, help, markdown }: the link names, without
                       .lnk — help is the PDF, markdown its source
      launch           "link" opens each shortcut through the shell
                       (ShellExecute of the .lnk, what a click does);
                       "start-menu" opens the shell the way a person does:
                       Start, type its name, Enter
      pathOption       whether the install added the root to the persistent PATH
      winget           optional { identifier, name, productCode,
                       manifestDirectory, installer, scope }: install through `winget install --manifest`
                       as the signed-in user before the checks, with the
                       installer served from the guest over loopback HTTP, and
                       read `winget list`, which lists a package in no source under
                       its registration's product code
      wingetUninstall  with winget: uninstall through `winget uninstall` as
                       the signed-in user afterwards and check that the
                       installation, its shortcuts and its registration are
                       gone

    The terminal's text is read through UI Automation's TextPattern by a
    small reader run as the signed-in user, in that session: that is the
    evidence that the brief help appeared without anybody typing it, that it
    fits the window (its first line is in the visible range), and — through
    the text's foreground-colour attribute — that the status line is
    coloured. The commands typed into the shell write their answers to files
    the job reads: the evidence that the shell is usable and resolves this
    installation, and that --help, build --help and the absent help command
    answer as they should there.

    The result is the standard phase/check contract written to
    $env:TIGERWINLAB_JOB_RESULT.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$inputRoot = $env:TIGERWINLAB_JOB_INPUT
$artifactRoot = $env:TIGERWINLAB_JOB_ARTIFACTS
# UTF-8 explicitly: Windows PowerShell 5.1 reads a file without a BOM as ANSI.
$request = Get-Content -LiteralPath (Join-Path $inputRoot 'request.json') -Raw -Encoding UTF8 | ConvertFrom-Json

function Get-Member2 {
    param([object] $Object, [string] $Name, [object] $Default = $null)
    if ($null -eq $Object -or $null -eq $Object.PSObject.Properties[$Name]) { return $Default }
    $Object.$Name
}

$phases = [System.Collections.Generic.List[object]]::new()
$script:checks = $null
function Start-Phase {
    param([string] $Name)
    $script:checks = [System.Collections.Generic.List[object]]::new()
    $phases.Add([pscustomobject][ordered]@{ name = $Name; status = 'PASS'; checks = $script:checks })
    Write-Host "== $Name"
}
function Add-Check {
    param([string] $Name, [string] $Code, [ValidateSet('PASS', 'WARN', 'FAIL')] [string] $Status, [string] $Message)
    $script:checks.Add([pscustomobject][ordered]@{ name = $Name; code = $Code; status = $Status; message = $Message })
    Write-Host ("  [{0}] {1}: {2}" -f $Status, $Name, $Message)
}
function Test-Check {
    param([string] $Name, [string] $Code, [bool] $Pass, [string] $Message)
    Add-Check $Name $Code $(if ($Pass) { 'PASS' } else { 'FAIL' }) $Message
}

$version = [string] (Get-Member2 $request 'version')
$scope = [string] (Get-Member2 $request 'scope' 'user')
$launch = [string] (Get-Member2 $request 'launch' 'link')
$names = Get-Member2 $request 'shortcuts'
$winget = Get-Member2 $request 'winget'
$wingetUninstall = [bool] (Get-Member2 $request 'wingetUninstall' $false)
# Whether the install added the root to the persistent PATH.
$expectPath = [bool] (Get-Member2 $request 'pathOption' $false)
$work = 'C:\Users\Public\TigerSetupPresence'
$session = $TigerWinLabDesktop
if ($null -eq $session) { throw 'This payload needs the interactive session; start the job with -Desktop.' }
$info = Invoke-DesktopCommand -Session $session -Command 'info'
$sid = [string] (Get-Member2 $info 'sid')
if ([string]::IsNullOrWhiteSpace($sid)) {
    $sid = ([System.Security.Principal.NTAccount] [string] $info.userName).Translate([System.Security.Principal.SecurityIdentifier]).Value
}
$profilePath = [Environment]::ExpandEnvironmentVariables([string] (Get-ItemProperty -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$sid" -Name ProfileImagePath).ProfileImagePath)

function Expand-UserPath {
    <# %APPDATA%, %LOCALAPPDATA% and %USERPROFILE% are the signed-in user's, not this job account's. #>
    param([string] $Template)
    $text = $Template
    foreach ($pair in @(
            @('%LOCALAPPDATA%', (Join-Path $profilePath 'AppData\Local')),
            @('%APPDATA%', (Join-Path $profilePath 'AppData\Roaming')),
            @('%USERPROFILE%', $profilePath))) {
        $text = [regex]::Replace($text, [regex]::Escape($pair[0]), $pair[1].Replace('$', '$$'), 'IgnoreCase')
    }
    [Environment]::ExpandEnvironmentVariables($text).TrimEnd('\')
}

$installRoot = Expand-UserPath ([string] (Get-Member2 $request 'installRoot'))
$startMenuFolder = Expand-UserPath ([string] (Get-Member2 $request 'startMenuFolder'))
$shellLink = Join-Path $startMenuFolder ([string] $names.shell + '.lnk')
$helpLink = Join-Path $startMenuFolder ([string] $names.help + '.lnk')
$markdownLink = Join-Path $startMenuFolder ([string] $names.markdown + '.lnk')

if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
$null = New-Item -ItemType Directory -Path $work -Force
# The reader runs as the signed-in user, in that session, where UI Automation
# reaches the terminal's text.
# With -Probe it also writes <Out>.probe.json: the text in the visible range,
# and the foreground colour of each '|'-separated phrase in -Probe (a number,
# or "unsupported"/"mixed"/"absent").
$reader = @'
param([long] $Hwnd, [string] $Out, [string] $Probe = '')
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr] $Hwnd)
$condition = New-Object System.Windows.Automation.PropertyCondition ([System.Windows.Automation.AutomationElement]::IsTextPatternAvailableProperty, $true)
$elements = @($root) + @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition))
$text = New-Object System.Text.StringBuilder
$visible = New-Object System.Text.StringBuilder
$colours = @{}
foreach ($element in $elements) {
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern, [ref] $pattern)) {
        $null = $text.AppendLine($pattern.DocumentRange.GetText(-1))
        if ($Probe) {
            foreach ($range in @($pattern.GetVisibleRanges())) { $null = $visible.AppendLine($range.GetText(-1)) }
            foreach ($phrase in @($Probe.Trim('"') -split '\|')) {
                if ($colours.ContainsKey($phrase) -and $colours[$phrase] -ne 'absent') { continue }
                $found = $pattern.DocumentRange.FindText($phrase, $false, $false)
                $value = $(if ($null -eq $found) { 'absent' } else { $found.GetAttributeValue([System.Windows.Automation.TextPattern]::ForegroundColorAttribute) })
                $colours[$phrase] = $(if ($value -is [int]) { $value } elseif ($value -eq [System.Windows.Automation.AutomationElement]::NotSupported) { 'unsupported' } elseif ($value -eq [System.Windows.Automation.TextPattern]::MixedAttributeValue) { 'mixed' } else { [string] $value })
            }
        }
    }
}
[System.IO.File]::WriteAllText($Out, $text.ToString(), (New-Object System.Text.UTF8Encoding $false))
if ($Probe) {
    $json = New-Object PSObject -Property @{ visible = $visible.ToString(); colours = $colours } | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText("$Out.probe.json", $json, (New-Object System.Text.UTF8Encoding $false))
}
'@
[System.IO.File]::WriteAllText((Join-Path $work 'Read-WindowText.ps1'), $reader, (New-Object System.Text.UTF8Encoding $true))
# The icon Explorer shows for a file, as the signed-in user: its index in the
# system image list, for each of the '|'-separated paths.
$iconReader = @'
param([string] $Out, [string] $Paths)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class ShellIcon {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct Info { public IntPtr icon; public int index; public uint attributes;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string name;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)] public string type; }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr SHGetFileInfo(string path, uint attributes, ref Info info, uint size, uint flags);
    public static int Index(string path) { var info = new Info(); return SHGetFileInfo(path, 0, ref info, (uint) Marshal.SizeOf(info), 0x4000) == IntPtr.Zero ? -1 : info.index; }
}
"@
$result = @{}
foreach ($path in @($Paths.Trim('"') -split '\|')) { $result[$path] = [ShellIcon]::Index($path) }
[System.IO.File]::WriteAllText($Out, ($result | ConvertTo-Json), (New-Object System.Text.UTF8Encoding $false))
'@
[System.IO.File]::WriteAllText((Join-Path $work 'Read-ShellIcons.ps1'), $iconReader, (New-Object System.Text.UTF8Encoding $true))

function Save-Shot {
    param([string] $Name)
    try { $null = Save-DesktopCapture -Session $session -Name $Name -Destination (Join-Path $artifactRoot 'shots') } catch { Write-Host "  capture $Name failed: $($_.Exception.Message)" }
}
function Get-Windows {
    @(Invoke-DesktopCommand -Session $session -Command 'list-windows' | Where-Object { $null -ne $_ })
}
function Wait-Window {
    <# A visible top-level window whose title matches, or $null. #>
    param([string] $TitlePattern, [string] $ProcessPattern = '', [int] $TimeoutSeconds = 60)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $match = @(Get-Windows | Where-Object { [string] $_.title -match $TitlePattern -and ([string]::IsNullOrEmpty($ProcessPattern) -or [string] $_.processName -match $ProcessPattern) }) | Select-Object -First 1
        if ($null -ne $match) { return $match }
        Start-Sleep -Milliseconds 500
    }
    $null
}
function Wait-WindowGone {
    param([long] $Hwnd, [int] $TimeoutSeconds = 30)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (@(Get-Windows | Where-Object { [long] $_.hwnd -eq $Hwnd }).Count -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    $false
}
function Invoke-AsUser {
    <# A program run as the signed-in user in that session; its exit code and output. #>
    param([string] $FilePath, [string[]] $Arguments, [int] $TimeoutSeconds = 120)
    Invoke-DesktopCommand -Session $session -Command 'run' -Parameters @{ filePath = $FilePath; arguments = $Arguments; timeoutSeconds = $TimeoutSeconds }
}
function Open-Link {
    <# What a click on the shortcut does: the shell opens the .lnk. #>
    param([string] $Link)
    $null = Invoke-AsUser -FilePath 'cmd.exe' -Arguments @('/c', 'start', '""', "`"$Link`"") -TimeoutSeconds 30
}
function Open-FromStart {
    <# What a person does: Start, type the name, Enter. #>
    param([string] $Name, [string] $ShotName)
    $null = Invoke-DesktopCommand -Session $session -Command 'keyboard' -Parameters @{ action = 'keys'; keys = @('LWin') }
    Start-Sleep -Seconds 3
    $null = Invoke-DesktopCommand -Session $session -Command 'keyboard' -Parameters @{ action = 'text'; text = $Name }
    Start-Sleep -Seconds 4
    Save-Shot $ShotName
    $null = Invoke-DesktopCommand -Session $session -Command 'keyboard' -Parameters @{ action = 'keys'; keys = @('Enter') }
}
function Send-Line {
    param([long] $Hwnd, [string] $Text)
    $null = Invoke-DesktopCommand -Session $session -Command 'window' -Parameters @{ hwnd = $Hwnd; action = 'activate'; settleMilliseconds = 500 }
    $null = Invoke-DesktopCommand -Session $session -Command 'keyboard' -Parameters @{ action = 'text'; text = $Text }
    $null = Invoke-DesktopCommand -Session $session -Command 'keyboard' -Parameters @{ action = 'keys'; keys = @('Enter') }
}
function Read-WindowText {
    param([long] $Hwnd, [string] $Name, [string] $Probe = '')
    $out = Join-Path $work "$Name.txt"
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $work 'Read-WindowText.ps1'), '-Hwnd', [string] $Hwnd, '-Out', $out)
    if ($Probe) { $arguments += @('-Probe', "`"$Probe`"") }
    $run = Invoke-AsUser -FilePath 'powershell.exe' -Arguments $arguments -TimeoutSeconds 60
    if (Test-Path -LiteralPath $out) {
        $text = [System.IO.File]::ReadAllText($out, [System.Text.Encoding]::UTF8)
        Copy-Item -LiteralPath $out -Destination (Join-Path $artifactRoot "$Name.txt") -Force
        return $text
    }
    "(the reader wrote nothing; exit $($run.exitCode): $(@($run.stderr) -join ' ')"
}
function Read-Probe {
    <# What Read-WindowText -Probe found: the visible text and the colours. #>
    param([string] $Name)
    $path = Join-Path $work "$Name.txt.probe.json"
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    Copy-Item -LiteralPath $path -Destination (Join-Path $artifactRoot "$Name.probe.json") -Force
    [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
}
function Wait-File {
    param([string] $Path, [int] $TimeoutSeconds = 30)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $Path) {
            Start-Sleep -Milliseconds 500
            return (Get-Content -LiteralPath $Path -Raw -Encoding Default)
        }
        Start-Sleep -Milliseconds 500
    }
    $null
}
function Get-PersistentPath {
    $machine = (Get-Item -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $userKey = Get-Item -LiteralPath "Registry::HKEY_USERS\$sid\Environment" -ErrorAction SilentlyContinue
    $user = $(if ($null -ne $userKey) { $userKey.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } else { '' })
    [pscustomobject]@{ machine = [string] $machine; user = [string] $user }
}
function Test-PathHolds {
    param([string] $Value, [string] $Directory)
    @(([string] $Value) -split ';' | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_.Trim().Trim('"')).TrimEnd('\') } | Where-Object { $_ -ieq $Directory }).Count -gt 0
}

# WinGet runs as a person runs it, from a command prompt: the app execution
# alias started directly by the agent is refused ("Access is denied").
$source = $null
$sessionError = $null
try {
    if ($null -ne $winget) {
        Start-Phase 'winget-install'
        # Local manifests are an administrator's setting; the policy is how an
        # administrator sets it for the machine. The launch policy is the one
        # Microsoft's winget-pkgs sandbox harness sets, for this user only:
        # without it an unsigned installer WinGet downloaded waits behind an
        # "Open File - Security Warning" dialog. Hash verification is untouched.
        $policy = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\AppInstaller'
        $null = New-Item -Path $policy -Force
        $null = New-ItemProperty -Path $policy -Name 'EnableLocalManifestFiles' -PropertyType DWord -Value 1 -Force
        $associations = "Registry::HKEY_USERS\$sid\Software\Microsoft\Windows\CurrentVersion\Policies\Associations"
        $null = New-Item -Path $associations -Force
        $null = New-ItemProperty -Path $associations -Name 'ModRiskFileTypes' -PropertyType String -Value '.bat;.exe;.reg;.vbs;.chm;.msi;.js;.cmd' -Force

        $installer = Join-Path $inputRoot ([string] $winget.installer)
        $port = & {
            $probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
            $probe.Start(); try { [int] ([System.Net.IPEndPoint] $probe.LocalEndpoint).Port } finally { $probe.Stop() }
        }
        $requestLog = Join-Path $work 'installer-requests.log'
        $listener = [System.Net.HttpListener]::new()
        $listener.Prefixes.Add("http://127.0.0.1:$port/")
        $listener.Start()
        $runspace = [runspacefactory]::CreateRunspace(); $runspace.Open()
        $worker = [powershell]::Create(); $worker.Runspace = $runspace
        $null = $worker.AddScript({
                param($Listener, $File, $LogPath)
                $bytes = [System.IO.File]::ReadAllBytes($File)
                while ($Listener.IsListening) {
                    try { $context = $Listener.GetContext() } catch { break }
                    $status = 200
                    try {
                        $context.Response.ContentType = 'application/octet-stream'
                        $context.Response.ContentLength64 = $bytes.Length
                        $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
                    }
                    catch { $status = 499 }
                    finally {
                        Add-Content -LiteralPath $LogPath -Value ('{0:o} {1} {2} {3}' -f [DateTime]::UtcNow, $context.Request.HttpMethod, $context.Request.Url.AbsolutePath, $status)
                        try { $context.Response.Close() } catch { }
                    }
                }
            }).AddArgument($listener).AddArgument($installer).AddArgument($requestLog)
        $null = $worker.BeginInvoke()
        $source = [pscustomobject]@{ listener = $listener; worker = $worker; runspace = $runspace }
        $url = "http://127.0.0.1:$port/$([System.IO.Path]::GetFileName($installer))"

        # The manifest as published, with only the installer URL pointing at
        # the loopback copy; InstallerSha256 stays the published hash, so
        # WinGet's own check is the real one.
        $manifests = Join-Path $work 'manifest'
        Copy-Item -LiteralPath (Join-Path $inputRoot ([string] $winget.manifestDirectory)) -Destination $manifests -Recurse -Force
        $installerManifest = @(Get-ChildItem -LiteralPath $manifests -Filter '*.installer.yaml')[0].FullName
        $yaml = [System.IO.File]::ReadAllText($installerManifest, [System.Text.Encoding]::UTF8)
        $yaml = [regex]::Replace($yaml, '(?m)^(\s*InstallerUrl:\s*).*$', { param($m) $m.Groups[1].Value + $url })
        [System.IO.File]::WriteAllText($installerManifest, $yaml, (New-Object System.Text.UTF8Encoding $false))

        $install = Invoke-AsUser -FilePath 'cmd.exe' -Arguments @('/c', 'winget', 'install', '--manifest', $manifests, '--scope', [string] $winget.scope,
            '--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity') -TimeoutSeconds 600
        $installText = (@($install.stdout) + @($install.stderr)) -join "`n"
        [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'winget-install.log'), $installText, (New-Object System.Text.UTF8Encoding $false))
        Test-Check 'winget install --manifest' 'winget.install' ($install.exitCode -eq 0) "exit $($install.exitCode): $(($installText -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 3) -join ' | ')"
        $requests = @($(if (Test-Path -LiteralPath $requestLog) { Get-Content -LiteralPath $requestLog }))
        Test-Check 'WinGet downloaded the candidate installer' 'winget.download' (@($requests | Where-Object { $_ -match ' GET .* 200$' }).Count -ge 1) ($requests -join '; ')

        # A package in no source is listed under its registration,
        # ARP\<scope>\<architecture>\<ProductCode>: the product code the manifest
        # declares is what correlates it. By package identifier it correlates only
        # once a source carries the package.
        $list = Invoke-AsUser -FilePath 'cmd.exe' -Arguments @('/c', 'winget', 'list', '--name', [string] $winget.name, '--exact', '--accept-source-agreements', '--disable-interactivity') -TimeoutSeconds 120
        $listText = (@($list.stdout) -join "`n")
        [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'winget-list.log'), $listText, (New-Object System.Text.UTF8Encoding $false))
        $row = @($listText -split "`n" | Where-Object { $_ -match ('\\' + [regex]::Escape([string] $winget.productCode) + '(\s|$)') }) | Select-Object -First 1
        Test-Check "winget list shows $($winget.name) $version under ProductCode $($winget.productCode)" 'winget.list' ($list.exitCode -eq 0 -and $null -ne $row -and $row -match [regex]::Escape($version)) "exit $($list.exitCode): $(($row -replace '\s+', ' ').Trim())"
    }

    $pathBefore = Get-PersistentPath

    Start-Phase 'start-menu'
    Test-Check 'Start Menu folder' 'presence.folder' (Test-Path -LiteralPath $startMenuFolder -PathType Container) $startMenuFolder
    foreach ($link in @($shellLink, $helpLink, $markdownLink)) {
        Test-Check "$([System.IO.Path]::GetFileName($link))" 'presence.shortcut' (Test-Path -LiteralPath $link -PathType Leaf) $link
    }
    $extra = @(Get-ChildItem -LiteralPath $startMenuFolder -Force -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notin @($shellLink, $helpLink, $markdownLink) } | ForEach-Object { $_.Name })
    Test-Check 'the folder holds the three shortcuts only' 'presence.folder.exact' ($extra.Count -eq 0) $(if ($extra.Count -gt 0) { "also: $($extra -join ', ')" } else { 'nothing else' })

    # The icons a person sees: Explorer's own answer for each link and for
    # what it opens, as the signed-in user whose file associations apply.
    $exe = Join-Path $installRoot 'tiger-setup.exe'
    $pdf = Join-Path $installRoot 'help\TigerSetup-Help.pdf'
    $markdown = Join-Path $installRoot 'help\TigerSetup-Help.md'
    $iconOut = Join-Path $work 'icons.json'
    $iconPaths = @($shellLink, $helpLink, $markdownLink, $exe, $pdf, $markdown)
    $null = Invoke-AsUser -FilePath 'powershell.exe' -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $work 'Read-ShellIcons.ps1'), '-Out', $iconOut, '-Paths', "`"$($iconPaths -join '|')`"") -TimeoutSeconds 60
    $icons = $(if (Test-Path -LiteralPath $iconOut) { Copy-Item -LiteralPath $iconOut -Destination (Join-Path $artifactRoot 'icons.json') -Force; Get-Content -LiteralPath $iconOut -Raw | ConvertFrom-Json } else { $null })
    $index = { param($Path) $(if ($null -ne $icons -and $null -ne $icons.PSObject.Properties[$Path]) { [int] $icons.$Path } else { -2 }) }
    $exeIcon = & $index $exe
    $shellIcon = & $index $shellLink
    Add-Check "$($names.shell) shows TigerSetup's icon" 'presence.icon.shell' $(if ($exeIcon -ge 0 -and $shellIcon -eq $exeIcon) { 'PASS' } else { 'WARN' }) "link $shellIcon, tiger-setup.exe $exeIcon (the link's icon location is checked from the host)"
    foreach ($pair in @(@($helpLink, $pdf, 'PDF'), @($markdownLink, $markdown, 'Markdown'))) {
        $linkIcon = & $index $pair[0]; $documentIcon = & $index $pair[1]
        Test-Check "$([System.IO.Path]::GetFileNameWithoutExtension($pair[0])) shows the $($pair[2]) file's icon, not TigerSetup's" 'presence.icon.help' ($linkIcon -ge 0 -and $linkIcon -eq $documentIcon -and $linkIcon -ne $exeIcon) "link $linkIcon, document $documentIcon, tiger-setup.exe $exeIcon"
    }

    Start-Phase 'shell'
    # The shell must work whether or not the install added the root to PATH:
    # the presence rows install with the option off, WinGet with its default.
    $pathHeld = (Test-PathHolds $pathBefore.machine $installRoot) -or (Test-PathHolds $pathBefore.user $installRoot)
    Test-Check "the persistent PATH $(if ($expectPath) { 'holds' } else { 'does not hold' }) the install root" 'shell.path.persistent' ($pathHeld -eq $expectPath) "machine: $($pathBefore.machine) | user: $($pathBefore.user)"
    if ($launch -eq 'start-menu') { Open-FromStart -Name ([string] $names.shell) -ShotName 'start-search.png' } else { Open-Link $shellLink }
    $shell = Wait-Window -TitlePattern '^TigerSetup Shell' -TimeoutSeconds 60
    Test-Check 'TigerSetup Shell opens' 'shell.window' ($null -ne $shell) $(if ($null -ne $shell) { "'$($shell.title)' ($($shell.processName), $($shell.className))" } else { "no window titled 'TigerSetup Shell': $((Get-Windows | ForEach-Object { "'$($_.title)' ($($_.processName))" }) -join ', ')" })
    if ($null -ne $shell) {
        $hwnd = [long] $shell.hwnd
        $text = ''
        $status = 'tiger-setup is on PATH in this window.'
        $plain = 'Build Windows installers from TigerSetup.toml.'
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while ([DateTime]::UtcNow -lt $deadline) {
            $text = Read-WindowText -Hwnd $hwnd -Name 'shell-opened' -Probe "$status|$plain|TigerSetup $version"
            if ($text -match 'Getting started:') { break }
            Start-Sleep -Seconds 2
        }
        Save-Shot 'shell-opened.png'
        $probe = Read-Probe -Name 'shell-opened'
        $brief = @($status, 'Getting started:', '  tiger-setup build TigerSetup.toml', '  tiger-setup --help', '  tiger-setup build --help', '  Start > TigerSetup > TigerSetup Help')
        $missing = @($brief | Where-Object { $text.IndexOf($_, [StringComparison]::Ordinal) -lt 0 })
        Test-Check 'the brief help appears by itself' 'shell.help' ($missing.Count -eq 0) $(if ($missing.Count -eq 0) { 'the terminal shows the brief help' } else { "missing $($missing -join ' / '); terminal text: $($text.Substring(0, [Math]::Min(400, $text.Length)))" })
        Test-Check 'the complete reference is not what opens' 'shell.help.brief' ($text -notmatch 'Usage: tiger-setup' -and $text -notmatch '(?m)^Commands:') 'no Usage: or Commands: block in the opened window'
        Test-Check "the shell names TigerSetup $version and its folder" 'shell.banner' ($text -match ('TigerSetup ' + [regex]::Escape($version)) -and $text.IndexOf($installRoot, [StringComparison]::OrdinalIgnoreCase) -ge 0) "expected 'TigerSetup $version' and '$installRoot'"
        # It fits the window as it opens: the banner and the last line of the
        # brief help are both in the visible range, nothing scrolled away.
        $visible = $(if ($null -ne $probe) { [string] $probe.visible } else { '' })
        Test-Check 'the brief help fits the window it opens in' 'shell.help.fits' ($visible -match ('TigerSetup ' + [regex]::Escape($version)) -and $visible.IndexOf('Start > TigerSetup > TigerSetup Help', [StringComparison]::Ordinal) -ge 0) "visible: $(($visible -split "`r?`n" | Where-Object { $_ -match '\S' } | Select-Object -First 3) -join ' | ') ... ($(@($visible -split "`r?`n" | Where-Object { $_ -match '\S' }).Count) non-empty lines)"
        # Colour: the status line's foreground differs from the plain text's.
        $colours = $(if ($null -ne $probe) { $probe.colours } else { $null })
        $statusColour = $(if ($null -ne $colours -and $null -ne $colours.PSObject.Properties[$status]) { $colours.$status } else { 'unread' })
        $plainColour = $(if ($null -ne $colours -and $null -ne $colours.PSObject.Properties[$plain]) { $colours.$plain } else { 'unread' })
        $comparable = $statusColour -is [int] -or $statusColour -is [long]
        Add-Check 'the PATH status is coloured' 'shell.colour' $(if ($comparable -and ($plainColour -is [int] -or $plainColour -is [long]) -and $statusColour -ne $plainColour) { 'PASS' } elseif ($comparable) { 'FAIL' } else { 'WARN' }) `
            "status foreground $statusColour, plain text $plainColour$(if (-not $comparable) { ' (the terminal reports no colour attribute; see shell-opened.png)' })"

        # Who is running: %ComSpec%, started by this installation's tiger-setup.exe.
        $interpreter = @(Get-CimInstance Win32_Process -Filter "Name='cmd.exe'" | Where-Object { [string] $_.CommandLine -match 'title TigerSetup Shell' }) | Select-Object -First 1
        $parent = $(if ($null -ne $interpreter) { Get-CimInstance Win32_Process -Filter "ProcessId=$($interpreter.ParentProcessId)" } else { $null })
        Test-Check 'the shell is %ComSpec%' 'shell.comspec' ($null -ne $interpreter -and [string] $interpreter.ExecutablePath -ieq [Environment]::ExpandEnvironmentVariables('%ComSpec%')) $(if ($null -ne $interpreter) { "$($interpreter.ExecutablePath): $($interpreter.CommandLine)" } else { 'no cmd.exe carries the shell title' })
        Test-Check 'started by the installed tiger-setup.exe' 'shell.owner' ($null -ne $parent -and [string] $parent.ExecutablePath -ieq (Join-Path $installRoot 'tiger-setup.exe')) $(if ($null -ne $parent) { "$($parent.ExecutablePath) $($parent.CommandLine)" } else { 'no parent process' })

        # A person uses it: the answers land in files the job reads.
        $versionFile = Join-Path $work 'version.txt'; $whereFile = Join-Path $work 'where.txt'; $cdFile = Join-Path $work 'cd.txt'
        Send-Line -Hwnd $hwnd -Text "tiger-setup --version > `"$versionFile`" & where tiger-setup > `"$whereFile`" & cd > `"$cdFile`""
        $reported = Wait-File -Path $cdFile
        $versionText = [string] (Wait-File -Path $versionFile -TimeoutSeconds 5)
        $whereText = [string] (Wait-File -Path $whereFile -TimeoutSeconds 5)
        Test-Check "typed tiger-setup --version answers $version" 'shell.version' ($versionText -match ('(^|\s)' + [regex]::Escape($version) + '(\s|$)')) "version: $($versionText.Trim())"
        $firstFound = @($whereText -split "`r?`n" | Where-Object { $_ -match '\S' }) | Select-Object -First 1
        Test-Check 'tiger-setup resolves to this installation' 'shell.resolves' ([string] $firstFound -ieq (Join-Path $installRoot 'tiger-setup.exe')) "where: $($whereText.Trim() -replace "`r?`n", ' | ')"
        Test-Check "the shell starts in the user's profile" 'shell.cwd' (([string] $reported).Trim() -ieq $profilePath) "cd: $(([string] $reported).Trim())"

        # The complete reference, a command's own help, and no help command.
        $helpFile = Join-Path $work 'help.txt'; $buildHelpFile = Join-Path $work 'build-help.txt'; $helpCommandFile = Join-Path $work 'help-command.txt'; $doneFile = Join-Path $work 'help-done.txt'
        Send-Line -Hwnd $hwnd -Text "tiger-setup --help > `"$helpFile`" & tiger-setup build --help > `"$buildHelpFile`" & (tiger-setup help > `"$helpCommandFile`" 2>&1 && echo accepted> `"$doneFile`" || echo refused> `"$doneFile`")"
        $helpCommand = ([string] (Wait-File -Path $doneFile)).Trim()
        $fullHelp = [string] (Wait-File -Path $helpFile -TimeoutSeconds 5)
        $buildHelp = [string] (Wait-File -Path $buildHelpFile -TimeoutSeconds 5)
        $helpCommandText = [string] (Wait-File -Path $helpCommandFile -TimeoutSeconds 5)
        foreach ($file in @($helpFile, $buildHelpFile, $helpCommandFile)) { if (Test-Path -LiteralPath $file) { Copy-Item -LiteralPath $file -Destination $artifactRoot -Force } }
        Test-Check 'tiger-setup --help is the complete reference' 'shell.help.full' ($fullHelp -match 'Usage: tiger-setup' -and $fullHelp -match '(?m)^Commands:' -and $fullHelp -match '(?m)^\s+build\s' -and $fullHelp -match '--help-brief') "$(@($fullHelp -split "`r?`n").Count) lines"
        Test-Check 'tiger-setup --help names no installed location' 'shell.help.nolocation' ($fullHelp -match 'Usage:' -and $fullHelp -notmatch 'Start > TigerSetup' -and $fullHelp -notmatch 'TigerSetup-Help' -and $fullHelp.IndexOf($installRoot, [StringComparison]::OrdinalIgnoreCase) -lt 0) 'no Start Menu, help file or install folder in it'
        Test-Check 'tiger-setup build --help' 'shell.help.build' ($buildHelp -match 'Usage: tiger-setup(\.exe)? build' -and $buildHelp -match '--fast') "$(@($buildHelp -split "`r?`n").Count) lines"
        Test-Check 'there is no tiger-setup help command' 'shell.help.command' ($helpCommand -eq 'refused' -and $helpCommandText -match "unrecognized subcommand 'help'") "$helpCommand`: $(($helpCommandText -split "`r?`n" | Where-Object { $_ -match '\S' } | Select-Object -First 1))"
        Save-Shot 'shell-used.png'
        Send-Line -Hwnd $hwnd -Text 'exit'
        Test-Check 'exit closes the shell' 'shell.exit' (Wait-WindowGone -Hwnd $hwnd) 'the window went away after exit'
    }
    $pathAfter = Get-PersistentPath
    Test-Check 'opening the shell changed no persistent PATH' 'shell.path.unchanged' ($pathAfter.machine -ceq $pathBefore.machine -and $pathAfter.user -ceq $pathBefore.user) "machine unchanged: $($pathAfter.machine -ceq $pathBefore.machine); user unchanged: $($pathAfter.user -ceq $pathBefore.user)"

    # TigerSetup Help is the PDF, and opens directly: no question first.
    Start-Phase 'help-pdf'
    Open-Link $helpLink
    $viewer = Wait-Window -TitlePattern 'TigerSetup-Help\.pdf' -TimeoutSeconds 60
    Start-Sleep -Seconds 2
    Save-Shot 'help-pdf.png'
    Test-Check "$($names.help) opens the PDF" 'help.pdf.opens' ($null -ne $viewer) $(if ($null -ne $viewer) { "'$($viewer.title)' ($($viewer.processName))" } else { "no window shows TigerSetup-Help.pdf: $((Get-Windows | ForEach-Object { "'$($_.title)' ($($_.processName))" }) -join ', ')" })
    if ($null -ne $viewer) { $null = Invoke-DesktopCommand -Session $session -Command 'window' -Parameters @{ hwnd = [long] $viewer.hwnd; action = 'close' } }

    # TigerSetup Help (Markdown) is the source, the second form.
    Start-Phase 'help-markdown'
    $before = @(Get-Windows | ForEach-Object { [long] $_.hwnd })
    Open-Link $markdownLink
    $notepad = $null
    $picker = $false
    $seen = @{}
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while ($null -eq $notepad -and [DateTime]::UtcNow -lt $deadline) {
        $windows = Get-Windows
        $notepad = @($windows | Where-Object { [string] $_.title -match 'TigerSetup-Help' -and [string] $_.processName -match 'notepad' }) | Select-Object -First 1
        if ($null -ne $notepad) { break }
        foreach ($window in @($windows | Where-Object { $before -notcontains [long] $_.hwnd })) {
            $seen[[string] $window.hwnd] = "'$($window.title)' ($($window.processName), $($window.className))"
        }
        # Windows' app picker ("Select an app to open this .md file") is an
        # Explorer popup that shows neither a window nor UI Automation
        # elements to this session, and takes no keyboard focus. A person
        # clicks "Notepad", then "Just once"; so does the row, where the
        # centred picker puts them on the lab's fixed 1920x1080, 100% display,
        # with the owner of each point recorded first.
        if (-not $picker -and [DateTime]::UtcNow -gt $deadline.AddSeconds(-38)) {
            $picker = $true
            Save-Shot 'help-markdown-picker.png'
            foreach ($point in @(@{ what = 'Notepad'; x = 831; y = 458 }, @{ what = 'Just once'; x = 1062; y = 681 })) {
                $hit = $(try { Invoke-DesktopCommand -Session $session -Command 'hit-test' -Parameters @{ x = $point.x; y = $point.y } } catch { $null })
                $seen[$point.what] = "$($point.what) at $($point.x),$($point.y): $(if ($null -ne $hit) { ($hit | ConvertTo-Json -Depth 3 -Compress) } else { 'no hit-test' })"
                $null = Invoke-DesktopCommand -Session $session -Command 'mouse' -Parameters @{ action = 'click'; x = $point.x; y = $point.y; settleMilliseconds = 1000 }
            }
        }
        Start-Sleep -Seconds 1
    }
    if ($null -eq $notepad) { Write-Host "  new windows: $(@($seen.Values) -join '; ')" }
    Save-Shot 'help-markdown.png'
    Test-Check "$($names.markdown) opens the Markdown" 'help.markdown.opens' ($null -ne $notepad) $(if ($null -ne $notepad) { "'$($notepad.title)' ($($notepad.processName))" } else { "no window shows TigerSetup-Help.md; new windows: $(@($seen.Values) -join '; ')" })
    Add-Check 'no app picker for Markdown' 'help.markdown.direct' $(if ($picker) { 'WARN' } else { 'PASS' }) $(if ($picker) { "Windows has no default app for .md and asked which app opens it; Notepad was chosen, just once ($(@($seen.Values) -join '; ')). Accepted: the Markdown is the secondary help" } else { 'it opened without a question' })
    if ($null -ne $notepad) { $null = Invoke-DesktopCommand -Session $session -Command 'window' -Parameters @{ hwnd = [long] $notepad.hwnd; action = 'close' } }

    if ($null -ne $winget -and $wingetUninstall) {
        Start-Phase 'winget-uninstall'
        $remove = Invoke-AsUser -FilePath 'cmd.exe' -Arguments @('/c', 'winget', 'uninstall', '--product-code', [string] $winget.productCode, '--exact',
            '--accept-source-agreements', '--disable-interactivity') -TimeoutSeconds 600
        $removeText = (@($remove.stdout) + @($remove.stderr)) -join "`n"
        [System.IO.File]::WriteAllText((Join-Path $artifactRoot 'winget-uninstall.log'), $removeText, (New-Object System.Text.UTF8Encoding $false))
        Test-Check 'winget uninstall' 'winget.uninstall' ($remove.exitCode -eq 0) "exit $($remove.exitCode): $(($removeText -split "`n" | Where-Object { $_ -match '\S' } | Select-Object -Last 3) -join ' | ')"
        Start-Sleep -Seconds 3
        Test-Check 'the install root is gone' 'winget.cleanup.root' (-not (Test-Path -LiteralPath $installRoot)) $installRoot
        Test-Check 'the Start Menu folder is gone' 'winget.cleanup.folder' (-not (Test-Path -LiteralPath $startMenuFolder)) $startMenuFolder
        $hive = $(if ($scope -eq 'machine') { 'HKEY_LOCAL_MACHINE' } else { "HKEY_USERS\$sid" })
        $arp = "Registry::$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\$([string] $winget.identifier)"
        Test-Check 'the registration is gone' 'winget.cleanup.registration' (-not (Test-Path -LiteralPath $arp)) $arp
        $list = Invoke-AsUser -FilePath 'cmd.exe' -Arguments @('/c', 'winget', 'list', '--name', [string] $winget.name, '--exact', '--accept-source-agreements', '--disable-interactivity') -TimeoutSeconds 120
        $listed = @(@($list.stdout) | Where-Object { [string] $_ -match ('\\' + [regex]::Escape([string] $winget.productCode) + '(\s|$)') }).Count -gt 0
        Test-Check 'winget list no longer shows it' 'winget.cleanup.list' (-not $listed) "exit $($list.exitCode)"
    }
}
catch {
    $sessionError = ('{0} (line {1}: {2})' -f $_.Exception.Message, $_.InvocationInfo.ScriptLineNumber, $_.InvocationInfo.Line.Trim())
    Write-Host "acceptance failed: $sessionError"
    if ($null -eq $script:checks) { Start-Phase 'run' }
    Add-Check 'acceptance run' 'run.error' 'FAIL' $sessionError
}
finally {
    if ($null -ne $source) {
        try { $source.listener.Stop(); $source.listener.Close() } catch { }
        try { $source.worker.Dispose(); $source.runspace.Dispose() } catch { }
    }
    try { Copy-DesktopAgentLog -Session $session -Destination $artifactRoot } catch { }
}

foreach ($phase in $phases) {
    $phase.status = $(if (@($phase.checks | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { 'FAIL' } elseif (@($phase.checks | Where-Object { $_.status -eq 'WARN' }).Count -gt 0) { 'WARN' } else { 'PASS' })
}
$result = [pscustomobject][ordered]@{
    session = [pscustomobject]@{ userName = $info.userName; sid = $sid; profile = $profilePath }
    installRoot = $installRoot
    startMenuFolder = $startMenuFolder
    error = $sessionError
    phases = @($phases)
}
$json = $result | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($env:TIGERWINLAB_JOB_RESULT, $json, (New-Object System.Text.UTF8Encoding $false))
[System.IO.File]::WriteAllText((Join-Path $artifactRoot 'presence-acceptance.json'), $json, (New-Object System.Text.UTF8Encoding $false))
if ($null -ne $sessionError -or @($phases | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0) { exit 1 }
exit 0
