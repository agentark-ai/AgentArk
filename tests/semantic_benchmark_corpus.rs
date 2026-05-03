//! Self-consistency tests for the semantic benchmark corpus.
//!
//! The corpus at `tests/fixtures/semantic_benchmark.json` is the acceptance
//! surface for the new TurnPlanner. These tests run without invoking a
//! planner — they verify the corpus itself is well-formed and keeps
//! coverage for every major capability category.
//!
//! Real plan-shape assertions (embedding similarity checks, step-resolver
//! assertions, paraphrase invariance over emitted plans) are added as a
//! second test binary once the TurnPlanner module is in place.
//!
//! Assertion style: structural only. No test here checks against the exact
//! text of a user prompt, a canonical paraphrase, or an emitted
//! capability_need. Hardcoded expectation sets (valid object kinds, valid
//! side-effect variants, required category coverage) are intentional per
//! project convention that hardcoded *test* expectations are fine.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Corpus {
    schema_version: String,
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    id: String,
    category: String,
    prompts: Prompts,
    expected: Expected,
    #[serde(default)]
    #[allow(dead_code)]
    notes: String,
}

#[derive(Debug, Deserialize)]
struct Prompts {
    #[allow(dead_code)]
    canonical: String,
    #[serde(default)]
    paraphrases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Expected {
    goal_count: usize,
    goals: Vec<ExpectedGoal>,
    #[serde(default)]
    clarification_needed: bool,
    #[serde(default)]
    requires_secret_sidecar: bool,
    #[serde(default)]
    #[allow(dead_code)]
    turn_confidence_min: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ExpectedGoal {
    #[serde(default)]
    capability_target: Option<String>,
    #[serde(default)]
    capability_target_any_of: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    embedding_similarity_min: Option<f64>,
    #[serde(default)]
    object_ref: Option<ExpectedObjectRef>,
    #[serde(default)]
    side_effect: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    hints: Option<serde_json::Value>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    authorization_gated: bool,
    #[serde(default)]
    #[allow(dead_code)]
    may_degrade_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedObjectRef {
    kind: String,
    resolution_family: String,
}

const SUPPORTED_SCHEMA: &str = "v1";

const VALID_OBJECT_KINDS: &[&str] = &[
    "background_session",
    "watcher",
    "task",
    "reminder",
    "app",
    "calendar_event",
    "integration",
    "skill",
    "conversation",
];

const VALID_RESOLUTION_FAMILIES: &[&str] = &[
    "by_id",
    "most_recent_in_context",
    "last_agent_created",
    "by_description",
];

const VALID_SIDE_EFFECTS: &[&str] = &[
    "read",
    "notify",
    "create_object",
    "modify_object",
    "delete_object",
    "none",
];

const REQUIRED_CATEGORIES: &[&str] = &[
    "app_build",
    "workspace_write",
    "watcher",
    "background_session",
    "calendar",
    "reminder",
    "browser",
    "deep_research",
    "arksystem_inspect",
    "product_help",
    "integration_install",
    "skill_import",
    "swarm",
    "secret_intake",
    "mixed",
];

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("semantic_benchmark.json")
}

fn load_corpus() -> Corpus {
    let path = corpus_path();
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read benchmark corpus at {}: {e}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("benchmark corpus at {} does not parse: {e}", path.display()))
}

#[test]
fn corpus_schema_version_is_supported() {
    let corpus = load_corpus();
    assert_eq!(
        corpus.schema_version, SUPPORTED_SCHEMA,
        "unsupported corpus schema: {}",
        corpus.schema_version
    );
    assert!(
        !corpus.entries.is_empty(),
        "corpus must have at least one entry"
    );
}

#[test]
fn corpus_entry_ids_are_unique() {
    let corpus = load_corpus();
    let mut seen: HashSet<&str> = HashSet::new();
    for entry in &corpus.entries {
        assert!(
            seen.insert(entry.id.as_str()),
            "duplicate entry id: {}",
            entry.id
        );
    }
}

#[test]
fn every_entry_has_paraphrases_for_non_brittleness() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        assert!(
            entry.prompts.paraphrases.len() >= 2,
            "entry {} needs at least 2 paraphrases to prove non-brittleness; has {}",
            entry.id,
            entry.prompts.paraphrases.len()
        );
    }
}

#[test]
fn expected_goal_count_equals_goals_array_length() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        assert_eq!(
            entry.expected.goal_count,
            entry.expected.goals.len(),
            "entry {}: expected.goal_count does not match goals[]",
            entry.id
        );
    }
}

#[test]
fn every_goal_has_a_capability_target_or_any_of() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        for (i, goal) in entry.expected.goals.iter().enumerate() {
            let has_target = goal.capability_target.is_some();
            let has_any_of = !goal.capability_target_any_of.is_empty();
            assert!(
                has_target || has_any_of,
                "entry {} goal[{}] has neither capability_target nor capability_target_any_of",
                entry.id,
                i
            );
            assert!(
                !(has_target && has_any_of),
                "entry {} goal[{}] sets both capability_target and capability_target_any_of; pick one",
                entry.id,
                i
            );
        }
    }
}

