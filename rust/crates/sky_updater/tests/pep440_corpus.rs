use std::cmp::Ordering;
use std::str::FromStr;

use pep440_rs::Version;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    left: String,
    right: String,
    ordering: i8,
}

#[test]
fn ordering_matches_packaging_version_golden_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("pep440_ordering.json")).expect("valid PEP 440 corpus");
    for case in cases {
        let left = Version::from_str(&case.left).expect("valid left version");
        let right = Version::from_str(&case.right).expect("valid right version");
        let actual = match left.cmp(&right) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        };
        assert_eq!(actual, case.ordering, "{} vs {}", case.left, case.right);
    }
}
