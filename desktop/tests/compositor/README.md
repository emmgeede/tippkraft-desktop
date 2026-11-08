# Compositor tests (F22)

Checks that `tippkraft focus` reports every window focus change on real compositors.

## Sway (headless, runs in CI)

```sh
cargo build --release
# directly, if sway and foot are installed:
tests/compositor/sway.sh target/release/tippkraft
# or in a container:
docker build -t tippkraft-sway-test -f tests/compositor/Dockerfile.sway tests/compositor
docker run --rm -v $PWD/tests/compositor/sway.sh:/test/sway.sh:ro -v $PWD/target/release/tippkraft:/test/tippkraft:ro tippkraft-sway-test
```

## Hyprland (nested, needs a running Wayland session and a GPU)

Hyprland always needs a render device. On a host with Mesa drivers (AMD, Intel) the container works:

```sh
docker build -t tippkraft-hyprland-test -f tests/compositor/Dockerfile.hyprland tests/compositor
docker run --rm --user $(id -u):$(id -g) --group-add $(stat -c %g /dev/dri/renderD128) --device /dev/dri \
  --tmpfs /run/user/$(id -u):mode=0700,uid=$(id -u),gid=$(id -g) -e XDG_RUNTIME_DIR=/run/user/$(id -u) \
  -e WAYLAND_DISPLAY=$WAYLAND_DISPLAY -v $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY:/run/user/$(id -u)/$WAYLAND_DISPLAY \
  -v $PWD/tests/compositor/hyprland.sh:/test/hyprland.sh:ro -v $PWD/target/release/tippkraft:/test/tippkraft:ro tippkraft-hyprland-test
```

With the proprietary NVIDIA driver the container needs the NVIDIA Container Toolkit, otherwise run `hyprland.sh` inside a Hyprland session (VM or host).
