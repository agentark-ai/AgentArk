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
