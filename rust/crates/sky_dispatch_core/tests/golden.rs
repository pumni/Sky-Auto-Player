use serde::Deserialize;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::testing::{SimulationResult, simulate_schedule};
use smallvec::SmallVec;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Corpus {
    schema_version: u32,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
struct Scenario {
    name: String,
    allowed_scan_codes: Vec<u16>,
    config: Config,
    actions: Vec<Action>,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
struct Config {
    min_hold_us: u64,
    send_latency_us: u64,
}

#[derive(Debug, Deserialize)]
struct Action {
    source_action_index: u32,
    kind: ActionKind,
    at_us: u64,
    scan_codes: Vec<u16>,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct Expected {
    outcome: String,
    result: Option<SimulationResult>,
    error_contains: Option<String>,
}

#[test]
fn frozen_core_dispatch_vectors_match_rust_core() {
    let corpus: Corpus = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/golden/native_dispatch/core_simulation.json"
    )))
    .expect("golden corpus must be valid JSON");

    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.scenarios.len(), 11);

    for scenario in corpus.scenarios {
        let actions: Vec<KeyActionInput> = scenario
            .actions
            .into_iter()
            .map(|action| KeyActionInput {
                source_action_index: action.source_action_index,
                kind: action.kind,
                scheduled_us: action.at_us,
                scan_codes: SmallVec::from_vec(action.scan_codes),
                reason: Arc::from(action.reason),
            })
            .collect();

        let actual = simulate_schedule(
            &actions,
            &scenario.allowed_scan_codes,
            scenario.config.min_hold_us,
            scenario.config.send_latency_us,
        );

        match scenario.expected.outcome.as_str() {
            "finished" => {
                let expected = scenario
                    .expected
                    .result
                    .unwrap_or_else(|| panic!("{} is missing expected result", scenario.name));
                assert_eq!(
                    actual.unwrap_or_else(|error| panic!("{} failed: {error}", scenario.name)),
                    expected,
                    "{}",
                    scenario.name
                );
            }
            "error" => {
                let expected = scenario.expected.error_contains.as_deref().unwrap_or("");
                let error = actual.expect_err(&scenario.name).to_string();
                assert!(
                    error.contains(expected),
                    "{}: {error:?} does not contain {expected:?}",
                    scenario.name
                );
            }
            other => panic!("{} has unknown expected outcome {other:?}", scenario.name),
        }
    }
}
