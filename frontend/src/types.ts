export type UpdateStatus = {
  state: string;
  apply_supported: boolean;
  apply_message?: string | null;
  latest_version?: string | null;
  latest_tag?: string | null;
  release_url?: string | null;
  checked_at?: string | null;
};

export type StatusResponse = {
  did: string;
  memory_entries: number;
  skills_loaded?: number;
  actions_loaded?: number;
  tasks_pending: number;
  version: string;
  runtime_health?: RuntimeHealth | null;
  update?: UpdateStatus | null;
};

export type RuntimeHealth = {
  uptime_seconds: number;
  cpu_percent?: number | null;
  ram_percent?: number | null;
  memory_pressure_percent?: number | null;
  memory_used_bytes?: number | null;
  memory_total_bytes?: number | null;
  memory_source?: string | null;
  memory_container_count?: number | null;
  disk_read_bytes_per_sec?: number | null;
  disk_write_bytes_per_sec?: number | null;
  temperature_celsius?: number | null;
  load_average_1m?: number | null;
  sampled_at: string;
};

export type Task = {
  id: string;
  description: string;
  action?: string;
  status: unknown;
  task_kind?: string;
  task_kind_label?: string;
  created_at?: string;
  scheduled_for?: string | null;
  cron?: string;
  arguments?: Record<string, unknown>;
  result?: unknown;
};

export type BackgroundSessionCounts = {
  tasks_total: number;
  tasks_queued: number;
  tasks_running: number;
  tasks_waiting: number;
  tasks_paused: number;
  tasks_done: number;
  tasks_failed: number;
  tasks_cancelled: number;
  watchers_total: number;
  watchers_active: number;
  watchers_paused: number;
  watchers_triggered: number;
  watchers_stopped: number;
};

export type BackgroundSessionSummary = {
  id: string;
  title: string;
  objective: string;
  status: string;
  summary?: string | null;
  current_focus?: string | null;
  waiting_on?: string | null;
  next_expected_action?: string | null;
  last_error?: string | null;
  preferred_delivery_channel?: string | null;
  channel?: string | null;
  conversation_id?: string | null;
  linked_task_ids: string[];
  linked_watcher_ids: string[];
  created_at: string;
  updated_at: string;
  last_activity_at: string;
  live_summary: string;
  counts: BackgroundSessionCounts;
  ui_kind?: string;
  default_visible?: boolean;
};

export type BackgroundSessionEvent = {
  id: string;
  at: string;
  kind: string;
  summary: string;
  detail?: string | null;
  actor?: string | null;
};

export type BackgroundSessionLinkedTask = {
  id: string;
  description: string;
  action: string;
  task_kind?: string;
  task_kind_label?: string;
  status: string;
  created_at: string;
  scheduled_for?: string | null;
  cron?: string | null;
  result?: string | null;
};

export type BackgroundSessionLinkedWatcher = {
  id: string;
  description: string;
  poll_action: string;
  status: string;
  created_at: string;
  last_poll_at?: string | null;
  poll_count?: number;
  notify_channel?: string | null;
  last_error?: string | null;
  last_poll_outcome?: string | null;
  notification_attempts?: Array<Record<string, unknown>>;
  trigger_result?: string | null;
};

export type BackgroundSessionRun = {
  id: string;
  automation_id: string;
  kind: string;
  title: string;
  action: string;
  trigger: string;
  status: string;
  attempt: number;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  summary: string;
  output_preview?: string | null;
  error?: string | null;
  next_retry_at?: string | null;
};

export type BackgroundSessionDetail = {
  session: BackgroundSessionSummary;
  session_detail: {
    working_memory?: string | null;
    channel?: string | null;
    conversation_id?: string | null;
    events: BackgroundSessionEvent[];
  };
  linked_tasks: BackgroundSessionLinkedTask[];
  linked_watchers: BackgroundSessionLinkedWatcher[];
  recent_runs: BackgroundSessionRun[];
  missing_links: {
    task_ids: string[];
    watcher_ids: string[];
  };
};

export type BackgroundSessionsResponse = {
  sessions: BackgroundSessionSummary[];
  total: number;
};

export type Notification = {
  id: string;
  level: string;
  title?: string;
  body: string;
  created_at: string;
  read: boolean;
  source?: string;
  metadata?: Record<string, unknown>;
};

