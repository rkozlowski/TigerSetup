<#
    .SYNOPSIS
    Focused tests for eng\Clean-TigerSetup.ps1.

    .DESCRIPTION
    Every scenario builds a synthetic repository-shaped directory under the
    temporary directory - Cargo.toml and a stand-in for every location the
    cleanup policy names, plus the tracked files it must never touch - runs
    the cleanup script against it with -RepositoryRoot, and checks the
    filesystem and the -PassThru result. The real checkout is never cleaned.

    The registry scenario is the one exception to "synthetic only": it
    creates a probe subkey under HKCU\Software\TigerSetupTests, the namespace
    -TestState removes, and a neighbour key beside it that must survive. It
    removes the namespace as a whole, exactly as -TestState does for a
    developer, so do not run it while `cargo test` is writing there.

    Scenarios:

      measure      -Measure removes nothing and adds up to the bytes written
      default      routine targets go, artifacts and everything tracked stay
      all          -All removes artifacts too; the survivors are exactly the
                   preserved set
      whatif       -All -TestState -WhatIf changes nothing on disk or in the
                   registry
      absent       a repository with none of the targets is cleaned quietly
      payload      packages\test-app\*\payload is removed per version and the
                   manifests beside it stay
      links        a junction as a target, inside a target and on the way to
                   a target is skipped, never followed, and fails the run
      registry     -TestState removes the test namespace only, tolerates its
                   absence and honours -WhatIf
      failure      a locked file leaves a truthful partial/failed state, an
                   honest reclaimed size and exit code 1 - through
                   `cargo clean` for target\ and through Remove-Item elsewhere

    .EXAMPLE
    pwsh -File eng\Test-CleanTigerSetup.ps1
#>
#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:cleanScript = Join-Path $PSScriptRoot 'Clean-TigerSetup.ps1'
$script:failures = [System.Collections.Generic.List[string]]::new()
$script:checks = 0
$script:scenario = ''

function Assert-True {
    param([Parameter(Mandatory)] [bool] $Condition, [Parameter(Mandatory)] [string] $Message)
    $script:checks++
    if (-not $Condition) {
        $script:failures.Add("$($script:scenario): $Message")
        Write-Host "    FAIL  $Message"
    }
}

function Write-SizedFile {
    param([Parameter(Mandatory)] [string] $Path, [Parameter(Mandatory)] [int] $Size)
    $directory = Split-Path -Parent $Path
    $null = New-Item -ItemType Directory -Path $directory -Force
    $bytes = [byte[]]::new($Size)
    [System.IO.File]::WriteAllBytes($Path, $bytes)
}

