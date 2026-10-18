<#
    .SYNOPSIS
    Proves this machine has what the release build needs, before it is paid
    for: the release workflow's build job runs it first, and the runner
    preflight (ci.yml, release-readiness) runs it on a hosted runner during
    release preparation.

    .DESCRIPTION
    Checks, and reports every result:

      powershell   PowerShell 7, which every release script requires
      git          git answers
      rust         rustc and cargo answer, and the toolchain holds the standard
                   library for the target .cargo/config.toml builds for
      msvc         Visual Studio's C++ tools (cl.exe, link.exe), which the
                   loader and SQLite are compiled and linked with
      windows-sdk  the Windows SDK's rc.exe, which compiles every executable's
                   version resource
      dotnet       a .NET 10 SDK, to build tiger-mark, and the .NET 10 Desktop
                   Runtime it runs on
      webview2     the WebView2 Runtime, which tiger-mark renders through
      gh           the GitHub CLI, which creates the draft release

    -GitHubAccess adds what the job's own credentials can do, read-only: gh
    with GH_TOKEN reads the repository and lists its releases, and git with the
    checkout's credentials reads origin. Writing - the tag and the draft - is
    not probed: nothing here creates anything, and the publish job's first
    write is the tag push, which a rerun completes (RELEASING.md, Recovery).

    It proves availability, not correctness: tiger-mark's rendering is proved
    by Build-TigerMark.ps1, and the build by building. It runs anywhere; its
    title says where. Exit codes: 0 PASS, 1 FAIL.

    .EXAMPLE
    pwsh -File eng\release\Test-ReleaseRunner.ps1
#>
#Requires -Version 7.0
[CmdletBinding()]
param(
    # Also prove the job's read access to GitHub (gh with GH_TOKEN, git with
    # the checkout's credentials).
    [switch] $GitHubAccess
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'TigerSetupRelease.psm1')
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$facts = Get-TigerSetupReleaseFacts
$checks = [Collections.Generic.List[object]]::new()
function Add-Check { param($Id, $Status, $Observed, $Remediation = '') $checks.Add((New-TigerSetupReleaseCheck -Id $Id -Status $Status -Observed $Observed -Remediation $Remediation)) }

function Invoke-Native {
    <# A native command's exit code and text, or exit -1 when it is not there. #>
    param([Parameter(Mandatory)] [string] $Name, [string[]] $Arguments = @())
    if ($null -eq (Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue)) { return [pscustomobject]@{ code = -1; text = "$Name is not on PATH" } }
    $previous = $PSNativeCommandUseErrorActionPreference
    try {
        $PSNativeCommandUseErrorActionPreference = $false
        $text = (& $Name @Arguments 2>&1 | Out-String).Trim()
        [pscustomobject]@{ code = $global:LASTEXITCODE; text = $text }
    }
    finally { $PSNativeCommandUseErrorActionPreference = $previous; $global:LASTEXITCODE = 0 }
}

$version = $PSVersionTable.PSVersion
Add-Check 'powershell' $(if ($version.Major -ge 7) { 'PASS' } else { 'FAIL' }) "PowerShell $version."

$git = Invoke-Native git @('--version')
Add-Check 'git' $(if ($git.code -eq 0) { 'PASS' } else { 'FAIL' }) $git.text 'Install Git for Windows.'

# The target .cargo/config.toml names, and the standard library for it.
$target = [regex]::Match((Get-Content -LiteralPath (Join-Path $repoRoot '.cargo\config.toml') -Raw), '(?m)^\s*target\s*=\s*"(?<t>[^"]+)"').Groups['t'].Value
$rustc = Invoke-Native rustc @('-vV')
$cargo = Invoke-Native cargo @('--version')
$sysroot = Invoke-Native rustc @('--print', 'sysroot')
$std = if ($sysroot.code -eq 0 -and $target) { Join-Path $sysroot.text "lib\rustlib\$target\lib" } else { '' }
if ($rustc.code -ne 0 -or $cargo.code -ne 0) { Add-Check 'rust' FAIL "rustc: $($rustc.text); cargo: $($cargo.text)" 'Install the stable Rust toolchain (rustup).' }
elseif (-not $target -or -not (Test-Path -LiteralPath $std -PathType Container)) { Add-Check 'rust' FAIL "The toolchain has no standard library for '$target' ($std)." "rustup target add $target" }
else {
    $release = [regex]::Match($rustc.text, '(?m)^release:\s*(\S+)').Groups[1].Value
    Add-Check 'rust' PASS "rustc $release, $($cargo.text), standard library for $target."
}

