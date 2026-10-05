use gpui::{Bounds, Pixels, Point, point, px, size};
use zeron_proto::device_hub::{DeviceOrientation, DeviceScreenConfig};

pub const IOS_PLACEHOLDER_ASPECT: f32 = 9.0 / 19.5;

pub fn fit_device_frame(aspect: f32, width: f32, height: f32, right_inset: f32) -> (f32, f32) {
    let available = (width - right_inset).max(0.0);
    if available == 0.0 || height == 0.0 {
        return (0.0, 0.0);
    }
    let by_height = (height * aspect, height);
    if by_height.0 <= available {
        by_height
    } else {
        (available, available / aspect)
    }
}

pub fn centered_frame(bounds: Bounds<Pixels>, aspect: f32) -> Bounds<Pixels> {
    let (width, height) = fit_device_frame(
        aspect,
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
        0.0,
    );
    let origin = point(
        bounds.origin.x + (bounds.size.width - px(width)) / 2.0,
        bounds.origin.y + (bounds.size.height - px(height)) / 2.0,
    );
    Bounds::new(origin, size(px(width), px(height)))
}

pub fn normalized(frame: Bounds<Pixels>, position: Point<Pixels>) -> (f32, f32) {
    let ratio = |offset: Pixels, extent: Pixels| {
        if extent <= px(0.0) {
            0.0
        } else {
            (f32::from(offset) / f32::from(extent)).clamp(0.0, 1.0)
        }
    };
    (
        ratio(position.x - frame.origin.x, frame.size.width),
        ratio(position.y - frame.origin.y, frame.size.height),
    )
}

pub fn position_at(frame: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    point(
        frame.origin.x + frame.size.width * x.clamp(0.0, 1.0),
        frame.origin.y + frame.size.height * y.clamp(0.0, 1.0),
    )
}

pub fn raw_point(screen: Option<&DeviceScreenConfig>, x: f32, y: f32) -> (f32, f32) {
    let Some(screen) = screen.filter(|screen| screen.width <= screen.height) else {
        return (x, y);
    };
    match screen.orientation {
        DeviceOrientation::LandscapeLeft => (y, 1.0 - x),
        DeviceOrientation::LandscapeRight => (1.0 - y, x),
        DeviceOrientation::PortraitUpsideDown => (1.0 - x, 1.0 - y),
        DeviceOrientation::Portrait => (x, y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn fits_a_flat_device_beside_the_fixed_controls_rail() {
        let (width, height) = fit_device_frame(IOS_PLACEHOLDER_ASPECT, 480.0, 1000.0, 56.0);
        assert!(width + 56.0 <= 480.0);
        assert!(height <= 1000.0);
        assert!(close(width / height, IOS_PLACEHOLDER_ASPECT));
    }

    #[test]
    fn height_limited_first_then_width_limited_and_empty_when_no_room() {
        assert_eq!(fit_device_frame(0.5, 1000.0, 400.0, 0.0), (200.0, 400.0));
        assert_eq!(fit_device_frame(0.5, 100.0, 400.0, 0.0), (100.0, 200.0));
        assert_eq!(fit_device_frame(0.5, 56.0, 400.0, 56.0), (0.0, 0.0));
        assert_eq!(fit_device_frame(0.5, 100.0, 0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn the_frame_is_centred_and_points_normalize_with_clamping() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(400.0), px(400.0)));
        let frame = centered_frame(bounds, 0.5);
        assert_eq!(frame.origin, point(px(110.0), px(20.0)));
        assert_eq!(frame.size, size(px(200.0), px(400.0)));
        assert_eq!(normalized(frame, point(px(160.0), px(220.0))), (0.25, 0.5));
        assert_eq!(normalized(frame, point(px(0.0), px(900.0))), (0.0, 1.0));
        assert_eq!(position_at(frame, 0.25, 0.5), point(px(160.0), px(220.0)));
    }

    #[test]
    fn rotations_remap_into_the_raw_portrait_framebuffer() {
        let screen = |orientation, width, height| DeviceScreenConfig {
            width,
            height,
            orientation,
            ..Default::default()
        };
        let left = screen(DeviceOrientation::LandscapeLeft, 588, 1280);
        let right = screen(DeviceOrientation::LandscapeRight, 588, 1280);
        let down = screen(DeviceOrientation::PortraitUpsideDown, 588, 1280);
        let up = screen(DeviceOrientation::Portrait, 588, 1280);
        assert_eq!(raw_point(Some(&left), 0.25, 0.75), (0.75, 0.75));
        assert_eq!(raw_point(Some(&right), 0.25, 0.75), (0.25, 0.25));
        assert_eq!(raw_point(Some(&down), 0.25, 0.75), (0.75, 0.25));
        assert_eq!(raw_point(Some(&up), 0.25, 0.75), (0.25, 0.75));
        assert_eq!(raw_point(None, 0.25, 0.75), (0.25, 0.75));
        let already_landscape = screen(DeviceOrientation::LandscapeLeft, 1280, 588);
        assert_eq!(
            raw_point(Some(&already_landscape), 0.25, 0.75),
            (0.25, 0.75)
        );
    }
}