export type ApprovalLogEntry = {
  id: string;
  action_name: string;
  arguments: string;
  rule_name: string;
  status: string;
  requested_at: string;
  resolved_at?: string | null;
  resolved_by?: string | null;
};

export type BrowserHandoffStatus = {
  id: string;
  conversation_id?: string | null;
  task_description: string;
  profile_id?: string | null;
  profile_name?: string | null;
  status: string;
  question?: string | null;
  summary?: string | null;
  reason?: string | null;
  created_at: string;
  updated_at: string;
  page_url?: string | null;
  page_title?: string | null;
  live_view_enabled: boolean;
  live_view_port?: number | null;
  live_view_path?: string | null;
  can_claim: boolean;
  can_release: boolean;
  can_complete: boolean;
};

export type BrowserSessionSummary = BrowserHandoffStatus;

export type BrowserSessionsResponse = {
  sessions: BrowserSessionSummary[];
  total: number;
};

export type PulseRemediationSpec =
  | { kind: "tunnel_start_verify" }
  | { kind: "tunnel_restart_verify" }
  | { kind: "app_restart"; app_id: string }
  | { kind: "readonly_investigation"; topic: "memory_capture_health" }
  | {
      kind: "managed_app_operation";
      app_id: string;
      operation:
        | "compile_python_requirements"
        | "generate_cargo_lockfile"
        | "remove_npm_install_hooks";
    }
  | { kind: "shell_command"; command: string };

export type PulseDoctorFinding = {
  severity: string;
  category: string;
  target: string;
  title: string;
  evidence: string;
  root_cause: string;
  fix_command: string;
  remediation?: PulseRemediationSpec | null;
  user_actionable?: boolean;
};

export type PulseRunFixRequest = {
  fix_command?: string;
  remediation?: PulseRemediationSpec;
  issue_title?: string;
  target?: string;
  event_timestamp?: string;
  finding_index?: number;
};

export type PulseCleanupCandidate = {
  id: string;
  category: string;
  category_label: string;
  path_label: string;
  size_bytes: number;
  age_seconds: number;
  age_days: number;
  risk: string;
  reason: string;
  selected_by_default: boolean;
  app_id?: string | null;
  requires_app_stop?: boolean;
};

export type PulseCleanupPreviewResponse = {
  status: string;
  archive_root: string;
  legacy_archive_root: string;
  archive_retention_days: number;
  candidates: PulseCleanupCandidate[];
  category_counts: Array<{ category: string; count: number; size_bytes: number }>;
  total_size_bytes: number;
  active_job?: unknown;
};

export type PulseCleanupRequest = {
  candidate_ids: string[];
  confirm_archive: boolean;
  event_timestamp?: string;
  finding_index?: number;
};

export type TraceSummary = {
  id: string;
  message_preview: string;
  channel: string;
  status: string;
  step_count: number;
  started_at: string;
  duration_ms?: number;
};

export type TraceOperationalEvent = {
  id: string;
  source: string;
  trace_id?: string | null;
  created_at: string;
  channel: string;
  event_type: string;
  success: boolean;
  outcome: string;
  tool_name?: string | null;
  latency_ms?: number | null;
  details?: unknown;
};

export type TraceResponse = {
  history: TraceSummary[];
  history_total?: number;
  recent_events?: TraceOperationalEvent[];
  recent_events_total?: number;
  recent_events_offset?: number;
  recent_events_limit?: number;
};

export type AutonomyActionExecutionResponse = {
  status: string;
  message?: string;
  queued?: boolean;
  trace_id?: string;
  result?: Record<string, unknown>;
};

export type SentinelSettings = {
  enabled: boolean;
  watch_in_app: boolean;
  watch_connected_services: boolean;
  infer_new_automations: boolean;
  confidence_threshold: number;
  max_proposals_per_scan: number;
};

export type SentinelSettingsResponse = {
  settings: SentinelSettings;
  autonomy_mode: "off" | "assist" | "auto" | string;
  daily_run_limit?: number | null;
  quiet_hours_start?: string | null;
  quiet_hours_end?: string | null;
  agent_paused?: boolean;
};