# Visual Studio's C++ tools, found the way the cc crate finds them.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
    (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null | Select-Object -First 1)
}
$global:LASTEXITCODE = 0
$msvc = @(if ($vs) { Get-ChildItem -LiteralPath (Join-Path $vs 'VC\Tools\MSVC') -Directory -ErrorAction SilentlyContinue | Sort-Object { [version] ($_.Name -replace '[^0-9.]', '') } -Descending })
$tools = if ($msvc.Count) { Join-Path $msvc[0].FullName 'bin\Hostx64\x64' } else { '' }
if ($tools -and (Test-Path -LiteralPath (Join-Path $tools 'cl.exe')) -and (Test-Path -LiteralPath (Join-Path $tools 'link.exe'))) {
    Add-Check 'msvc' PASS "MSVC $($msvc[0].Name) in $vs."
}
else { Add-Check 'msvc' FAIL "No Visual Studio installation with the x64 C++ tools (cl.exe, link.exe) was found$(if ($vs) { " in $vs" })." 'Install Visual Studio Build Tools with the C++ workload.' }

$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$rc = @(Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue | Where-Object Name -Match '^10\.' |
        Sort-Object { [version] $_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'x64\rc.exe' } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })
Add-Check 'windows-sdk' $(if ($rc.Count) { 'PASS' } else { 'FAIL' }) $(if ($rc.Count) { "rc.exe: $($rc[0])" } else { "No Windows SDK rc.exe under $kits." }) 'Install the Windows 10 or 11 SDK.'

$sdks = Invoke-Native dotnet @('--list-sdks')
$runtimes = Invoke-Native dotnet @('--list-runtimes')
$sdk10 = @($sdks.text -split "`n" | Where-Object { $_ -match '^10\.' })
$desktop10 = @($runtimes.text -split "`n" | Where-Object { $_ -match '^Microsoft\.WindowsDesktop\.App 10\.' })
if ($sdk10.Count -and $desktop10.Count) { Add-Check 'dotnet' PASS ".NET SDK $(($sdk10[-1] -split ' ')[0]); $(($desktop10[-1] -split ' \[')[0])." }
else { Add-Check 'dotnet' FAIL "SDK 10: $(if ($sdk10.Count) { 'present' } else { 'missing' }); Desktop Runtime 10: $(if ($desktop10.Count) { 'present' } else { 'missing' }) ($($sdks.text) | $($runtimes.text))." 'Set up the .NET 10 SDK (actions/setup-dotnet), which carries the Desktop Runtime.' }

# WebView2's Evergreen Runtime registers its version under EdgeUpdate.
$client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$webview = @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$client"
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client"
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client"
) | ForEach-Object {
    $item = Get-ItemProperty -LiteralPath $_ -ErrorAction SilentlyContinue
    if ($null -ne $item -and $null -ne $item.PSObject.Properties['pv']) { [string] $item.pv }
} | Where-Object { $_ -and $_ -ne '0.0.0.0' } | Select-Object -First 1
Add-Check 'webview2' $(if ($webview) { 'PASS' } else { 'FAIL' }) $(if ($webview) { "WebView2 Runtime $webview." } else { 'The WebView2 Runtime is not installed.' }) 'Install the WebView2 Evergreen Runtime.'

$gh = Invoke-Native gh @('--version')
Add-Check 'gh' $(if ($gh.code -eq 0) { 'PASS' } else { 'FAIL' }) (($gh.text -split "`n")[0]) 'Install the GitHub CLI.'

if ($GitHubAccess) {
    $repository = Invoke-TigerSetupGitHubApi "repos/$($facts.Repository)"
    $releases = Invoke-TigerSetupGitHubApi "repos/$($facts.Repository)/releases?per_page=1"
    $whose = if ($env:GITHUB_ACTIONS -ceq 'true') { "the job's GH_TOKEN" } else { "this machine's gh login" }
    if ($repository.ok -and $releases.ok) { Add-Check 'access/gh' PASS "gh reads $($facts.Repository) and lists its releases with $whose." }
    else { Add-Check 'access/gh' FAIL "gh with ${whose}: $($repository.error) $($releases.error)".Trim() 'Give the job contents: read and GH_TOKEN: github.token.' }
    $origin = Invoke-TigerSetupGit $repoRoot @('ls-remote', '--heads', 'origin', $facts.DefaultBranch)
    if ($origin.ok -and $origin.output) { Add-Check 'access/git' PASS "git reads origin's $($facts.DefaultBranch) with the checkout's credentials." }
    else { Add-Check 'access/git' FAIL "git ls-remote origin: $($origin.output)" 'Check out with actions/checkout, which configures the job''s token for origin.' }
}

$where = if ($env:GITHUB_ACTIONS -ceq 'true') { "runner $env:RUNNER_NAME ($env:ImageOS $env:ImageVersion)" } else { "$env:COMPUTERNAME (not a runner)" }
exit (Write-TigerSetupReleaseReport -Title "Release build prerequisites on $where" -Checks $checks.ToArray())
