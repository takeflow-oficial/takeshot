# Testing on macOS

CI builds and lints the macOS app but cannot click through it. Before a release, someone with a Mac runs this list on the `takeshot-X.Y.Z-macos-universal.dmg` from the release (or from the `takeshot-macos-universal` artifact of a workflow run). Note the macOS version and the chip (Apple Silicon or Intel) in the PR or the release discussion, with what failed.

## Install

1. Open the `.dmg`: the window shows Takeshot on the left, Applications on the right and an arrow between them, on a light lavender background (design page D2). Drag Takeshot to Applications.
2. The app is not notarized by Apple yet, so macOS blocks it the first time. Open System Settings > Privacy & Security, click "Open Anyway" next to Takeshot and confirm. If macOS says the app is damaged, run `xattr -cr /Applications/Takeshot.app` and open it again.
3. Expect the Takeshot icon in the menu bar and the main window (Home).

## Screen recording

4. Press `Ctrl+Shift+2` (area). macOS asks for Screen Recording: allow it in System Settings > Privacy & Security > Screen & System Audio Recording, then quit Takeshot from the menu bar icon and open it again.
5. `Ctrl+Shift+2` again: the screen dims, drag an area. Expect the tools in a column beside the area, the options next to them and Pin, Save, Edit and Copy under the area. Draw a rectangle and an arrow, press Copy, paste in Preview (File > New from Clipboard).
6. `Ctrl+Shift+1` (full screen) opens the editor with the whole screen.
7. On a second monitor, if there is one: the area capture works there and the bars stay on that monitor. Then turn on Settings > Capture > "Area capture on every screen": every monitor dims. Choose the area on one, then on another (the first one's area and marks go away), with monitors of different scales if you have them, and check that the window covers each monitor exactly. Then Edit, Copy and Save (turn "Save straight to the folder" off to see the dialog, which must show above the capture): the picture is that of the monitor with the area, and the editor and the notice open there.
8. Turn off Settings > Capture > "Hide Takeshot when capturing" with the main window open and press `Ctrl+Shift+2`: the window stays on screen under the dim and can be part of the capture. With the setting on (the default) it goes away first.

## Windows and title bar

9. The main window and the editor draw their own 44 px title bar with the traffic lights inside it, vertically centred with the bar's buttons. Drag the window by the bar, double-click it to zoom, resize it from the edges.
10. Switch the theme in Settings (TakeUI and Coral UI, light and dark): every window follows, the menu bar icon included.
11. Turn off Settings > General > Window and tray > "Show in the Dock": the Dock icon and the Cmd+Tab entry go away, the menu bar icon stays, and the main window stays open and still takes keys. From a capture, open Edit and Pin: both show and take focus without a Dock icon. Quit and open Takeshot again: it starts with no Dock icon. Turn the setting on: the icon comes back.

## Menu bar icon

12. Click the icon: the menu shows capture area, full screen and in 3 seconds, open Takeshot, the recent captures, style, settings, theme, about and quit.
13. A recent capture copies on click and opens in the editor with Shift+click.

## Pin and editor

14. From a capture, Pin: the picture floats over other windows. Drag the middle to move it, a corner to resize it (it keeps its proportions), scroll to zoom, right-click for the menu, double-click or Esc to close.
15. In the editor, add a text, change its size with the wheel, undo and redo, save with `Cmd+S`, and check the PNG in Finder.

## Start at login and quit

16. Settings > General > "Start with the system": log out and in. Takeshot starts in the menu bar only. The entry is `~/Library/LaunchAgents/Takeshot.plist`.
17. Open an editor, draw something, then quit from the menu bar icon: the editor asks (Discard, Cancel, Save and close). Cancel keeps everything open; Discard closes everything. The same from a terminal: with that editor open, `/Applications/Takeshot.app/Contents/MacOS/takeshot --quit` waits for the answer and exits with 1 after Cancel (everything stays open), or with 0 once Takeshot is gone; with no answer for 30 seconds it exits with 2 and nothing closes.
18. Logs: after steps 1 to 17, `~/Library/Logs/takeshot` holds `takeshot-resident.log` and `takeshot-capture.log`, `takeshot-edit.log` and `takeshot-pin.log` for the windows you opened (the warm capture is a Windows feature, so there is no `takeshot-standby.log` here). Each starts with a `start takeshot <version> macos <arch> role=<role> flags=...` line (flags only, no file names), and the log of a window that closed normally ends with `exit`. Open the folder in Console.app or Finder and check that no line holds a path of your captures.

## Uninstall

19. Quit Takeshot, drag it from Applications to the Trash. Its data: settings in `~/Library/Application Support/takeshot`, thumbnails in `~/Library/Caches/takeshot`, logs in `~/Library/Logs/takeshot`, the login entry above.