function New-SyntheticRepository {
    <#
        A repository-shaped directory: every policy target with known bytes,
        and the files the cleanup must leave alone. Returns the root and the
        byte count per routine target.
    #>
    param([switch] $Empty)

    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("TigerSetupCleanTest-" + [guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $root

    # A minimal Cargo package so that `cargo metadata` and `cargo clean` have
    # something real to answer for.
    Set-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Value "[package]`nname = `"synthetic`"`nversion = `"0.0.0`"`nedition = `"2021`"`n"
    Write-SizedFile -Path (Join-Path $root 'src\lib.rs') -Size 0
    Set-Content -LiteralPath (Join-Path $root 'AGENTS.md') -Value '# synthetic'

    $preserved = @(
        '.git\HEAD', '.gitignore', 'CLAUDE.md', 'README.md', 'TigerSetup-Design.md',
        '.cargo\config.toml', '.claude\agents\implementer.md', '.claude\worktrees\wt-1\notes.txt',
        'crates\tigersetup-engine\src\lib.rs', 'docs\assets\TigerSetup.ico', 'proto\metadata.proto',
        'lab\Invoke-MatrixRows.ps1', 'lab\README.md',
        'packages\test-app\New-TestAppPayload.ps1', 'packages\test-app\README.md',
        'packages\test-app\1.0.0\TigerSetup.toml', 'packages\test-app\1.1.0\TigerSetup.toml',
        'packages\test-app\1.2.0\TigerSetup.toml',
        # A top-level, tracked sibling of the per-version glob targets: the
        # glob enumerates every subdirectory of packages\test-app, including
        # this one, and must never reach into it.
        'packages\test-app\actions\build-cache.ps1', 'packages\test-app\actions\clear-cache.cmd',
        'packages\test-launch\TigerSetup.toml', 'packages\test-launch\Build-Package.ps1',
        'packages\TigerMarkView\TigerSetup.toml', 'packages\TigerMarkView\Build-Package.ps1',
        'packages\tigersetup\TigerSetup.toml', 'packages\tigersetup\Build-Package.ps1',
        'benchmark\README.md', 'benchmark\scripts\Build-Installers.ps1',
        # results\ is committed evidence, including two campaigns' raw lab
        # trees that predate the results\<campaign>\lab\ ignore split and
        # stay tracked despite matching that pattern (benchmark\README.md,
        # "What a campaign commits") - the cleanup policy must never reach
        # into results\ at all.
        'benchmark\results\apps.json', 'benchmark\results\lab\evidence.json',
        'benchmark\results\0.9.0\lab\evidence.json',
        'benchmark\compression-spike\README.md', 'benchmark\compression-spike\scripts\Acquire-Corpus.ps1',
        'benchmark\compression-spike\tool\Cargo.toml', 'benchmark\compression-spike\tool\src\main.rs'
    )
    foreach ($relative in $preserved) {
        # A comment line: valid wherever Cargo reads it (.cargo\config.toml).
        $path = Join-Path $root $relative
        $null = New-Item -ItemType Directory -Path (Split-Path -Parent $path) -Force
        Set-Content -LiteralPath $path -Value '# synthetic'
    }

    $routine = [ordered]@{
        'target' = @{ 'target\debug\synthetic.d' = 3000; 'target\x86_64-pc-windows-msvc\release\tiger-setup.exe' = 70000; 'target\tmp\fixture\a.bin' = 5000 }
        '.is_screenshots' = @{ '.is_screenshots\wizard.png' = 2500 }
        'lab/results' = @{ 'lab\results\run-1\summary.json' = 1200; 'lab\results\run-1\row.json' = 800; 'lab\results\run-2\artifact.zip' = 40000 }
        'packages/test-app/*/payload' = @{ 'packages\test-app\1.0.0\payload\bin\app.exe' = 20000; 'packages\test-app\1.0.0\payload\readme.txt' = 300; 'packages\test-app\1.1.0\payload\bin\app.exe' = 21000 }
        'packages/test-app/*/payload-extras' = @{ 'packages\test-app\1.0.0\payload-extras\icon.ico' = 1000; 'packages\test-app\1.1.0\payload-extras\icon.ico' = 1100 }
        'packages/test-app/*/payload-tools' = @{ 'packages\test-app\1.0.0\payload-tools\tool.exe' = 900; 'packages\test-app\1.1.0\payload-tools\tool.exe' = 950 }
        'packages/test-app/*/dependencies' = @{ 'packages\test-app\1.0.0\dependencies\dep.msi' = 1500; 'packages\test-app\1.1.0\dependencies\dep.msi' = 1600 }
        'packages/test-app/*/actions' = @{ 'packages\test-app\1.0.0\actions\build.ps1' = 400; 'packages\test-app\1.1.0\actions\build.ps1' = 420 }
        'packages/test-launch/stage' = @{ 'packages\test-launch\stage\tiger-setup.exe' = 60000; 'packages\test-launch\stage\tigersetup-setup.exe' = 55000 }
        'packages/TigerMarkView/source' = @{ 'packages\TigerMarkView\source\.git\HEAD' = 40; 'packages\TigerMarkView\source\src\Program.cs' = 900 }
        'packages/TigerMarkView/publish' = @{ 'packages\TigerMarkView\publish\TigerMarkView.dll' = 33000 }
        'packages/tigersetup/stage' = @{ 'packages\tigersetup\stage\tiger-setup.exe' = 60000; 'packages\tigersetup\stage\tigersetup-setup.exe' = 55000 }
        'benchmark/downloads' = @{ 'benchmark\downloads\App-1.0.zip' = 5000 }
        'benchmark/canonical' = @{ 'benchmark\canonical\App\file.bin' = 6000 }
        'benchmark/artifacts' = @{ 'benchmark\artifacts\App-Setup.exe' = 7000 }
        'benchmark/compression-spike/corpus' = @{ 'benchmark\compression-spike\corpus\downloads\app.zip' = 8000 }
        'benchmark/compression-spike/work' = @{ 'benchmark\compression-spike\work\plans\stage1.json' = 300 }
        'benchmark/compression-spike/tool/target' = @{ 'benchmark\compression-spike\tool\target\release\cspike.exe' = 9000 }
    }
    $retained = @{ 'artifacts\tigersetup\TigerSetup-0.0.0-Setup.exe' = 90000; 'artifacts\TigerMarkView\TigerMarkView-Setup.exe' = 45000 }

    $sizes = @{}
    if (-not $Empty) {
        foreach ($name in $routine.Keys) {
            $total = 0
            foreach ($relative in $routine[$name].Keys) {
                Write-SizedFile -Path (Join-Path $root $relative) -Size $routine[$name][$relative]
                $total += $routine[$name][$relative]
            }
            $sizes[$name] = $total
        }
        $total = 0
        foreach ($relative in $retained.Keys) {
            Write-SizedFile -Path (Join-Path $root $relative) -Size $retained[$relative]
            $total += $retained[$relative]
        }
        $sizes['artifacts'] = $total
    }

    [pscustomobject]@{
        root = $root
        preserved = @($preserved + @('Cargo.toml', 'src\lib.rs', 'AGENTS.md') | Sort-Object)
        sizes = $sizes
        routineBytes = [long] (($routine.Keys | ForEach-Object { if ($sizes.ContainsKey($_)) { $sizes[$_] } else { 0 } } | Measure-Object -Sum).Sum)
    }
}

function Get-FileSet {
    <# Every file below a root as a sorted relative path, not following links. #>
    param([Parameter(Mandatory)] [string] $Root)
    @(Get-ChildItem -LiteralPath $Root -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($Root.Length + 1) } | Sort-Object)
}

function Invoke-Clean {
    <#
        Runs the script in-process against a synthetic root and returns its
        result object, exit code and report lines.
    #>
    param([Parameter(Mandatory)] [string] $Root, [hashtable] $Arguments = @{})
    $output = @(& $script:cleanScript -RepositoryRoot $Root -PassThru @Arguments 6>&1)
    $exitCode = $LASTEXITCODE
    $result = $output | Where-Object { $_ -isnot [System.Management.Automation.InformationRecord] } | Select-Object -Last 1
    $lines = @($output | Where-Object { $_ -is [System.Management.Automation.InformationRecord] } | ForEach-Object { "$($_.MessageData)" })
    [pscustomobject]@{ result = $result; exitCode = $exitCode; lines = $lines }
}

function Get-TargetState {
    param([Parameter(Mandatory)] $Result, [Parameter(Mandatory)] [string] $Name)
    ($Result.targets | Where-Object { $_.name -eq $Name } | Select-Object -First 1).state
}

function Remove-SyntheticRepository {
    param([Parameter(Mandatory)] [string] $Root)
    if (Test-Path -LiteralPath $Root) { Remove-Item -LiteralPath $Root -Recurse -Force -ErrorAction SilentlyContinue }
}

function Invoke-Scenario {
    param([Parameter(Mandatory)] [string] $Name, [Parameter(Mandatory)] [scriptblock] $Body)
    $script:scenario = $Name
    Write-Host "  $Name"
    try { & $Body }
    catch {
        $script:failures.Add("${Name}: threw $($_.Exception.Message) at $($_.InvocationInfo.PositionMessage)")
        Write-Host "    FAIL  threw: $($_.Exception.Message)"
    }
}

Write-Host "Clean-TigerSetup tests"

Invoke-Scenario 'measure' {
    $repo = New-SyntheticRepository
    try {
        $before = Get-FileSet -Root $repo.root
        $run = Invoke-Clean -Root $repo.root -Arguments @{ Measure = $true }
        $after = Get-FileSet -Root $repo.root
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        Assert-True ($run.result.mode -eq 'measure') "mode is measure"
        Assert-True ((Compare-Object $before $after | Measure-Object).Count -eq 0) "no file was removed or added"
        Assert-True ($run.result.reclaimed -eq 0) "nothing is reported as reclaimed"
        $states = @($run.result.targets | ForEach-Object { $_.state } | Sort-Object -Unique)
        Assert-True (@($states | Where-Object { $_ -notin @('measured', 'absent') }).Count -eq 0) "every target is measured or absent ($($states -join ', '))"
        $routine = [long] (($run.result.targets | Where-Object { $_.category -eq 'routine' } | Measure-Object -Property bytesBefore -Sum).Sum)
        Assert-True ($routine -eq $repo.routineBytes) "routine bytes add up to what was written ($routine vs $($repo.routineBytes))"
        $artifacts = ($run.result.targets | Where-Object { $_.name -eq 'artifacts' }).bytesBefore
        Assert-True ($artifacts -eq $repo.sizes['artifacts']) "artifacts bytes are measured ($artifacts)"
        $payload = ($run.result.targets | Where-Object { $_.name -eq 'packages/test-app/*/payload' }).bytesBefore
        Assert-True ($payload -eq $repo.sizes['packages/test-app/*/payload']) "wildcard payload bytes aggregate across versions ($payload)"
        Assert-True ($run.result.registry.path -eq 'HKCU\Software\TigerSetupTests') "the registry namespace is reported"
        Assert-True (@($run.lines | Where-Object { $_ -match 'Routine disposable' }).Count -eq 1) "the report has a routine total"
        Assert-True (@($run.lines | Where-Object { $_ -match 'cargo:' }).Count -eq 0) "cargo clean was not run"
    }
    finally { Remove-SyntheticRepository -Root $repo.root }
}

Invoke-Scenario 'default' {
    $repo = New-SyntheticRepository
    try {
        $run = Invoke-Clean -Root $repo.root
        $remaining = Get-FileSet -Root $repo.root
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        Assert-True ($run.result.succeeded) "the result says it succeeded"
        foreach ($name in @(
                'target', '.is_screenshots', 'lab/results',
                'packages/test-app/*/payload', 'packages/test-app/*/payload-extras', 'packages/test-app/*/payload-tools',
                'packages/test-app/*/dependencies', 'packages/test-app/*/actions', 'packages/test-launch/stage',
                'packages/TigerMarkView/source', 'packages/TigerMarkView/publish', 'packages/tigersetup/stage',
                'benchmark/downloads', 'benchmark/canonical', 'benchmark/artifacts',
                'benchmark/compression-spike/corpus', 'benchmark/compression-spike/work', 'benchmark/compression-spike/tool/target'
            )) {
            Assert-True ((Get-TargetState $run.result $name) -eq 'removed') "$name is removed"
        }
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'benchmark\results\lab\evidence.json')) "committed benchmark\results\lab evidence survives"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'benchmark\results\0.9.0\lab\evidence.json')) "committed benchmark\results\0.9.0\lab evidence survives"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'packages\test-app\actions\build-cache.ps1')) "the top-level tracked packages\test-app\actions script survives"
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $repo.root 'target'))) "target\ is gone (cargo clean)"
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $repo.root 'lab\results'))) "lab\results is gone"
        Assert-True ((Get-TargetState $run.result 'artifacts') -eq 'kept') "artifacts is kept"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'artifacts\tigersetup\TigerSetup-0.0.0-Setup.exe')) "the release installer survives"
        Assert-True ((Get-TargetState $run.result '.claude/worktrees') -eq 'kept') ".claude/worktrees is kept"
        Assert-True ($run.result.reclaimed -eq $repo.routineBytes) "reclaimed equals the routine bytes ($($run.result.reclaimed) vs $($repo.routineBytes))"
        $expected = @($repo.preserved + @('artifacts\tigersetup\TigerSetup-0.0.0-Setup.exe', 'artifacts\TigerMarkView\TigerMarkView-Setup.exe') | Sort-Object)
        $difference = @(Compare-Object $expected $remaining)
        Assert-True ($difference.Count -eq 0) "exactly the preserved files remain ($(($difference | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join ', '))"
        Assert-True (@($run.lines | Where-Object { $_ -match '^Cleanup complete:' }).Count -eq 1) "the report ends with a completion line"
        Assert-True ($run.result.registry.state -eq ($(if ($run.result.registry.present) { 'present' } else { 'absent' }))) "the registry namespace is untouched without -TestState"
    }
    finally { Remove-SyntheticRepository -Root $repo.root }
}