export type SentinelObservation = {
  id: string;
  fingerprint: string;
  kind: string;
  title: string;
  detail: string;
  source_kind: string;
  source_id?: string | null;
  source_label?: string | null;
  confidence: number;
  priority: number;
  created_at: string;
  updated_at: string;
  metadata?: Record<string, unknown>;
};

export type SentinelProposal = {
  id: string;
  fingerprint: string;
  proposal_kind: string;
  status: string;
  title: string;
  detail: string;
  rationale: string;
  source_kind: string;
  source_id?: string | null;
  source_label?: string | null;
  confidence: number;
  priority: number;
  created_at: string;
  updated_at: string;
  snoozed_until?: string | null;
  approved_at?: string | null;
  dismissed_at?: string | null;
  trace_id?: string | null;
  run_status?: string | null;
  last_run_summary?: string | null;
  action?: RecommendedAction | null;
  chat_suggestion_id?: string | null;
  metadata?: Record<string, unknown>;
};

export type SentinelScanState = {
  last_started_at?: string | null;
  last_completed_at?: string | null;
  last_status?: string | null;
  last_error?: string | null;
  last_trigger?: string | null;
  last_created_observations?: number;
  last_created_proposals?: number;
  last_auto_executed?: number;
  open_proposals?: number;
};

export type SentinelBackgroundLearningJob = {
  status: string;
  last_started_at?: string | null;
  last_completed_at?: string | null;
  summary?: string | null;
  changed?: boolean;
  runs?: number;
  stats?: Record<string, unknown>;
  last_error?: string | null;
};

export type SentinelBackgroundLearning = {
  status: string;
  last_started_at?: string | null;
  last_completed_at?: string | null;
  summary?: string | null;
  changed?: boolean;
  jobs?: Partial<
    Record<
      "reflection_pass" | "experience_consolidation" | "pattern_induction" | "candidate_generation",
      SentinelBackgroundLearningJob
    >
  > & Record<string, SentinelBackgroundLearningJob>;
};

export type SentinelFeedResponse = {
  generated_at: string;
  scan: SentinelScanState;
  observations: SentinelObservation[];
  proposals: SentinelProposal[];
  background_learning?: SentinelBackgroundLearning | null;
  stats: {
    open_proposals: number;
    completed_recently: number;
    connected_services: number;
    important_service_events: number;
    in_app_events?: number;
    chat_suggestions?: number;
    recent_runs: number;
    auto_mode_enabled: boolean;
  };
};

export type RecommendedAction = {
  id: string;
  title: string;
  summary?: string;
  description?: string;
  action_kind: string;
  payload: Record<string, unknown>;
  requires_approval?: boolean;
  trust?: {
    level?: string;
    score?: number;
    requires_approval?: boolean;
    reasons?: string[];
    blocked?: boolean;
  };
  readiness?: DevelopmentalReadiness | null;
};

export type DevelopmentalReadiness = {
  policy_version?: string;
  score?: number;
  stage?: string;
  label?: string;
  plain_summary?: string;
  allows_review?: boolean;
  allows_auto?: boolean;
  reasons?: string[];
  blockers?: string[];
  signals?: unknown;
};

export type BriefingResponse = {
  generated_at: string;
  scope: string;
  top_risks: Array<{ title?: string; summary?: string; detail?: string; severity?: string }>;
  top_opportunities: Array<{ title?: string; summary?: string; detail?: string; score?: number }>;
  recommended_actions?: RecommendedAction[];
  recommended_skills?: RecommendedAction[];
  trust_summary: Record<string, unknown>;
};

export type LlmAnalyticsTotals = {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_prompt_tokens: number;
  cache_creation_prompt_tokens: number;
  request_count: number;
  estimated_count: number;
  cost_usd?: number | null;
};

export type LlmAnalyticsPoint = {
  bucket_start: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_prompt_tokens: number;
  cache_creation_prompt_tokens: number;
  request_count: number;
  primary_prompt_tokens: number;
  primary_completion_tokens: number;
  primary_total_tokens: number;
  primary_cached_prompt_tokens: number;
  primary_cache_creation_prompt_tokens: number;
  primary_request_count: number;
  helper_prompt_tokens: number;
  helper_completion_tokens: number;
  helper_total_tokens: number;
  helper_cached_prompt_tokens: number;
  helper_cache_creation_prompt_tokens: number;
  helper_request_count: number;
  cost_usd?: number | null;
};

