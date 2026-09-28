use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::time::Duration;

use dispatch2::DispatchQueue;
use gpui::{App, Task};
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSColor, NSFont, NSFontWeightSemibold, NSLineBreakMode, NSPanel, NSTextField, NSView,
    NSWindowSharingType,
};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};
use zeron_proto::computer::CuaShieldFrame;

use super::shield::{cocoa_rect, overlay_panel, primary_max_y, set_layer};
use super::window_info::{self, WindowEntry};
use crate::computer::presence::{self, RingSpec, RingTone};

const TICK: Duration = Duration::from_millis(250);
const OUTSET: f64 = 3.0;
const PILL_ROOM: f64 = 26.0;
const PILL_HEIGHT: f64 = 20.0;
const PILL_PADDING: f64 = 10.0;
const DOT: f64 = 6.0;
const RADIUS: f64 = 12.0;
const LINE: f64 = 2.0;

#[repr(C)]
struct CGPath {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGPath {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPathCreateWithRoundedRect(
        rect: NSRect,
        corner_width: f64,
        corner_height: f64,
        transform: *const c_void,
    ) -> *const CGPath;
    fn CGPathRelease(path: *const CGPath);
}

pub(super) fn placements(
    specs: &[RingSpec],
    windows: &[WindowEntry],
) -> Vec<(RingSpec, CuaShieldFrame)> {
    specs
        .iter()
        .filter_map(|spec| {
            let bounds = windows
                .iter()
                .find(|window| window.id == spec.window && window.pid == spec.pid)?
                .bounds;
            (bounds.size.width > 0.0 && bounds.size.height > 0.0).then(|| {
                (
                    spec.clone(),
                    CuaShieldFrame {
                        x: bounds.origin.x - OUTSET,
                        y: bounds.origin.y - OUTSET - PILL_ROOM,
                        width: bounds.size.width + OUTSET * 2.0,
                        height: bounds.size.height + OUTSET * 2.0 + PILL_ROOM,
                    },
                )
            })
        })
        .collect()
}

struct Ring {
    spec: RingSpec,
    panel: Retained<NSPanel>,
    shape: Retained<AnyObject>,
    pill: Retained<NSView>,
    field: Retained<NSTextField>,
    dot: Retained<NSView>,
}

thread_local! {
    static RINGS: RefCell<Vec<Ring>> = const { RefCell::new(Vec::new()) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
    static CAPTURABLE: Cell<bool> = const { Cell::new(false) };
}

fn stroke(tone: RingTone) -> Retained<NSColor> {
    match tone {
        RingTone::Accent => NSColor::controlAccentColor(),
        RingTone::Warning => NSColor::systemYellowColor(),
    }
}

fn sharing() -> NSWindowSharingType {
    if CAPTURABLE.get() {
        NSWindowSharingType::ReadOnly
    } else {
        NSWindowSharingType::None
    }
}

fn shape_layer(tone: RingTone, dashed: bool) -> Option<Retained<AnyObject>> {
    let class = AnyClass::get(c"CAShapeLayer")?;
    let layer: Retained<AnyObject> = unsafe { msg_send![class, layer] };
    let color = stroke(tone).CGColor();
    let clear = NSColor::clearColor().CGColor();
    unsafe {
        let _: () = msg_send![&*layer, setStrokeColor: &*color];
        let _: () = msg_send![&*layer, setFillColor: &*clear];
        let _: () = msg_send![&*layer, setLineWidth: LINE];
        let _: () = msg_send![&*layer, setShadowColor: &*color];
        let _: () = msg_send![&*layer, setShadowOpacity: 0.35f32];
        let _: () = msg_send![&*layer, setShadowRadius: 8.0f64];
        let _: () = msg_send![&*layer, setShadowOffset: NSSize::new(0.0, 0.0)];
        if dashed {
            let pattern =
                NSArray::from_retained_slice(&[NSNumber::new_f64(6.0), NSNumber::new_f64(4.0)]);
            let _: () = msg_send![&*layer, setLineDashPattern: &*pattern];
        }
    }
    Some(layer)
}

impl Ring {
    fn build(mtm: MainThreadMarker, spec: RingSpec, rect: NSRect) -> Option<Self> {
        let panel = overlay_panel(mtm, rect);
        panel.setSharingType(sharing());
        let content = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), rect.size),
        );
        content.setWantsLayer(true);
        let root: Option<Retained<AnyObject>> = unsafe { msg_send![&*content, layer] };
        let shape = shape_layer(spec.tone, spec.dashed)?;
        unsafe {
            let _: () = msg_send![&*root?, addSublayer: &*shape];
        }
        let color = stroke(spec.tone);
        let pill = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        set_layer(
            &pill,
            &NSColor::blackColor().colorWithAlphaComponent(0.85),
            None,
            PILL_HEIGHT / 2.0,
        );
        let pill_layer: Option<Retained<AnyObject>> = unsafe { msg_send![&*pill, layer] };
        if let Some(pill_layer) = pill_layer {
            let border = color.colorWithAlphaComponent(0.5).CGColor();
            unsafe {
                let _: () = msg_send![&*pill_layer, setBorderColor: &*border];
                let _: () = msg_send![&*pill_layer, setBorderWidth: 1.0f64];
            }
        }
        let dot = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(PILL_PADDING, (PILL_HEIGHT - DOT) / 2.0),
                NSSize::new(DOT, DOT),
            ),
        );
        set_layer(&dot, &color, None, DOT / 2.0);
        let field = NSTextField::labelWithString(&NSString::from_str(&spec.label), mtm);
        field.setFont(Some(&NSFont::systemFontOfSize_weight(11.5, unsafe {
            NSFontWeightSemibold
        })));
        field.setTextColor(Some(&NSColor::whiteColor()));
        field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        pill.addSubview(&dot);
        pill.addSubview(&field);
        content.addSubview(&pill);
        panel.setContentView(Some(&content));
        let ring = Self {
            spec,
            panel,
            shape,
            pill,
            field,
            dot,
        };
        ring.layout(rect.size);
        ring.panel.orderFrontRegardless();
        Some(ring)
    }

    fn layout(&self, size: NSSize) {
        let ring = NSRect::new(
            NSPoint::new(LINE / 2.0, LINE / 2.0),
            NSSize::new(
                (size.width - LINE).max(0.0),
                (size.height - PILL_ROOM - LINE).max(0.0),
            ),
        );
        unsafe {
            let path = CGPathCreateWithRoundedRect(ring, RADIUS, RADIUS, std::ptr::null());
            if !path.is_null() {
                let _: () = msg_send![&*self.shape, setPath: path];
                CGPathRelease(path);
            }
        }
        self.field.sizeToFit();
        let text = self.field.frame().size;
        let lead = PILL_PADDING + DOT + 6.0;
        let width = (lead + text.width + PILL_PADDING).min((size.width - 24.0).max(0.0));
        self.pill.setFrame(NSRect::new(
            NSPoint::new((size.width - width) / 2.0, size.height - PILL_HEIGHT),
            NSSize::new(width, PILL_HEIGHT),
        ));
        self.field.setFrame(NSRect::new(
            NSPoint::new(lead, (PILL_HEIGHT - text.height) / 2.0),
            NSSize::new((width - lead - PILL_PADDING).max(0.0), text.height),
        ));
        self.dot.setHidden(width < lead + PILL_PADDING);
    }

    fn update(&mut self, spec: RingSpec, rect: NSRect) {
        let resized = self.panel.frame() != rect;
        let relabeled = self.spec.label != spec.label;
        if resized {
            self.panel.setFrame_display(rect, true);
        }
        if relabeled {
            self.field.setStringValue(&NSString::from_str(&spec.label));
        }
        if resized || relabeled {
            self.layout(rect.size);
        }
        self.spec = spec;
    }

    fn close(self) {
        self.panel.orderOut(None);
        self.panel.close();
    }
}

