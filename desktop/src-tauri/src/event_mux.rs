//! Bounded delivery bridge for native application events.
//!
//! Python Core remains the source of the existing live event stream. This
//! mux gives future native services the same delivery boundary without
//! importing Tauri or frontend DTOs into `sky_app_core`. It deliberately maps
//! only events that already have a stable `UiEvent` representation.

use crate::ui_events::{CatalogChangedPayload, UiEvent};
use sky_app_core::events::{ApplicationEvent, EventSink};
use std::collections::VecDeque;

const MAX_BUFFERED_EVENTS: usize = 128;

#[derive(Debug, Default)]
pub(crate) struct NativeEventMux {
    buffered: VecDeque<UiEvent>,
    dropped_events: u64,
}

impl NativeEventMux {
    pub(crate) fn buffered_len(&self) -> usize {
        self.buffered.len()
    }

    pub(crate) fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    fn map_event(event: ApplicationEvent) -> UiEvent {
        match event {
            ApplicationEvent::CatalogChanged { generation, total } => UiEvent::CatalogChanged {
                // `v` is the existing delivery schema version, not an event
                // counter. Ordering is provided by the bounded queue.
                v: 1,
                payload: CatalogChangedPayload {
                    generation,
                    total: total as u64,
                },
            },
        }
    }
}

impl EventSink for NativeEventMux {
    fn publish(&mut self, event: ApplicationEvent) {
        let mapped = Self::map_event(event);

        // Catalog changes are state notifications. If one is already queued,
        // replace it with the newest generation rather than allowing a reload
        // storm to consume the bounded queue.
        if matches!(&mapped, UiEvent::CatalogChanged { .. })
            && let Some(existing) = self
                .buffered
                .iter_mut()
                .find(|event| matches!(event, UiEvent::CatalogChanged { .. }))
        {
            *existing = mapped;
            return;
        }

        if self.buffered.len() == MAX_BUFFERED_EVENTS {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }
        self.buffered.push_back(mapped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_existing_catalog_event_without_changing_wire_shape() {
        let mut mux = NativeEventMux::default();
        mux.publish(ApplicationEvent::CatalogChanged {
            generation: 4,
            total: 9,
        });
        assert_eq!(mux.buffered_len(), 1);
        assert_eq!(mux.dropped_events(), 0);
        assert!(matches!(
            mux.buffered.front(),
            Some(UiEvent::CatalogChanged { v: 1, payload })
                if payload.generation == 4 && payload.total == 9
        ));
    }

    #[test]
    fn coalesces_catalog_state_and_stays_bounded() {
        let mut mux = NativeEventMux::default();
        for generation in 1..=256 {
            mux.publish(ApplicationEvent::CatalogChanged {
                generation,
                total: generation as usize,
            });
        }
        assert_eq!(mux.buffered_len(), 1);
        assert_eq!(mux.dropped_events(), 0);
        assert!(matches!(
            mux.buffered.front(),
            Some(UiEvent::CatalogChanged { payload, .. }) if payload.generation == 256
        ));
    }
}
