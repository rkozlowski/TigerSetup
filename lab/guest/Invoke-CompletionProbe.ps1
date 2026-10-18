<#
    .SYNOPSIS
    Reads the PowerShell profiles of the account it runs as, and asks a real
    PowerShell session for tab completion, inside a TigerWinLab guest.

    .DESCRIPTION
    A guest-side helper of lab\Invoke-CompletionRows.ps1, staged beside the
    installer and run by guest\Invoke-SetupCommands.ps1 as an ordinary command
    in the session the row names. It prints one JSON document, which the
    command runner embeds; nothing here judges it.

      -Mode Profiles   the account, its Documents folder as Windows resolves it,
                       and each edition's CurrentUserAllHosts profile
                       (Documents\WindowsPowerShell\profile.ps1 and
                       Documents\PowerShell\profile.ps1): whether its directory
                       and file exist, and the file's exact bytes, base64
      -Mode Seed       writes -SeedBase64 as the Windows PowerShell profile
                       first — what the user had before anything was
                       installed — then reports as Profiles does
      -Mode Complete   starts -Shell (powershell.exe or pwsh.exe) the way a
                       person opens a session, profiles included, with
                       -ExecutionPolicy -Policy for that process only when one
                       is named, and asks it for the completions of -Line at
                       its end (TabExpansion2), with PATH as a new session
                       gets it; reports the matches, the exit code and stderr

    The child gets the execution policy the row names, or none: the
    -ExecutionPolicy this helper itself was started with travels in an
    environment variable (PSExecutionPolicyPreference), which is removed
    before the child starts so that a "default policy" probe is one.

    Windows PowerShell 5.1: no ProcessStartInfo.ArgumentList, so the child's
    command is passed as -EncodedCommand and the line needs no quoting.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [ValidateSet('Profiles', 'Seed', 'Complete')] [string] $Mode,
    [string] $Shell,
    [string] $Policy,
    [string] $Line = 'TigerSetupTestAction al',
    [string] $SeedBase64
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-Profiles {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $documents = [Environment]::GetFolderPath('MyDocuments')
    $profiles = @(foreach ($edition in @('WindowsPowerShell', 'PowerShell')) {
            $directory = Join-Path $documents $edition
            $file = Join-Path $directory 'profile.ps1'
            $bytes = $null
            if (Test-Path -LiteralPath $file -PathType Leaf) { $bytes = [IO.File]::ReadAllBytes($file) }
            $base64 = $null
            if ($null -ne $bytes) { $base64 = [Convert]::ToBase64String($bytes) }
            [ordered]@{
                edition = $edition
                path = $file
                directoryExists = (Test-Path -LiteralPath $directory -PathType Container)
                exists = ($null -ne $bytes)
                base64 = $base64
            }
        })
    [ordered]@{ user = $identity.Name; sid = $identity.User.Value; documents = $documents; profiles = $profiles }
}

switch ($Mode) {
    'Seed' {
        $directory = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'WindowsPowerShell'
        $null = New-Item -ItemType Directory -Path $directory -Force
        [IO.File]::WriteAllBytes((Join-Path $directory 'profile.ps1'), [Convert]::FromBase64String($SeedBase64))
        Get-Profiles | ConvertTo-Json -Depth 5 -Compress
    }
    'Profiles' {
        Get-Profiles | ConvertTo-Json -Depth 5 -Compress
    }
    'Complete' {
        Remove-Item -LiteralPath 'Env:PSExecutionPolicyPreference' -ErrorAction SilentlyContinue
        $escaped = $Line.Replace("'", "''")
        $command = "`$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User'); " +
            "(TabExpansion2 -inputScript '$escaped' -cursorColumn $($Line.Length)).CompletionMatches | ForEach-Object { `$_.CompletionText }"
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
        $arguments = '-NoLogo -NonInteractive'
        if (-not [string]::IsNullOrWhiteSpace($Policy)) { $arguments += " -ExecutionPolicy $Policy" }
        $arguments += " -EncodedCommand $encoded"
        $start = New-Object System.Diagnostics.ProcessStartInfo
        $start.FileName = $Shell
        $start.Arguments = $arguments
        $start.UseShellExecute = $false
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.CreateNoWindow = $true
        $process = [System.Diagnostics.Process]::Start($start)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()
        $found = @($stdout.Result -split "`r?`n" | Where-Object { $_ -ne '' })
        [ordered]@{
            shell = $Shell
            policy = $Policy
            exitCode = $process.ExitCode
            matches = $found
            stderr = $stderr
        } | ConvertTo-Json -Depth 4 -Compress
    }
}
