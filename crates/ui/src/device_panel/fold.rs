use std::time::Duration;

use gpui::{Context, Entity, SharedString, Subscription, Task};
use zeron_proto::device_hub::{
    DeviceFoldState, DevicePlatform, DeviceScreenConfig, FoldPosture, HingePose,
};
use zeron_rpc::methods;

use super::stream_view::{DeviceStreamView, StreamChanged};
use crate::icons;
use crate::state::EngineHandle;

const FOLD_RETRY: Duration = Duration::from_secs(3);

pub struct PoseButton {
    pub pose: HingePose,
    pub id: &'static str,
    pub label: &'static str,
    pub tip: &'static str,
    pub glyph: &'static str,
}

const fn pose(
    pose: HingePose,
    id: &'static str,
    label: &'static str,
    tip: &'static str,
    glyph: &'static str,
) -> PoseButton {
    PoseButton {
        pose,
        id,
        label,
        tip,
        glyph,
    }
}

pub const FOLD_SHAPES: [PoseButton; 3] = [
    pose(
        HingePose::Closed,
        "device-pose-closed",
        "Closed stand",
        "Closed",
        icons::DUO_CLOSED,
    ),
    pose(
        HingePose::Book,
        "device-pose-book",
        "Book stand",
        "Book / bookshelf",
        icons::DUO_BOOK,
    ),
    pose(
        HingePose::Open,
        "device-pose-open",
        "Open stand",
        "Open",
        icons::DUO_OPEN,
    ),
];

pub const STANCES: [PoseButton; 2] = [
    pose(
        HingePose::Laptop,
        "device-pose-laptop",
        "Laptop stand",
        "Laptop",
        icons::DUO_LAPTOP,
    ),
    pose(
        HingePose::Tent,
        "device-pose-tent",
        "Tent stand",
        "Tent",
        icons::DUO_TENT,
    ),
];

pub fn hinged(screen: Option<&DeviceScreenConfig>) -> bool {
    screen.is_some_and(|screen| screen.supports_hinge_angle == Some(true))
}

