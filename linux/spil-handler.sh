#!/bin/sh
# Spilarr: håndterer spil://-links fra Spilarr-webappen på Linux.
#   spil://mount?p=<sti>    mount ISO fra sharen og åbn den (udisksctl, ellers fuseiso/archivemount)
#   spil://unmount?p=<sti>  afmonter ISO
#   spil://run?p=<sti>      kør .exe med Proton (umu-run) eller wine, ét prefix pr. spil
#   spil://open?p=<sti>     åbn mappen
# <sti> er relativ til spilmappen (SPIL_ROOT i ~/.config/spilarr.conf); alt uden for den afvises.
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/spilarr.conf"
[ -f "$CONF" ] && . "$CONF"
ROOT="${SPIL_ROOT:-__LINUX_ROOT__}"
PREFIXES="${SPIL_PREFIXES:-$HOME/Games/spilarr}"
MNT="${SPIL_MNT:-${XDG_RUNTIME_DIR:-/tmp}/spilarr}"

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

case "$action" in
mount)
    case "$path" in *.[iI][sS][oO]) ;; *) fejl "Kun .iso-filer kan mountes.";; esac
    if command -v udisksctl >/dev/null && [ -e /dev/loop-control ]; then
        loop=$(losetup -j "$path" 2>/dev/null | cut -d: -f1 | head -1)
        [ -n "$loop" ] || loop=$(udisksctl loop-setup -r -f "$path" 2>&1 | grep -o '/dev/loop[0-9]*' | head -1)
        [ -n "$loop" ] || fejl "udisksctl kunne ikke oprette loop-enhed for $path"
        dir=$(findmnt -n -o TARGET "$loop" 2>/dev/null)
        [ -n "$dir" ] || dir=$(udisksctl mount -b "$loop" 2>&1 | sed -n 's/.* at \(.*\)$/\1/p' | sed 's/\.$//')
    else
        dir="$MNT/$(slug "$path" .iso)"
        if ! mountpoint -q "$dir" 2>/dev/null; then
            mkdir -p "$dir"
            if command -v fuseiso >/dev/null; then fuseiso "$path" "$dir" || fejl "fuseiso fejlede"
            elif command -v archivemount >/dev/null; then archivemount -o readonly "$path" "$dir" || fejl "archivemount fejlede"
            else fejl "Kan ikke mounte: installer udisks2 (med loop-enheder) eller fuseiso/archivemount."
            fi
        fi
    fi
    [ -d "$dir" ] || fejl "ISO'en blev ikke mountet."
    xdg-open "$dir" >/dev/null 2>&1 &
    ;;
unmount)
    loop=$(losetup -j "$path" 2>/dev/null | cut -d: -f1 | head -1)
    if [ -n "$loop" ]; then
        udisksctl unmount -b "$loop" >/dev/null 2>&1; udisksctl loop-delete -b "$loop" >/dev/null 2>&1
    else
        dir="$MNT/$(slug "$path" .iso)"
        fusermount -u "$dir" 2>/dev/null || fusermount3 -u "$dir" 2>/dev/null || umount "$dir" 2>/dev/null
        rmdir "$dir" 2>/dev/null
    fi
    besked "Afmonteret: $(basename "$path")"
    ;;
run)
    case "$path" in *.[eE][xX][eE]) ;; *) fejl "Kun .exe kan køres.";; esac
    export WINEPREFIX="$PREFIXES/$(slug "$(game_dir)")"
    mkdir -p "$WINEPREFIX"
    cd "$(dirname "$path")" || exit 1
    if command -v umu-run >/dev/null; then GAMEID="${GAMEID:-umu-default}" umu-run "$path" &
    elif command -v wine >/dev/null; then wine "$path" &
    else fejl "Hverken umu-run eller wine er installeret."
    fi
    besked "Starter $(basename "$path") i $WINEPREFIX"
    ;;
open)
    xdg-open "$path" >/dev/null 2>&1 &
    ;;
*)
    fejl "Ukendt handling: $action"
    ;;
esac