export type LlmAnalyticsBreakdownRow = {
  provider: string;
  model: string;
  channel?: string | null;
  purpose?: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_prompt_tokens: number;
  cache_creation_prompt_tokens: number;
  request_count: number;
  cost_usd?: number | null;
};

export type LlmAnalyticsResponse = {
  range: {
    since: string;
    until: string;
    bucket: "hour" | "day" | "week" | string;
    truncated?: boolean;
  };
  totals: LlmAnalyticsTotals;
  truncated?: boolean;
  series: LlmAnalyticsPoint[];
  by_model: LlmAnalyticsBreakdownRow[];
  by_channel: LlmAnalyticsBreakdownRow[];
  by_purpose: LlmAnalyticsBreakdownRow[];
};

export type IntegrationConfigField = {
  key: string;
  label: string;
  input_type: "text" | "password" | "textarea" | "select";
  placeholder?: string;
  required: boolean;
  options?: string[];
};

export type IntegrationItem = {
  id: string;
  name: string;
  description: string;
  icon: string;
  status: "not_configured" | "needs_auth" | "connected" | "error" | string;
  enabled: boolean;
  status_detail?: string | null;
  auth_url?: string | null;
  config_fields?: IntegrationConfigField[] | null;
  config_help?: string | null;
  configure_button?: string | null;
  config_values?: Record<string, unknown> | null;
};

export type IntegrationAuthManifestField = {
  key: string;
  label: string;
  placeholder?: string | null;
  help?: string | null;
  input_type?: string | { kind?: string; options?: string[] } | null;
  required?: boolean;
};

export type IntegrationAuthManifest = {
  integration_id: string;
  display_name: string;
  description?: string | null;
  docs_url?: string | null;
  warning?: string | null;
  mode?: ({ kind?: string; fields?: IntegrationAuthManifestField[] } & Record<string, unknown>) | null;
  post_submit?: Record<string, unknown> | null;
};

export type CustomMessagingChannel = {
  id: string;
  name: string;
  description?: string;
  enabled?: boolean;
  docs_url?: string | null;
  runtime_channel_id: string;
  configured: boolean;
  requires_auth: boolean;
  required_secret_count?: number;
  auth_manifest?: IntegrationAuthManifest | null;
  auth_profile_id?: string | null;
  last_tested_at?: string | null;
  last_test_outcome?: string | null;
  last_test_message?: string | null;
};

export type ExtensionPackAuthMode =
  | "none"
  | "api_key"
  | "basic"
  | "oauth2_external"
  | string;

export type ExtensionPackTrustLevel = "trusted" | "unverified" | string;

export type ExtensionPackSourceKind =
  | "bundled_registry"
  | "local_manifest"
  | "direct_url"
  | "scaffolded"
  | "local_path"
  | "uploaded_bundle"
  | string;

export type ExtensionPackManifest = {
  sdk_version: string;
  id: string;
  name: string;
  version: string;
  kind: string;
  publisher?: string;
  publisher_did?: string | null;
  description?: string;
  docs_url?: string | null;
  signature?: string | null;
  draft: boolean;
  tags?: string[];
  auth: {
    mode: ExtensionPackAuthMode;
    required_secrets?: string[];
    required_scopes?: string[];
    metadata?: Record<string, unknown> | null;
  };
  features: Array<{
    id: string;
    kind: string;
    title?: string;
    description?: string;
    read_only: boolean;
    experimental: boolean;
    binding?: { kind: string; config?: Record<string, unknown> | null } | null;
  }>;
  metadata?: Record<string, unknown> | null;
};

export type PackFeatureSummary = {
  id: string;
  kind: string;
  title: string;
  description: string;
  read_only: boolean;
  experimental: boolean;
  binding_kind?: string | null;
};

export type ExtensionPackRuntimeStatus =
  | "not_required"
  | "missing"
  | "ready"
  | "error"
  | string;

