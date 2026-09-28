use std::cell::{Cell, RefCell};

use dispatch2::DispatchQueue;
use futures::channel::mpsc::UnboundedSender;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSColor, NSEventModifierFlags, NSImage, NSImageSymbolConfiguration, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSArray, NSNumber, NSString};

use crate::computer::presence::PresenceTone;
use crate::computer::status_menu::{MenuAction, MenuRow, StatusMenu};

const PULSE: &str = "zeron-pulse";

struct TargetState {
    actions: RefCell<Vec<MenuAction>>,
    sender: UnboundedSender<MenuAction>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ZeronStatusMenuTarget"]
    #[ivars = TargetState]
    struct MenuTarget;

    unsafe impl NSObjectProtocol for MenuTarget {}

    impl MenuTarget {
        #[unsafe(method(activate:))]
        fn activate(&self, sender: &NSMenuItem) {
            let action = usize::try_from(sender.tag())
                .ok()
                .and_then(|index| self.ivars().actions.borrow().get(index).cloned());
            if let Some(action) = action {
                let _ = self.ivars().sender.unbounded_send(action);
            }
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker, sender: UnboundedSender<MenuAction>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetState {
            actions: RefCell::default(),
            sender,
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct Item {
    status: Retained<NSStatusItem>,
    target: Retained<MenuTarget>,
    pulsing: bool,
}

thread_local! {
    static ITEM: RefCell<Option<Item>> = const { RefCell::new(None) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
}

fn menu_item(mtm: MainThreadMarker, title: &str, key: &str) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            None,
            &NSString::from_str(key),
        )
    }
}

fn build_menu(mtm: MainThreadMarker, target: &MenuTarget, rows: &[MenuRow]) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let mut actions = Vec::new();
    for row in rows {
        let item = match row {
            MenuRow::Separator => NSMenuItem::separatorItem(mtm),
            MenuRow::Header(text) | MenuRow::Info(text) => {
                let item = menu_item(mtm, text, "");
                item.setEnabled(false);
                item
            }
            MenuRow::Action {
                label,
                hint,
                action,
            } => {
                let key = if hint.as_deref() == Some("Esc") {
                    "\u{1b}"
                } else {
                    ""
                };
                let item = menu_item(mtm, label, key);
                item.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
                unsafe {
                    item.setTarget(Some(target));
                    item.setAction(Some(sel!(activate:)));
                }
                item.setTag(actions.len() as isize);
                actions.push(action.clone());
                item
            }
        };
        menu.addItem(&item);
    }
    target.ivars().actions.replace(actions);
    menu
}

fn set_pulse(item: &mut Item, mtm: MainThreadMarker, on: bool) {
    if item.pulsing == on {
        return;
    }
    let Some(button) = item.status.button(mtm) else {
        return;
    };
    button.setWantsLayer(true);
    let layer: Option<Retained<AnyObject>> = unsafe { msg_send![&*button, layer] };
    let Some(layer) = layer else {
        return;
    };
    let key = NSString::from_str(PULSE);
    if on {
        let Some(class) = AnyClass::get(c"CABasicAnimation") else {
            return;
        };
        let path = NSString::from_str("opacity");
        unsafe {
            let animation: Retained<AnyObject> = msg_send![class, animationWithKeyPath: &*path];
            let _: () = msg_send![&*animation, setFromValue: &*NSNumber::new_f64(1.0)];
            let _: () = msg_send![&*animation, setToValue: &*NSNumber::new_f64(0.35)];
            let _: () = msg_send![&*animation, setDuration: 0.7f64];
            let _: () = msg_send![&*animation, setAutoreverses: true];
            let _: () = msg_send![&*animation, setRepeatCount: f32::INFINITY];
            let _: () = msg_send![&*layer, addAnimation: &*animation, forKey: &*key];
        }
    } else {
        unsafe {
            let _: () = msg_send![&*layer, removeAnimationForKey: &*key];
        }
    }
    item.pulsing = on;
}

fn render(item: &mut Item, mtm: MainThreadMarker, menu: &StatusMenu) {
    if let Some(button) = item.status.button(mtm) {
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("cursorarrow"),
            Some(&NSString::from_str("Zeron is controlling this Mac")),
        );
        let tint = match menu.tone {
            PresenceTone::Paused => NSColor::systemYellowColor(),
            _ => NSColor::controlAccentColor(),
        };
        let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(
            &NSArray::from_retained_slice(&[tint]),
        );
        if let Some(image) = image.and_then(|image| image.imageWithSymbolConfiguration(&palette)) {
            image.setTemplate(false);
            button.setImage(Some(&image));
        }
    }
    let built = build_menu(mtm, &item.target, &menu.rows);
    item.status.setMenu(Some(&built));
    set_pulse(item, mtm, menu.tone != PresenceTone::Paused);
}

fn remove_item() {
    if let Some(item) = ITEM.with(|cell| cell.borrow_mut().take()) {
        NSStatusBar::systemStatusBar().removeStatusItem(&item.status);
    }
}

pub fn apply(menu: Option<StatusMenu>, sender: &UnboundedSender<MenuAction>) {
    let sender = sender.clone();
    DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(menu) = menu.filter(|_| !CLOSED.get()) else {
            remove_item();
            return;
        };
        ITEM.with(|cell| {
            let mut slot = cell.borrow_mut();
            let item = slot.get_or_insert_with(|| Item {
                status: NSStatusBar::systemStatusBar()
                    .statusItemWithLength(NSVariableStatusItemLength),
                target: MenuTarget::new(mtm, sender),
                pulsing: false,
            });
            render(item, mtm, &menu);
        });
    });
}

pub fn remove() {
    DispatchQueue::main().exec_async(|| {
        CLOSED.set(true);
        remove_item();
    });
}

#[cfg(feature = "appshots-fixture")]
fn fixture_menu(mtm: MainThreadMarker) -> Option<Retained<NSMenu>> {
    ITEM.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|item| item.status.menu(mtm))
    })
}

// Menu tracking inside a main-queue block would stall the main queue, so the fixture pops it from a run loop timer.
#[cfg(feature = "appshots-fixture")]
pub fn open_menu() {
    DispatchQueue::main().exec_async(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let status = ITEM.with(|cell| cell.borrow().as_ref().map(|item| item.status.clone()));
        if let (Some(status), Some(menu)) = (status, fixture_menu(mtm)) {
            unsafe {
                let _: () = msg_send![
                    &*status,
                    performSelector: sel!(popUpStatusItemMenu:),
                    withObject: &*menu,
                    afterDelay: 0.0f64
                ];
            }
        }
    });
}

#[cfg(feature = "appshots-fixture")]
pub fn close_menu() {
    DispatchQueue::main().exec_async(|| {
        if let Some(menu) = MainThreadMarker::new().and_then(fixture_menu) {
            menu.cancelTracking();
        }
    });
}

#[cfg(feature = "appshots-fixture")]
pub fn perform(title: &str) {
    let title = title.to_owned();
    DispatchQueue::main().exec_async(move || {
        if let Some(menu) = MainThreadMarker::new().and_then(fixture_menu) {
            let index = menu.indexOfItemWithTitle(&NSString::from_str(&title));
            if index >= 0 {
                menu.performActionForItemAtIndex(index);
            }
        }
    });
}