#[test]
fn depends_on_references_are_valid_positional_ids() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        let goal_count = entry.expected.goals.len();
        for (i, goal) in entry.expected.goals.iter().enumerate() {
            for dep in &goal.depends_on {
                let index = dep
                    .strip_prefix("goal_")
                    .and_then(|rest| rest.parse::<usize>().ok())
                    .unwrap_or_else(|| {
                        panic!(
                            "entry {} goal[{}] has malformed depends_on reference: {} (expected 'goal_<index>')",
                            entry.id, i, dep
                        )
                    });
                assert!(
                    index < goal_count,
                    "entry {} goal[{}] depends on {} but only {} goals are defined",
                    entry.id,
                    i,
                    dep,
                    goal_count
                );
                assert!(
                    index != i,
                    "entry {} goal[{}] depends on itself",
                    entry.id,
                    i
                );
            }
        }
    }
}

#[test]
fn object_refs_use_known_kinds_and_resolution_families() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        for (i, goal) in entry.expected.goals.iter().enumerate() {
            if let Some(object_ref) = &goal.object_ref {
                assert!(
                    VALID_OBJECT_KINDS.contains(&object_ref.kind.as_str()),
                    "entry {} goal[{}] object_ref.kind={} is not a known ObjectKind",
                    entry.id,
                    i,
                    object_ref.kind
                );
                assert!(
                    VALID_RESOLUTION_FAMILIES.contains(&object_ref.resolution_family.as_str()),
                    "entry {} goal[{}] object_ref.resolution_family={} is not a known resolution family",
                    entry.id,
                    i,
                    object_ref.resolution_family
                );
            }
        }
    }
}

#[test]
fn side_effects_use_known_variants() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        for (i, goal) in entry.expected.goals.iter().enumerate() {
            if let Some(side_effect) = &goal.side_effect {
                assert!(
                    VALID_SIDE_EFFECTS.contains(&side_effect.as_str()),
                    "entry {} goal[{}] side_effect={} is not a known StepSideEffect variant",
                    entry.id,
                    i,
                    side_effect
                );
            }
        }
    }
}

#[test]
fn entry_has_goals_or_requests_clarification() {
    let corpus = load_corpus();
    for entry in &corpus.entries {
        if entry.expected.goals.is_empty() {
            assert!(
                entry.expected.clarification_needed,
                "entry {} has no goals but clarification_needed=false",
                entry.id
            );
        }
    }
}

#[test]
fn corpus_covers_all_required_categories() {
    let corpus = load_corpus();
    let seen: HashSet<&str> = corpus
        .entries
        .iter()
        .map(|entry| entry.category.as_str())
        .collect();
    for required in REQUIRED_CATEGORIES {
        assert!(
            seen.contains(required),
            "corpus missing coverage for required category: {}",
            required
        );
    }
}

#[test]
fn secret_intake_entries_flag_sidecar_requirement() {
    let corpus = load_corpus();
    let secret_entries: Vec<&Entry> = corpus
        .entries
        .iter()
        .filter(|entry| entry.category == "secret_intake")
        .collect();
    assert!(
        !secret_entries.is_empty(),
        "corpus must include at least one secret_intake case"
    );
    for entry in secret_entries {
        assert!(
            entry.expected.requires_secret_sidecar,
            "secret_intake entry {} must set requires_secret_sidecar=true",
            entry.id
        );
    }
}

#[test]
fn mixed_category_entries_have_multiple_goals() {
    let corpus = load_corpus();
    let mixed_entries: Vec<&Entry> = corpus
        .entries
        .iter()
        .filter(|entry| entry.category == "mixed")
        .collect();
    assert!(
        !mixed_entries.is_empty(),
        "corpus must include at least one mixed-intent case"
    );
    for entry in mixed_entries {
        assert!(
            entry.expected.goals.len() >= 2,
            "mixed entry {} must have >= 2 goals (found {}); otherwise it is not a mixed-intent test",
            entry.id,
            entry.expected.goals.len()
        );
    }
}

#[test]
fn corpus_summary_for_observability() {
    // Not a strict assertion; prints a structured summary so that
    // corpus growth is visible in `cargo test -- --nocapture`.
    let corpus = load_corpus();
    let total_entries = corpus.entries.len();
    let total_paraphrases: usize = corpus
        .entries
        .iter()
        .map(|e| e.prompts.paraphrases.len())
        .sum();
    let multi_goal_entries = corpus
        .entries
        .iter()
        .filter(|e| e.expected.goals.len() > 1)
        .count();
    let secret_sidecar_entries = corpus
        .entries
        .iter()
        .filter(|e| e.expected.requires_secret_sidecar)
        .count();
    let authorization_gated_goals: usize = corpus
        .entries
        .iter()
        .flat_map(|e| e.expected.goals.iter())
        .filter(|g| g.authorization_gated)
        .count();

    eprintln!(
        "semantic_benchmark_corpus: entries={total_entries} paraphrases={total_paraphrases} \
         multi_goal={multi_goal_entries} secret_sidecar={secret_sidecar_entries} \
         authorization_gated_goals={authorization_gated_goals}"
    );

    assert!(total_entries >= 20, "corpus too small ({total_entries})");
    assert!(
        total_paraphrases >= 60,
        "too few paraphrases for invariance testing ({total_paraphrases})"
    );
    assert!(multi_goal_entries >= 3, "need multi-goal coverage");
}
