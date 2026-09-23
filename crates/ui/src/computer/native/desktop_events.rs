use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSObjectProtocol, NSOperationQueue, NSString,
};

use crate::computer::desktop_lifecycle::{
    ComputerDesktopLifecycle, ComputerDesktopLifecycleHost, DesktopPowerEvent,
};

pub const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";
pub const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Center {
    Distributed,
    Workspace,
}

enum Name {
    Literal(&'static str),
    Workspace(fn() -> &'static NSNotificationName),
}

const SOURCES: [(Center, Name, DesktopPowerEvent); 6] = [
    (
        Center::Distributed,
        Name::Literal(SCREEN_LOCKED),
        DesktopPowerEvent::LockScreen,
    ),
    (
        Center::Distributed,
        Name::Literal(SCREEN_UNLOCKED),
        DesktopPowerEvent::UnlockScreen,
    ),
    (
        Center::Workspace,
        Name::Workspace(|| unsafe { NSWorkspaceWillSleepNotification }),
        DesktopPowerEvent::Suspend,
    ),
    (
        Center::Workspace,
        Name::Workspace(|| unsafe { NSWorkspaceDidWakeNotification }),
        DesktopPowerEvent::Resume,
    ),
    (
        Center::Workspace,
        Name::Workspace(|| unsafe { NSWorkspaceSessionDidResignActiveNotification }),
        DesktopPowerEvent::UserDidResignActive,
    ),
    (
        Center::Workspace,
        Name::Workspace(|| unsafe { NSWorkspaceSessionDidBecomeActiveNotification }),
        DesktopPowerEvent::UserDidBecomeActive,
    ),
];

unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

pub fn session_says_locked(session: &CFDictionary) -> bool {
    let key = CFString::from_static_string("CGSSessionScreenIsLocked");
    let Some(value) = session.find(key.as_concrete_TypeRef().cast()) else {
        return false;
    };
    let value = unsafe { CFType::wrap_under_get_rule(*value as CFTypeRef) };
    if let Some(flag) = value.downcast::<CFBoolean>() {
        return flag == CFBoolean::true_value();
    }
    value
        .downcast::<CFNumber>()
        .and_then(|number| number.to_i64())
        .is_some_and(|number| number != 0)
}

pub fn screen_already_locked() -> bool {
    let session = unsafe { CGSessionCopyCurrentDictionary() };
    if session.is_null() {
        return false;
    }
    let session: CFDictionary = unsafe { CFDictionary::wrap_under_create_rule(session) };
    session_says_locked(&session)
}

type Observers = Vec<(Center, Retained<ProtocolObject<dyn NSObjectProtocol>>)>;

thread_local! {
    static OBSERVERS: RefCell<HashMap<u64, Observers>> = RefCell::new(HashMap::new());
}

static NEXT_WATCH: AtomicU64 = AtomicU64::new(0);

pub struct DesktopLifecycleWatch {
    lifecycle: Arc<ComputerDesktopLifecycle>,
    id: u64,
}

impl DesktopLifecycleWatch {
    pub fn start(
        host: Arc<dyn ComputerDesktopLifecycleHost>,
        runtime: tokio::runtime::Handle,
        on_error: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        let lifecycle = Arc::new(ComputerDesktopLifecycle::register(
            host,
            runtime,
            on_error,
            screen_already_locked(),
        ));
        let id = NEXT_WATCH.fetch_add(1, Ordering::Relaxed);
        let observed = lifecycle.clone();
        DispatchQueue::main().exec_async(move || {
            let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
            let distributed = NSDistributedNotificationCenter::defaultCenter();
            let queue = NSOperationQueue::mainQueue();
            let observers: Observers = SOURCES
                .iter()
                .map(|(center, name, event)| {
                    let lifecycle = observed.clone();
                    let event = *event;
                    let block =
                        RcBlock::new(move |_: NonNull<NSNotification>| lifecycle.handle(event));
                    let literal;
                    let name: &NSNotificationName = match name {
                        Name::Literal(name) => {
                            literal = NSString::from_str(name);
                            &literal
                        }
                        Name::Workspace(name) => name(),
                    };
                    let target: &NSNotificationCenter = match center {
                        Center::Distributed => &distributed,
                        Center::Workspace => &workspace,
                    };
                    let token = unsafe {
                        target.addObserverForName_object_queue_usingBlock(
                            Some(name),
                            None,
                            Some(&queue),
                            &block,
                        )
                    };
                    (*center, token)
                })
                .collect();
            OBSERVERS.with(|all| all.borrow_mut().insert(id, observers));
        });
        Self { lifecycle, id }
    }
}

impl Drop for DesktopLifecycleWatch {
    fn drop(&mut self) {
        self.lifecycle.dispose();
        let id = self.id;
        DispatchQueue::main().exec_async(move || {
            let Some(observers) = OBSERVERS.with(|all| all.borrow_mut().remove(&id)) else {
                return;
            };
            let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
            let distributed = NSDistributedNotificationCenter::defaultCenter();
            for (center, token) in observers {
                let target: &NSNotificationCenter = match center {
                    Center::Distributed => &distributed,
                    Center::Workspace => &workspace,
                };
                unsafe { target.removeObserver(token.as_ref()) };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::string::CFString;

    #[test]
    fn every_gate_has_its_pause_and_resume_source() {
        let events: Vec<DesktopPowerEvent> = SOURCES.iter().map(|(_, _, event)| *event).collect();
        for event in [
            DesktopPowerEvent::LockScreen,
            DesktopPowerEvent::UnlockScreen,
            DesktopPowerEvent::Suspend,
            DesktopPowerEvent::Resume,
            DesktopPowerEvent::UserDidResignActive,
            DesktopPowerEvent::UserDidBecomeActive,
        ] {
            assert_eq!(
                events.iter().filter(|e| **e == event).count(),
                1,
                "{event:?}"
            );
        }
        let lock_center = |wanted| {
            SOURCES
                .iter()
                .find(|(_, _, event)| *event == wanted)
                .map(|(center, _, _)| *center)
        };
        assert_eq!(
            lock_center(DesktopPowerEvent::LockScreen),
            Some(Center::Distributed)
        );
        assert_eq!(
            lock_center(DesktopPowerEvent::Suspend),
            Some(Center::Workspace)
        );
        for (_, name, _) in &SOURCES {
            if let Name::Workspace(name) = name {
                assert!(name().to_string().starts_with("NSWorkspace"));
            }
        }
    }

    #[test]
    fn the_session_dictionary_reports_an_already_locked_screen() {
        let key = CFString::from_static_string("CGSSessionScreenIsLocked");
        let locked = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        assert!(session_says_locked(&locked.to_untyped()));
        let numeric =
            CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFNumber::from(1i32).as_CFType())]);
        assert!(session_says_locked(&numeric.to_untyped()));
        let unlocked = CFDictionary::from_CFType_pairs(&[(
            CFString::from_static_string("kCGSSessionOnConsoleKey").as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        assert!(!session_says_locked(&unlocked.to_untyped()));
        let _ = screen_already_locked();
    }
}
