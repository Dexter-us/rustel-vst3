# Installs the bundle for the current user without administrator access.
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$Bundle = (Join-Path $PSScriptRoot "Rustel.vst3"),
    [string]$Destination = (Join-Path $env:LOCALAPPDATA "Programs\Common\VST3")
)

$ErrorActionPreference = "Stop"
if (!(Test-Path -LiteralPath (Join-Path $Bundle "Contents\x86_64-win\Rustel.vst3"))) {
    throw "No Windows x64 Rustel.vst3 bundle was found at '$Bundle'."
}
$target = Join-Path $Destination "Rustel.vst3"
if (Test-Path -LiteralPath $target) {
    throw "Rustel is already installed at '$target'. Close your DAW and move the existing bundle before installing."
}
if ($PSCmdlet.ShouldProcess($target, "Install Rustel VST3 for the current Windows user")) {
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -LiteralPath $Bundle -Destination $target -Recurse
    Write-Output "Installed at $target. Rescan VST3 plugins in your DAW."
}