Invoke-Scenario 'all' {
    $repo = New-SyntheticRepository
    try {
        $run = Invoke-Clean -Root $repo.root -Arguments @{ All = $true }
        $remaining = Get-FileSet -Root $repo.root
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        Assert-True ((Get-TargetState $run.result 'artifacts') -eq 'removed') "artifacts is removed"
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $repo.root 'artifacts'))) "artifacts\ is gone"
        Assert-True ((Get-TargetState $run.result '.claude/worktrees') -eq 'kept') ".claude/worktrees is still kept"
        $difference = @(Compare-Object $repo.preserved $remaining)
        Assert-True ($difference.Count -eq 0) "exactly the preserved files remain ($(($difference | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join ', '))"
        Assert-True ($run.result.reclaimed -eq ($repo.routineBytes + $repo.sizes['artifacts'])) "reclaimed includes the artifacts"
    }
    finally { Remove-SyntheticRepository -Root $repo.root }
}

Invoke-Scenario 'whatif' {
    $repo = New-SyntheticRepository
    $probe = "HKCU:\Software\TigerSetupTests\CleanProbe-$([guid]::NewGuid().ToString('N'))"
    try {
        $null = New-Item -Path $probe -Force
        $before = Get-FileSet -Root $repo.root
        $run = Invoke-Clean -Root $repo.root -Arguments @{ All = $true; TestState = $true; WhatIf = $true }
        $after = Get-FileSet -Root $repo.root
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        Assert-True ($run.result.mode -eq 'whatif') "mode is whatif"
        Assert-True ((Compare-Object $before $after | Measure-Object).Count -eq 0) "no file was removed or added"
        Assert-True ((Get-TargetState $run.result 'target') -eq 'whatif') "target is reported as what-if"
        Assert-True ((Get-TargetState $run.result 'artifacts') -eq 'whatif') "artifacts is reported as what-if"
        Assert-True ($run.result.registry.state -eq 'whatif') "the registry namespace is reported as what-if"
        Assert-True (Test-Path -LiteralPath $probe) "the registry probe survives"
        Assert-True ($run.result.reclaimed -eq 0) "nothing is reported as reclaimed"
    }
    finally {
        Remove-Item -LiteralPath $probe -Recurse -Force -ErrorAction SilentlyContinue
        Remove-SyntheticRepository -Root $repo.root
    }
}

