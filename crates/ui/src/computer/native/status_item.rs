use std::cell::{Cell, RefCell};

use dispatch2::DispatchQueue;
use futures::channel::mpsc::UnboundedSender;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSColor, NSImage, NSImageSymbolConfiguration, NSMenu, NSMenuItem, NSRunningApplication,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize, NSString};

use crate::computer::presence::DrivenApp;
use crate::computer::status_menu::{MENU_TITLE, MenuAction, MenuRow, StatusMenu};

const PULSE: &str = "zeron-pulse";

struct TargetState {
    actions: Vec<MenuAction>,
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
            if let Some(action) = action_for_tag(&self.ivars().actions, sender.tag()) {
                let _ = self.ivars().sender.unbounded_send(action);
            }
        }
    }
);

impl MenuTarget {
    fn new(
        mtm: MainThreadMarker,
        actions: Vec<MenuAction>,
        sender: UnboundedSender<MenuAction>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetState { actions, sender });
        unsafe { msg_send![super(this), init] }
    }
}

struct Item {
    status: Retained<NSStatusItem>,
    sender: UnboundedSender<MenuAction>,
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

fn menu_actions(rows: &[MenuRow]) -> (Vec<Option<isize>>, Vec<MenuAction>) {
    let mut actions = Vec::new();
    let tags = rows
        .iter()
        .map(|row| match row {
            MenuRow::Action { action, .. } => {
                actions.push(action.clone());
                Some(actions.len() as isize - 1)
            }
            _ => None,
        })
        .collect();
    (tags, actions)
}

fn action_for_tag(actions: &[MenuAction], tag: isize) -> Option<MenuAction> {
    usize::try_from(tag)
        .ok()
        .and_then(|index| actions.get(index).cloned())
}

fn build_menu(
    mtm: MainThreadMarker,
    rows: &[MenuRow],
    sender: &UnboundedSender<MenuAction>,
) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let (tags, actions) = menu_actions(rows);
    let target = MenuTarget::new(mtm, actions, sender.clone());
    for (row, tag) in rows.iter().zip(tags) {
        let item = match row {
            MenuRow::Separator => NSMenuItem::separatorItem(mtm),
            MenuRow::Header(text) | MenuRow::Info(text) => {
                let item = menu_item(mtm, text, "");
                item.setEnabled(false);
                item
            }
            MenuRow::Action { label, symbol, .. } => {
                let item = menu_item(mtm, label, "");
                if let Some(image) = symbol.and_then(|name| self::symbol(name, label)) {
                    image.setTemplate(true);
                    item.setImage(Some(&image));
                }
                // NSMenuItem.target is weak; representedObject keeps this menu's own target alive with it.
                unsafe {
                    item.setTarget(Some(&target));
                    item.setRepresentedObject(Some(&target));
                    item.setAction(Some(sel!(activate:)));
                }
                item.setTag(tag.unwrap_or(-1));
                item
            }
        };
        menu.addItem(&item);
    }
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

const ICON_SIZE: f64 = 16.0;
const STACK_STEP: f64 = 10.0;
const STACK_LIMIT: usize = 3;

fn symbol(name: &str, description: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(description)),
    )
}

fn app_icon(app: &DrivenApp) -> Option<Retained<NSImage>> {
    let pid = i32::try_from(app.pid?).ok()?;
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?.icon()
}

#[allow(deprecated)]
fn stacked(cursor: &NSImage, icons: &[Retained<NSImage>]) -> Retained<NSImage> {
    let width = ICON_SIZE + 4.0 + ICON_SIZE + STACK_STEP * (icons.len() as f64 - 1.0);
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(width, ICON_SIZE + 2.0));
    image.lockFocus();
    cursor.drawInRect(NSRect::new(
        NSPoint::new(0.0, 1.0),
        NSSize::new(ICON_SIZE, ICON_SIZE),
    ));
    for (index, icon) in icons.iter().enumerate() {
        icon.drawInRect(NSRect::new(
            NSPoint::new(ICON_SIZE + 4.0 + STACK_STEP * index as f64, 1.0),
            NSSize::new(ICON_SIZE, ICON_SIZE),
        ));
    }
    image.unlockFocus();
    image
}

