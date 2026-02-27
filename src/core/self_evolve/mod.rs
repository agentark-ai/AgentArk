//! Self-Evolve: policy-first and code-evolution engine
//!
//! Default path evolves runtime strategy/policy with benchmark + lineage + gates.
//! Codebase mutation remains available behind explicit opt-in.

pub mod agent;
pub mod coding_guidelines;
pub mod policy_evolution;
pub mod security_review;
pub mod strategy_runtime;
pub mod tools;

pub use agent::{SelfEvolveAgent, SelfEvolveConfig};
pub use policy_evolution::{
    PolicyEvolutionConfig, PolicyEvolutionEngine, ROUTING_COMPLEXITY_POLICY_KEY,
};
