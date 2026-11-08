# Testing on Linux

Two scripts check the Linux packages in Docker, from Windows, macOS or Linux:

```sh
scripts/linux-packages.sh   # builds the .deb, .AppImage and .tar.gz in the CI image (scripts/ci-linux)
scripts/linux-smoke.sh      # opens them in clean Ubuntu 24.04 containers and saves what they drew
```

`linux-packages.sh` puts the packages in `target/packages-linux` and prints the shared libraries the binary needs (`dpkg-shlibdeps`), which is where the `.deb`'s `depends` in `Cargo.toml` comes from. `linux-smoke.sh` then:

- installs the `.deb` with `apt` (which pulls its dependencies) and runs `takeshot --screenshot` on **X11** (Xvfb) and on **Wayland** (sway with no screen, `WLR_BACKENDS=headless`);
- runs the AppImage on X11 in a container with no GTK installed, so it must carry its own libraries;
- checks the resident's window (`scripts/container/resident-check.sh`, `scripts/linux-smoke.sh resident` for just this part): started in the tray with no window, then three times a second launch shows the window and the X (with "Minimize to tray" chosen) closes it, the resident outliving each one. On **Wayland** (sway, whose bar hosts the tray icons) it does that and closes the window with `swaymsg kill`; it then takes a capture on the sample screen and checks that the "Capture saved" notice reaches mako, the notification daemon, and opens no window. On **X11** (Xvfb with openbox) there is no tray host: the hidden start gives up after 30 s and shows the window, the X (`wmctrl -c`) only minimizes it, and a second launch brings it back.

The pictures land in `target/packages-linux/smoke`. Look at each one: the window drawn whole, in the theme asked for.

## What the containers cannot check

These need a desktop session and go through by hand before a release (a VM or a Linux machine):

1. The tray icon and its menu (an AppIndicator panel: GNOME with the AppIndicator extension, KDE, XFCE with the StatusNotifier plugin).
2. `PrtSc` and `Shift+PrtSc` on X11; on Wayland, a desktop shortcut running `takeshot --capture`, and the capture through the desktop portal.
3. Settings > General > "Start with the system": log out and in; the entry is `~/.config/autostart/Takeshot.desktop`.
4. Quit from the tray (or Settings > General when there is no tray) closes the editors and pinned pictures too.
5. On Wayland (KDE, GNOME): "Minimize to tray" leaves no entry in the taskbar, the tray's "Open" (and a second launch) brings the window back to the workspace you are on, and Copy or Save in a capture shows a notification in the desktop's corner while the screen goes back to normal at once.

## Last run

2026-11-08, v0.1.0, Docker Desktop on Windows 10, `ubuntu:24.04`:

| Package | Session | Result |
|---|---|---|
| `.deb` (apt pulled GTK, Mesa and the rest from its `depends`) | X11, Xvfb 1280 x 800 | Home and the editor (Coral UI) drawn whole |
| `.deb` | Wayland, sway headless (pixman) | Home drawn whole; Mesa falls back to software rendering, with EGL warnings |
| `.AppImage` | X11, Xvfb, no GTK installed (only the AppImage excludelist's libraries) | Style drawn whole: GTK and its libraries come from the AppImage |

The AppImage, like every AppImage, expects the libraries of AppImageCommunity's excludelist (glibc, GL, X11, fontconfig, freetype, harfbuzz, fribidi and a few more) from the system: any desktop has them, a bare container does not, so the script installs exactly those.

### The resident's window

2026-11-08, the branch that fixed [takeflow-oficial/takeshot#4](https://github.com/takeflow-oficial/takeshot/issues/4) and [#5](https://github.com/takeflow-oficial/takeshot/issues/5), the same containers (`scripts/linux-smoke.sh resident`):

| Session | Result |
|---|---|
| X11, Xvfb with openbox, no tray host | The hidden start showed the window after 30 s; the X only minimized it; a second launch brought it back |
| Wayland, sway with swaybar as the tray host, mako | The hidden start had no window. Three times: a second launch showed the window, the X destroyed it, the resident stayed. The saved notice reached mako and opened no window |

By hand on Kubuntu (Plasma 6.6.6, Wayland), with the `.deb` of that branch: minimizing to the tray leaves nothing in the taskbar, the window comes back from the tray and from a second launch, and Copy and Save show a desktop notification.
