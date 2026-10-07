#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayFrame {
    pub id: u32,
    // Quartz global points: top-left origin at the primary display, y down.
    pub bounds: [f64; 4],
    pub scale: f64,
    pub primary: bool,
}

impl DisplayFrame {
    pub fn origin(&self) -> (f64, f64) {
        (self.bounds[0], self.bounds[1])
    }

    pub fn size(&self) -> (f64, f64) {
        (self.bounds[2], self.bounds[3])
    }

    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        let [bx, by, bw, bh] = self.bounds;
        x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    fn distance_to(&self, (x, y): (f64, f64)) -> f64 {
        let [bx, by, bw, bh] = self.bounds;
        let dx = (bx - x).max(0.0).max(x - (bx + bw));
        let dy = (by - y).max(0.0).max(y - (by + bh));
        dx.hypot(dy)
    }
}

pub fn display_for_point(displays: &[DisplayFrame], point: (f64, f64)) -> Option<&DisplayFrame> {
    displays.iter().min_by(|a, b| {
        a.distance_to(point)
            .partial_cmp(&b.distance_to(point))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

pub fn cocoa_frame(displays: &[DisplayFrame], bounds: [f64; 4]) -> [f64; 4] {
    let primary_height = displays
        .iter()
        .find(|d| d.primary)
        .map(|d| d.bounds[3])
        .unwrap_or(bounds[1] + bounds[3]);
    [
        bounds[0],
        primary_height - (bounds[1] + bounds[3]),
        bounds[2],
        bounds[3],
    ]
}

pub fn active_displays() -> Vec<DisplayFrame> {
    use core_graphics::display::{CGDisplay, CGDisplayBounds, CGMainDisplayID};
    let primary = unsafe { CGMainDisplayID() };
    CGDisplay::active_displays()
        .unwrap_or_default()
        .into_iter()
        .map(|id| {
            let rect = unsafe { CGDisplayBounds(id) };
            let scale = crate::tools::get_screen_size::get_backing_scale(id);
            DisplayFrame {
                id,
                bounds: [
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                ],
                scale: if scale > 0.0 { scale } else { 1.0 },
                primary: id == primary,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reina_layout() -> Vec<DisplayFrame> {
        vec![
            DisplayFrame {
                id: 1,
                bounds: [0.0, 0.0, 1512.0, 982.0],
                scale: 2.0,
                primary: true,
            },
            DisplayFrame {
                id: 2,
                bounds: [-44.0, -1080.0, 1920.0, 1080.0],
                scale: 1.0,
                primary: false,
            },
        ]
    }

    #[test]
    fn points_pick_the_display_that_shows_them() {
        let displays = reina_layout();
        assert_eq!(display_for_point(&displays, (700.0, 500.0)).unwrap().id, 1);
        assert_eq!(display_for_point(&displays, (500.0, -500.0)).unwrap().id, 2);
        assert_eq!(display_for_point(&displays, (-30.0, -2.0)).unwrap().id, 2);
        assert_eq!(display_for_point(&displays, (1700.0, 400.0)).unwrap().id, 1);
        assert_eq!(
            display_for_point(&displays, (1700.0, -400.0)).unwrap().id,
            2
        );
    }

    #[test]
    fn quartz_bounds_flip_into_appkit_frames() {
        let displays = reina_layout();
        assert_eq!(
            cocoa_frame(&displays, displays[0].bounds),
            [0.0, 0.0, 1512.0, 982.0]
        );
        assert_eq!(
            cocoa_frame(&displays, displays[1].bounds),
            [-44.0, 982.0, 1920.0, 1080.0]
        );
        let below = [200.0, 982.0, 1280.0, 800.0];
        assert_eq!(
            cocoa_frame(&displays, below),
            [200.0, -800.0, 1280.0, 800.0]
        );
    }

    #[test]
    #[ignore = "reads the live display layout; run with --ignored on a Mac"]
    fn live_layout_round_trips_every_display() {
        let displays = active_displays();
        for d in &displays {
            let centre = (
                d.bounds[0] + d.bounds[2] / 2.0,
                d.bounds[1] + d.bounds[3] / 2.0,
            );
            assert_eq!(display_for_point(&displays, centre).unwrap().id, d.id);
            println!(
                "display {} bounds {:?} scale {} cocoa {:?}",
                d.id,
                d.bounds,
                d.scale,
                cocoa_frame(&displays, d.bounds)
            );
        }
    }
}
