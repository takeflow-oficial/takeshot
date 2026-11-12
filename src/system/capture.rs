//! Grabbing the screen and the usable area of a monitor, plus a made-up desktop picture used for demos and checks.

use image::RgbaImage;
use tiny_skia as sk;

pub struct Shot {
    pub image: RgbaImage,
    /// Monitor position and scale, used to place the overlay window.
    pub x: i32,
    pub y: i32,
    pub scale: f32,
}

fn cursor() -> Option<(i32, i32)> {
    match mouse_position::mouse_position::Mouse::get_mouse_position() {
        mouse_position::mouse_position::Mouse::Position { x, y } => Some((x, y)),
        mouse_position::mouse_position::Mouse::Error => None,
    }
}

/// Capture the monitor under the mouse (or the primary one).
pub fn grab() -> Result<Shot, String> {
    #[cfg(target_os = "linux")]
    if use_portal(
        crate::system::resident::on_wayland(),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
    ) {
        return grab_wayland();
    }
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let mon = cursor_monitor(&monitors)
        .or_else(|| monitors.into_iter().next())
        .ok_or_else(|| "nenhum monitor encontrado".to_string())?;
    shot_of(&mon)
}

/// Capture every monitor, the one under the mouse (or the primary one) first. A monitor that
/// cannot be grabbed is left out; only the first one is required.
pub fn grab_all() -> Result<Vec<Shot>, String> {
    let mut monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    if let Some(first) = cursor_monitor(&monitors).and_then(|m| {
        let id = m.id().ok()?;
        monitors.iter().position(|o| o.id().ok() == Some(id))
    }) {
        monitors.swap(0, first);
    }
    if monitors.is_empty() {
        return Err("nenhum monitor encontrado".to_string());
    }
    first_required(monitors.iter().map(shot_of))
}

/// The grabs that worked, in order; the first one failing fails them all (it is the window the
/// user sees first), the others are skipped.
fn first_required<T, E>(grabs: impl IntoIterator<Item = Result<T, E>>) -> Result<Vec<T>, E> {
    let mut grabs = grabs.into_iter();
    let mut ok = Vec::new();
    if let Some(first) = grabs.next() {
        ok.push(first?);
    }
    ok.extend(grabs.filter_map(Result::ok));
    Ok(ok)
}

/// The monitor under the mouse, else the primary one.
fn cursor_monitor(monitors: &[xcap::Monitor]) -> Option<xcap::Monitor> {
    cursor()
        .and_then(|(x, y)| xcap::Monitor::from_point(x, y).ok())
        .or_else(|| {
            monitors
                .iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .cloned()
        })
}

fn shot_of(mon: &xcap::Monitor) -> Result<Shot, String> {
    let image = mon.capture_image().map_err(|e| e.to_string())?;
    Ok(Shot {
        image,
        x: mon.x().unwrap_or(0),
        y: mon.y().unwrap_or(0),
        scale: mon.scale_factor().unwrap_or(1.0).max(0.5),
    })
}

/// Whether `grab` asks the desktop portal instead of xcap: on a GNOME Wayland session, where
/// xcap's own Wayland path ends in a wlroots protocol GNOME lacks and hides why its portal
/// attempt failed. `desktops` is `XDG_CURRENT_DESKTOP`, a colon-separated list (`ubuntu:GNOME`).
/// KDE and wlroots keep xcap's path: the portal was not tried on them.
#[cfg(any(target_os = "linux", test))]
fn use_portal(wayland: bool, desktops: Option<&str>) -> bool {
    wayland
        && desktops.is_some_and(|list| {
            list.split(':')
                .any(|name| name.trim().eq_ignore_ascii_case("GNOME"))
        })
}

