mod cleanup;
mod packet_send;
#[cfg(any(test, feature = "test-support"))]
mod packet_send_test_support;
mod preflight;
mod state;

#[cfg(any(test, feature = "test-support"))]
pub type CustomEmitterFn =
    Box<dyn Fn(&[u16], bool) -> super::outcome::PlatformSendResult + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
pub type CustomPacketEmitterFn = Box<
    dyn Fn(super::outcome::PhysicalPacket) -> super::outcome::SendTransactionOutcome + Send + Sync,
>;

/// Test-only deterministic physical probe used by the cleanup FSM.
///
/// The signature receives the still-unresolved mask and the transport-confirmed
/// mask so a test can model transport progress across retry attempts. When no
/// probe is installed, a custom emitter must never synthesize a physical
/// verdict; the probe resolves to Inconclusive (fail-closed).
#[cfg(any(test, feature = "test-support"))]
pub type CustomProbeFn =
    Box<dyn Fn(u16, u16) -> super::physical::InstrumentPhysicalState + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, AtomicU64};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseScope {
    Tracked,
    FullInstrument,
}

#[cfg(test)]
std::thread_local! {
    static TEST_RELEASE_SLEEP_COUNT_VALUE: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) struct TestReleaseSleepCount;

#[cfg(test)]
impl TestReleaseSleepCount {
    pub(crate) fn store(&self, value: usize, _ordering: std::sync::atomic::Ordering) {
        TEST_RELEASE_SLEEP_COUNT_VALUE.set(value);
    }

    pub(crate) fn load(&self, _ordering: std::sync::atomic::Ordering) -> usize {
        TEST_RELEASE_SLEEP_COUNT_VALUE.get()
    }

    fn increment(&self) {
        TEST_RELEASE_SLEEP_COUNT_VALUE.set(TEST_RELEASE_SLEEP_COUNT_VALUE.get() + 1);
    }
}

#[cfg(test)]
pub(crate) static TEST_RELEASE_SLEEP_COUNT: TestReleaseSleepCount = TestReleaseSleepCount;

#[cfg(not(test))]
fn release_retry_sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[cfg(test)]
fn release_retry_sleep(_ms: u64) {
    TEST_RELEASE_SLEEP_COUNT.increment();
}

#[derive(Default)]
pub struct TrackedKeyState {
    pub active_mask: u16,
    pub possibly_active_mask: u16,
    pub failed_release_mask: u16,
    pub last_error: Option<String>,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    pub timing_error: Option<crate::clock::QpcError>,
    #[cfg(any(test, feature = "test-support"))]
    pub custom_emitter: Option<CustomEmitterFn>,
    #[cfg(any(test, feature = "test-support"))]
    pub custom_packet_emitter: Option<CustomPacketEmitterFn>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) custom_probe: Option<CustomProbeFn>,
    #[cfg(any(test, feature = "test-support"))]
    pub full_instrument_release_calls: u64,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) full_instrument_release_counter: Option<Arc<AtomicU64>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) force_preflight_failure: Option<Arc<AtomicBool>>,
    qpc_clock: Option<crate::clock::QpcClock>,
}
