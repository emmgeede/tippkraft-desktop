#!/usr/bin/env bash
# F22: startet Sway ohne Bildschirm und prüft, dass `tippkraft focus` jeden Fensterwechsel meldet.
# Aufruf: tests/compositor/sway.sh <pfad/zu/tippkraft>   (braucht sway, swaymsg, foot)
set -euo pipefail
BIN=$(realpath "${1:?Pfad zur tippkraft-Binärdatei}")

export XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman
unset NIRI_SOCKET HYPRLAND_INSTANCE_SIGNATURE WAYLAND_DISPLAY SWAYSOCK
printf 'output HEADLESS-1 resolution 1024x768\n' > "$XDG_RUNTIME_DIR/config"

sway -c "$XDG_RUNTIME_DIR/config" > "$XDG_RUNTIME_DIR/sway.log" 2>&1 &
trap 'kill $(jobs -p) 2>/dev/null || true' EXIT
for _ in $(seq 100); do compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock" > /dev/null && break; sleep 0.1; done
SWAYSOCK=$(compgen -G "$XDG_RUNTIME_DIR/sway-ipc.*.sock") || { cat "$XDG_RUNTIME_DIR/sway.log"; exit 1; }
export SWAYSOCK

LOG="$XDG_RUNTIME_DIR/fokus.log"
"$BIN" fokus > "$LOG" 2>&1 &
sleep 0.5

fenster() { # app_id: öffnet ein Terminal und wartet, bis es da ist
  swaymsg -q exec "foot --app-id=$1 sleep 60"
  for _ in $(seq 100); do swaymsg -t get_tree | grep -q "\"app_id\": \"$1\"" && return; sleep 0.1; done
  echo "Fenster $1 erscheint nicht"; exit 1
}
fenster eins
fenster zwei
swaymsg -q "[app_id=eins] focus"
sleep 0.3
swaymsg -q "[app_id=eins] kill"
sleep 0.5

erwartet=$'focus: -\nfocus: eins\nfocus: zwei\nfocus: eins\nfocus: -\nfocus: zwei'
if [ "$(cat "$LOG")" = "$erwartet" ]; then
  echo "Sway: ok"
else
  echo "Sway: FEHLER"; echo "--- erwartet:"; echo "$erwartet"; echo "--- bekommen:"; cat "$LOG"; exit 1
fi