fn button_image(menu: &StatusMenu) -> Option<Retained<NSImage>> {
    let cursor = symbol("cursorarrow", MENU_TITLE)?;
    if !menu.active() {
        cursor.setTemplate(true);
        return Some(cursor);
    }
    let palette = NSImageSymbolConfiguration::configurationWithPaletteColors(
        &NSArray::from_retained_slice(&[NSColor::controlAccentColor()]),
    );
    let cursor = cursor.imageWithSymbolConfiguration(&palette)?;
    let icons: Vec<_> = menu
        .apps
        .iter()
        .filter_map(app_icon)
        .take(STACK_LIMIT)
        .collect();
    let image = if icons.is_empty() {
        cursor
    } else {
        stacked(&cursor, &icons)
    };
    image.setTemplate(false);
    Some(image)
}

fn render(item: &mut Item, mtm: MainThreadMarker, menu: &StatusMenu) {
    if let Some(button) = item.status.button(mtm)
        && let Some(image) = button_image(menu)
    {
        button.setImage(Some(&image));
    }
    let built = build_menu(mtm, &menu.rows, &item.sender);
    item.status.setMenu(Some(&built));
    set_pulse(item, mtm, menu.active());
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
                sender,
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
pub fn export_fixture_images(directory: std::path::PathBuf, name: String) {
    DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let write = |image: &NSImage, file: String| {
            if let Some(data) = image.TIFFRepresentation() {
                let _ = std::fs::write(directory.join(file), data.to_vec());
            }
        };
        ITEM.with(|cell| {
            let slot = cell.borrow();
            let Some(item) = slot.as_ref() else {
                return;
            };
            if let Some(image) = item.status.button(mtm).and_then(|button| button.image()) {
                write(&image, format!("{name}-button.tiff"));
            }
            if let Some(menu) = item.status.menu(mtm) {
                for index in 0..menu.numberOfItems() {
                    let Some(entry) = menu.itemAtIndex(index) else {
                        continue;
                    };
                    let image = entry.image();
                    println!(
                        "menu item {:?} image {}",
                        entry.title().to_string(),
                        image.is_some()
                    );
                    if let Some(image) = image {
                        write(&image, format!("{name}-item-{index}.tiff"));
                    }
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(label: &str, action: MenuAction) -> MenuRow {
        MenuRow::Action {
            label: label.into(),
            symbol: None,
            action,
        }
    }

    fn stop(name: &str) -> MenuAction {
        MenuAction::StopApp {
            app: DrivenApp {
                name: name.into(),
                pid: Some(42),
            },
            chats: vec!["a".into()],
        }
    }

    #[test]
    fn a_click_on_an_open_menu_resolves_only_against_that_menus_actions() {
        let shown = vec![
            MenuRow::Header(MENU_TITLE.into()),
            MenuRow::Separator,
            row("Stop Using Ghostty", stop("Ghostty")),
            MenuRow::Separator,
            row("Open chat", MenuAction::OpenChat("a".into())),
        ];
        let rebuilt = vec![
            MenuRow::Header(MENU_TITLE.into()),
            MenuRow::Separator,
            row("Stop Using Calculator", stop("Calculator")),
            row("Stop Using Ghostty", stop("Ghostty")),
            MenuRow::Separator,
            row("Open chat", MenuAction::OpenChat("a".into())),
        ];
        let (tags, actions) = menu_actions(&shown);
        let (_, rebuilt_actions) = menu_actions(&rebuilt);
        for (row, tag) in shown.iter().zip(&tags) {
            match row {
                MenuRow::Action { action, .. } => assert_eq!(
                    action_for_tag(&actions, tag.expect("an action row has a tag")).as_ref(),
                    Some(action),
                    "the shown menu keeps its own actions after a rebuild"
                ),
                _ => assert_eq!(*tag, None),
            }
        }
        let open_chat = tags[4].unwrap();
        assert_eq!(
            action_for_tag(&rebuilt_actions, open_chat),
            Some(stop("Ghostty")),
            "one shared vec would have turned Open chat into a Stop"
        );
        assert_eq!(action_for_tag(&actions, -1), None);
        assert_eq!(action_for_tag(&actions, actions.len() as isize), None);
    }
}