export type ExtensionPackView = {
  manifest: ExtensionPackManifest;
  installed: boolean;
  enabled: boolean;
  trust_level: ExtensionPackTrustLevel;
  verification_status: string;
  verification_detail?: string | null;
  source_kind: ExtensionPackSourceKind;
  source_url?: string | null;
  needs_auth: boolean;
  status: string;
  status_detail?: string | null;
  runtime_required: boolean;
  runtime_status: ExtensionPackRuntimeStatus;
  runtime_detail?: string | null;
  runtime_executable?: string | null;
  supports_connect_url: boolean;
  supports_webhook: boolean;
  webhook_path?: string | null;
  feature_summaries: PackFeatureSummary[];
};

export type ExtensionPackConnectionView = {
  connection: {
    id: string;
    pack_id: string;
    name: string;
    enabled: boolean;
    auth_profile_id?: string | null;
    metadata?: Record<string, unknown> | null;
    last_error?: string | null;
    last_tested_at?: string | null;
    created_at: string;
    updated_at: string;
  };
  state: string;
  auth_mode: ExtensionPackAuthMode;
  has_secret: boolean;
};

export type ExtensionPackSearchResponse = {
  query: string;
  installed: ExtensionPackView[];
  catalog: ExtensionPackView[];
  not_found: boolean;
  next_steps: string[];
};

export type ExtensionPackEventRecord = {
  id: string;
  pack_id: string;
  feature_id: string;
  connection_id?: string | null;
  event_type: string;
  provider_event_id?: string | null;
  transport: string;
  status: string;
  outcome?: string | null;
  response_preview?: string | null;
  metadata?: Record<string, unknown> | null;
  payload?: Record<string, unknown> | string | null;
  received_at: string;
  processed_at?: string | null;
};

export type ExtensionPackEventsResponse = {
  pack_id: string;
  count: number;
  items: ExtensionPackEventRecord[];
};

export type GoogleWorkspaceOAuthClientSettings = {
  configured: boolean;
  source: string;
  source_label: string;
  managed_externally: boolean;
  client_id_hint?: string | null;
  secret_configured: boolean;
  redirect_uri: string;
};

export type IntegrationSyncStatus = {
  integration_id: string;
  integration_name: string;
  supported: boolean;
  enabled: boolean;
  connected: boolean;
  integration_enabled: boolean;
  sync_kind: string;
  poll_interval_secs: number;
  importance_threshold: number;
  notify_on_important: boolean;
  push_to_preferred_channel: boolean;
  last_sync_at?: string | null;
  last_success_at?: string | null;
  last_error?: string | null;
  last_item_at?: string | null;
  recent_item_count: number;
};

export type IntegrationSyncFeedItem = {
  id: string;
  integration_id: string;
  integration_name: string;
  kind: string;
  title: string;
  summary: string;
  url?: string | null;
  occurred_at?: string | null;
  detected_at: string;
  importance: number;
  important: boolean;
  outcome: string;
};

export type GatewayChannelDescriptor = {
  id: string;
  kind: string;
  name: string;
  description: string;
  status: string;
  enabled: boolean;
  configured: boolean;
  supports_pairing: boolean;
  supports_threads: boolean;
  supports_groups: boolean;
  supports_broadcast: boolean;
  delivery_mode?: string | null;
  account_count?: number;
  route_count?: number;
  connected_account_count?: number;
  last_error?: string | null;
  docs_url?: string | null;
  capabilities?: string[];
  metadata?: Record<string, unknown> | null;
};

export type GatewayChannelAccount = {
  id: string;
  channel_id: string;
  label: string;
  enabled: boolean;
  status: string;
  peer_scope?: string | null;
  default_agent_id?: string | null;
  last_seen_at?: string | null;
  last_error?: string | null;
  metadata?: Record<string, unknown> | null;
};

export type GatewayChannelsResponse = {
  summary: {
    supported: number;
    configured: number;
    connected: number;
    attention_needed: number;
  };
  channels: GatewayChannelDescriptor[];
  accounts: GatewayChannelAccount[];
};

export type GatewayBroadcastGroup = {
  id: string;
  name: string;
  description?: string | null;
  enabled: boolean;
  member_count: number;
  channels?: string[];
  targets?: string[];
};

export type GatewayRouteRule = {
  id: string;
  name: string;
  enabled: boolean;
  priority: number;
  channel_id?: string | null;
  account_id?: string | null;
  match_kind: string;
  match_value: string;
  target_kind: string;
  target_value: string;
  agent_id?: string | null;
  conversation_scope?: string | null;
  broadcast_group_id?: string | null;
  notes?: string | null;
  created_at?: string | null;
  updated_at?: string | null;
};