/// Wayland lets no app read the screen: the desktop portal takes the picture. The portal gives
/// the whole desktop; the monitor's part is cut from it, or with no monitor list the whole.
#[cfg(target_os = "linux")]
fn grab_wayland() -> Result<Shot, String> {
    let (desktop, interactive) = portal::screenshot()?;
    let monitors = xcap::Monitor::all().unwrap_or_default();
    let Some(mon) = cursor_monitor(&monitors).or_else(|| monitors.first().cloned()) else {
        return Ok(Shot {
            image: desktop,
            x: 0,
            y: 0,
            scale: 1.0,
        });
    };
    let frame = |m: &xcap::Monitor| {
        (
            m.x().unwrap_or(0),
            m.y().unwrap_or(0),
            m.width().unwrap_or(0),
            m.height().unwrap_or(0),
        )
    };
    let layout: Vec<_> = monitors.iter().map(frame).collect();
    // The desktop's own picker returns what the user chose there, not the whole desktop.
    let image = if interactive {
        desktop
    } else {
        let (x, y, w, h) = monitor_part((desktop.width(), desktop.height()), &layout, frame(&mon));
        image::imageops::crop_imm(&desktop, x, y, w, h).to_image()
    };
    Ok(Shot {
        image,
        x: mon.x().unwrap_or(0),
        y: mon.y().unwrap_or(0),
        scale: mon.scale_factor().unwrap_or(1.0).max(0.5),
    })
}

/// Where monitor `m` (x, y, width, height) lies in a picture of size `img` of the whole desktop
/// laid out as `monitors`. The picture may be in another scale than the monitors' coordinates
/// (Wayland's fractional scaling), so the layout is stretched to the picture's width.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn monitor_part(
    img: (u32, u32),
    monitors: &[(i32, i32, u32, u32)],
    m: (i32, i32, u32, u32),
) -> (u32, u32, u32, u32) {
    let min_x = monitors.iter().map(|r| r.0).min().unwrap_or(0);
    let min_y = monitors.iter().map(|r| r.1).min().unwrap_or(0);
    let max_x = monitors
        .iter()
        .map(|r| r.0 + r.2 as i32)
        .max()
        .unwrap_or(img.0 as i32);
    let k = img.0 as f32 / (max_x - min_x).max(1) as f32;
    let x = (((m.0 - min_x) as f32 * k).round() as u32).min(img.0);
    let y = (((m.1 - min_y) as f32 * k).round() as u32).min(img.1);
    let w = ((m.2 as f32 * k).round() as u32).min(img.0 - x);
    let h = ((m.3 as f32 * k).round() as u32).min(img.1 - y);
    (x, y, w, h)
}

/// The desktop portal's Screenshot (org.freedesktop.portal.Screenshot), over zbus.
#[cfg(target_os = "linux")]
mod portal {
    use image::RgbaImage;
    use std::collections::HashMap;
    use std::time::Duration;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    /// The desktop as a picture, and whether the desktop's own picker took it. A quiet request
    /// comes first; when the desktop refuses it (no permission), its picker asks the user.
    pub fn screenshot() -> Result<(RgbaImage, bool), String> {
        match request(false, Duration::from_secs(10)) {
            Ok(img) => Ok((img, false)),
            Err(Failure::Refused(2)) => request(true, Duration::from_secs(300))
                .map(|img| (img, true))
                .map_err(|e| e.text()),
            Err(e) => Err(e.text()),
        }
    }

    enum Failure {
        /// The portal answered with this code: 1 cancelled, 2 refused or failed.
        Refused(u32),
        Other(String),
    }

    impl Failure {
        fn text(&self) -> String {
            match self {
                Failure::Refused(1) => "the desktop's screenshot was cancelled".to_string(),
                Failure::Refused(code) => {
                    format!("the desktop refused the screenshot (portal response {code})")
                }
                Failure::Other(e) => e.clone(),
            }
        }
    }

    fn other(e: impl std::fmt::Display) -> Failure {
        Failure::Other(format!("desktop portal: {e}"))
    }

