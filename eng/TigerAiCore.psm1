<#
    .SYNOPSIS
    Resolves a registered Tiger Lab or tool through TigerAiCore.

    .DESCRIPTION
    Repository layout is not topology. A Lab or tool TigerSetup's scripts use —
    TigerWinLab for the lab rows, tiger-mark for the installed help's PDF, the
    TigerMarkView checkout for its package — is resolved by TigerAiCore's own
    `tools/Resolve-TigerAiCoreResource.ps1`, which reads the machine
    configuration named by TigerAiCoreConfig. This module is the one place
    TigerSetup reaches that resolver: no sibling-directory guess, no filesystem
    scan, no TigerSetup-specific environment variable. A caller that accepts an
    explicit path treats it as an override for one run, not as a second
    discovery system.
#>

Set-StrictMode -Version Latest

function Get-TigerAiCoreRoot {
    <#
        .SYNOPSIS
        The TigerAiCore repository named by the machine configuration.

        .DESCRIPTION
        The configuration file named by TigerAiCoreConfig declares `core` in its
        managed block. That value is the only supported way to reach TigerAiCore's
        own tools, including the resource resolver. Nothing is discovered.
    #>
    [CmdletBinding()]
    param()

    $configPath = $env:TigerAiCoreConfig
    if ([string]::IsNullOrWhiteSpace($configPath) -or -not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
        throw 'TigerAiCoreConfig is not set, or does not name a readable file, so registered Labs and tools are unavailable.'
    }
    foreach ($line in Get-Content -LiteralPath $configPath) {
        if ($line.Trim() -match '^core\s*=\s*"(?<path>[^"]+)"\s*(?:#.*)?$') {
            # A TOML basic string escapes its backslashes; the path does not.
            return $Matches['path'].Replace('\\', '\')
        }
    }
    throw "The TigerAiCore configuration $configPath declares no 'core' path, so its resource resolver cannot be reached."
}

function Resolve-TigerAiCoreRegistration {
    <#
        .SYNOPSIS
        The registration of one Lab or tool: Name, Kind, Type, Path, PathExists
        and ConfigPath, as the resolver reports them.

        .DESCRIPTION
        Resolver exit codes: 0 resolved, 1 unavailable on this machine, 2 a
        broken configuration or an invalid registration. Anything but 0 is an
        exception naming what was asked for; a registration whose path does not
        exist is one too.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [ValidateSet('Lab', 'Tool')] [string] $Kind,
        [Parameter(Mandatory)] [string] $Name
    )

    $resolver = Join-Path (Get-TigerAiCoreRoot) 'tools/Resolve-TigerAiCoreResource.ps1'
    if (-not (Test-Path -LiteralPath $resolver -PathType Leaf)) {
        throw "The TigerAiCore resource resolver was not found at $resolver."
    }
    $stdout = & (Get-Process -Id $PID).Path -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File $resolver "-$Kind" $Name -Json 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($stdout | Out-String)
    switch ($exitCode) {
        0 { }
        1 { throw "The $($Kind.ToLowerInvariant()) $Name is not registered on this machine: $($text.Trim())" }
        default { throw "Resolving the $($Kind.ToLowerInvariant()) $Name failed (exit $exitCode): $($text.Trim())" }
    }
    $registration = $text | ConvertFrom-Json
    if (-not $registration.PathExists) {
        throw "The registered $($Kind.ToLowerInvariant()) $Name names '$($registration.Path)', which does not exist (from $($registration.ConfigPath))."
    }
    $registration
}

Export-ModuleMember -Function Get-TigerAiCoreRoot, Resolve-TigerAiCoreRegistration
