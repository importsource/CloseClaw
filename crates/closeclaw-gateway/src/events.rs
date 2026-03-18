use closeclaw_core::types::Event;
use tokio::sync::broadcast;

/// System-wide event bus backed by a tokio broadcast channel.
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn sender(&self) -> broadcast::Sender<Event> {
        self.tx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: Event) {
        // Ignore send errors (no receivers)
        let _ = self.tx.send(event);
    }
}