export type GatewayRoutingSimulation = {
  matched: boolean;
  rule_id?: string | null;
  rule_name?: string | null;
  target_kind?: string | null;
  target_value?: string | null;
  conversation_scope?: string | null;
  broadcast_group_id?: string | null;
  reason?: string | null;
};

export type GatewayRoutingResponse = {
  summary: {
    rules: number;
    enabled_rules: number;
    broadcast_groups: number;
  };
  rules: GatewayRouteRule[];
  broadcast_groups: GatewayBroadcastGroup[];
};

export type GatewayOpsOverview = {
  generated_at: string;
  service_summaries: Array<{
    id: string;
    title: string;
    status: string;
    summary?: string | null;
    details?: string | null;
    total_count?: number | null;
    attention_count: number;
  }>;
  operator_checks: Array<{
    id: string;
    title: string;
    passed: boolean;
    severity: string;
    message: string;
    details?: string | null;
  }>;
  pulse_highlights: Array<{
    source: string;
    severity: string;
    title: string;
    message: string;
    target?: string | null;
    note?: string | null;
  }>;
  doctor_highlights: Array<{
    source: string;
    severity: string;
    title: string;
    message: string;
    target?: string | null;
    note?: string | null;
  }>;
};

export type DeviceNodeRecord = {
  id: string;
  display_name: string;
  transport: string;
  state: string;
  capabilities?: string[];
  labels?: string[];
  platform?: string | null;
  owner?: string | null;
  last_heartbeat_at?: string | null;
  last_error?: string | null;
  permissions_granted?: number;
  command_count?: number;
  metadata?: Record<string, string>;
};

export type NodesResponse = {
  status?: string;
  generated_at?: string;
  summary?: {
    total: number;
    paired: number;
    online: number;
    degraded: number;
    offline: number;
    revoked: number;
    capabilities?: Record<string, number>;
  };
  nodes: DeviceNodeRecord[];
};

export type NodeCommandsResponse = {
  status?: string;
  commands: Array<{
    id: string;
    node_id: string;
    command: string;
    requested_at: string;
    completed_at?: string | null;
    success: boolean;
    exit_code?: number | null;
    output_preview?: string | null;
    actor?: string | null;
    context?: Record<string, string>;
  }>;
};

export type CompanionRiskLevel = "low" | "high" | string;

export type CompanionCapabilityDescriptor = {
  id: string;
  label: string;
  description: string;
  risk: CompanionRiskLevel;
  resource_kinds?: string[];
};

export type CompanionPreset = {
  id: string;
  label: string;
  description: string;
  platform: string;
  capability_ids: string[];
};

export type CompanionCommandDescriptor = {
  id: string;
  label: string;
  capability: string;
  action: string;
  description?: string;
  risk: CompanionRiskLevel;
};

export type CompanionDeviceRecord = {
  id: string;
  display_name: string;
  preset_id: string;
  platform: string;
  model?: string | null;
  state: string;
  transport: string;
  available_capabilities?: string[];
  granted_capabilities?: string[];
  token_capabilities?: string[];
  declared_commands?: CompanionCommandDescriptor[];
  paired_at: string;
  last_seen_at?: string | null;
  owner?: string | null;
  metadata?: Record<string, string>;
  attestation?: {
    provider?: string | null;
    platform?: string | null;
    verified?: boolean;
    evidence_fingerprint?: string | null;
    verified_at?: string | null;
    reason?: string | null;
  };
  trusted_unattested?: boolean;
  active_grant_id: string;
  command_count?: number;
};

export type CompanionPairingSession = {
  id: string;
  code: string;
  preset_id: string;
  display_name: string;
  platform: string;
  requested_capabilities?: string[];
  requested_resources?: Record<string, string[]>;
  status: string;
  created_at: string;
  expires_at: string;
  claimed_at?: string | null;
  approved_at?: string | null;
  completed_at?: string | null;
  claimed_device_public_key?: string | null;
  attestation?: {
    provider?: string | null;
    platform?: string | null;
    verified?: boolean;
    evidence_fingerprint?: string | null;
    verified_at?: string | null;
    reason?: string | null;
  };
  trusted_unattested?: boolean;
  metadata?: Record<string, string>;
  claim_attempts?: number;
  failed_claim_attempts?: number;
  claim_locked_until?: string | null;
};

