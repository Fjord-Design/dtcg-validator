use std::collections::BTreeSet;
use std::fs;
use std::panic::catch_unwind;
use std::path::Path;

use dtcg_validator::validate;
use serde::Deserialize;

const FIXTURE_DIRECTORY: &str = "tests/fixtures/dtcg-2025.10";
const EXPECTED_ENTRIES: usize = 220;
const EXPECTED_PREPROCESSING_ENTRIES: usize = 48;

// The published report specifies neither array minimums nor a blanket ban on
// unknown $ metadata. Keep the upstream declarations and fixture bytes intact.
const ACCEPTED_REPORT_DISAGREEMENTS: [&str; 3] = [
    "gradient-empty-array",
    "strokeStyle-empty-dashArray",
    "metadata-unsupported-dollar-property",
];

#[derive(Deserialize)]
struct Manifest {
    version: String,
    tests: Vec<EvaluationTest>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
enum EvaluationType {
    PositiveEvaluationTest,
    NegativeEvaluationTest,
}

#[derive(Deserialize)]
struct EvaluationTest {
    id: String,
    #[serde(rename = "type")]
    evaluation_type: EvaluationType,
    input: String,
    features: Vec<String>,
}

#[test]
fn every_pinned_official_format_fixture_matches_the_published_report() {
    let manifest: Manifest =
        serde_json::from_str(include_str!("fixtures/dtcg-2025.10/manifest.json"))
            .expect("the unchanged pinned manifest must deserialize");
    let fixture_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIRECTORY);
    let expected_overrides: BTreeSet<_> = ACCEPTED_REPORT_DISAGREEMENTS.into_iter().collect();
    let mut found_overrides = BTreeSet::new();
    let mut declared_ids = BTreeSet::new();
    let mut exercised_ids = BTreeSet::new();
    let mut exercised = 0;
    let mut preprocessing_entries = 0;
    let mut preprocessing_exercised = 0;
    let mut positive_entries = 0;
    let mut negative_entries = 0;
    let mut failures = Vec::new();

    if manifest.version != "2025.10" {
        failures.push(format!("unexpected manifest version: {}", manifest.version));
    }

    for entry in &manifest.tests {
        if !declared_ids.insert(entry.id.as_str()) {
            failures.push(format!("{}: duplicate manifest ID", entry.id));
        }
        let preprocessing = entry
            .features
            .iter()
            .any(|feature| feature == "preprocessing-required");
        preprocessing_entries += usize::from(preprocessing);
        let upstream_positive = match entry.evaluation_type {
            EvaluationType::PositiveEvaluationTest => {
                positive_entries += 1;
                true
            }
            EvaluationType::NegativeEvaluationTest => {
                negative_entries += 1;
                false
            }
        };
        let overridden = expected_overrides.contains(entry.id.as_str());
        if overridden {
            found_overrides.insert(entry.id.as_str());
            if entry.evaluation_type != EvaluationType::NegativeEvaluationTest {
                failures.push(format!(
                    "{}: an override must remain a NegativeEvaluationTest upstream",
                    entry.id
                ));
            }
        }
        let expected_valid = upstream_positive || overridden;
        let input = match fs::read_to_string(fixture_directory.join(&entry.input)) {
            Ok(input) => input,
            Err(error) => {
                failures.push(format!("{} ({}): {error}", entry.id, entry.input));
                continue;
            }
        };

        // The validator owns preprocessing. Never filter by features or rewrite
        // fixture input before exercising the public API.
        exercised += 1;
        preprocessing_exercised += usize::from(preprocessing);
        exercised_ids.insert(entry.id.as_str());
        let report = match catch_unwind(|| validate(&input)) {
            Ok(report) => report,
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic");
                failures.push(format!(
                    "{} ({}): validator panicked: {message}",
                    entry.id, entry.input
                ));
                continue;
            }
        };
        if report.valid != expected_valid {
            failures.push(format!(
                "{} ({}): expected valid={expected_valid} (upstream {:?}, override={overridden}), got {report:#?}",
                entry.id, entry.input, entry.evaluation_type
            ));
        }
        if report.valid != report.diagnostics.is_empty() {
            failures.push(format!(
                "{} ({}): report validity disagrees with diagnostics: {report:#?}",
                entry.id, entry.input
            ));
        }
    }

    if manifest.tests.len() != EXPECTED_ENTRIES {
        failures.push(format!(
            "pinned manifest must contain {EXPECTED_ENTRIES} entries, found {}",
            manifest.tests.len()
        ));
    }
    if (positive_entries, negative_entries) != (93, 127) {
        failures.push(format!(
            "pinned manifest must declare 93 positive and 127 negative entries, found {positive_entries} and {negative_entries}"
        ));
    }
    if preprocessing_entries != EXPECTED_PREPROCESSING_ENTRIES {
        failures.push(format!(
            "pinned manifest must contain {EXPECTED_PREPROCESSING_ENTRIES} preprocessing-required entries, found {preprocessing_entries}"
        ));
    }
    if found_overrides != expected_overrides {
        failures.push(format!(
            "override IDs must be exactly {expected_overrides:?}, found {found_overrides:?}"
        ));
    }
    if exercised != manifest.tests.len() || exercised_ids != declared_ids {
        failures.push(format!(
            "every entry must exercise validate: exercised {exercised}/{}; missing IDs: {:?}",
            manifest.tests.len(),
            declared_ids.difference(&exercised_ids).collect::<Vec<_>>()
        ));
    }
    if preprocessing_exercised != preprocessing_entries {
        failures.push(format!(
            "every preprocessing-required entry must exercise validate: exercised {preprocessing_exercised}/{preprocessing_entries}"
        ));
    }

    assert!(
        failures.is_empty(),
        "{} conformance failure(s); exercised {exercised}/{} entries, including {preprocessing_exercised}/{preprocessing_entries} preprocessing-required:\n{}",
        failures.len(),
        manifest.tests.len(),
        failures.join("\n\n")
    );
}
