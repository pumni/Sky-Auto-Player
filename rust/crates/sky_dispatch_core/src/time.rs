//! Opaque, checked time domains used by the native dispatch path.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Physical idle interval separating cold and hot SendInput classifications.
///
/// The same threshold is used by the production worker and the isolated
/// calibration process. It is intentionally expressed at the configuration
/// boundary; callers convert it once into their local QPC tick domain.
pub const SEND_COLD_THRESHOLD_US: u64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimeArithmeticError {
    #[error("timestamp arithmetic overflow")]
    Overflow,
    #[error("timestamp arithmetic underflow")]
    Underflow,
    #[error("timestamps are not in monotonic order")]
    NegativeOrder,
}

#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct QpcTicks {
    value: u64,
}

#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct TimelineTicks {
    value: u64,
}

#[repr(transparent)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct DurationTicks {
    value: u64,
}

impl QpcTicks {
    pub const ZERO: Self = Self { value: 0 };

    pub const fn from_raw(value: u64) -> Self {
        Self { value }
    }

    pub const fn as_u64(self) -> u64 {
        self.value
    }

    pub fn checked_add_duration(
        self,
        duration: DurationTicks,
    ) -> Result<Self, TimeArithmeticError> {
        self.value
            .checked_add(duration.as_u64())
            .map(|value| Self { value })
            .ok_or(TimeArithmeticError::Overflow)
    }

    pub fn checked_duration_since(
        self,
        earlier: Self,
    ) -> Result<DurationTicks, TimeArithmeticError> {
        self.value
            .checked_sub(earlier.value)
            .map(DurationTicks::from_raw)
            .ok_or(TimeArithmeticError::NegativeOrder)
    }
}

impl TimelineTicks {
    pub const ZERO: Self = Self { value: 0 };

    pub const fn from_raw(value: u64) -> Self {
        Self { value }
    }

    pub const fn as_u64(self) -> u64 {
        self.value
    }

    pub fn checked_add_duration(
        self,
        duration: DurationTicks,
    ) -> Result<Self, TimeArithmeticError> {
        self.value
            .checked_add(duration.as_u64())
            .map(|value| Self { value })
            .ok_or(TimeArithmeticError::Overflow)
    }

    pub fn checked_sub_duration(
        self,
        duration: DurationTicks,
    ) -> Result<Self, TimeArithmeticError> {
        self.value
            .checked_sub(duration.as_u64())
            .map(|value| Self { value })
            .ok_or(TimeArithmeticError::Underflow)
    }

    pub fn checked_duration_since(
        self,
        earlier: Self,
    ) -> Result<DurationTicks, TimeArithmeticError> {
        self.value
            .checked_sub(earlier.value)
            .map(DurationTicks::from_raw)
            .ok_or(TimeArithmeticError::NegativeOrder)
    }
}

impl DurationTicks {
    pub const ZERO: Self = Self { value: 0 };

    pub const fn from_raw(value: u64) -> Self {
        Self { value }
    }

    pub const fn as_u64(self) -> u64 {
        self.value
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, TimeArithmeticError> {
        self.value
            .checked_add(rhs.value)
            .map(|value| Self { value })
            .ok_or(TimeArithmeticError::Overflow)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, TimeArithmeticError> {
        self.value
            .checked_sub(rhs.value)
            .map(|value| Self { value })
            .ok_or(TimeArithmeticError::Underflow)
    }
}