    fn request(interactive: bool, wait: Duration) -> Result<RgbaImage, Failure> {
        let (tx, rx) = std::sync::mpsc::channel();
        // A hung portal must not hang the capture: the call runs apart and is waited on.
        std::thread::spawn(move || {
            let _ = tx.send(call(interactive));
        });
        let uri = rx
            .recv_timeout(wait)
            .map_err(|_| other("no answer from the screenshot portal"))??;
        let (path, _) = gtk::glib::filename_from_uri(&uri).map_err(other)?;
        let img = image::open(&path).map(|i| i.to_rgba8()).map_err(other);
        // The portal leaves its picture as a file (GNOME, in the Pictures folder); Takeshot keeps
        // only what the user saves.
        let _ = std::fs::remove_file(&path);
        img
    }

    /// One Screenshot request; the picture's URI.
    fn call(interactive: bool) -> Result<String, Failure> {
        let bus = Connection::session().map_err(other)?;
        let token = format!("takeshot{}{}", std::process::id(), u8::from(interactive));
        let sender = bus
            .unique_name()
            .ok_or_else(|| other("no bus name"))?
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = Proxy::new(
            &bus,
            "org.freedesktop.portal.Desktop",
            path,
            "org.freedesktop.portal.Request",
        )
        .map_err(other)?;
        // Listening before asking, so a quick answer is not missed.
        let mut answers = request.receive_signal("Response").map_err(other)?;
        let portal = Proxy::new(
            &bus,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Screenshot",
        )
        .map_err(other)?;
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(true));
        options.insert("interactive", Value::from(interactive));
        let _: OwnedObjectPath = portal.call("Screenshot", &("", options)).map_err(other)?;
        let msg = answers.next().ok_or_else(|| other("no response"))?;
        let (code, results): (u32, HashMap<String, OwnedValue>) =
            msg.body().deserialize().map_err(other)?;
        if code != 0 {
            return Err(Failure::Refused(code));
        }
        results
            .get("uri")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .ok_or_else(|| other("no picture in the response"))
    }
}

fn fill(pm: &mut sk::Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: [u8; 4]) {
    let mut p = sk::Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    if let Some(path) = crate::engine::render::rounded_rect(x, y, w, h, [r; 4]) {
        pm.fill_path(
            &path,
            &p,
            sk::FillRule::Winding,
            sk::Transform::identity(),
            None,
        );
    }
}