pub fn duo_selected(screen: &DeviceScreenConfig, pose: HingePose) -> bool {
    match pose {
        HingePose::Laptop | HingePose::Tent => screen.hinge_pose == Some(pose),
        _ => {
            let fold = screen.hinge_angle.map(|angle| match angle {
                0 => HingePose::Closed,
                180 => HingePose::Open,
                _ => HingePose::Book,
            });
            fold == Some(pose)
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct HingeQueue {
    active: Option<HingePose>,
    queued: Option<HingePose>,
}

impl HingeQueue {
    pub fn enqueue(&mut self, pose: HingePose) -> Option<HingePose> {
        if self.active.is_some() {
            self.queued = Some(pose);
            return None;
        }
        self.active = Some(pose);
        self.active
    }

    pub fn finish(&mut self, ok: bool) -> Option<HingePose> {
        self.active = self.queued.take().filter(|_| ok);
        self.active
    }

    pub fn pending(&self) -> bool {
        self.active.is_some()
    }
}

pub struct FoldControls {
    engine: EngineHandle,
    host_id: String,
    device_id: String,
    platform: DevicePlatform,
    hinge: HingeQueue,
    android: Option<DeviceFoldState>,
    android_pending: bool,
    read_for: Option<Option<(u32, u32)>>,
    error: Option<SharedString>,
    request: Task<()>,
    _view: Subscription,
}

impl FoldControls {
    pub fn new(
        engine: EngineHandle,
        host_id: String,
        device_id: String,
        platform: DevicePlatform,
        view: &Entity<DeviceStreamView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observer = cx.subscribe(view, |fold, view, _: &StreamChanged, cx| {
            let (live, size) = {
                let view = view.read(cx);
                (view.input_live(), view.frame_size())
            };
            fold.view_changed(live, size, cx);
        });
        Self {
            engine,
            host_id,
            device_id,
            platform,
            hinge: HingeQueue::default(),
            android: None,
            android_pending: false,
            read_for: None,
            error: None,
            request: Task::ready(()),
            _view: observer,
        }
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn android(&self) -> Option<&DeviceFoldState> {
        self.android.as_ref().filter(|fold| fold.supported)
    }

    pub fn android_pending(&self) -> bool {
        self.android_pending
    }

    pub fn set_pose(&mut self, pose: HingePose, cx: &mut Context<Self>) {
        if let Some(pose) = self.hinge.enqueue(pose) {
            self.error = None;
            self.send_pose(pose, cx);
        }
        cx.notify();
    }

    fn send_pose(&mut self, pose: HingePose, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let params = serde_json::json!({ "deviceId": self.device_id, "pose": pose });
        self.request = cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(methods::SET_HUB_DEVICE_HINGE, params)
                .await
                .map_err(|error| error.to_string());
            this.update(cx, |fold, cx| {
                if let Err(error) = &reply {
                    fold.error = Some(error.clone().into());
                }
                if let Some(next) = fold.hinge.finish(reply.is_ok()) {
                    fold.send_pose(next, cx);
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn view_changed(&mut self, live: bool, size: Option<(u32, u32)>, cx: &mut Context<Self>) {
        if self.platform != DevicePlatform::Android
            || !live
            || self.android_pending
            || self.read_for == Some(size)
        {
            return;
        }
        self.read_for = Some(size);
        self.fold_request(None, cx);
    }

    pub fn set_android(&mut self, posture: FoldPosture, cx: &mut Context<Self>) {
        if self.android_pending {
            return;
        }
        self.android_pending = true;
        self.error = None;
        self.fold_request(Some(posture), cx);
        cx.notify();
    }

    fn fold_request(&mut self, posture: Option<FoldPosture>, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let params = serde_json::json!({
            "hostId": self.host_id,
            "deviceId": self.device_id,
            "posture": posture,
        });
        self.request = cx.spawn(async move |this, cx| {
            loop {
                let reply = engine
                    .client()
                    .call(methods::HUB_DEVICE_FOLD, params.clone())
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|value| {
                        serde_json::from_value::<DeviceFoldState>(value).map_err(|e| e.to_string())
                    });
                let retry = this
                    .update(cx, |fold, cx| {
                        cx.notify();
                        match (reply, posture) {
                            (Ok(state), _) => {
                                fold.android = Some(state);
                                fold.android_pending = false;
                                false
                            }
                            (Err(error), Some(_)) => {
                                fold.error = Some(error.into());
                                fold.android_pending = false;
                                false
                            }
                            (Err(_), None) => true,
                        }
                    })
                    .unwrap_or(false);
                if !retry {
                    return;
                }
                cx.background_executor().timer(FOLD_RETRY).await;
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn hinge_pending(&self) -> bool {
        self.hinge.pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(hinge_angle: Option<u8>, hinge_pose: Option<HingePose>) -> DeviceScreenConfig {
        DeviceScreenConfig {
            supports_hinge_angle: Some(true),
            hinge_angle,
            hinge_pose,
            ..Default::default()
        }
    }

    #[test]
    fn the_selected_pose_follows_the_hinge_angle_and_the_stance() {
        let selected = |screen: &DeviceScreenConfig| {
            FOLD_SHAPES
                .iter()
                .chain(&STANCES)
                .filter(|button| duo_selected(screen, button.pose))
                .map(|button| button.tip)
                .collect::<Vec<_>>()
        };
        assert!(selected(&screen(None, None)).is_empty(), "fresh boot");
        assert_eq!(
            selected(&screen(Some(0), Some(HingePose::Closed))),
            ["Closed"]
        );
        assert_eq!(
            selected(&screen(Some(180), Some(HingePose::Open))),
            ["Open"]
        );
        let book = "Book / bookshelf";
        assert_eq!(selected(&screen(Some(90), Some(HingePose::Book))), [book]);
        assert_eq!(
            selected(&screen(Some(90), Some(HingePose::Laptop))),
            [book, "Laptop"]
        );
        assert_eq!(
            selected(&screen(Some(80), Some(HingePose::Tent))),
            [book, "Tent"]
        );
        assert_eq!(selected(&screen(Some(45), None)), [book]);
        assert!(hinged(Some(&screen(None, None))));
        assert!(!hinged(Some(&DeviceScreenConfig::default())));
        assert!(!hinged(None));
    }

    #[test]
    fn one_pose_is_in_flight_the_newest_click_waits_and_a_failure_drops_it() {
        let mut queue = HingeQueue::default();
        assert_eq!(queue.enqueue(HingePose::Closed), Some(HingePose::Closed));
        assert!(queue.pending());
        assert_eq!(queue.enqueue(HingePose::Book), None);
        assert_eq!(queue.enqueue(HingePose::Tent), None, "replaces Book");
        assert_eq!(queue.finish(true), Some(HingePose::Tent));
        assert_eq!(queue.finish(true), None);
        assert!(!queue.pending());

        assert_eq!(queue.enqueue(HingePose::Open), Some(HingePose::Open));
        assert_eq!(queue.enqueue(HingePose::Laptop), None);
        assert_eq!(queue.finish(false), None, "an error drops the queued pose");
        assert_eq!(queue, HingeQueue::default());
    }
}
