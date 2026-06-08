/// Sentinel configuration (loaded from settings, with sensible defaults)
pub struct SentinelConfig {
    /// How often to check process health (seconds) - used by http.rs process watchdog
    pub _process_check_interval: u64,
    /// How often to check for due tasks (seconds)
    pub scheduler_interval: u64,
    /// Maximum time to sleep before rescanning watcher schedules (seconds)
    pub watcher_interval: u64,
    /// How often to poll connected integrations for new activity (seconds)
    pub integration_sync_interval: u64,
    /// How often to consolidate execution experiences into learned memory (seconds)
    pub experience_consolidation_interval: u64,
    /// How often to compact idle background-session state (seconds)
    pub background_session_consolidation_interval: u64,
    /// How often to reflect on consolidated execution runs and extract heuristics (seconds)
    pub heuristic_reflection_interval: u64,
    /// How often to induce procedural patterns from learned procedures (seconds)
    pub pattern_induction_interval: u64,
    /// How often to generate approval-gated learning candidates (seconds)
    pub candidate_generation_interval: u64,
    /// How often to apply learned Memory health-review feedback (seconds)
    pub arkmemory_learned_review_interval: u64,
    /// How often to expire old approvals (seconds)
    pub approval_expiry_interval: u64,
    /// How often to run Pulse (seconds, 0 = disabled)
    pub pulse_interval: u64,
    /// How often to check for unused deployed apps (seconds).
    /// Notifications sent once per day per unused app.
    pub unused_app_check_interval: u64,
    /// How often to run proactive autonomy analysis scans (seconds).
    pub auto_analysis_interval: u64,
    /// How often to reconcile orphaned sandbox containers (seconds).
    pub container_reaper_interval: u64,
}

impl Default for SentinelConfig {
    fn default() -> Self {
        Self {
            _process_check_interval: 30,
            scheduler_interval: 30,
            watcher_interval: 15 * 60,
            integration_sync_interval: 120,
            experience_consolidation_interval: 1800,
            background_session_consolidation_interval: 1800,
            heuristic_reflection_interval: 750,
            pattern_induction_interval: 900,
            candidate_generation_interval: 1200,
            arkmemory_learned_review_interval: 6 * 3600,
            approval_expiry_interval: 300,
            pulse_interval: 1800,            // 30 minutes
            unused_app_check_interval: 3600, // Check hourly, notify once daily per unused app
            auto_analysis_interval: 1800,    // 30 minutes
            container_reaper_interval: 300,  // 5 minutes
        }
    }
}
