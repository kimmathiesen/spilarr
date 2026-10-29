# Installerer Spilarr-hjælperen for den aktuelle bruger (ingen administrator):
#   irm __SERVER__/windows/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'Spilarr'
New-Item -ItemType Directory -Force $dir | Out-Null
Invoke-WebRequest '__SERVER__/windows/spil-handler.ps1' -UseBasicParsing -OutFile (Join-Path $dir 'spil-handler.ps1')

$k = 'HKCU:\Software\Classes\spil'
New-Item -Force $k | Out-Null
Set-Item $k 'URL:Spilarr'
New-ItemProperty $k 'URL Protocol' -Value '' -Force | Out-Null
New-Item -Force "$k\shell\open\command" | Out-Null
# conhost --headless: ingen konsolvindue blinker
$cmd = "conhost.exe --headless powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$dir\spil-handler.ps1`" `"%1`""
Set-Item "$k\shell\open\command" $cmd
Write-Host "Spilarr-hjælperen er installeret i $dir"
