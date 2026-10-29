#!/bin/sh
# Spilarr: håndterer spil://-links fra Spilarr-webappen på Linux.
#   spil://mount?p=<sti>    mount ISO fra sharen og åbn den (udisksctl). Uden loop-enheder (fx steam-headless)
#                           pakkes ISO'en i stedet ud med 7z til SPIL_EXTRACT – spil-ISO'er er UDF, som FUSE-værktøjer ikke kan læse
#   spil://unmount?p=<sti>  afmonter ISO (eller slet udpakningen)
#   spil://run?p=<sti>      kør .exe med Proton (umu-run) eller wine, ét prefix pr. spil
#   spil://open?p=<sti>     åbn mappen
# <sti> er relativ til spilmappen (SPIL_ROOT i ~/.config/spilarr.conf); alt uden for den afvises.
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/spilarr.conf"
[ -f "$CONF" ] && . "$CONF"
ROOT="${SPIL_ROOT:-__LINUX_ROOT__}"
PREFIXES="${SPIL_PREFIXES:-$HOME/Games/spilarr}"
EXTRACT="${SPIL_EXTRACT:-$HOME/Games/spilarr/.iso}"

besked() {
    echo "Spilarr: $1" >&2
    if command -v notify-send >/dev/null; then notify-send -a Spilarr "Spilarr" "$1"
    elif command -v zenity >/dev/null; then zenity --info --title Spilarr --text "$1"
    elif command -v kdialog >/dev/null; then kdialog --title Spilarr --msgbox "$1"
    fi
}
fejl() { besked "$1"; exit 1; }

url="$1"
action="${url#spil://}"; action="${action%%\?*}"; action="${action%/}"
q="${url#*\?}"; enc=""
for kv in $(echo "$q" | tr '&' ' '); do case "$kv" in p=*) enc="${kv#p=}";; esac; done
[ -n "$enc" ] || fejl "Mangler sti i $url"
# URL-dekod (%XX og +)
rel=$(printf '%s' "$enc" | sed 's/+/ /g; s/%\([0-9A-Fa-f][0-9A-Fa-f]\)/\\x\1/g' | xargs -0 printf '%b')
case "/$rel/" in */../*) fejl "Ugyldig sti: $rel";; esac
case "$rel" in /*) fejl "Ugyldig sti: $rel";; esac
path="$ROOT/$rel"
[ -e "$path" ] || fejl "Kan ikke nå $path – er spilmappen mountet? (SPIL_ROOT i $CONF)"

slug() { printf '%s' "$(basename "$1")" | tr -c 'A-Za-z0-9._-' '_' ; }
game_dir() { echo "$rel" | cut -d/ -f1; }

# Kører en .exe med Proton (umu-run) eller wine i spillets eget prefix.
run_exe() {
    export WINEPREFIX="$PREFIXES/$(slug "$(game_dir)")"
    mkdir -p "$WINEPREFIX"
    cd "$(dirname "$1")" || exit 1
    if command -v umu-run >/dev/null; then GAMEID="${GAMEID:-umu-default}" umu-run "$1" &
    elif command -v wine >/dev/null; then wine "$1" &
    else fejl "Hverken umu-run eller wine er installeret."
    fi
    besked "Starter $(basename "$1") i $WINEPREFIX"
}

case "$action" in
mount)
    case "$path" in *.[iI][sS][oO]) ;; *) fejl "Kun .iso-filer kan mountes.";; esac
    if command -v udisksctl >/dev/null && [ -e /dev/loop-control ]; then
        loop=$(losetup -j "$path" 2>/dev/null | cut -d: -f1 | head -1)
        [ -n "$loop" ] || loop=$(udisksctl loop-setup -r -f "$path" 2>&1 | grep -o '/dev/loop[0-9]*' | head -1)
        [ -n "$loop" ] || fejl "udisksctl kunne ikke oprette loop-enhed for $path"
        dir=$(findmnt -n -o TARGET "$loop" 2>/dev/null)
        [ -n "$dir" ] || dir=$(udisksctl mount -b "$loop" 2>&1 | sed -n 's/.* at \(.*\)$/\1/p' | sed 's/\.$//')
    elif command -v 7z >/dev/null; then
        dir="$EXTRACT/$(slug "$path")"
        if [ ! -f "$dir/.spilarr-ok" ]; then
            rm -rf "$dir"; mkdir -p "$dir"
            besked "Kan ikke mounte her – pakker $(basename "$path") ud til $dir (slettes igen med Afmonter) …"
            7z x -y -bso0 -bsp0 -o"$dir" "$path" || { rm -rf "$dir"; fejl "7z kunne ikke pakke $(basename "$path") ud."; }
            touch "$dir/.spilarr-ok"
        fi
    else
        fejl "Kan ikke mounte: installer udisks2 (med loop-enheder) eller 7z."
    fi
    [ -d "$dir" ] || fejl "ISO'en blev ikke mountet."
    xdg-open "$dir" >/dev/null 2>&1 &
    # Linux kan ikke dobbeltklikke en .exe – start installeren direkte, hvis ISO'en har en
    setup=$(find "$dir" -maxdepth 1 -iname 'setup.exe' | head -1)
    [ -n "$setup" ] && run_exe "$setup"
    ;;
unmount)
    loop=$(losetup -j "$path" 2>/dev/null | cut -d: -f1 | head -1)
    if [ -n "$loop" ]; then
        udisksctl unmount -b "$loop" >/dev/null 2>&1; udisksctl loop-delete -b "$loop" >/dev/null 2>&1
        besked "Afmonteret: $(basename "$path")"
    elif [ -d "$EXTRACT/$(slug "$path")" ]; then
        rm -rf "${EXTRACT:?}/$(slug "$path")"
        besked "Udpakning slettet: $(basename "$path")"
    else
        besked "$(basename "$path") er ikke mountet."
    fi
    ;;
run)
    case "$path" in *.[eE][xX][eE]) ;; *) fejl "Kun .exe kan køres.";; esac
    run_exe "$path"
    ;;
open)
    xdg-open "$path" >/dev/null 2>&1 &
    ;;
*)
    fejl "Ukendt handling: $action"
    ;;
esac
