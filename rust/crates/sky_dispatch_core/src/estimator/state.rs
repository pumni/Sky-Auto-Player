use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleWindowJson {
    pub samples: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstimatorStateJson {
    pub version: u32,
    pub max_events: usize,
    pub down: Vec<SampleWindowJson>,
    pub up: Vec<SampleWindowJson>,
    pub mixed: Vec<SampleWindowJson>,
}