/// A neutral, made-up app window (no real brands), for previews and the self-check.
pub fn sample_window(w: u32, h: u32) -> RgbaImage {
    let mut pm = sk::Pixmap::new(w, h).unwrap();
    let (wf, hf) = (w as f32, h as f32);
    let u = wf / 1000.0;
    fill(&mut pm, 0.0, 0.0, wf, hf, 0.0, [250, 250, 251, 255]);
    // sidebar
    fill(&mut pm, 0.0, 0.0, 210.0 * u, hf, 0.0, [242, 242, 245, 255]);
    for i in 0..7 {
        let y = 70.0 * u + i as f32 * 42.0 * u;
        let c = if i == 1 {
            [255, 228, 214, 255]
        } else {
            [0, 0, 0, 0]
        };
        fill(
            &mut pm,
            14.0 * u,
            y - 10.0 * u,
            182.0 * u,
            34.0 * u,
            8.0 * u,
            c,
        );
        fill(
            &mut pm,
            28.0 * u,
            y,
            16.0 * u,
            14.0 * u,
            4.0 * u,
            [190, 192, 200, 255],
        );
        fill(
            &mut pm,
            54.0 * u,
            y + 2.0 * u,
            (70.0 + (i * 23 % 60) as f32) * u,
            10.0 * u,
            5.0 * u,
            [170, 172, 180, 255],
        );
    }
    // header
    fill(
        &mut pm,
        240.0 * u,
        34.0 * u,
        260.0 * u,
        22.0 * u,
        6.0 * u,
        [40, 42, 48, 255],
    );
    fill(
        &mut pm,
        240.0 * u,
        68.0 * u,
        380.0 * u,
        12.0 * u,
        6.0 * u,
        [180, 182, 190, 255],
    );
    fill(
        &mut pm,
        wf - 170.0 * u,
        34.0 * u,
        130.0 * u,
        34.0 * u,
        9.0 * u,
        [255, 94, 58, 255],
    );
    // cards
    let cards = [[255, 94, 58, 255], [63, 160, 201, 255], [86, 171, 122, 255]];
    for (i, c) in cards.iter().enumerate() {
        let x = 240.0 * u + i as f32 * 245.0 * u;
        fill(
            &mut pm,
            x,
            110.0 * u,
            225.0 * u,
            120.0 * u,
            12.0 * u,
            [255, 255, 255, 255],
        );
        fill(
            &mut pm,
            x + 18.0 * u,
            130.0 * u,
            70.0 * u,
            10.0 * u,
            5.0 * u,
            [175, 177, 186, 255],
        );
        fill(
            &mut pm,
            x + 18.0 * u,
            156.0 * u,
            120.0 * u,
            26.0 * u,
            6.0 * u,
            [36, 38, 44, 255],
        );
        fill(
            &mut pm,
            x + 18.0 * u,
            198.0 * u,
            160.0 * u,
            8.0 * u,
            4.0 * u,
            [235, 236, 240, 255],
        );
        fill(
            &mut pm,
            x + 18.0 * u,
            198.0 * u,
            (60.0 + i as f32 * 40.0) * u,
            8.0 * u,
            4.0 * u,
            *c,
        );
    }
    // chart
    let top = 255.0 * u;
    fill(
        &mut pm,
        240.0 * u,
        top,
        735.0 * u,
        hf - top - 24.0 * u,
        12.0 * u,
        [255, 255, 255, 255],
    );
    let bars = [
        0.45, 0.62, 0.38, 0.8, 0.55, 0.92, 0.7, 0.5, 0.66, 0.84, 0.58, 0.74,
    ];
    let base = hf - 50.0 * u;
    let avail = base - top - 60.0 * u;
    for (i, b) in bars.iter().enumerate() {
        let bh = avail * b;
        let c = if i == 5 {
            [255, 94, 58, 255]
        } else {
            [214, 216, 224, 255]
        };
        fill(
            &mut pm,
            270.0 * u + i as f32 * 58.0 * u,
            base - bh,
            34.0 * u,
            bh,
            6.0 * u,
            c,
        );
    }
    crate::engine::render::from_pixmap(&pm)
}

/// A fake desktop: wallpaper with the sample window on it.
pub fn sample_desktop(w: u32, h: u32) -> RgbaImage {
    let mut pm = sk::Pixmap::new(w, h).unwrap();
    let stops = vec![
        sk::GradientStop::new(0.0, sk::Color::from_rgba8(28, 44, 70, 255)),
        sk::GradientStop::new(1.0, sk::Color::from_rgba8(92, 120, 140, 255)),
    ];
    if let Some(sh) = sk::LinearGradient::new(
        sk::Point::from_xy(0.0, 0.0),
        sk::Point::from_xy(w as f32, h as f32),
        stops,
        sk::SpreadMode::Pad,
        sk::Transform::identity(),
    ) {
        let p = sk::Paint {
            shader: sh,
            ..Default::default()
        };
        pm.fill_rect(
            sk::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap(),
            &p,
            sk::Transform::identity(),
            None,
        );
    }
    let win = sample_window((w as f32 * 0.62) as u32, (h as f32 * 0.6) as u32);
    let wpm = crate::engine::render::to_pixmap(&win);
    pm.draw_pixmap(
        (w as f32 * 0.19) as i32,
        (h as f32 * 0.17) as i32,
        wpm.as_ref(),
        &sk::PixmapPaint::default(),
        sk::Transform::identity(),
        None,
    );
    // taskbar
    fill(
        &mut pm,
        0.0,
        h as f32 - 44.0,
        w as f32,
        44.0,
        0.0,
        [18, 20, 26, 235],
    );
    crate::engine::render::from_pixmap(&pm)
}

pub fn sample_shot() -> Shot {
    Shot {
        image: sample_desktop(1600, 900),
        x: 0,
        y: 0,
        scale: 1.0,
    }
}

