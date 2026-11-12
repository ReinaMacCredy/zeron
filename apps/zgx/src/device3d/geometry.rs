use zeron_proto::device_hub::{DeviceOrientation, DeviceScreenConfig};

pub const IOS_PLACEHOLDER_ASPECT: f32 = 9.0 / 19.5;

pub fn display_rotation(screen: Option<&DeviceScreenConfig>) -> DeviceOrientation {
    screen
        .filter(|screen| screen.width <= screen.height)
        .map_or(DeviceOrientation::Portrait, |screen| screen.orientation)
}

pub fn raw_point(rotation: DeviceOrientation, x: f32, y: f32) -> (f32, f32) {
    match rotation {
        DeviceOrientation::LandscapeLeft => (y, 1.0 - x),
        DeviceOrientation::LandscapeRight => (1.0 - y, x),
        DeviceOrientation::PortraitUpsideDown => (1.0 - x, 1.0 - y),
        DeviceOrientation::Portrait => (x, y),
    }
}

pub fn display_point(rotation: DeviceOrientation, x: f32, y: f32) -> (f32, f32) {
    match rotation {
        DeviceOrientation::LandscapeLeft => (1.0 - y, x),
        DeviceOrientation::LandscapeRight => (y, 1.0 - x),
        DeviceOrientation::PortraitUpsideDown => (1.0 - x, 1.0 - y),
        DeviceOrientation::Portrait => (x, y),
    }
}
