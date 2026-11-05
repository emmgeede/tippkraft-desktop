#!/bin/sh
# tippkraft installer: https://github.com/emmgeede/tippkraft-desktop
# Downloads the latest release, verifies its SHA-256 checksum and runs `tippkraft setup`.
# It never runs sudo.
set -eu

BASE="https://github.com/emmgeede/tippkraft-desktop/releases/latest/download"
ASSET="tippkraft-x86_64-linux"

fail() {
  printf 'tippkraft: %s\n' "$1" >&2
  exit 1
}

[ "$(uname -s)" = "Linux" ] || fail "tippkraft only runs on Linux."
[ "$(uname -m)" = "x86_64" ] || fail "only x86_64 is supported for now (this machine is $(uname -m))."
[ -n "${WAYLAND_DISPLAY:-}" ] || fail "no Wayland session found. tippkraft needs Wayland (Niri, Sway or Hyprland)."
if ! id -nG | tr ' ' '\n' | grep -qx input; then
  fail "your user is not in the 'input' group, which tippkraft needs to read the keyboard.
Run:   sudo usermod -aG input \"\$USER\"
then log out and back in and run this installer again."
fi
if [ ! -w /dev/uinput ]; then
  fail "tippkraft cannot write to /dev/uinput, which it needs to type text. Run:
echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\", OPTIONS+=\"static_node=uinput\"' | sudo tee /etc/udev/rules.d/60-tippkraft-uinput.rules
sudo udevadm control --reload && sudo modprobe uinput && sudo udevadm trigger --sysname-match=uinput --action=add
then run this installer again."
fi
command -v curl >/dev/null 2>&1 || fail "curl is missing."
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is missing."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading tippkraft ..."
curl -fsSL -o "$tmp/$ASSET" "$BASE/$ASSET"
curl -fsSL -o "$tmp/$ASSET.sha256" "$BASE/$ASSET.sha256"
(cd "$tmp" && sha256sum -c --quiet "$ASSET.sha256") || fail "checksum mismatch, nothing was installed."

chmod +x "$tmp/$ASSET"
"$tmp/$ASSET" setup
