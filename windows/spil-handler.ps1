# Spilarr: håndterer spil://-links fra Spilarr-webappen.
#   spil://mount?p=<sti>    mount ISO fra sharen og åbn drevet
#   spil://unmount?p=<sti>  afmonter ISO
#   spil://run?p=<sti>      kør installer (.exe) fra sharen
#   spil://open?p=<sti>     åbn mappen i Stifinder
# <sti> er relativ til spil-sharen; alt uden for den afvises.
param([string]$Url)
$ErrorActionPreference = 'Stop'
$Root = '__UNC_ROOT__'
Add-Type -AssemblyName System.Windows.Forms

function Besked($tekst, $ikon = 'Information') {
    [System.Windows.Forms.MessageBox]::Show($tekst, 'Spilarr', 'OK', $ikon) | Out-Null
}

try {
    $u = [uri]$Url
    $action = $u.Host.ToLower()
    $p = ($u.Query.TrimStart('?') -split '&' | Where-Object { $_ -like 'p=*' } | Select-Object -First 1)
    if (-not $p) { throw "Mangler sti i $Url" }
    $rel = [uri]::UnescapeDataString($p.Substring(2)) -replace '/', '\'
    if ($rel -match '(^|\\)\.\.(\\|$)' -or $rel -match '^\\' -or $rel -match ':') { throw "Ugyldig sti: $rel" }
    $path = Join-Path $Root $rel

    if (-not (Test-Path -LiteralPath $path)) {
        Start-Process explorer.exe (Split-Path $Root)
        throw "Kan ikke nå:`n$path`n`nLog ind på sharen i Stifinder (gem adgangskoden) og prøv igen."
    }

    switch ($action) {
        'mount' {
            if ($path -notmatch '\.iso$') { throw 'Kun .iso-filer kan mountes.' }
            $img = Get-DiskImage -ImagePath $path
            if (-not $img.Attached) { $img = Mount-DiskImage -ImagePath $path -PassThru }
            $vol = $null
            foreach ($i in 1..20) { $vol = $img | Get-Volume -ErrorAction SilentlyContinue; if ($vol.DriveLetter) { break }; Start-Sleep -Milliseconds 300 }
            if (-not $vol.DriveLetter) { throw 'ISO er mountet, men fik intet drevbogstav.' }
            Start-Process explorer.exe "$($vol.DriveLetter):\"
        }
        'unmount' {
            Dismount-DiskImage -ImagePath $path | Out-Null
            Besked "Afmonteret:`n$(Split-Path $rel -Leaf)"
        }
        'run' {
            if ($path -notmatch '\.exe$') { throw 'Kun .exe kan køres.' }
            Start-Process -FilePath $path -WorkingDirectory (Split-Path $path)
        }
        'open' { Start-Process explorer.exe $path }
        default { throw "Ukendt handling: $action" }
    }
} catch {
    Besked $_.Exception.Message 'Error'
    exit 1
}