export type CompanionCommandRecord = {
  id: string;
  device_id: string;
  capability: string;
  action: string;
  arguments?: unknown;
  requested_scopes?: string[];
  resource_scope?: Record<string, string[]>;
  risk: CompanionRiskLevel;
  approval_status: string;
  status: string;
  requested_at: string;
  approved_at?: string | null;
  dispatched_at?: string | null;
  completed_at?: string | null;
  success?: boolean;
  result_preview?: string | null;
  result_trust?: string;
  actor?: string | null;
  audit_reason?: string | null;
  error?: string | null;
};

export type CompanionAuditEvent = {
  id: string;
  event_type: string;
  device_id?: string | null;
  command_id?: string | null;
  grant_id?: string | null;
  actor?: string | null;
  surface?: string | null;
  decision: string;
  reason: string;
  timestamp: string;
  metadata?: Record<string, string>;
  previous_hash?: string | null;
  event_hash?: string | null;
};

export type CompanionPresetsResponse = {
  presets: CompanionPreset[];
  capabilities: CompanionCapabilityDescriptor[];
  protocol_version: string;
};

export type CompanionProtocolDocument = {
  protocol_version: string;
  websocket_path: string;
  auth: string;
  pairing: string;
  messages: string[];
  commands?: CompanionCommandDescriptor[];
  security: string[];
};

export type CompanionDevicesResponse = {
  status?: string;
  devices: CompanionDeviceRecord[];
  overview?: {
    generated_at: string;
    total: number;
    online: number;
    pending_pairing: number;
    pending_approvals: number;
    revoked: number;
  };
  pairing_sessions?: CompanionPairingSession[];
  pending_approvals?: CompanionCommandRecord[];
};

export type CompanionCommandsResponse = {
  status?: string;
  commands: CompanionCommandRecord[];
};

export type CompanionAuditResponse = {
  status?: string;
  events: CompanionAuditEvent[];
};

export type CompanionMobileAccessChannel = {
  id: string;
  label: string;
  kind: string;
  configured: boolean;
  ready: boolean;
  detail: string;
  settings_path: string;
};

export type CompanionMobileAccessResponse = {
  phone_companion: {
    paired: number;
    online: number;
    capabilities: string[];
    can_read_phone_messages: boolean;
    detail: string;
  };
  message_channels: CompanionMobileAccessChannel[];
  sms: CompanionMobileAccessChannel;
  truth_notes: string[];
};

export type BrowserProfileRecord = {
  id: string;
  name: string;
  description?: string | null;
  target_kind: string;
  target_endpoint?: string | null;
  target_profile_path?: string | null;
  target_workspace?: string | null;
  login_state: string;
  login_checked_at?: string | null;
  login_note?: string | null;
  lock?: {
    owner: string;
    reason?: string | null;
    locked_at: string;
    expires_at?: string | null;
  } | null;
  recent_sessions?: Array<{
    id: string;
    started_at: string;
    ended_at?: string | null;
    duration_secs?: number | null;
    outcome: string;
    title?: string | null;
    url?: string | null;
    channel?: string | null;
    note?: string | null;
  }>;
  tags?: string[];
  enabled?: boolean;
  last_used_at?: string | null;
  last_error?: string | null;
  metadata?: Record<string, unknown> | null;
};

export type BrowserProfilesResponse = {
  summary?: {
    total: number;
    sandbox: number;
    host: number;
    remote_cdp: number;
    locked: number;
    logged_in: number;
    needs_attention: number;
  };
  profiles: BrowserProfileRecord[];
};

