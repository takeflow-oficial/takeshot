#!/usr/bin/env bash
# Opens the Linux packages from target/packages-linux (scripts/linux-packages.sh) in clean Ubuntu
# 24.04 containers and saves what each window drew into target/packages-linux/smoke:
#   - the .deb installed with apt (its dependencies pulled in), on X11 (Xvfb) and on Wayland (sway
#     with no screen);
#   - the AppImage on X11, in a container with no GTK installed (it must carry its own libraries).
# Each run is `takeshot --screenshot`: the window draws ~120 frames and saves its picture.
# Then the resident's window is checked (scripts/container/resident-check.sh): it is made when
# wanted, destroyed to hide it to the tray, and made again, on X11 and on Wayland.
#
#   scripts/linux-smoke.sh [screens|resident]   (default: both)
set -eu
only=${1:-all}
want() { [ "$only" = all ] || [ "$only" = "$1" ]; }
cd "$(git rev-parse --show-toplevel)"
dir=target/packages-linux
deb=$(cd "$dir" && ls takeshot-*-linux-x86_64.deb)
appimage=$(cd "$dir" && ls takeshot-*-linux-x86_64.AppImage)
mkdir -p "$dir/smoke"
mount="$(pwd -W 2> /dev/null || pwd)/$dir:/pk"
checks="$(pwd -W 2> /dev/null || pwd)/scripts/container:/t:ro"
# apt in a container now and then fails on the network: once more, then its log.
prelude='
  set -e
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq > /dev/null
  get() {
    apt-get install -y -qq --no-install-recommends "$@" > /tmp/apt.log 2>&1 ||
      apt-get install -y -qq --no-install-recommends "$@" > /tmp/apt.log 2>&1 ||
      { tail -20 /tmp/apt.log; exit 1; }
  }
'
# --init: as PID 1, xvfb-run never hears that Xvfb is ready.
run() { MSYS_NO_PATHCONV=1 docker run --rm --init -v "$mount" -v "$checks" ubuntu:24.04 bash -c "$prelude $1"; }

if want screens; then
echo "== deb on X11"
run "
  get /pk/$deb xvfb xauth libgl1-mesa-dri
  command -v takeshot
  ls /usr/share/applications | grep -i takeshot
  xvfb-run -a -s '-screen 0 1280x800x24' takeshot --page inicio --lang en --screenshot /pk/smoke/deb-x11-home.png
  xvfb-run -a -s '-screen 0 1280x800x24' takeshot --page editor --lang en --theme coral --screenshot /pk/smoke/deb-x11-editor.png
"

echo "== deb on Wayland (sway)"
run "
  get /pk/$deb sway libgl1-mesa-dri libegl1
  export XDG_RUNTIME_DIR=/tmp/xdg && mkdir -p -m 700 \$XDG_RUNTIME_DIR
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /dev/null > /tmp/sway.log 2>&1 &
  for i in \$(seq 50); do [ -S \$XDG_RUNTIME_DIR/wayland-1 ] && break; sleep 0.2; done
  ls \$XDG_RUNTIME_DIR
  WAYLAND_DISPLAY=wayland-1 takeshot --page inicio --lang en --mode dark --screenshot /pk/smoke/deb-wayland-home.png
"

echo "== AppImage on X11, no GTK installed"
# Only what every desktop has: the libraries AppImages leave out by design (AppImageCommunity's
# excludelist: glibc, GL, X11, fonts and a few more), plus xkbcommon. GTK and the rest must come
# from the AppImage.
run "
  get xvfb xauth libgl1-mesa-dri libgl1 libegl1 libglx0 libopengl0 libdrm2 libgbm1 libxcb1 libx11-6 \
    libx11-xcb1 libxcb-dri2-0 libxcb-dri3-0 libwayland-client0 libasound2t64 libfontconfig1 libfreetype6 \
    libharfbuzz0b libfribidi0 libcom-err2 libexpat1 libgpg-error0 libice6 libsm6 libusb-1.0-0 libuuid1 \
    zlib1g libgmp10 libpipewire-0.3-0t64 libxkbcommon0 libxkbcommon-x11-0
  if dpkg -s libgtk-3-0t64 > /dev/null 2>&1; then echo 'GTK is installed: the check proves nothing'; exit 1; fi
  cp /pk/$appimage /tmp/takeshot.AppImage && chmod +x /tmp/takeshot.AppImage
  xvfb-run -a -s '-screen 0 1280x800x24' /tmp/takeshot.AppImage --appimage-extract-and-run --page estilo --lang en --screenshot /pk/smoke/appimage-x11-style.png
"
ls -la "$dir/smoke"
fi

if want resident; then
  echo "== deb on X11: the resident's window comes and goes"
  run "
    get /pk/$deb xvfb xauth libgl1-mesa-dri openbox wmctrl x11-utils procps dbus dbus-bin
    dbus-run-session -- bash /t/resident-check.sh x11
  "

  echo "== deb on Wayland (sway): the window comes and goes, the notice is a notification (mako)"
  run "
    get /pk/$deb sway libgl1-mesa-dri libegl1 jq procps mako-notifier dbus dbus-bin
    dbus-run-session -- bash /t/resident-check.sh wayland
  "
fi
