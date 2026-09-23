use std::ffi::c_void;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton};
use core_graphics::geometry::CGPoint;
use foreign_types::ForeignType;

pub const MOUSE_EVENT_CREATE_FAILED: &str = "mouse_event_create_failed";
pub const FLAGS_EVENT_CREATE_FAILED: &str = "flags_event_create_failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseStep {
    LeftMouseUp,
    RightMouseUp,
    OtherMouseUp,
    ClearModifiers,
}

pub const RELEASE_PLAN: [(ReleaseStep, &str); 4] = [
    (ReleaseStep::LeftMouseUp, "left_mouse_up"),
    (ReleaseStep::RightMouseUp, "right_mouse_up"),
    (ReleaseStep::OtherMouseUp, "other_mouse_up"),
    (ReleaseStep::ClearModifiers, "modifier_flags"),
];

pub fn build_release_events<E>(
    mut create: impl FnMut(ReleaseStep) -> Option<E>,
) -> Result<Vec<E>, &'static str> {
    RELEASE_PLAN
        .iter()
        .map(|(step, _)| {
            create(*step).ok_or(match step {
                ReleaseStep::ClearModifiers => FLAGS_EVENT_CREATE_FAILED,
                _ => MOUSE_EVENT_CREATE_FAILED,
            })
        })
        .collect()
}

unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> core_graphics::sys::CGEventRef;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: CGEventType,
        position: CGPoint,
        button: CGMouseButton,
    ) -> core_graphics::sys::CGEventRef;
}

fn sourceless_event() -> Option<CGEvent> {
    let event = unsafe { CGEventCreate(std::ptr::null()) };
    (!event.is_null()).then(|| unsafe { CGEvent::from_ptr(event) })
}

fn create(step: ReleaseStep, location: CGPoint) -> Option<CGEvent> {
    let (event_type, button) = match step {
        ReleaseStep::LeftMouseUp => (CGEventType::LeftMouseUp, CGMouseButton::Left),
        ReleaseStep::RightMouseUp => (CGEventType::RightMouseUp, CGMouseButton::Right),
        ReleaseStep::OtherMouseUp => (CGEventType::OtherMouseUp, CGMouseButton::Center),
        ReleaseStep::ClearModifiers => {
            let event = sourceless_event()?;
            event.set_type(CGEventType::FlagsChanged);
            event.set_flags(CGEventFlags::empty());
            return Some(event);
        }
    };
    let event = unsafe { CGEventCreateMouseEvent(std::ptr::null(), event_type, location, button) };
    (!event.is_null()).then(|| unsafe { CGEvent::from_ptr(event) })
}

pub fn release_held_input() -> Result<(), String> {
    let location = sourceless_event()
        .map(|event| event.location())
        .unwrap_or(CGPoint::new(0.0, 0.0));
    let events = build_release_events(|step| create(step, location))
        .map_err(|reason| format!("Held input release failed: {reason}."))?;
    for event in &events {
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_every_button_before_clearing_modifiers() {
        let mut created = Vec::new();
        let events = build_release_events(|step| {
            created.push(step);
            Some(step)
        })
        .unwrap();
        assert_eq!(events, created);
        assert_eq!(
            events,
            vec![
                ReleaseStep::LeftMouseUp,
                ReleaseStep::RightMouseUp,
                ReleaseStep::OtherMouseUp,
                ReleaseStep::ClearModifiers,
            ]
        );
        assert_eq!(
            RELEASE_PLAN.map(|(_, detail)| detail),
            [
                "left_mouse_up",
                "right_mouse_up",
                "other_mouse_up",
                "modifier_flags"
            ]
        );
    }

    #[test]
    fn a_failed_creation_posts_nothing_and_names_the_reason() {
        let mut calls = 0;
        let mouse = build_release_events(|step| {
            calls += 1;
            (calls != 2).then_some(step)
        });
        assert_eq!(mouse, Err(MOUSE_EVENT_CREATE_FAILED));
        let flags = build_release_events(|step| match step {
            ReleaseStep::ClearModifiers => None,
            step => Some(step),
        });
        assert_eq!(flags, Err(FLAGS_EVENT_CREATE_FAILED));
    }

    #[test]
    fn the_native_events_carry_the_planned_types() {
        let location = CGPoint::new(12.0, 34.0);
        let events = build_release_events(|step| create(step, location)).unwrap();
        let kinds: Vec<i64> = events
            .iter()
            .map(|event| unsafe { CGEventGetType(event.as_ptr()) } as i64)
            .collect();
        assert_eq!(
            kinds,
            vec![
                CGEventType::LeftMouseUp as i64,
                CGEventType::RightMouseUp as i64,
                CGEventType::OtherMouseUp as i64,
                CGEventType::FlagsChanged as i64,
            ]
        );
        assert_eq!(events[3].get_flags(), CGEventFlags::empty());
        assert_eq!(events[0].location().x, 12.0);
    }

    unsafe extern "C" {
        fn CGEventGetType(event: core_graphics::sys::CGEventRef) -> u32;
    }
}
