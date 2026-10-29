#!/bin/sh
# Installerer Spilarrs spil://-hjælper for den aktuelle bruger (ingen root).
#   curl -fsSL __SERVER__/linux/install.sh | sh
#   curl -fsSL __SERVER__/linux/install.sh | sh -s -- /sti/til/spilmappen
set -e
DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/spilarr.conf"
mkdir -p "$DIR/spilarr" "$DIR/applications" "$(dirname "$CONF")"

curl -fsSL "__SERVER__/linux/spil-handler.sh" -o "$DIR/spilarr/spil-handler.sh"
chmod +x "$DIR/spilarr/spil-handler.sh"

# Spilmappen: argument, ellers steam-headless' mount, ellers et gvfs-mount af sharen
root="$1"
if [ -z "$root" ]; then
    for c in /mnt/torrents /mnt/spil "/run/user/$(id -u)/gvfs/smb-share:server=__SMB_HOST__,share=data/games"; do
        [ -d "$c" ] && [ -n "$(ls -A "$c" 2>/dev/null)" ] && root="$c" && break
    done
fi
if [ -z "$root" ] && command -v gio >/dev/null; then
    echo "Mounter smb://__SMB_HOST__/data med gio (gemmer login i nøgleringen) …"
    gio mount "smb://__SMB_HOST__/data" || true
    c="/run/user/$(id -u)/gvfs/smb-share:server=__SMB_HOST__,share=data/games"
    [ -d "$c" ] && root="$c"
fi
[ -n "$root" ] || root="__LINUX_ROOT__"
if [ ! -f "$CONF" ]; then
    echo "SPIL_ROOT=\"$root\"" > "$CONF"
else
    sed -i "s|^SPIL_ROOT=.*|SPIL_ROOT=\"$root\"|" "$CONF"
fi

cat > "$DIR/applications/spilarr-handler.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Spilarr-hjælper
Exec=$DIR/spilarr/spil-handler.sh %u
NoDisplay=true
MimeType=x-scheme-handler/spil;
EOF
xdg-mime default spilarr-handler.desktop x-scheme-handler/spil
command -v update-desktop-database >/dev/null && update-desktop-database "$DIR/applications" 2>/dev/null || true

echo "Spilarr-hjælper installeret i $DIR/spilarr"
echo "Spilmappe: $root (ret i $CONF)"
[ -d "$root" ] || echo "ADVARSEL: $root findes ikke endnu – mount sharen og ret SPIL_ROOT."
for t in udisksctl 7z umu-run wine; do command -v $t >/dev/null && echo "  fundet: $t"; done
true