Invoke-Scenario 'absent' {
    $repo = New-SyntheticRepository -Empty
    try {
        $run = Invoke-Clean -Root $repo.root -Arguments @{ All = $true }
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        $states = @($run.result.targets | Where-Object { $_.category -ne 'kept' } | ForEach-Object { $_.state } | Sort-Object -Unique)
        Assert-True (@($states | Where-Object { $_ -ne 'absent' }).Count -eq 0) "every routine and retained target is absent ($($states -join ', '))"
        Assert-True ($run.result.reclaimed -eq 0) "nothing is reclaimed"
        Assert-True (@($run.lines | Where-Object { $_ -match 'cargo:' }).Count -eq 0) "cargo clean is not run for an absent target directory"
    }
    finally { Remove-SyntheticRepository -Root $repo.root }
}

Invoke-Scenario 'payload' {
    $repo = New-SyntheticRepository
    try {
        $run = Invoke-Clean -Root $repo.root
        Assert-True ((Get-TargetState $run.result 'packages/test-app/*/payload') -eq 'removed') "the payload group is removed"
        foreach ($version in @('1.0.0', '1.1.0')) {
            Assert-True (-not (Test-Path -LiteralPath (Join-Path $repo.root "packages\test-app\$version\payload"))) "$version\payload is gone"
            Assert-True (Test-Path -LiteralPath (Join-Path $repo.root "packages\test-app\$version\TigerSetup.toml")) "$version\TigerSetup.toml stays"
        }
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'packages\test-app\1.2.0\TigerSetup.toml')) "a version without a payload is left alone"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'packages\test-app\New-TestAppPayload.ps1')) "the payload generator stays"
    }
    finally { Remove-SyntheticRepository -Root $repo.root }
}

