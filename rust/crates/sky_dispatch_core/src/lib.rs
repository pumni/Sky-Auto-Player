//! Core dispatch domain logic for Sky Auto Player.
//! Pure Rust — no PyO3, no Windows API bindings.

#![forbid(unsafe_code)]

pub mod clock;
pub mod compile;
pub mod coordinator;
pub mod model;
pub mod testing;
pub mod time;

pub const SCHEMA_VERSION: u32 = 4;

pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_version() {
        assert_eq!(core_version(), "0.1.0");
        assert_eq!(SCHEMA_VERSION, 4);
    }
}