fn apply_on_main(mtm: MainThreadMarker, placed: Vec<(RingSpec, CuaShieldFrame)>) {
    let placed = if CLOSED.get() { Vec::new() } else { placed };
    let max_y = primary_max_y(mtm);
    RINGS.with(|cell| {
        let mut rings = cell.take();
        let (mut kept, gone): (Vec<_>, Vec<_>) = rings.drain(..).partition(|ring| {
            placed.iter().any(|(spec, _)| {
                spec.chat_id == ring.spec.chat_id
                    && spec.tone == ring.spec.tone
                    && spec.dashed == ring.spec.dashed
            })
        });
        gone.into_iter().for_each(Ring::close);
        for (spec, frame) in placed {
            let rect = cocoa_rect(&frame, max_y.unwrap_or(frame.y + frame.height));
            match kept
                .iter_mut()
                .find(|ring| ring.spec.chat_id == spec.chat_id)
            {
                Some(ring) => ring.update(spec, rect),
                None => kept.extend(Ring::build(mtm, spec, rect)),
            }
        }
        cell.replace(kept);
    });
}

fn apply(placed: Vec<(RingSpec, CuaShieldFrame)>) {
    DispatchQueue::main().exec_async(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            apply_on_main(mtm, placed);
        }
    });
}

pub fn release_all() {
    DispatchQueue::main().exec_async(|| {
        CLOSED.set(true);
        if let Some(mtm) = MainThreadMarker::new() {
            apply_on_main(mtm, Vec::new());
        }
    });
}