Invoke-Scenario 'links' {
    $repo = New-SyntheticRepository
    $outside = Join-Path ([System.IO.Path]::GetTempPath()) ("TigerSetupCleanOutside-" + [guid]::NewGuid().ToString('N'))
    try {
        # Three placements: the target itself is a junction, a junction lives
        # inside a target, and a junction sits on the way to a target.
        Write-SizedFile -Path (Join-Path $outside 'results\keep.bin') -Size 4096
        Write-SizedFile -Path (Join-Path $outside 'inside\keep.bin') -Size 4096
        Write-SizedFile -Path (Join-Path $outside 'tigersetup\stage\keep.bin') -Size 4096
        Remove-Item -LiteralPath (Join-Path $repo.root 'lab\results') -Recurse -Force
        $null = New-Item -ItemType Junction -Path (Join-Path $repo.root 'lab\results') -Target (Join-Path $outside 'results')
        $null = New-Item -ItemType Junction -Path (Join-Path $repo.root 'packages\TigerMarkView\source\link') -Target (Join-Path $outside 'inside')
        Remove-Item -LiteralPath (Join-Path $repo.root 'packages\tigersetup') -Recurse -Force
        $null = New-Item -ItemType Junction -Path (Join-Path $repo.root 'packages\tigersetup') -Target (Join-Path $outside 'tigersetup')

        $measure = Invoke-Clean -Root $repo.root -Arguments @{ Measure = $true }
        Assert-True (((($measure.result.targets | Where-Object { $_.name -eq 'lab/results' }).bytesBefore)) -eq 0) "a junction target is not measured through"

        $run = Invoke-Clean -Root $repo.root
        Assert-True ($run.exitCode -eq 1) "exit code is 1 (was $($run.exitCode))"
        Assert-True (-not $run.result.succeeded) "the result says the cleanup was incomplete"
        foreach ($name in @('lab/results', 'packages/TigerMarkView/source', 'packages/tigersetup/stage')) {
            Assert-True ((Get-TargetState $run.result $name) -eq 'skipped') "$name is skipped"
        }
        Assert-True (Test-Path -LiteralPath (Join-Path $outside 'results\keep.bin')) "the junction target's content survives"
        Assert-True (Test-Path -LiteralPath (Join-Path $outside 'inside\keep.bin')) "the content behind the inner junction survives"
        Assert-True (Test-Path -LiteralPath (Join-Path $outside 'tigersetup\stage\keep.bin')) "the content behind the junction on the path survives"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'packages\TigerMarkView\source\src\Program.cs')) "a target containing a junction is left whole"
        Assert-True (Test-Path -LiteralPath (Join-Path $repo.root 'lab\results')) "the junction itself is left in place"
        Assert-True ((Get-TargetState $run.result 'target') -eq 'removed') "independent targets are still cleaned"
        Assert-True (@($run.lines | Where-Object { $_ -match 'junction or symbolic link' }).Count -ge 3) "each link is named in the report"
        Assert-True (@($run.lines | Where-Object { $_ -match '^Cleanup incomplete:' }).Count -eq 1) "the report says the cleanup was incomplete"
    }
    finally {
        # Remove the junctions before the trees so nothing is deleted through them.
        foreach ($link in @('lab\results', 'packages\TigerMarkView\source\link', 'packages\tigersetup')) {
            $path = Join-Path $repo.root $link
            if (Test-Path -LiteralPath $path) { [System.IO.Directory]::Delete($path) }
        }
        Remove-SyntheticRepository -Root $repo.root
        Remove-SyntheticRepository -Root $outside
    }
}

