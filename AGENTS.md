# Takeshot agent guide

Takeshot is a screenshot tool written in Rust with egui: capture with PrtSc, annotate, and style the result with backgrounds and watermarks. The redesign in progress turns it into a tray utility with editable objects. Product context and principles: [PRODUCT.md](PRODUCT.md). Visual source of truth: [design/takeshot-ui.fig](design/takeshot-ui.fig) (open it in OpenPencil or Figma).

## Parity

Takeshot ships two themes, **TakeUI** and **Coral UI**, each with light, dark and system modes, over one layout. A theme changes the colours (`Tokens`), the brand mark and name (`ui::brand`) and the look of a few kit components (selection marks, the active tool, the switch knob, the slider line); it never moves a function. Parity means a visible change looks right in all four looks: TakeUI and Coral UI, light and dark. Use the `implementar-nos-dois-temas` skill for any change a user can see.

In `design/takeshot-ui.fig`, page A holds the layout and TakeUI's look, and page "C · Propostas (2026-11-08)" the changes to that layout approved on 2026-11-08 (Início in a grid or a list with a detail panel, underlined tabs, the object bar, the capture's bars beside the area); Início's day titles on page C were dropped after approval: the captures run in one grid or list and each tells its day. page "D · Instalador (2026-11-08)" the Windows installer and the macOS dmg window (their pictures come from `assets/brand/installer-*.svg` and `dmg-background.svg`). Coral UI's colours and components are in the "B · Design system" frame of page B (page B's screens explore a layout the app does not use).

## Layout

| Folder | Holds |
|---|---|
| `src/main.rs` | entry point: arguments, shared state, window modes |
| `src/engine/` | drawing engine: annotations, objects, pointer policy, rendering, background presets |
| `src/system/` | operating system and data: screen capture and monitor work areas, clipboard and files, the captures folder and its thumbnails (`library`, with their disk cache in `thumbcache`), fonts, settings, desktop notifications (`notify`), and the resident's parts (`resident`, `headless`, `tray`, `hotkeys`, `prtsc_hook`, `instance`, `autostart`, `recents`, `borderless`) |
| `src/ui/` | design system: `assets` (embedded fonts and the icon font), `tokens` (colours per theme and mode), `i18n` (strings), `icon` (Lucide), `style` (egui visuals), `kit` (components), `brand` (icons, mark and the theme's wordmark), `chrome` (the theme's title bar: drag, double click, resize edges, window buttons, the native fallback), `gallery` (all of it on one screen) |
| `src/screens/` | window state and behaviour (`home`, `editor` and `overlay` hold the main window's, the editor's and the capture's state and carry out their `MainAction`s, `EditorAction`s and `CaptureAction`s; the editor's canvas is `Editor::canvas`), the pin window and the copy notice, and the resident's `shell` (main window, tray menu, About and close dialogs) |
| `src/views/` | how the screens draw, in both themes: the main window (`shell`, `home`, `style`, `settings`), the editor (bars, panel, object bar) and the capture's bars, from the state in `screens/`; they return what the user asked for as actions, which `screens/` carries out. `begin_frame` applies the look when it changes |
| `assets/` | embedded fonts with their licenses, and the brand SVG masters with the PNG, ICO and ICNS files generated from them |

Every `.rs` file starts with a `//!` doc comment (after any crate attributes) stating its responsibility; read those lines to find your way, and give every new file one. Behavior lives in `engine/` and `system/` so every window shares it.

## Interface rules

- Hover and press change color only; every widget keeps its size and position (egui `expansion` stays 0).
- The pointer shows what a click will do: arrow on bars and panels, crosshair only while drawing on the image, move over draggable objects and inside the capture area, resize on handles, text cursor while typing.
- Every user-facing string is a `Msg` key in `src/ui/i18n.rs` with English and Portuguese text; the exhaustive `match` turns a missing translation into a compile error. Show it with `t(Msg::…)`.
- Widgets come from `src/ui/kit.rs` and colours from `Tokens`; a new widget joins the kit with a size that never depends on hover, and the kit test covers it.
- Motion is limited to 80 ms color transitions and the 150 ms "Copied" notice.
- All code, assets and values are original work or come from openly licensed sources listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Objects on the picture

Editor and capture share one behaviour:

- Annotations are objects in `engine::doc::Doc`: ids, selection, transforms and an undo history of snapshots. A whole drag is one undo step (call `begin_change` once, then `translate_selected` or `resize` per frame). Selecting never changes `revision`, which is how the editor knows there is nothing to save.
- Screens route the pointer and keys through `Annotator::handle_canvas`, `handle_object_keys` and `handle_history_keys`, and set the pointer over the picture from `engine::cursor::pick` instead of picking icons by hand.
- Any tool moves the object under the pointer: `Doc::hit` is the object itself (an outline's line, a filled shape, a text), never its box, and `engine::cursor::pick` shows the move cross there; off every object the tool draws. The Text tool edits a text it clicks (the caret goes where it was clicked); a double click edits a text with any tool.
- The text being typed is an egui `TextEdit` placed over the picture by `Annotator::handle_typing`, in the text's font and size: Enter keeps it, Shift+Enter starts a line, Esc keeps it and leaves; its dashed frame moves it. Text is sized by the em in the picture too (`fonts::px_scale`), so it never jumps between live drawing and the final image.
- The wheel: a plain notch changes the width (or the text size) by one step (`wheel_notches` reads the events: egui smooths the wheel over frames); Ctrl+wheel and the pinch zoom the editor in proportion around the pointer, and the preview is rendered again 150 ms after the zoom rests, within the GPU's texture side.
- A slider drag is one undo step too: a view sends `Gesture(true)` when it is grabbed and `Gesture(false)` when let go (`Doc::begin_gesture`, `end_gesture`); a frame with no button down closes one left open. Restyles go through `Doc::restyle_selected(&Restyle)`, whose colour and opacity are apart, so a multi-selection keeps each object's other half. Picking from a list goes through `Annotator::pick`, which copies the object's style into the options, as a click on the canvas does.
- Each frame: `bake()` first (it returns annotations to render only when the picture is out of date), then the pointer, then `paint_selection`. During a drag only its start re-bakes, to take the dragged objects out, and `paint_selection` paints `painted_live()` until the drop re-bakes. A big capture never re-renders while a drag moves something; arrow nudges and Ctrl+D re-render once per press.

## The capture's bars

`engine::placement::capture_bars` places them, tested on its own: the tools in one column beside the area (right, else left) from its top, past its bottom only when the area is lower; the options panel next to them; the actions in a row under the area (over it without room) ending at its right edge. The tools' side depends on their own width alone, so they stay put while the options come and go; the options go beside them away from the area, else on the area's other side, and only with no room left do they cover the area. The view hands it the frames' drawn sizes from the frame before.

## The resident

`takeshot` with no arguments starts the resident: one process per user owns the tray icon, the global hotkeys and the main window, and a second launch only shows it. Captures, editors and pinned images run as their own processes (`--capture`, `--edit`, `--pin`).

- Event sources (hotkeys, the PrtSc hook, the tray, a second launch) only send a `ResidentEvent` and request a repaint; `App::logic` drains them, so they act while no window is shown. On Windows and macOS the root viewport is an invisible host and the main window a child viewport. On Linux the root is the main window, and Wayland cannot hide a window: `run_resident_windows` (`main.rs`) makes the main window when `Shell::wants_window` and destroys it to hide it to the tray (`Shell::hide` closes it, and `leaving` keeps that close from being asked about again); between windows `system::headless` runs one egui pass per wake-up on a context nobody sees, and the senders of `ResidentEvent`s are retargeted (`Resident::retarget`) to the window that exists. A window made again carries a fresh `Home` (its textures belong to its context), opened on the page `Shell::take_page` holds. On Wayland a Show for a window that is not in front replaces it, since a client cannot focus itself. The window is only destroyed while a panel shows the tray icon (`Resident::tray_shown`: a StatusNotifier host is registered on the session bus, `tray::shown`); with none (stock GNOME) the X only minimizes it, and a hidden start (`--minimized`) waits `headless::TRAY_WAIT` (30 s) for a panel before showing the window, so the app is never left with no way back.
- The copy and save notice is a window in the theme's look, except on Wayland, where a client gets neither a position nor always-on-top and the capture window is full screen: there `system::notify` shows it as a desktop notification (a click on a saved capture opens its folder) and the capture window closes at once.
- A capture whose selection is on screen holds `instance::CaptureSlot`; another `--capture` exits before grabbing the screen.
- Editors, pinned pictures and the capture register with the resident (`instance::register_child`) and listen on a socket of their own (`listen_for_quit`). The tray's Quit goes in two steps (`QuitRun`): it asks each whether it may close (an editor with unsaved edits asks its user), and only once all have answered tells them to close, or to stay when one cancelled, so one Cancel keeps everything open, the resident included. A child answers in `App::logic`, which runs while its window is minimized or hidden.
- A borderless, rounded, transparent window on Windows needs `borderless::flatten_borderless`: egui-winit always asks winit for the undecorated shadow, which shows as a 1 px line across the top and a square shadow around the corners.

## Verify before calling work done

1. `cargo fmt --check` prints nothing.
2. `cargo clippy --all-targets -- -D warnings` finishes with zero warnings, pre-existing ones included.
3. `mkdir -p shots` once (the folder is git-ignored), then `cargo test > shots/test.log 2>&1` and read the log. Send test output to a file and run one test command at a time: a pipe holds a hung run open.
4. For visible changes, render the screens and compare them with the design file: `design/screens/` holds the references exported from the `.fig` (`takeui-*` from page A, `c-*` the page C direction; Coral UI is the same layout, checked against the "B · Design system" frame for colours and components), and the exact measures (sizes, padding, gaps, font sizes) are on the `.fig`'s nodes, read through OpenPencil. `--screenshot out.png` saves the window after ~120 frames and works with every window: `takeshot --screenshot out.png` (main window), `takeshot --edit --screenshot out.png` (editor with a sample picture), `takeshot --capture --screenshot out.png` (capture overlay on a sample). `takeshot` alone starts the resident (close a running one first); debug builds are console programs, so a script that launches one must redirect its output or the new console window takes the focus. `--lang en|pt|auto`, `--theme takeui|coral` and `--mode light|dark|system` override the saved look for this run in every window drawn from the tokens; `--page inicio|estilo|ajustes` opens the main window on that page, `--page editor` the editor and `--page captura` the capture (both on the sample picture, the capture with an area and a mark), `--page aviso` the copy notice, and `--page componentes` the component gallery; `--captures-dir <folder>` shows another folder in Início and `--select <file name>` starts it with that capture chosen (its detail panel filled). None of them touches the saved settings. Screenshots for the repo or a PR use a sample folder, never the user's own captures. `scripts/screenshots.sh` renders the gallery, the three pages, the editor, the capture and the notice in Portuguese in all four theme/mode combinations into `shots/`, with `design/screens` as Início's captures (build with `cargo build --release` first).
5. Before a push, `scripts/check.sh` runs what CI runs: fmt, clippy and tests here, then the same on Linux in a container (`scripts/ci-linux`, needs Docker) plus clippy for macOS, on the commit being pushed. Code behind `#[cfg(...)]` for another system compiles only there, so steps 1 to 3 on Windows miss it. The pre-push hook runs it once a clone has `git config core.hooksPath .githooks`.

## Tools

- `scripts/dev.sh [takeshot arguments]`: live preview. Rebuilds into `target/dev-watch` and reopens the window whenever `src/`, `assets/` or `Cargo.toml` change (default `--page componentes`).
- Installers: `cargo-packager` 0.11.8 (`cargo install cargo-packager --version 0.11.8 --locked`) reads `[package.metadata.packager]` in `Cargo.toml`. On Windows, `cargo build --release` then `cargo packager --release --formats nsis,wix`; the NSIS template is `packaging/windows/installer.nsi` (cargo-packager's, with Takeshot's changes marked: it runs without an administrator, asks who to install for, just me under `%LOCALAPPDATA%\Programs` or every user, relaunching itself elevated only then, and the uninstaller removes the login entry). Its icon is `assets/brand/takeshot.ico` and its pictures `packaging/windows/installer-*.bmp`, rendered by `cargo run --example brand`. `scripts/linux-packages.sh` builds the deb, AppImage and tar.gz in the CI image and prints the libraries the binary needs; `scripts/linux-smoke.sh` opens them in clean Ubuntu containers (X11 and Wayland), saves what they drew, and checks that the resident's window comes and goes (`scripts/container/resident-check.sh`). `docs/testing-macos.md` is the manual check for the Mac.
- `cargo run --example brand`: renders `assets/brand/*.svg` into the PNGs, `takeshot.ico` and `Takeshot.icns`. Run it after changing an SVG and commit the outputs; `build.rs` embeds `takeshot.ico` in the Windows executable.

## Git

- Branch `feat/<topic>` or `fix/<topic>` from `main`, open a PR to `main` and fill in the template.
- Commit messages: English, [Conventional Commits](https://www.conventionalcommits.org/).
- CI (`.github/workflows/ci.yml`) runs fmt, clippy and the tests on Windows, macOS and Linux for every pull request, and builds the seven installers as artifacts when the pull request touches how they are made (`Cargo.toml`, `Cargo.lock`, `build.rs`, `packaging/`, `assets/brand/`, `.github/`); both call `.github/workflows/build.yml`.
- A release is one button: bump `version` in `Cargo.toml` in a pull request, merge it, then run Actions > release > Run workflow on `main` (or `gh workflow run release`). It tests, builds the installers, tags `vX.Y.Z` on that commit and publishes the GitHub Release; it refuses a version that is out already. No tag is pushed by hand.
