# Installs Rustel and its supported Csound runtime for the current user.
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$Bundle = (Join-Path $PSScriptRoot "Rustel.vst3"),
    [string]$CsoundRuntime = (Join-Path $PSScriptRoot "CsoundRuntime"),
    [string]$Destination = (Join-Path $env:LOCALAPPDATA "Programs\Common\VST3"),
    [string]$CsoundDestination = (Join-Path $env:LOCALAPPDATA "Programs\Rustel\Csound-6.18.1")
)

$ErrorActionPreference = "Stop"
if (!(Test-Path -LiteralPath (Join-Path $Bundle "Contents\x86_64-win\Rustel.vst3"))) {
    throw "No Windows x64 Rustel.vst3 bundle was found at '$Bundle'."
}
foreach ($required in @("csound64.dll", "stdutil.dll")) {
    if (!(Test-Path -LiteralPath (Join-Path $CsoundRuntime $required))) {
        throw "The bundled Csound runtime is missing '$required' at '$CsoundRuntime'."
    }
}
foreach ($required in @("VCRUNTIME140.dll", "MSVCP140.dll", "VCRUNTIME140_1.dll")) {
    if (!(Test-Path -LiteralPath (Join-Path $env:WINDIR "System32\$required"))) {
        throw "The Microsoft Visual C++ x64 runtime is missing '$required'. Install the Microsoft Visual C++ 2015–2022 x64 Redistributable, then run this installer again: https://aka.ms/vs/17/release/vc_redist.x64.exe"
    }
}

function Install-FolderWithBackup {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Target
    )

    $parent = Split-Path -Parent $Target
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $backup = $null
    if (Test-Path -LiteralPath $Target) {
        $backup = "$Target.backup-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
        Move-Item -LiteralPath $Target -Destination $backup
    }
    try {
        New-Item -ItemType Directory -Force -Path $Target | Out-Null
        Copy-Item -Path (Join-Path $Source "*") -Destination $Target -Recurse -Force
    } catch {
        if (Test-Path -LiteralPath $Target) {
            Remove-Item -LiteralPath $Target -Recurse -Force
        }
        if ($backup -and (Test-Path -LiteralPath $backup)) {
            Move-Item -LiteralPath $backup -Destination $Target
        }
        throw
    }
    if ($backup) {
        Write-Output "Previous files were preserved at $backup"
    }
}

$vstTarget = Join-Path $Destination "Rustel.vst3"
if ($PSCmdlet.ShouldProcess($CsoundDestination, "Install bundled Csound 6.18.1")) {
    Install-FolderWithBackup -Source $CsoundRuntime -Target $CsoundDestination
}
if ($PSCmdlet.ShouldProcess($vstTarget, "Install Rustel VST3 for the current Windows user")) {
    Install-FolderWithBackup -Source $Bundle -Target $vstTarget
}

if ($PSCmdlet.ShouldProcess("Current-user environment", "Configure Rustel to use bundled Csound")) {
    $csoundDll = Join-Path $CsoundDestination "csound64.dll"
    [Environment]::SetEnvironmentVariable("RUSTEL_CSOUND_LIBRARY", $csoundDll, "User")
    [Environment]::SetEnvironmentVariable("OPCODE6DIR64", $CsoundDestination, "User")

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathEntries = @($userPath -split ";" | Where-Object { $_ })
    if (!($pathEntries | Where-Object { $_.TrimEnd("\") -ieq $CsoundDestination.TrimEnd("\") })) {
        $pathEntries += $CsoundDestination
        [Environment]::SetEnvironmentVariable("Path", ($pathEntries -join ";"), "User")
    }

    if (!("RustelEnvironment.NativeMethods" -as [type])) {
        Add-Type -Namespace RustelEnvironment -Name NativeMethods -MemberDefinition @"
            [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
            public static extern System.IntPtr SendMessageTimeout(
                System.IntPtr hWnd, int Msg, System.IntPtr wParam, string lParam,
                int fuFlags, int uTimeout, out System.IntPtr lpdwResult);
"@
    }
    $broadcastResult = [IntPtr]::Zero
    [void][RustelEnvironment.NativeMethods]::SendMessageTimeout(
        [IntPtr]0xffff, 0x001A, [IntPtr]::Zero, "Environment",
        0x0002, 5000, [ref]$broadcastResult)
}

if ($WhatIfPreference) {
    Write-Output "Validation passed; -WhatIf made no changes."
} else {
    Write-Output "Installed Rustel and Csound 6.18.1 for the current user."
    Write-Output "Restart your DAW, rescan VST3 plugins, then enable 'Allow native Csound' in Rustel."
}
