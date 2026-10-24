use futures::StreamExt;
use futures::stream::BoxStream;
use nexus_api::Event;
use tokio::sync::broadcast;

/// The host event bus. Every subscriber sees every event published after it subscribed; a
/// subscriber that falls more than the buffer behind skips the oldest events.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new(buffer: usize) -> Self {
        Self { tx: broadcast::channel(buffer).0 }
    }

    pub fn publish(&self, event: Event) {
        tracing::trace!(?event, "event");
        // No subscribers is fine.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// The events as a stream, skipping any a slow reader missed.
    pub fn stream(&self) -> BoxStream<'static, Event> {
        futures::stream::unfold(self.subscribe(), |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(event) => return Some((event, rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}
