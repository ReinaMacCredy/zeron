use std::sync::Arc;

use futures::FutureExt;

use super::{ComputerFrameSubscriber, ComputerManager, ComputerManagerSubscription, error_message};
use crate::computer::backend::{
    ComputerBackendMethod, ComputerError, ComputerFrameListener, ComputerStreamFrame,
};

impl ComputerManager {
    pub fn subscribe_frames(
        &self,
        subscriber: ComputerFrameSubscriber,
    ) -> ComputerManagerSubscription {
        self.engage_backend();
        let id = self
            .next_listener
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        {
            let mut state = self.state();
            state.frame_subscribers.push((id, subscriber));
            state.stream.desired = true;
            state.stream.epoch += 1;
        }
        self.spawn_reconcile();
        ComputerManagerSubscription {
            manager: self.weak.clone(),
            id,
            frames: true,
        }
    }

    pub(super) fn unsubscribe_frames(&self, id: u64) {
        let reconcile = {
            let mut state = self.state();
            state
                .frame_subscribers
                .retain(|(candidate, _)| *candidate != id);
            if state.frame_subscribers.is_empty() {
                state.stream.desired = false;
                state.stream.epoch += 1;
                true
            } else {
                false
            }
        };
        if reconcile {
            self.spawn_reconcile();
        }
    }

    fn spawn_reconcile(&self) {
        let Some(manager) = self.strong() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let mut state = self.state();
        let previous = state.stream_tail.take();
        let next = async move {
            if let Some(previous) = previous {
                previous.await;
            }
            if let Err(error) = manager.reconcile_stream().await {
                manager.record_error(&error_message(&error));
            }
        }
        .boxed()
        .shared();
        state.stream_tail = Some(next.clone());
        runtime.spawn(next);
    }

    pub fn latest_frame(&self) -> Option<ComputerStreamFrame> {
        self.state().latest_frame.clone()
    }

    fn stream_wanted(&self, epoch: u64) -> bool {
        let state = self.state();
        !state.disposed
            && state.stream.desired
            && state.stream.epoch == epoch
            && !state.frame_subscribers.is_empty()
    }

    fn frame_listener(&self) -> ComputerFrameListener {
        let manager = self.weak.clone();
        Arc::new(move |frame: ComputerStreamFrame| {
            if let Some(manager) = manager.upgrade() {
                manager.handle_frame(frame);
            }
        })
    }

    fn reset_transport(&self) {
        self.state().latest_frame = None;
    }

    async fn reconcile_stream(&self) -> Result<(), ComputerError> {
        let _transition = self.stream_transition.lock().await;
        let (disposed, desired, attached, epoch) = {
            let state = self.state();
            (
                state.disposed,
                state.stream.desired,
                state.stream.attached,
                state.stream.epoch,
            )
        };
        if disposed || !desired {
            if !attached {
                return Ok(());
            }
            self.state().stream.attached = false;
            self.reset_transport();
            return self.backend.detach_stream().await;
        }
        if attached {
            return Ok(());
        }
        self.backend.attach_stream(self.frame_listener()).await?;
        if !self.stream_wanted(epoch) {
            self.backend.detach_stream().await?;
            self.reset_transport();
            return Ok(());
        }
        self.state().stream.attached = true;
        Ok(())
    }

    pub async fn request_keyframe(&self) -> Result<(), ComputerError> {
        let epoch = {
            let state = self.state();
            if !state.stream.attached || state.frame_subscribers.is_empty() {
                return Ok(());
            }
            state.stream.epoch
        };
        let _transition = self.stream_transition.lock().await;
        if !self.stream_wanted(epoch) || !self.state().stream.attached {
            return Ok(());
        }
        if self
            .backend
            .supports(ComputerBackendMethod::RequestKeyframe)
        {
            return self.backend.request_keyframe().await;
        }
        self.backend.detach_stream().await?;
        self.state().stream.attached = false;
        self.reset_transport();
        if !self.stream_wanted(epoch) {
            return Ok(());
        }
        self.backend.attach_stream(self.frame_listener()).await?;
        if !self.stream_wanted(epoch) {
            self.state().stream.attached = false;
            self.reset_transport();
            return self.backend.detach_stream().await;
        }
        self.state().stream.attached = true;
        Ok(())
    }

    pub async fn flush_stream_transitions(&self) {
        let tail = self.state().stream_tail.clone();
        if let Some(tail) = tail {
            tail.await;
        }
        drop(self.stream_transition.lock().await);
    }

    pub(super) async fn stream_teardown(&self) -> Result<(), ComputerError> {
        let _transition = self.stream_transition.lock().await;
        let attached = std::mem::replace(&mut self.state().stream.attached, false);
        if !attached {
            return Ok(());
        }
        self.reset_transport();
        self.backend.detach_stream().await
    }

    fn handle_frame(&self, frame: ComputerStreamFrame) {
        let subscribers: Vec<ComputerFrameSubscriber> = {
            let mut state = self.state();
            if state.disposed || (!state.stream.desired && !state.stream.attached) {
                return;
            }
            state.latest_frame = Some(frame.clone());
            state
                .frame_subscribers
                .iter()
                .map(|(_, subscriber)| subscriber.clone())
                .collect()
        };
        for subscriber in subscribers {
            subscriber(&frame);
        }
    }
}
