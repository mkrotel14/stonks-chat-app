use gpui_kit::{point, px, size, App, Bounds, Pixels, WindowBounds};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::PathBuf};

pub(super) const MIN_WIDTH: f32 = 360.;
pub(super) const MIN_HEIGHT: f32 = 480.;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
struct Geometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    maximized: bool,
}

pub(super) struct WindowState {
    path: Option<PathBuf>,
    current: Geometry,
    written: Option<Geometry>,
}

impl WindowState {
    pub(super) fn load(cx: &App, persist: bool) -> Self {
        let path = persist
            .then(super::config_home)
            .and_then(Result::ok)
            .map(|dir| dir.join(super::APP_ID).join("window.json"));
        let written = path.as_ref().and_then(|path| {
            let mut data = Vec::new();
            File::open(path)
                .ok()?
                .take(4097)
                .read_to_end(&mut data)
                .ok()?;
            (data.len() <= 4096).then_some(())?;
            serde_json::from_slice::<Geometry>(&data)
                .ok()
                .filter(Geometry::valid)
        });
        let default = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        let current = written.unwrap_or_else(|| Geometry::from_bounds(default, false));
        Self {
            path,
            current,
            written,
        }
    }

    pub(super) fn bounds(&self, cx: &App) -> WindowBounds {
        let mut bounds = self.current.bounds();
        // Wayland positions its own toplevels. On X11 recenter if the previous
        // monitor disappeared, including while the app was hidden in the tray.
        if !cx.displays().iter().any(|display| {
            display
                .bounds()
                .contains(&point(bounds.origin.x + px(48.), bounds.origin.y + px(24.)))
        }) {
            bounds = Bounds::centered(None, bounds.size, cx);
        }
        if self.current.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }
    }

    pub(super) fn capture(&mut self, bounds: WindowBounds) {
        match bounds {
            WindowBounds::Windowed(bounds) => {
                let candidate = Geometry::from_bounds(bounds, false);
                if candidate.valid() {
                    self.current = candidate;
                }
            }
            // X11 can report the maximized rectangle, not the restore rectangle.
            WindowBounds::Maximized(_) => self.current.maximized = true,
            WindowBounds::Fullscreen(_) => {}
        }
    }

    pub(super) fn flush(&mut self) {
        let Some(path) = &self.path else { return };
        if self.written == Some(self.current) {
            return;
        }
        match serde_json::to_vec(&self.current)
            .map_err(std::io::Error::other)
            .and_then(|bytes| super::atomic_write(path, &bytes))
        {
            Ok(()) => self.written = Some(self.current),
            Err(error) => eprintln!("Stonks window state save failed: {}", error.kind()),
        }
    }
}

impl Geometry {
    fn from_bounds(bounds: Bounds<Pixels>, maximized: bool) -> Self {
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
            maximized,
        }
    }

    fn bounds(self) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        }
    }

    fn valid(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (MIN_WIDTH..=16384.).contains(&self.width)
            && (MIN_HEIGHT..=16384.).contains(&self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_maximized_without_overwriting_restore_size_or_saving_fullscreen() {
        let normal = Geometry {
            x: -100.,
            y: 50.,
            width: 1000.,
            height: 700.,
            maximized: false,
        };
        let mut state = WindowState {
            path: None,
            current: normal,
            written: None,
        };
        state.capture(WindowBounds::Fullscreen(Bounds::default()));
        assert_eq!(state.current, normal);
        state.capture(WindowBounds::Maximized(Bounds::default()));
        assert_eq!(
            state.current,
            Geometry {
                maximized: true,
                ..normal
            }
        );
        state.capture(WindowBounds::Windowed(Bounds::default()));
        assert!(state.current.maximized);
        state.capture(WindowBounds::Windowed(normal.bounds()));
        assert_eq!(state.current, normal);
    }
}