export type ModelFailoverResponse = {
  summary?: {
    auth_profiles: number;
    providers: number;
    disabled_providers: number;
    cooling_providers: number;
    chains: number;
    session_pins: number;
  };
  auth_profiles: Array<{
    id: string;
    name: string;
    provider_id: string;
    provider_kind?: string | null;
    base_url?: string | null;
    model_id?: string | null;
    credential_ref?: string | null;
    enabled: boolean;
    priority: number;
    last_used_at?: string | null;
    last_error?: string | null;
    session_pin?: {
      session_id: string;
      model_id?: string | null;
      chain_id?: string | null;
      provider_id?: string | null;
      auth_profile_id?: string | null;
      reason?: string | null;
      pinned_at?: string | null;
      expires_at?: string | null;
    } | null;
    tags?: string[];
    metadata?: Record<string, unknown> | null;
  }>;
  provider_health: Array<{
    provider_id: string;
    provider_kind?: string | null;
    enabled: boolean;
    disabled: boolean;
    cooldown_until?: string | null;
    last_success_at?: string | null;
    last_failure_at?: string | null;
    success_count: number;
    failure_count: number;
    last_error?: string | null;
    health_note?: string | null;
    session_pin_count?: number;
    metadata?: Record<string, unknown> | null;
  }>;
  fallback_chains: Array<{
    id: string;
    name: string;
    enabled: boolean;
    ordered_candidates: Array<{
      provider_id: string;
      auth_profile_id?: string | null;
      priority: number;
      reason?: string | null;
    }>;
    session_pin?: {
      session_id: string;
      model_id?: string | null;
      chain_id?: string | null;
      provider_id?: string | null;
      auth_profile_id?: string | null;
      reason?: string | null;
      pinned_at?: string | null;
      expires_at?: string | null;
    } | null;
    notes?: string | null;
    metadata?: Record<string, unknown> | null;
  }>;
};

export type SkillImportRequest = {
  url: string;
  name?: string;
  force?: boolean;
  model?: string;
  preview_only?: boolean;
  selected_urls?: string[];
  enabled?: boolean;
};

export type SkillImportResponse = {
  status: "ok" | "blocked" | "needs_secrets" | string;
  name: string;
  message: string;
  source_url?: string;
  enabled?: boolean;
  visible_in_catalog?: boolean;
  enabled_by_default?: boolean;
  catalog_index_status?: "queued" | "refreshed" | "unmodified" | "unavailable" | string;
  compatibility?: {
    mode?: "workflow_only" | "cli_skill" | "bundle_references" | string;
    install_route?: "workflow" | "cli_skill" | "extension_pack" | string;
    unsupported_references?: string[];
    warnings?: string[];
  };
  imported_count?: number;
  failed_count?: number;
  imported?: Array<{ url?: string; result?: SkillImportResponse }>;
  failed?: Array<{ url?: string; error?: string }>;
  secrets?: {
    required_env?: string[];
    missing_env?: string[];
    bindings?: Record<string, string>;
  };
  security?: {
    threat_level?: string;
    warnings?: string[];
    findings?: Array<{
      category?: string;
      label?: string;
      description?: string;
      explanation?: string;
      matched_text?: string;
      line?: number;
      file?: string;
      severity?: number;
      contextual?: boolean;
    }>;
    blocked?: boolean;
    total_severity?: number;
    risk_score_10?: number;
    risk_band?: "secure" | "review" | "risky" | string;
    total_findings?: number;
    contextual_findings?: number;
    capabilities?: Array<{
      kind?: string;
      target?: string | null;
      evidence?: string | null;
      confidence?: number | null;
    }>;
    matched_rules?: Array<{
      id?: string;
      effect?: string;
      message?: string;
      severity?: number;
    }>;
    review_model?: string;
    review_summary?: string;
  };
};

export type SkillSecretsResponse = {
  required_env: string[];
  missing_env: string[];
  bindings: Record<string, string>;
  configured: Record<string, boolean>;
};

export type SkillSecretsUpdateRequest = {
  secrets: Array<{
    env: string;
    store_as?: string;
    value?: string;
  }>;
};

export type SkillTestResponse = {
  status: "ok" | "error" | "needs_input" | string;
  mode?: "workflow" | "native" | string;
  skill?: string;
  arguments?: unknown;
  output?: string;
  error?: string;
  message?: string;
  missing_inputs?: string[];
  required_inputs?: string[];
};

// Backward-compatible alias while older components still refer to actions as skills.
export type RecommendedSkill = RecommendedAction;
export type ActionImportRequest = SkillImportRequest;
export type ActionImportResponse = SkillImportResponse;
export type ActionSecretsResponse = SkillSecretsResponse;
export type ActionSecretsUpdateRequest = SkillSecretsUpdateRequest;
export type ActionTestResponse = SkillTestResponse;
