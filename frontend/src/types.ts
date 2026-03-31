export type StatusResponse = {
  did: string;
  memory_entries: number;
  skills_loaded?: number;
  actions_loaded?: number;
  tasks_pending: number;
  version: string;
};

export type Task = {
  id: string;
  description: string;
  status: unknown;
  created_at?: string;
  cron?: string;
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
  linked_task_ids: string[];
  linked_watcher_ids: string[];
  created_at: string;
  updated_at: string;
  last_activity_at: string;
  live_summary: string;
  counts: BackgroundSessionCounts;
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
  status: string;
  created_at: string;
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
  notify_channel?: string | null;
  last_error?: string | null;
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
    project_id?: string | null;
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

export type ArkPulseRemediationSpec =
  | { kind: "tunnel_start_verify" }
  | { kind: "tunnel_restart_verify" }
  | { kind: "app_restart"; app_id: string }
  | { kind: "shell_command"; command: string };

export type ArkPulseDoctorFinding = {
  severity: string;
  category: string;
  target: string;
  title: string;
  evidence: string;
  root_cause: string;
  fix_command: string;
  remediation?: ArkPulseRemediationSpec | null;
  user_actionable?: boolean;
};

export type ArkPulseRunFixRequest = {
  fix_command?: string;
  remediation?: ArkPulseRemediationSpec;
  issue_title?: string;
  target?: string;
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

export type TraceResponse = {
  history: TraceSummary[];
  history_total?: number;
};

export type RecommendedSkill = {
  id: string;
  title: string;
  summary?: string;
  description?: string;
  skill_kind: string;
  payload: Record<string, unknown>;
  requires_approval?: boolean;
  trust?: {
    level?: string;
    score?: number;
    requires_approval?: boolean;
    reasons?: string[];
  };
};

export type BriefingResponse = {
  generated_at: string;
  scope: string;
  top_risks: Array<{ title?: string; summary?: string; detail?: string; severity?: string }>;
  top_opportunities: Array<{ title?: string; summary?: string; detail?: string; score?: number }>;
  recommended_skills: RecommendedSkill[];
  trust_summary: Record<string, unknown>;
};

export type MemoryContextSummary = {
  id: string;
  summary: string;
  memory_type: string;
  timestamp: string;
  channel?: string;
  importance: number;
};

export type PredictiveNudge = {
  id: string;
  type: string;
  title: string;
  detail: string;
  confidence: number;
  priority: number;
  source?: string;
  recommended_skill?: RecommendedSkill;
  memory_clues?: MemoryContextSummary[];
};

export type PredictiveNudgesResponse = {
  generated_at: string;
  nudges: PredictiveNudge[];
  hidden_count?: number;
};

export type LlmAnalyticsTotals = {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  request_count: number;
  estimated_count: number;
  cost_usd?: number | null;
};

export type LlmAnalyticsPoint = {
  bucket_start: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  request_count: number;
  primary_prompt_tokens: number;
  primary_completion_tokens: number;
  primary_total_tokens: number;
  primary_request_count: number;
  helper_prompt_tokens: number;
  helper_completion_tokens: number;
  helper_total_tokens: number;
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
  request_count: number;
  cost_usd?: number | null;
};

export type LlmAnalyticsResponse = {
  range: { since: string; until: string; bucket: "hour" | "day" | "week" | string };
  totals: LlmAnalyticsTotals;
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
};

export type SkillImportResponse = {
  status: "ok" | "blocked" | "needs_secrets" | string;
  name: string;
  message: string;
  source_url?: string;
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
      description?: string;
      matched_text?: string;
      line?: number;
      severity?: number;
    }>;
    blocked?: boolean;
    total_severity?: number;
    risk_score_10?: number;
    risk_band?: "secure" | "review" | "risky" | string;
    total_findings?: number;
    contextual_findings?: number;
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

// Backward-compatible aliases while moving from "actions" to "skills".
export type RecommendedAction = RecommendedSkill;
export type ActionImportRequest = SkillImportRequest;
export type ActionImportResponse = SkillImportResponse;
export type ActionSecretsResponse = SkillSecretsResponse;
export type ActionSecretsUpdateRequest = SkillSecretsUpdateRequest;
export type ActionTestResponse = SkillTestResponse;
