#!/usr/bin/env bash
# F22: startet Hyprland verschachtelt (braucht eine laufende Wayland-Sitzung und GPU) und prüft,
# dass `tippkraft focus` jeden Fensterwechsel meldet. Aufruf: tests/compositor/hyprland.sh <pfad/zu/tippkraft>
set -euo pipefail
BIN=$(realpath "${1:?Pfad zur tippkraft-Binärdatei}")
: "${WAYLAND_DISPLAY:?Hyprland läuft verschachtelt und braucht WAYLAND_DISPLAY}"
unset NIRI_SOCKET SWAYSOCK HYPRLAND_INSTANCE_SIGNATURE

KONF=$(mktemp -d)
cat > "$KONF/hyprland.conf" <<'KONFIG'
monitor = , 1024x768, auto, 1
misc {
  disable_hyprland_logo = true
  disable_splash_rendering = true
}
ecosystem {
  no_update_news = true
  no_donation_nag = true
}
animations {
  enabled = false
}
KONFIG

Hyprland -c "$KONF/hyprland.conf" > "$KONF/hyprland.log" 2>&1 &
trap 'kill $(jobs -p) 2>/dev/null || true' EXIT
for _ in $(seq 100); do compgen -G "$XDG_RUNTIME_DIR/hypr/*/.socket2.sock" > /dev/null && break; sleep 0.1; done
SOCK=$(compgen -G "$XDG_RUNTIME_DIR/hypr/*/.socket2.sock") || { tail -30 "$KONF/hyprland.log"; exit 1; }
HYPRLAND_INSTANCE_SIGNATURE=$(basename "$(dirname "$SOCK")")
export HYPRLAND_INSTANCE_SIGNATURE

LOG="$KONF/fokus.log"
WAYLAND_DISPLAY= "$BIN" fokus > "$LOG" 2>&1 &
sleep 0.5

fenster() { # Klasse: öffnet ein Terminal und wartet, bis es da ist
  hyprctl -q dispatch exec "foot --app-id=$1 sleep 60"
  for _ in $(seq 100); do hyprctl clients -j | grep -q "\"class\": \"$1\"" && return; sleep 0.1; done
  echo "Fenster $1 erscheint nicht"; exit 1
}
fenster eins
fenster zwei
hyprctl -q dispatch focuswindow class:eins
sleep 0.3
hyprctl -q dispatch closewindow class:eins
sleep 0.5

erwartet=$'focus: -\nfocus: eins\nfocus: zwei\nfocus: eins\nfocus: zwei'
if [ "$(cat "$LOG")" = "$erwartet" ]; then
  echo "Hyprland: ok"
else
  echo "Hyprland: FEHLER"; echo "--- erwartet:"; echo "$erwartet"; echo "--- bekommen:"; cat "$LOG"; exit 1
fi