Invoke-Scenario 'registry' {
    $repo = New-SyntheticRepository -Empty
    $namespace = 'HKCU:\Software\TigerSetupTests'
    $suffix = [guid]::NewGuid().ToString('N')
    $probe = "$namespace\CleanProbe-$suffix"
    $neighbour = "HKCU:\Software\TigerSetupTests-Neighbour-$suffix"
    try {
        $null = New-Item -Path $probe -Force
        $null = New-Item -Path $neighbour -Force
        $null = New-ItemProperty -Path $neighbour -Name 'keep' -Value 'yes' -PropertyType String -Force

        $plain = Invoke-Clean -Root $repo.root
        Assert-True ((Test-Path -LiteralPath $probe)) "the namespace survives a run without -TestState"
        Assert-True ($plain.result.registry.state -eq 'present') "the namespace is reported present, not removed"

        $dry = Invoke-Clean -Root $repo.root -Arguments @{ TestState = $true; WhatIf = $true }
        Assert-True ((Test-Path -LiteralPath $probe)) "the namespace survives -TestState -WhatIf"
        Assert-True ($dry.result.registry.state -eq 'whatif') "the registry state is what-if"

        $run = Invoke-Clean -Root $repo.root -Arguments @{ TestState = $true }
        Assert-True ($run.exitCode -eq 0) "exit code is 0 (was $($run.exitCode))"
        Assert-True ($run.result.registry.state -eq 'removed') "the registry state is removed"
        Assert-True (-not (Test-Path -LiteralPath $namespace)) "the namespace is gone"
        Assert-True ((Test-Path -LiteralPath $neighbour)) "the neighbouring key survives"
        Assert-True ((Get-ItemPropertyValue -LiteralPath $neighbour -Name 'keep') -eq 'yes') "the neighbouring key's value survives"

        $again = Invoke-Clean -Root $repo.root -Arguments @{ TestState = $true }
        Assert-True ($again.exitCode -eq 0) "an absent namespace is tolerated (exit $($again.exitCode))"
        Assert-True ($again.result.registry.state -eq 'absent') "the registry state is absent"
    }
    finally {
        Remove-Item -LiteralPath $neighbour -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $probe -Recurse -Force -ErrorAction SilentlyContinue
        Remove-SyntheticRepository -Root $repo.root
    }
}