/// The area without the taskbar of the monitor nearest to `p`, in physical pixels: the tray menus
/// and the notice stay inside it. `None` where the system does not say (callers use the whole monitor).
#[cfg(windows)]
pub fn work_area(p: egui::Pos2) -> Option<egui::Rect> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    let pt = POINT {
        x: p.x as i32,
        y: p.y as i32,
    };
    // SAFETY: plain Win32 queries; MONITORINFO is zeroed and its size set as the API requires.
    unsafe {
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let r = info.rcWork;
        Some(egui::Rect::from_min_max(
            egui::pos2(r.left as f32, r.top as f32),
            egui::pos2(r.right as f32, r.bottom as f32),
        ))
    }
}

#[cfg(not(windows))]
pub fn work_area(_p: egui::Pos2) -> Option<egui::Rect> {
    None
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn the_work_area_is_part_of_the_nearest_monitor() {
        use super::work_area;
        // Any point, even far off screen, gets the nearest monitor and its area without the taskbar.
        for p in [egui::pos2(0.0, 0.0), egui::pos2(-10000.0, -10000.0)] {
            let r = work_area(p).expect("a monitor");
            assert!(r.width() > 0.0 && r.height() > 0.0, "{r:?}");
        }
    }

    #[test]
    fn only_a_gnome_wayland_session_asks_the_portal() {
        use super::use_portal;
        assert!(use_portal(true, Some("ubuntu:GNOME")));
        assert!(use_portal(true, Some("GNOME")));
        assert!(use_portal(true, Some("gnome-classic:gnome")));
        assert!(!use_portal(false, Some("ubuntu:GNOME")), "X11 keeps xcap");
        assert!(!use_portal(true, Some("KDE")), "KDE keeps xcap");
        assert!(!use_portal(true, Some("sway")), "wlroots keeps xcap");
        assert!(!use_portal(true, Some("X-Cinnamon")));
        assert!(!use_portal(true, None));
    }

    #[test]
    fn the_monitor_part_follows_the_layout_and_the_picture_scale() {
        use super::monitor_part;
        let two = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
        // Same scale: the second monitor's own rectangle.
        assert_eq!(
            monitor_part((3200, 1080), &two, two[1]),
            (1920, 0, 1280, 1024)
        );
        // A picture twice the layout's size: everything doubles.
        assert_eq!(
            monitor_part((6400, 2160), &two, two[1]),
            (3840, 0, 2560, 2048)
        );
        // A monitor left of the origin.
        let left = [(-1280, 0, 1280, 1024), (0, 0, 1920, 1080)];
        assert_eq!(
            monitor_part((3200, 1080), &left, left[1]),
            (1280, 0, 1920, 1080)
        );
        // A layout larger than the picture is cut to it.
        assert_eq!(
            monitor_part((1920, 500), &two[..1], two[0]),
            (0, 0, 1920, 500)
        );
    }

    #[test]
    fn a_monitor_that_cannot_be_grabbed_does_not_cost_the_others() {
        fn r(v: &[Result<u8, &'static str>]) -> Result<Vec<u8>, &'static str> {
            super::first_required(v.iter().cloned())
        }
        assert_eq!(r(&[Ok(1), Ok(2), Ok(3)]), Ok(vec![1, 2, 3]));
        assert_eq!(r(&[Ok(1), Err("x"), Ok(3)]), Ok(vec![1, 3]));
        assert_eq!(r(&[Ok(1), Err("x")]), Ok(vec![1]));
        assert_eq!(
            r(&[Err("first"), Ok(2)]),
            Err("first"),
            "the first is required"
        );
    }

    /// Needs a real display: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn grabs_the_screen() {
        let s = super::grab().expect("capture");
        assert!(s.image.width() > 100 && s.image.height() > 100);
        crate::system::actions::save_png(&s.image, std::path::Path::new("shots/real-capture.png"))
            .unwrap();
    }
}