#[cfg(feature = "appshots-fixture")]
pub fn set_capturable(capturable: bool) {
    DispatchQueue::main().exec_async(move || {
        CAPTURABLE.set(capturable);
        RINGS.with(|cell| {
            for ring in cell.borrow().iter() {
                ring.panel.setSharingType(sharing());
            }
        });
    });
}

#[derive(Default)]
struct Driver {
    specs: Vec<RingSpec>,
    ticker: Option<Task<()>>,
}

pub fn install(cx: &mut App) {
    let Some(map) = presence::map(cx) else {
        return;
    };
    let mut driver = Driver::default();
    cx.observe(&map, move |map, cx| {
        let specs = presence::ring_specs(map.read(cx).chats());
        if specs == driver.specs {
            return;
        }
        driver.specs = specs.clone();
        drop(driver.ticker.take());
        if specs.is_empty() {
            apply(Vec::new());
            return;
        }
        driver.ticker.replace(cx.spawn(async move |cx| {
            loop {
                let specs = specs.clone();
                let placed = cx
                    .background_executor()
                    .spawn(async move { placements(&specs, &window_info::on_screen_windows()) })
                    .await;
                apply(placed);
                cx.background_executor().timer(TICK).await;
            }
        }));
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};

    use super::*;

    fn spec(chat: &str, pid: i32, window: u32) -> RingSpec {
        RingSpec {
            chat_id: chat.into(),
            pid,
            window,
            tone: RingTone::Accent,
            dashed: false,
            label: "Zeron · Typing".into(),
        }
    }

    fn entry(id: u32, pid: i32, width: f64) -> WindowEntry {
        WindowEntry {
            id,
            pid,
            layer: 0,
            bounds: CGRect::new(&CGPoint::new(100.0, 200.0), &CGSize::new(width, 300.0)),
        }
    }

    #[test]
    fn a_ring_needs_its_window_on_screen_under_the_same_pid() {
        let specs = [spec("a", 42, 7), spec("b", 43, 8), spec("c", 44, 9)];
        let windows = [entry(7, 42, 400.0), entry(8, 99, 400.0), entry(9, 44, 0.0)];
        let placed = placements(&specs, &windows);
        assert_eq!(
            placed.len(),
            1,
            "a recycled id or an empty window gets no ring"
        );
        let (spec, frame) = &placed[0];
        assert_eq!(spec.chat_id, "a");
        assert_eq!(
            (frame.x, frame.y, frame.width, frame.height),
            (97.0, 200.0 - 3.0 - PILL_ROOM, 406.0, 306.0 + PILL_ROOM)
        );
        assert!(placements(&specs, &[]).is_empty(), "minimized or off Space");
    }
}
