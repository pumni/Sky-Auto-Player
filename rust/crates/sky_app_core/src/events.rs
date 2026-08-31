//! Bounded application events consumed by delivery adapters.

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationEvent {
    CatalogChanged { generation: u64, total: usize },
}

pub trait EventSink {
    fn publish(&mut self, event: ApplicationEvent);
}

#[derive(Debug, Default)]
pub struct VecEventSink {
    events: Vec<ApplicationEvent>,
}

impl VecEventSink {
    pub fn into_events(self) -> Vec<ApplicationEvent> {
        self.events
    }
}

impl EventSink for VecEventSink {
    fn publish(&mut self, event: ApplicationEvent) {
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_domain_values_without_delivery_types() {
        let mut sink = VecEventSink::default();
        sink.publish(ApplicationEvent::CatalogChanged {
            generation: 2,
            total: 3,
        });
        assert_eq!(sink.into_events().len(), 1);
    }
}
