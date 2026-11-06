#!/usr/bin/env bash
# F11: startet KWin ohne Bildschirm und prüft die Zwischenablage (AC-3).
# Aufruf im Container (siehe README) oder direkt in einer D-Bus-Sitzung mit kwin_wayland und wl-clipboard.
set -euo pipefail
BIN=$(realpath "${1:?Pfad zur tippkraft-Binärdatei}")
export XDG_RUNTIME_DIR=$(mktemp -d); chmod 700 "$XDG_RUNTIME_DIR"
unset NIRI_SOCKET SWAYSOCK HYPRLAND_INSTANCE_SIGNATURE
export XDG_CURRENT_DESKTOP=KDE WAYLAND_DISPLAY=wayland-tk
kwin_wayland --virtual --no-lockscreen --socket wayland-tk > "$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
trap 'kill $(jobs -p) 2>/dev/null || true' EXIT
for _ in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/wayland-tk" ] && break; sleep 0.1; done
[ -S "$XDG_RUNTIME_DIR/wayland-tk" ] || { cat "$XDG_RUNTIME_DIR/kwin.log"; exit 1; }
sleep 1

fehler() { echo "KDE: FEHLER – $1"; cat "$XDG_RUNTIME_DIR/kwin.log" | tail -20; exit 1; }
printf 'alt' | wl-copy
sleep 0.5
[ "$(wl-paste -n)" = "alt" ] || fehler "wl-copy setzt nichts"
"$BIN" ablage-test "neu 🙂" > "$XDG_RUNTIME_DIR/ablage.log" 2>&1 &
sleep 1.5
[ "$(wl-paste -n)" = "neu 🙂" ] || fehler "tippkraft setzt nicht: $(wl-paste -n 2>&1) / $(cat "$XDG_RUNTIME_DIR/ablage.log")"
wl-paste -l | grep -q x-kde-passwordManagerHint || fehler "Klipper-Hinweis fehlt"
[ "$(wl-paste -p -n)" = "neu 🙂" ] || fehler "Auswahl nicht gesetzt"
sleep 3
[ "$(wl-paste -n)" = "alt" ] || fehler "nicht zurückgesetzt: $(wl-paste -n 2>&1)"
[ -z "$(wl-paste -p -n 2>/dev/null)" ] || fehler "Auswahl nicht zurückgesetzt"
echo "KDE Zwischenablage: ok"

# AC-7: Fensterwechsel über das KWin-Skript
LOG="$XDG_RUNTIME_DIR/fokus.log"
"$BIN" fokus > "$LOG" 2>&1 &
sleep 1.5
warten() { for _ in $(seq 100); do grep -qx "fokus: $1" "$LOG" && return; sleep 0.1; done; fehler "kein 'fokus: $1' – $(cat "$LOG")"; }
foot --app-id=eins sleep 60 & EINS=$!
warten eins
foot --app-id=zwei sleep 60 & ZWEI=$!
warten zwei
kill $ZWEI
sleep 1
[ "$(tail -1 "$LOG")" = "fokus: eins" ] || fehler "nach Schließen nicht zurück auf eins – $(cat "$LOG")"
echo "KDE Fokus: ok"
