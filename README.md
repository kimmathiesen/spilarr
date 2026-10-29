# Spilarr

Et Radarr-agtigt bibliotek over spil, der ligger på en share (fx Unraid). Det er bygget i Rust (axum + askama + SQLite) og leveres som et lille Docker-image på ca. 30 MB.

- **Plakatvisning** af alle spilmapper med søgning, filter og sortering.
- **Infoside** med beskrivelse, genrer, screenshots og Metacritic. Metadata hentes fra Steams åbne API, så der skal ikke bruges en nøgle. Beskrivelserne er på dansk, hvor Steam har dem.
- **Installer-knap**:
  - ISO'er mountes direkte fra sharen på Windows-PC'en og kopieres ikke først.
  - GOG-installere køres direkte fra sharen.
  - Alle spil har en **Åbn mappe**-knap.
- **Ret match**, hvis den automatiske matchning rammer forkert. Man kan angive et Steam-appid, en søgetekst eller `ingen`.
- Automatisk scanning (standard hver 6. time) og knappen **Scan nu**.
- **Import fra qBittorrent** (siden **Import**): færdige torrents i en kategori (standard `games`) importeres til spilmappen.
  - Almindelige filer **hardlinkes**, så de ikke fylder dobbelt.
  - Rar-arkiver (også flerdelte) **pakkes ud**, så ISO'en kan mountes.
  - `sample/` og `.sfv` springes over.
  - Mappen navngives `Titel (År) - Gruppe` ud fra scene-navnet og årstallet fra Steam.
  - qB må gerne slette sin kopi efter seeding (fx *Remove torrent and its files*): biblioteket bliver liggende.

Søgning og download indgår ikke, og Prowlarr bruges ikke. Spilarr viser kun det, der allerede ligger på sharen.

## Mappestruktur

Én mappe pr. spil, navngivet `Spilnavn (År) - Gruppe`:

```
games/
  Cyberpunk 2077 (2020) - GOG/          setup_cyberpunk_2077_*.exe  → "Installer" kører setup
  Silent Hill 2 Remake (2024) - RUNE/   *.iso                       → "Installer" mounter ISO
    updates/                            opdateringer, patches, crackfixes
  portable/                             mapper herinde er hver ét spil (installerede kopier)
```

Mappetypen sættes efter indholdet:

| Type | Indhold |
|---|---|
| ISO | Mappen indeholder en ISO. |
| RAR | Der er kun rar-arkiver (udpakning er planlagt). |
| Installer | Mappen har en `setup*.exe` i roden eller ét niveau nede. |
| Portable | Mappen ligger i `portable/`. |

`updates/` og `sample/` ignoreres, når typen bestemmes.

## Installation på Unraid

1. Kopiér [unraid/spilarr.xml](unraid/spilarr.xml) til `/boot/config/plugins/dockerMan/templates-user/my-spilarr.xml`.
2. Gå til Docker → Add Container og vælg template `spilarr`.
3. Tjek stierne, og vælg Apply.

Opdateringer kommer via Unraids **Check for updates**. Hvert push til `main` bygger `ghcr.io/kimmathiesen/spilarr:latest` via GitHub Actions.

| Variabel | Standard | Betydning |
|---|---|---|
| `/data` | `/mnt/user/data` | Hele data-sharen med samme sti som i qBittorrent, så hardlinks virker. |
| `GAMES_DIR` | `/games` (template: `/data/games`) | Spilmappen inde i containeren. |
| `QBIT_URL` | tom | qB Web-UI, fx `http://192.168.68.58:8080`. Tom slår import fra. |
| `QBIT_CATEGORY` | `games` | Kategorien, der importeres. Den skal gemme uden for spilmappen, fx `/data/torrents/games`. |
| `IMPORT_MINUTES` | `5` | Antal minutter mellem tjek af qB. |
| `/config` | `/mnt/user/appdata/spilarr` | `spilarr.db` og `covers/`. |
| `UNC_ROOT` | `\\192.168.68.58\data\games` | Den samme mappe set fra Windows. |
| `SCAN_HOURS` | `6` | Antal timer mellem scanninger. |
| `PORT` | `8790` | Web-UI'ets port. |

## Windows-hjælperen (Installer-knappen)

Knapperne er `spil://`-links. Hjælperen installeres én gang pr. PC, uden administrator:

```powershell
irm http://<tower>:8790/windows/install.ps1 | iex
```

Scriptet lægger [windows/spil-handler.ps1](windows/spil-handler.ps1) i `%LOCALAPPDATA%\Spilarr` og registrerer `spil://` under `HKCU\Software\Classes\spil`.

Handleren kan:

| Link | Handling |
|---|---|
| `spil://mount` | Kører `Mount-DiskImage` på ISO'en og åbner drevet i Stifinder. |
| `spil://unmount` | Afmonterer ISO'en. |
| `spil://run` | Kører en `.exe`. |
| `spil://open` | Åbner mappen. |

Den afviser stier uden for `UNC_ROOT`. Windows skal kunne nå sharen, så åbn den én gang i Stifinder og gem login.

## Linux-hjælperen (CachyOS, steam-headless m.fl.)

```sh
curl -fsSL http://<tower>:8790/linux/install.sh | sh
```

Scriptet installerer [linux/spil-handler.sh](linux/spil-handler.sh) i `~/.local/share/spilarr` og registrerer `x-scheme-handler/spil` med `xdg-mime`. Det kræver ikke root.

- **Spilmappen** findes automatisk:
  - `/mnt/torrents` i steam-headless.
  - `/mnt/spil`.
  - Et `gio mount smb://<tower>/data`.
  - Du kan også angive den som argument: `… | sh -s -- /sti`.

  Stien gemmes i `~/.config/spilarr.conf` som `SPIL_ROOT`.
- **ISO** mountes med `udisksctl`. Uden loop-enheder, fx i containere, bruges `fuseiso` eller `archivemount`.
- **`.exe`** køres med Proton via `umu-run`, ellers med `wine`. Hvert spil får sit eget prefix i `~/Games/spilarr/<spilmappe>`.

## Udvikling

```bash
cargo test
GAMES_DIR=/sti/til/games CONFIG_DIR=./config cargo run
docker build -t spilarr .
```

Import bruger qB's API uden login. Docker-nettet (fx `172.16.0.0/12`) skal derfor stå under *Bypass authentication for clients in whitelisted IP subnets*.

Planlagt:

- **Fase 3:** udpakning af eksisterende rar-mapper i spilmappen.
