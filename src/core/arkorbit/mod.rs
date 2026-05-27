//! ArkOrbit: filesystem-backed, sandboxed orbit runtime.
//!
//! The redesign removes DB-backed orbit UI state and the host-DOM control bridge.
//! Orbits are L2 folders under `<DATA_DIR>/arkorbit/L2/orbits/<id>/`, while
//! firmware modules resolve from L0 on disk or the embedded fallback.

pub mod models;
pub mod orbit_agent;
pub mod service;
pub mod store;

pub use models::{
    Orbit, OrbitChatMessage, OrbitChatTranscriptSummary, OrbitFileEntry, OrbitManifest, OrbitUpdate,
};
pub use orbit_agent::{stream_orbit_chat_turn, OrbitAgentEvent, OrbitChatUsage};
pub use service::ArkOrbitService;
pub use store::{
    content_type_for_name, content_type_for_path, validate_readable_orbit_path,
    validate_writable_orbit_path, LayeredStore, ModuleLayer, ResolvedModule,
};
