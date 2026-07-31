//! Tick-domain time primitives for the real-time scheduling path.

use serde::{Deserialize, Serialize};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct QpcTicks(pub u64);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct TimelineTicks(pub u64);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct DurationTicks(pub u64);

impl QpcTicks {
    pub fn saturating_add(self, rhs: DurationTicks) -> QpcTicks {
        QpcTicks(self.0.saturating_add(rhs.0))
    }

    pub fn saturating_sub(self, rhs: DurationTicks) -> QpcTicks {
        QpcTicks(self.0.saturating_sub(rhs.0))
    }

    pub fn duration_since(self, earlier: QpcTicks) -> DurationTicks {
        DurationTicks(self.0.saturating_sub(earlier.0))
    }
}

impl std::ops::Add<DurationTicks> for QpcTicks {
    type Output = QpcTicks;
    fn add(self, rhs: DurationTicks) -> QpcTicks {
        self.saturating_add(rhs)
    }
}

impl std::ops::Sub<DurationTicks> for QpcTicks {
    type Output = QpcTicks;
    fn sub(self, rhs: DurationTicks) -> QpcTicks {
        self.saturating_sub(rhs)
    }
}

impl TimelineTicks {
    pub fn saturating_add(self, rhs: DurationTicks) -> TimelineTicks {
        TimelineTicks(self.0.saturating_add(rhs.0))
    }

    pub fn saturating_sub(self, rhs: DurationTicks) -> TimelineTicks {
        TimelineTicks(self.0.saturating_sub(rhs.0))
    }

    pub fn duration_since(self, earlier: TimelineTicks) -> DurationTicks {
        DurationTicks(self.0.saturating_sub(earlier.0))
    }
}

impl std::ops::Add<DurationTicks> for TimelineTicks {
    type Output = TimelineTicks;
    fn add(self, rhs: DurationTicks) -> TimelineTicks {
        self.saturating_add(rhs)
    }
}

impl std::ops::Sub<DurationTicks> for TimelineTicks {
    type Output = TimelineTicks;
    fn sub(self, rhs: DurationTicks) -> TimelineTicks {
        self.saturating_sub(rhs)
    }
}

impl DurationTicks {
    pub fn saturating_add(self, rhs: DurationTicks) -> DurationTicks {
        DurationTicks(self.0.saturating_add(rhs.0))
    }

    pub fn saturating_sub(self, rhs: DurationTicks) -> DurationTicks {
        DurationTicks(self.0.saturating_sub(rhs.0))
    }
}

impl std::ops::Add<DurationTicks> for DurationTicks {
    type Output = DurationTicks;
    fn add(self, rhs: DurationTicks) -> DurationTicks {
        self.saturating_add(rhs)
    }
}

impl std::ops::Sub<DurationTicks> for DurationTicks {
    type Output = DurationTicks;
    fn sub(self, rhs: DurationTicks) -> DurationTicks {
        self.saturating_sub(rhs)
    }
}
