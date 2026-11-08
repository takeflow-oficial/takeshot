#!/usr/bin/env bash
# Runs INSIDE a container with the .deb installed (scripts/linux-smoke.sh starts it):
#
#   resident-check.sh x11|wayland
#
# The resident has no window of its own on Linux: the main window is made when wanted and destroyed
# to hide it to the tray (Wayland cannot hide one), but only while a panel shows the tray icon; with
# none it would leave no way back.
#
# wayland: sway with a bar that hosts tray icons. Started in the tray (`--minimized`) there is no
#   window; then three times a second launch shows the window and the X (with "Minimize to tray"
#   chosen) destroys it, the resident outliving each. Then a capture on the sample screen: its
#   "Capture saved" notice must reach the desktop's notifications (mako) and open no window.
# x11: no tray host at all (openbox only). A hidden start waits for one, gives up after 30 s and
#   shows the window; the X then only minimizes it, and a second launch brings it back.
set -eu
mode=${1:?x11 or wayland}
export HOME=/root
mkdir -p "$HOME/.config/takeshot" /tmp/caps
# The X of the window minimizes to the tray without asking; captures save to a folder we can see.
echo '{"on_close":"Minimize","save_dir":"/tmp/caps"}' > "$HOME/.config/takeshot/settings.json"
export XDG_RUNTIME_DIR=/tmp/xdg
mkdir -p -m 700 "$XDG_RUNTIME_DIR"

fail() {
  echo "FAIL: $1"
  echo "---- takeshot.log"; cat /tmp/takeshot.log 2> /dev/null || true
  exit 1
}

case $mode in
  wayland)
    # swaybar hosts the tray icons (it owns org.kde.StatusNotifierWatcher).
    printf 'bar {\n  position top\n  tray_output *\n}\n' > /tmp/sway.conf
    WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /tmp/sway.conf > /tmp/sway.log 2>&1 &
    for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/wayland-1" ] && break; sleep 0.2; done
    export WAYLAND_DISPLAY=wayland-1
    SWAYSOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock | head -1)
    export SWAYSOCK
    count() { swaymsg -t get_tree | jq '[.. | objects | select(.app_id? == "takeshot")] | length'; }
    close_window() { swaymsg '[app_id="takeshot"] kill' > /dev/null; }
    ;;
  x11)
    Xvfb :99 -ac -screen 0 1280x800x24 > /tmp/xvfb.log 2>&1 &
    for _ in $(seq 50); do [ -S /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
    export DISPLAY=:99
    openbox > /tmp/openbox.log 2>&1 &
    sleep 1
    count() { wmctrl -l | grep -c ' Takeshot$' || true; }
    close_window() { wmctrl -c Takeshot; }
    # The window manager's word for it: iconified or not.
    minimized() { xprop -id "$(wmctrl -l | awk '/ Takeshot$/ {print $1; exit}')" _NET_WM_STATE | grep -q _NET_WM_STATE_HIDDEN; }
    ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac

# Waits up to $2 seconds for $1 windows.
wait_for() {
  for _ in $(seq $(($2 * 5))); do
    [ "$(count)" = "$1" ] && return 0
    sleep 0.2
  done
  return 1
}
alive() { pgrep -x takeshot > /dev/null; }

takeshot --minimized --lang en > /tmp/takeshot.log 2>&1 &
sleep 4
alive || fail "the resident did not start"
[ "$(count)" = 0 ] || fail "started in the tray, but a window is on screen"
echo "ok: started in the tray with no window"

if [ "$mode" = wayland ]; then
  for round in 1 2 3; do
    takeshot --lang en > /dev/null 2>&1 # a second launch asks the resident for its window
    wait_for 1 20 || fail "round $round: no window after the second launch"
    close_window
    wait_for 0 20 || fail "round $round: the window stayed after the X (is the tray host up?)"
    alive || fail "round $round: the resident left with its window"
    echo "ok: round $round, window shown, closed to the tray, resident still running"
  done

  # The copy and save notice is the desktop's: no window, and the notification exists.
  mako > /tmp/mako.log 2>&1 &
  sleep 1
  takeshot --capture --full --sample --lang en > /tmp/capture.log 2>&1 &
  capture=$!
  sleep 3
  [ "$(count)" = 0 ] || fail "the saved notice opened a window"
  makoctl list > /tmp/notifications.txt 2>&1 || true
  grep -q "Capture saved" /tmp/notifications.txt || {
    cat /tmp/notifications.txt /tmp/mako.log /tmp/capture.log
    fail "no \"Capture saved\" notification reached the desktop"
  }
  ls /tmp/caps/*.png > /dev/null 2>&1 || fail "the capture was not saved"
  wait "$capture" || true
  echo "ok: the saved notice is a desktop notification, with no window"
else
  # No panel shows a tray icon here: after TRAY_WAIT the hidden start shows its window.
  wait_for 1 45 || fail "a hidden start with no tray never showed its window"
  echo "ok: no tray appeared, so the window was shown"
  close_window
  sleep 3
  [ "$(count)" = 1 ] || fail "with no tray the X destroyed the window (no way back)"
  minimized || fail "with no tray the X should only minimize the window"
  alive || fail "the resident left with the X"
  echo "ok: the X only minimized the window"
  takeshot --lang en > /dev/null 2>&1
  for _ in $(seq 25); do minimized || break; sleep 0.2; done
  minimized && fail "a second launch did not bring the minimized window back"
  echo "ok: a second launch brought the window back"
fi
pkill -x takeshot || true
echo "ALL OK ($mode)"