Invoke-Scenario 'failure' {
    $repo = New-SyntheticRepository
    $lockedResult = Join-Path $repo.root 'lab\results\run-1\locked.bin'
    $lockedTarget = Join-Path $repo.root 'target\debug\locked.bin'
    Write-SizedFile -Path $lockedResult -Size 2048
    Write-SizedFile -Path $lockedTarget -Size 2048
    $handles = @()
    try {
        foreach ($path in @($lockedResult, $lockedTarget)) {
            $handles += [System.IO.File]::Open($path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::None)
        }
        $expectedBefore = $repo.routineBytes + 4096

        $run = Invoke-Clean -Root $repo.root
        Assert-True ($run.exitCode -eq 1) "exit code is 1 (was $($run.exitCode))"
        Assert-True (-not $run.result.succeeded) "the result says the cleanup was incomplete"
        Assert-True ((Get-TargetState $run.result 'lab/results') -in @('partial', 'failed')) "lab/results is partial or failed"
        Assert-True ((Get-TargetState $run.result 'target') -in @('partial', 'failed')) "target is partial or failed through cargo clean"
        $cargoTarget = $run.result.targets | Where-Object { $_.name -eq 'target' }
        Assert-True ($cargoTarget.message -match 'cargo clean exited') "the cargo failure is named ($($cargoTarget.message))"
        Assert-True (Test-Path -LiteralPath $lockedResult) "the locked file remains"
        Assert-True (Test-Path -LiteralPath $lockedTarget) "the locked target file remains"
        $remaining = [long] (($run.result.targets | Where-Object { $_.category -eq 'routine' } | Measure-Object -Property bytesAfter -Sum).Sum)
        Assert-True ($remaining -ge 4096) "the locked bytes are reported as remaining ($remaining)"
        Assert-True ($run.result.reclaimed -eq ($expectedBefore - $remaining)) "reclaimed is before minus remaining ($($run.result.reclaimed) vs $($expectedBefore - $remaining))"
        Assert-True (@($run.lines | Where-Object { $_ -match '^Cleanup incomplete:' }).Count -eq 1) "the report says the cleanup was incomplete"
        Assert-True ((Get-TargetState $run.result 'packages/tigersetup/stage') -eq 'removed') "independent targets are still cleaned"

        # The same failure through the process boundary, the way a developer
        # sees it.
        $null = & pwsh -NoProfile -File $script:cleanScript -RepositoryRoot $repo.root 2>&1
        Assert-True ($LASTEXITCODE -eq 1) "the process exit code is 1 (was $LASTEXITCODE)"
        Assert-True (Test-Path -LiteralPath $lockedResult) "the locked file still remains"
    }
    finally {
        foreach ($handle in $handles) { $handle.Dispose() }
        Remove-SyntheticRepository -Root $repo.root
    }
}

Write-Host ''
if ($script:failures.Count -eq 0) {
    Write-Host "Clean-TigerSetup tests: $($script:checks) check(s), all passed."
    exit 0
}
foreach ($failure in $script:failures) { Write-Host "FAIL: $failure" }
Write-Host "Clean-TigerSetup tests: $($script:failures.Count) of $($script:checks) check(s) failed."
exit 1
