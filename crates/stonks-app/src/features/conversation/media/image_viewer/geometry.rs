#[derive(Clone, Copy)]
pub(super) struct Zoom(u8);
impl Default for Zoom {
    fn default() -> Self {
        Self(4)
    }
}
impl Zoom {
    pub fn value(self) -> f32 {
        f32::from(self.0) / 4.
    }
    pub fn can_increase(self) -> bool {
        self.0 < 16
    }
    pub fn can_decrease(self) -> bool {
        self.0 > 1
    }
    pub fn change(&mut self, direction: i8) -> bool {
        let next = (i16::from(self.0) + i16::from(direction)).clamp(1, 16) as u8;
        let changed = next != self.0;
        self.0 = next;
        changed
    }
}
/// Metadata stabilizes a warm thumbnail while the full image loads. Once
/// loaded, decoded pixels are authoritative, as HTML naturalWidth/Height are.
pub(super) fn natural_size(
    metadata: Option<(u32, u32)>,
    decoded: Option<(u32, u32)>,
    ready: bool,
) -> (f32, f32) {
    let valid = |dimensions: &(u32, u32)| dimensions.0 > 0 && dimensions.1 > 0;
    let metadata = metadata.filter(valid);
    let decoded = decoded.filter(valid);
    let natural = if ready {
        decoded.or(metadata)
    } else {
        metadata.or(decoded)
    };
    natural
        .map(|(width, height)| (width as f32, height as f32))
        .unwrap_or_default()
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Insets {
    pub top: f32,
    pub side: f32,
    pub bottom: f32,
    pub chrome: f32,
}
#[derive(Debug, PartialEq)]
pub(super) struct Geometry {
    pub insets: Insets,
    pub image: (f32, f32),
    pub content: (f32, f32),
    pub image_origin: (f32, f32),
}
impl Geometry {
    pub fn new(viewport: (f32, f32), natural: (f32, f32), zoom: f32) -> Self {
        let small = viewport.0 <= 560. || viewport.1 <= 420.;
        let insets = if small {
            Insets {
                top: 64.,
                side: 12.,
                bottom: 12.,
                chrome: 10.,
            }
        } else {
            Insets {
                top: 76.,
                side: 32.,
                bottom: 32.,
                chrome: 20.,
            }
        };
        let available = (
            (viewport.0 - insets.side * 2.).max(0.),
            (viewport.1 - insets.top - insets.bottom).max(0.),
        );
        let valid =
            natural.0.is_finite() && natural.1.is_finite() && natural.0 > 0. && natural.1 > 0.;
        let fit = if valid {
            1_f32
                .min(available.0 / natural.0)
                .min(available.1 / natural.1)
        } else {
            0.
        };
        let image = if valid {
            (
                natural.0 * fit * zoom.clamp(0.25, 4.),
                natural.1 * fit * zoom.clamp(0.25, 4.),
            )
        } else {
            (0., 0.)
        };
        let content = (available.0.max(image.0), available.1.max(image.1));
        let image_origin = (
            (available.0 - image.0).max(0.) / 2.,
            (available.1 - image.1).max(0.) / 2.,
        );
        Self {
            insets,
            image,
            content,
            image_origin,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{natural_size, Geometry, Zoom};
    #[test]
    fn fits_landscape_and_portrait_without_upscaling_small_images() {
        let landscape = Geometry::new((1000., 800.), (2000., 1000.), 1.);
        assert_eq!(landscape.image, (936., 468.));
        assert_eq!(landscape.image_origin, (0., 112.));
        let portrait = Geometry::new((1000., 800.), (1000., 2000.), 1.);
        assert_eq!(portrait.image, (346., 692.));
        assert_eq!(
            Geometry::new((1000., 800.), (100., 50.), 1.).image,
            (100., 50.)
        );
    }
    #[test]
    fn zoom_is_relative_to_fit_and_scroll_content_covers_large_images() {
        let geometry = Geometry::new((1000., 800.), (2000., 1000.), 4.);
        assert_eq!(geometry.image, (3744., 1872.));
        assert_eq!(geometry.content, geometry.image);
        assert_eq!(geometry.image_origin, (0., 0.));
        let small = Geometry::new((500., 400.), (1000., 1000.), 1.);
        assert_eq!(small.insets.top, 64.);
        assert_eq!(small.image, (324., 324.));
    }
    #[test]
    fn quarter_steps_clamp_at_source_limits_without_float_drift() {
        let mut zoom = Zoom::default();
        for _ in 0..40 {
            zoom.change(-1);
        }
        assert_eq!(zoom.value(), 0.25);
        assert!(!zoom.can_decrease());
        assert!(!zoom.change(-1));
        for _ in 0..40 {
            zoom.change(1);
        }
        assert_eq!(zoom.value(), 4.);
        assert!(!zoom.can_increase());
        assert!(!zoom.change(1));
    }
    #[test]
    fn empty_natural_and_tiny_viewport_never_make_invalid_layout_dimensions() {
        let geometry = Geometry::new((10., 10.), (0., 0.), 1.);
        assert_eq!(geometry.image, (0., 0.));
        assert_eq!(geometry.content, (0., 0.));
        assert_eq!(
            Geometry::new((1000., 800.), (f32::NAN, 10.), 1.).image,
            (0., 0.)
        );
    }
    #[test]
    fn metadata_stabilizes_warm_preview_but_full_decode_supplies_natural_size() {
        assert_eq!(
            natural_size(Some((2000, 1000)), Some((1280, 640)), false),
            (2000., 1000.)
        );
        assert_eq!(
            natural_size(Some((2000, 1000)), Some((1000, 2000)), true),
            (1000., 2000.)
        );
        assert_eq!(
            natural_size(Some((0, 0)), Some((320, 200)), false),
            (320., 200.)
        );
    }
    #[test]
    fn fitted_image_preserves_stage_insets_at_live_window_size() {
        let geometry = Geometry::new((1058., 948.), (1600., 900.), 1.);
        let left = geometry.insets.side + geometry.image_origin.0;
        let right = left + geometry.image.0;
        assert!((left - 32.).abs() < 0.001);
        assert!((right - 1026.).abs() < 0.001);
        assert!((1058. - right - left).abs() < 0.001);
    }
}
