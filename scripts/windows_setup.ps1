<#
.SYNOPSIS
    RJ45 Sound Card - Windows Setup Script
.DESCRIPTION
    Installs and configures a virtual audio driver (VB-Cable or alternative)
    so the remote PC sees the shared sound card as a local audio device.
    
    VB-Cable is a third-party virtual audio driver that creates a
    loopback audio device. Free version supports 1 stereo channel,
    paid version supports up to 256 channels.
.NOTES
    Requires Administrator privileges.
    Version: 1.0
#>

param(
    [Parameter(Position = 0)]
    [ValidateSet('install', 'remove', 'status', 'help')]
    [string]$Command = 'help'
)

$ErrorActionPreference = 'Stop'
$ScriptName = Split-Path -Leaf $PSCommandPath
$VBCableUrl = "https://vb-audio.com/Cable/VBCABLE_Driver_Pack43.zip"
$VBCableZip = "$env:TEMP\VBCABLE_Driver_Pack43.zip"
$VBCableDir = "$env:TEMP\VBCABLE"
$VirtualDeviceName = "CABLE Input (VB-Audio Virtual Cable)"

function Show-Help {
    Write-Host @"
Usage: powershell -ExecutionPolicy Bypass -File $ScriptName [command]

Commands:
  install   Install VB-Cable virtual audio driver
  remove    Remove VB-Cable virtual audio driver
  status    Show status of virtual audio devices
  help      Show this help message

Prerequisites:
  - Windows 10/11 (64-bit)
  - Administrator privileges
  - Internet connection (for driver download)

VB-Cable creates a virtual audio device that appears as
"CABLE Input" and "CABLE Output" in Windows sound settings.

After installation, configure rjsc client:
  rjsc connect --virtual-device "$VirtualDeviceName"

Note: The free VB-Cable supports 1 stereo pair (2 channels).
      For multi-channel (MOTU 24I/O, etc.), purchase VB-Cable
      from https://vb-audio.com/Cable/ or use the VB-Cable A+B
      version for up to 8 channels.
"@
}

function Test-Admin {
    $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($currentUser)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Install-Driver {
    if (-not (Test-Admin)) {
        Write-Error "Administrator privileges required. Run as Administrator."
        return
    }

    # Check existing installation
    $existing = Get-PnpDevice | Where-Object { $_.FriendlyName -like "*VB-Audio*" }
    if ($existing) {
        Write-Host "VB-Cable appears to be already installed:"
        $existing | Format-Table Status, FriendlyName -AutoSize
        return
    }

    Write-Host "==> Downloading VB-Cable driver..."
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $VBCableUrl -OutFile $VBCableZip -UseBasicParsing
        Write-Host "    Downloaded to $VBCableZip"
    }
    catch {
        Write-Error "Failed to download VB-Cable: $_"
        Write-Host ""
        Write-Host "Manual installation:"
        Write-Host "  1. Visit https://vb-audio.com/Cable/"
        Write-Host "  2. Download and run the installer"
        Write-Host "  3. Reboot your computer"
        return
    }

    Write-Host "==> Extracting driver..."
    Expand-Archive -Path $VBCableZip -DestinationPath $VBCableDir -Force
    Write-Host "    Extracted to $VBCableDir"

    # Determine architecture
    $arch = if ([Environment]::Is64BitOperatingSystem) { "x64" } else { "x86" }
    $driverInf = Get-ChildItem -Path $VBCableDir -Recurse -Filter "*.inf" | 
        Where-Object { $_.Name -like "*$arch*" } | 
        Select-Object -First 1

    if (-not $driverInf) {
        Write-Error "No driver INF found for architecture: $arch"
        return
    }

    Write-Host "==> Installing driver: $($driverInf.Name)..."
    try {
        $null = & "pnputil" "/add-driver" $driverInf.FullName "/install"
        Write-Host "    Driver installation initiated."
    }
    catch {
        Write-Error "Driver installation failed: $_"
        Write-Host ""
        Write-Host "Try manual installation:"
        Write-Host "  1. Open Device Manager"
        Write-Host "  2. Right-click -> Add legacy hardware"
        Write-Host "  3. Install from list -> Have disk -> Browse to $($driverInf.Directory)"
        return
    }

    Write-Host ""
    Write-Host "==> Installation complete!"
    Write-Host ""
    Write-Host "A reboot is recommended. After reboot:"
    Write-Host "  1. Open Sound Settings (right-click speaker icon)"
    Write-Host "  2. Verify 'CABLE Input' appears in Output devices"
    Write-Host "  3. Run: rjsc list (to see available audio devices)"
    Write-Host "  4. Run: rjsc connect --virtual-device `"$VirtualDeviceName`""
}

function Remove-Driver {
    if (-not (Test-Admin)) {
        Write-Error "Administrator privileges required. Run as Administrator."
        return
    }

    Write-Host "==> Finding VB-Cable devices..."
    $devices = Get-PnpDevice | Where-Object { $_.FriendlyName -like "*VB-Audio*" }
    if (-not $devices) {
        Write-Host "    No VB-Cable devices found."
        return
    }

    foreach ($device in $devices) {
        Write-Host "    Removing: $($device.FriendlyName)..."
        $null = & "pnputil" "/remove-device" $device.InstanceId
    }

    Write-Host "==> Cleaning up driver packages..."
    $packages = Get-WindowsDriver -Online | Where-Object { $_.PackageName -like "*VB-Audio*" }
    foreach ($pkg in $packages) {
        Write-Host "    Removing driver package..."
        $null = & "pnputil" "/delete-driver" $pkg.PackageName
    }

    Write-Host "==> VB-Cable removal complete."
    Write-Host "    Reboot recommended."
}

function Show-Status {
    Write-Host "=== Virtual Audio Device Status ==="
    Write-Host ""

    $devices = Get-PnpDevice | Where-Object { $_.FriendlyName -like "*CABLE*" -or $_.FriendlyName -like "*VB-Audio*" }
    if ($devices) {
        Write-Host "VB-Cable devices:"
        $devices | Format-Table Status, Class, FriendlyName -AutoSize
    }
    else {
        Write-Host "VB-Cable: NOT INSTALLED"
    }

    Write-Host ""

    Write-Host "All audio endpoints:"
    Add-Type -AssemblyName System.Core
    $audioDevices = Get-ChildItem "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render" -ErrorAction SilentlyContinue
    $audioDevices += Get-ChildItem "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture" -ErrorAction SilentlyContinue
    if ($audioDevices) {
        foreach ($dev in $audioDevices) {
            $name = (Get-ItemProperty -Path "$($dev.PSPath)\Properties" -Name "{b3f8fa53-0004-438e-9003-51a46e139bfc},6" -ErrorAction SilentlyContinue)."(b3f8fa53-0004-438e-9003-51a46e139bfc),6"
            if ($name) {
                Write-Host "  $name"
            }
        }
    }

    Write-Host ""
    Write-Host "For client usage:"
    Write-Host "  rjsc connect --virtual-device `"$VirtualDeviceName`""
}

switch ($Command) {
    "install" { Install-Driver }
    "remove"  { Remove-Driver }
    "status"  { Show-Status }
    default   { Show-Help }
}
