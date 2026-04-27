import {
  Alert,
  Autocomplete,
  Box,
  Button,
  ButtonBase,
  CircularProgress,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  MenuItem,
  Stack,
  TextField,
  Typography
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import { WorkspacePageHeader, WorkspacePageShell } from "./WorkspacePage";

const REFRESH_MS = 8000;

type JsonRecord = Record<string, unknown>;

type Props = {
  autoRefresh: boolean;
};

type ProvisionedAgent = {
  id: string;
  name: string;
  displayName: string;
  isSystem: boolean;
  agentType: string;
  provider: string;
  model: string;
  llmBaseUrl: string;
  capabilities: string[];
  systemPrompt: string;
  accessScope: AccessScope;
  createdAt: string;
  status: string;
  enabled: boolean;
  lastTask: string;
  lastSummary: string;
  lastUpdate: string;
  lastActivityAt: string;
};

type AccessScope = {
  approved_permission_ids: string[];
  mcp_server_ids: string[];
  ssh_connection_names: string[];
  custom_api_ids: string[];
  integration_ids: string[];
  extension_pack_ids: string[];
  channel_ids: string[];
};

type AccessScopeKey = keyof AccessScope;

type ResourceAccessScopeKey = Exclude<AccessScopeKey, "approved_permission_ids">;

type BuilderOption = {
  id: string;
  label: string;
  helper: string;
  status: string;
  enabled: boolean;
};

type BuilderOptions = {
  mcpServers: BuilderOption[];
  sshConnections: BuilderOption[];
  customApis: BuilderOption[];
  integrations: BuilderOption[];
  extensionPacks: BuilderOption[];
  channels: BuilderOption[];
};

type AccessPlanAction = {
  name: string;
  reason: string;
};

type AccessPlanDetail = {
  actionName: string;
  reason: string;
  permissionIds: string[];
};

type AccessPlanGroup = {
  id: string;
  scopeField: AccessScopeKey;
  label: string;
  summary: string;
  reason: string;
  reviewBand: string;
  selectionMode: string;
  suggestedIds: string[];
  details: AccessPlanDetail[];
};

type AccessPlan = {
  implicitAccess: AccessPlanGroup[];
  requestedAccess: AccessPlanGroup[];
  suggestedActions: AccessPlanAction[];
  notes: string[];
};

type SwarmRunAgent = {
  id: string;
  agentName: string;
  agentRole: string;
  modelName: string;
  task: string;
  status: string;
  summary: string;
  latestUpdate: string;
  isSpecialist: boolean;
  elapsedMs?: number;
};

type SwarmRun = {
  id: string;
  conversationId: string;
  channel: string;
  request: string;
  status: string;
  summary: string;
  startedAt: string;
  updatedAt: string;
  completedAt: string;
  agentCount: number;
  agents: SwarmRunAgent[];
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function asRecords(value: unknown): JsonRecord[] {
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function pickRecords(value: unknown, key: string): JsonRecord[] {
  if (Array.isArray(value)) return asRecords(value);
  const obj = asRecord(value);
  return asRecords(obj[key]);
}

function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function bool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    return ["1", "true", "yes", "on"].includes(value.trim().toLowerCase());
  }
  return false;
}

function errMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Request failed.";
}

function formatTimestamp(value: unknown): string {
  const raw = str(value, "").trim();
  return formatUiDateTime(raw, { fallback: "-" });
}

function formatElapsedMs(value: unknown): string {
  const ms = Math.max(0, num(value, 0));
  if (!ms) return "";
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(secs >= 10 ? 0 : 1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return remSecs > 0 ? `${mins}m ${remSecs}s` : `${mins}m`;
}

function parseCapabilities(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value
      .map((item) => {
        if (typeof item === "string") return item.trim();
        const rec = asRecord(item);
        return str(rec.name, "").trim() || str(rec.description, "").trim();
      })
      .filter(Boolean);
  }
  const raw = str(value, "").trim();
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    return parseCapabilities(parsed);
  } catch {
    return raw
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  }
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean)));
}

function parseStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return uniqueStrings(
    value
      .map((item) => str(item, "").trim())
      .filter(Boolean)
  );
}

function emptyAccessScope(): AccessScope {
  return {
    approved_permission_ids: [],
    mcp_server_ids: [],
    ssh_connection_names: [],
    custom_api_ids: [],
    integration_ids: [],
    extension_pack_ids: [],
    channel_ids: []
  };
}

function cloneAccessScope(scope?: AccessScope | null): AccessScope {
  const current = scope ?? emptyAccessScope();
  return {
    approved_permission_ids: [...current.approved_permission_ids],
    mcp_server_ids: [...current.mcp_server_ids],
    ssh_connection_names: [...current.ssh_connection_names],
    custom_api_ids: [...current.custom_api_ids],
    integration_ids: [...current.integration_ids],
    extension_pack_ids: [...current.extension_pack_ids],
    channel_ids: [...current.channel_ids]
  };
}

function parseAccessScope(value: unknown): AccessScope {
  const record = asRecord(value);
  return {
    approved_permission_ids: parseStringArray(record.approved_permission_ids),
    mcp_server_ids: parseStringArray(record.mcp_server_ids),
    ssh_connection_names: parseStringArray(record.ssh_connection_names),
    custom_api_ids: parseStringArray(record.custom_api_ids),
    integration_ids: parseStringArray(record.integration_ids),
    extension_pack_ids: parseStringArray(record.extension_pack_ids),
    channel_ids: parseStringArray(record.channel_ids)
  };
}

function normalizeLifecycleStatus(status: unknown): string {
  const normalized = str(status, "").trim().toLowerCase();
  if (!normalized) return "idle";
  if (normalized === "busy") return "running";
  if (normalized === "success") return "completed";
  if (normalized === "cancelled" || normalized === "canceled") return "interrupted";
  if (normalized === "degraded") return "partial";
  return normalized;
}

function statusChipColor(status: unknown): "default" | "success" | "warning" | "error" {
  switch (normalizeLifecycleStatus(status)) {
    case "completed":
    case "provisioned":
    case "idle":
      return "success";
    case "running":
    case "assigned":
    case "synthesizing":
    case "partial":
      return "warning";
    case "failed":
    case "timed_out":
    case "panicked":
    case "interrupted":
    case "offline":
    case "disabled":
      return "error";
    default:
      return "default";
  }
}

function statusChipLabel(status: unknown): string {
  switch (normalizeLifecycleStatus(status)) {
    case "assigned":
      return "Assigned";
    case "running":
      return "Running";
    case "synthesizing":
      return "Synthesizing";
    case "completed":
      return "Completed";
    case "partial":
      return "Partial";
    case "failed":
      return "Failed";
    case "timed_out":
      return "Timed out";
    case "panicked":
      return "Panicked";
    case "interrupted":
      return "Stopped";
    case "offline":
      return "Offline";
    case "disabled":
      return "Disabled";
    case "provisioned":
      return "Provisioned";
    default:
      return "Idle";
  }
}

function statusDotColor(status: unknown): string {
  switch (normalizeLifecycleStatus(status)) {
    case "running":
    case "assigned":
    case "synthesizing":
      return "var(--ui-rgba-74-210-157-850)";
    case "failed":
    case "timed_out":
    case "panicked":
    case "offline":
      return "var(--ui-rgba-255-100-100-850)";
    case "completed":
    case "interrupted":
    case "partial":
    case "disabled":
      return "var(--ui-rgba-255-191-130-850)";
    default:
      return "var(--ui-rgba-180-200-220-500)";
  }
}

function toProvisionedAgents(data: unknown): ProvisionedAgent[] {
  return pickRecords(data, "agents")
    .map((agent) => ({
      id: str(agent.id, ""),
      name: str(agent.name, "Agent"),
      displayName: str(agent.display_name, str(agent.name, "Agent")),
      isSystem: bool(agent.is_system),
      agentType: str(agent.agent_type, "Agent"),
      provider: str(agent.llm_provider, "-"),
      model: str(agent.llm_model, "-"),
      llmBaseUrl: str(agent.llm_base_url, ""),
      capabilities: parseCapabilities(agent.capabilities),
      systemPrompt: str(agent.system_prompt, ""),
      accessScope: parseAccessScope(agent.access_scope),
      createdAt: str(agent.created_at, ""),
      status: normalizeLifecycleStatus(agent.status),
      enabled: bool(agent.enabled),
      lastTask: str(agent.last_task, ""),
      lastSummary: str(agent.last_summary, ""),
      lastUpdate: str(agent.last_update, ""),
      lastActivityAt: str(agent.last_activity_at, "")
    }))
    .sort((left, right) => {
      const leftTs = Date.parse(left.lastActivityAt || left.createdAt || "");
      const rightTs = Date.parse(right.lastActivityAt || right.createdAt || "");
      return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
    });
}

type AgentDraft = {
  description: string;
  name: string;
  agent_type: string;
  model_profile_id: string;
  llm_provider: string;
  llm_model: string;
  llm_base_url: string;
  llm_api_key: string;
  capabilities: string;
  system_prompt: string;
  access_scope: AccessScope;
};

type SavedModelProfile = {
  id: string;
  label: string;
  role: string;
  provider: string;
  model: string;
  baseUrl: string;
  enabled: boolean;
};

const EMPTY_DRAFT: AgentDraft = {
  description: "",
  name: "",
  agent_type: "",
  model_profile_id: "",
  llm_provider: "ollama",
  llm_model: "",
  llm_base_url: "http://localhost:11434",
  llm_api_key: "",
  capabilities: "",
  system_prompt: "",
  access_scope: emptyAccessScope()
};

function formatProfileCount(count: number): string {
  return `${count} profile${count === 1 ? "" : "s"}`;
}

function formatProfileRole(role: string): string {
  const normalized = role.trim().toLowerCase();
  switch (normalized) {
    case "primary":
      return "Primary";
    case "fast":
      return "Fast";
    case "code":
      return "Code";
    case "research":
      return "Research";
    case "fallback":
      return "Fallback";
    default:
      return normalized ? normalized.charAt(0).toUpperCase() + normalized.slice(1) : "Profile";
  }
}

function toSavedModelProfiles(data: unknown): SavedModelProfile[] {
  return pickRecords(data, "models")
    .map((slot) => ({
      id: str(slot.id, ""),
      label: str(slot.label, "").trim(),
      role: str(slot.role, "primary").trim(),
      provider: str(slot.provider, "").trim(),
      model: str(slot.model, "").trim(),
      baseUrl: str(slot.base_url, "").trim(),
      enabled: bool(slot.enabled)
    }))
    .filter((profile) => profile.id && profile.model)
    .sort((left, right) => {
      if (left.enabled !== right.enabled) return left.enabled ? -1 : 1;
      return left.label.localeCompare(right.label);
    });
}

function applyModelProfileToDraft(draft: AgentDraft, profile: SavedModelProfile | null): AgentDraft {
  if (!profile) {
    return {
      ...draft,
      model_profile_id: "",
      llm_provider: "",
      llm_model: "",
      llm_base_url: "",
      llm_api_key: ""
    };
  }
  return {
    ...draft,
    model_profile_id: profile.id,
    llm_provider: profile.provider,
    llm_model: profile.model,
    llm_base_url: profile.baseUrl,
    llm_api_key: ""
  };
}

function toBuilderOption(
  row: JsonRecord,
  idFallback: string,
  labelFallback: string,
  helper: string
): BuilderOption | null {
  const id = str(row.id, idFallback).trim();
  const label = str(row.name, labelFallback).trim() || id;
  if (!id) return null;
  return {
    id,
    label,
    helper,
    status: str(row.status, "").trim(),
    enabled: row.enabled === undefined ? true : bool(row.enabled)
  };
}

function formatBuilderStatus(status: string): string {
  const normalized = status.trim().toLowerCase();
  if (!normalized) return "";
  if (normalized === "ok") return "Healthy";
  return normalized
    .split(/[_\-\s]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function builderStatusLooksUsable(status: unknown): boolean {
  const normalized = str(status, "").trim().toLowerCase();
  if (!normalized) return true;
  return ![
    "disabled",
    "not_configured",
    "missing_config",
    "missing_token",
    "offline",
    "unavailable",
    "disconnected",
    "error",
    "failed"
  ].includes(normalized);
}

function sortBuilderOptions(options: BuilderOption[]): BuilderOption[] {
  return [...options].sort((left, right) => left.label.localeCompare(right.label));
}

function normalizeBuilderHelper(helper: string): string {
  return helper
    .replace(/\u00e2\u20ac\u00a2|\u2022/g, "|")
    .replace(/\s*\|\s*/g, " | ")
    .replace(/\s{2,}/g, " ")
    .replace(/^\|\s*|\s*\|$/g, "")
    .trim();
}

function filterBuilderOptions(options: BuilderOptions): BuilderOptions {
  const cleanedItems = (items: BuilderOption[]) =>
    items.map((item) => ({
      ...item,
      helper: normalizeBuilderHelper(item.helper)
    }));
  const usableItems = (items: BuilderOption[]) =>
    sortBuilderOptions(
      cleanedItems(items).filter((item) => item.enabled && builderStatusLooksUsable(item.status))
    );

  return {
    mcpServers: usableItems(options.mcpServers),
    sshConnections: sortBuilderOptions(cleanedItems(options.sshConnections)),
    customApis: sortBuilderOptions(cleanedItems(options.customApis).filter((item) => item.enabled)),
    integrations: usableItems(options.integrations),
    extensionPacks: usableItems(options.extensionPacks),
    channels: usableItems(options.channels)
  };
}

function toBuilderOptions(data: unknown): BuilderOptions {
  const payload = asRecord(data);
  const mcpServers = pickRecords(payload.mcp_servers, "mcp_servers")
    .map((row) =>
      toBuilderOption(
        row,
        str(row.id, ""),
        str(row.name, str(row.id, "")),
        [str(row.description, ""), `${num(row.tool_count, 0)} tools`, `${num(row.resource_count, 0)} resources`]
          .filter(Boolean)
          .join(" | ")
      )
    )
    .filter(
      (item): item is BuilderOption =>
        item !== null && item.enabled && builderStatusLooksUsable(item.status)
    );
  const sshConnections = pickRecords(payload.ssh_connections, "ssh_connections")
    .map((row) =>
      toBuilderOption(
        { ...row, id: str(row.name, "") },
        str(row.name, ""),
        str(row.name, ""),
        `${str(row.username, "")}@${str(row.host, "")}:${str(row.port, "")}`
      )
    )
    .filter((item): item is BuilderOption => Boolean(item));
  const customApis = pickRecords(payload.custom_apis, "custom_apis")
    .map((row) =>
      toBuilderOption(
        row,
        str(row.id, ""),
        str(row.name, str(row.id, "")),
        [str(row.base_url, ""), `${num(row.action_count, 0)} actions`].filter(Boolean).join(" | ")
      )
    )
    .filter((item): item is BuilderOption => item !== null && item.enabled);
  const integrations = pickRecords(payload.integrations, "integrations")
    .map((row) =>
      toBuilderOption(
        row,
        str(row.id, ""),
        str(row.name, str(row.id, "")),
        [str(row.description, ""), formatBuilderStatus(str(row.status, ""))].filter(Boolean).join(" | ")
      )
    )
    .filter(
      (item): item is BuilderOption =>
        item !== null && item.enabled && builderStatusLooksUsable(item.status)
    );
  const extensionPacks = pickRecords(payload.extension_packs, "extension_packs")
    .map((row) =>
      toBuilderOption(
        row,
        str(asRecord(row.manifest).id || row.id, ""),
        str(asRecord(row.manifest).name || row.name, str(asRecord(row.manifest).id || row.id, "")),
        [
          str(asRecord(row.manifest).description || row.description, ""),
          formatBuilderStatus(str(row.status, "")),
          str(row.runtime_status, "")
        ]
          .filter(Boolean)
          .join(" | ")
      )
    )
    .filter(
      (item): item is BuilderOption =>
        item !== null && item.enabled && builderStatusLooksUsable(item.status)
    );
  const channels = pickRecords(payload.channels, "channels")
    .flatMap((row) => {
      if (!bool(row.enabled) || !bool(row.configured) || !builderStatusLooksUsable(row.status)) {
        return [];
      }
      const option = toBuilderOption(
        row,
        str(row.id, ""),
        str(row.name, str(row.id, "")),
        [
          str(row.kind, ""),
          num(row.connected_account_count, 0) > 0
            ? `${num(row.connected_account_count, 0)} account${num(row.connected_account_count, 0) === 1 ? "" : "s"}`
            : "",
          "ready"
        ]
          .filter(Boolean)
          .join(" | ")
      );
      return option ? [option] : [];
    });
  return {
    mcpServers: pickRecords(payload.mcp_servers, "mcp_servers")
      .map((row) =>
        toBuilderOption(
          row,
          str(row.id, ""),
          str(row.name, str(row.id, "")),
          [str(row.description, ""), `${num(row.tool_count, 0)} tools`, `${num(row.resource_count, 0)} resources`]
            .filter(Boolean)
            .join(" | ")
        )
      )
      .filter((item): item is BuilderOption => Boolean(item)),
    sshConnections: pickRecords(payload.ssh_connections, "ssh_connections")
      .map((row) =>
        toBuilderOption(
          { ...row, id: str(row.name, "") },
          str(row.name, ""),
          str(row.name, ""),
          `${str(row.username, "")}@${str(row.host, "")}:${str(row.port, "")}`
        )
      )
      .filter((item): item is BuilderOption => Boolean(item)),
    customApis: pickRecords(payload.custom_apis, "custom_apis")
      .map((row) =>
        toBuilderOption(
          row,
          str(row.id, ""),
          str(row.name, str(row.id, "")),
          [str(row.base_url, ""), `${num(row.action_count, 0)} actions`].filter(Boolean).join(" | ")
        )
      )
      .filter((item): item is BuilderOption => Boolean(item)),
    integrations: pickRecords(payload.integrations, "integrations")
      .map((row) =>
        toBuilderOption(
          row,
          str(row.id, ""),
          str(row.name, str(row.id, "")),
          [str(row.description, ""), str(row.status, "")].filter(Boolean).join(" | ")
        )
      )
      .filter((item): item is BuilderOption => Boolean(item)),
    extensionPacks: pickRecords(payload.extension_packs, "extension_packs")
      .map((row) =>
        toBuilderOption(
          row,
          str(asRecord(row.manifest).id || row.id, ""),
          str(asRecord(row.manifest).name || row.name, str(asRecord(row.manifest).id || row.id, "")),
          [
            str(asRecord(row.manifest).description || row.description, ""),
            str(row.runtime_status, ""),
            str(row.status, "")
          ]
            .filter(Boolean)
            .join(" | ")
        )
      )
      .filter((item): item is BuilderOption => Boolean(item)),
    channels: pickRecords(payload.channels, "channels")
      .map((row) =>
        toBuilderOption(
          row,
          str(row.id, ""),
          str(row.name, str(row.id, "")),
          [
            str(row.kind, ""),
            bool(row.configured) ? "configured" : "not configured",
            str(row.status, "")
          ]
            .filter(Boolean)
            .join(" | ")
        )
      )
      .filter((item): item is BuilderOption => Boolean(item))
  };
}

function buildOptionMap(options: BuilderOption[]): Record<string, BuilderOption> {
  return options.reduce<Record<string, BuilderOption>>((acc, option) => {
    acc[option.id] = option;
    return acc;
  }, {});
}

function formatPermissionLabel(permissionId: string): string {
  const normalized = permissionId.trim().toLowerCase();
  switch (normalized) {
    case "code_execute":
      return "Code execution";
    case "shell":
      return "Shell commands";
    case "file_write":
      return "File writes";
    case "scheduler":
      return "Task scheduling";
    case "local_network_discovery":
      return "Local network discovery";
    case "browser_auto":
      return "Browser automation";
    case "app_hosting":
      return "App hosting";
    case "messaging_send":
      return "Messaging send";
    case "broad_network":
      return "Broad network actions";
    case "ssh":
      return "SSH execution";
    case "gmail":
      return "Gmail access";
    case "calendar_write":
      return "Calendar write";
    case "google_workspace_command":
      return "Workspace command execution";
    case "watcher":
      return "Background watchers";
    default:
      return normalized
        .split(/[_\-\s]+/)
        .filter(Boolean)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join(" ");
  }
}

function parseAccessPlanDetail(value: unknown): AccessPlanDetail {
  const record = asRecord(value);
  return {
    actionName: str(record.action_name, ""),
    reason: str(record.reason, ""),
    permissionIds: parseStringArray(record.permission_ids)
  };
}

function parseAccessPlanGroup(value: unknown): AccessPlanGroup {
  const record = asRecord(value);
  const scopeField = str(record.scope_field, "approved_permission_ids") as AccessScopeKey;
  return {
    id: str(record.id, `${scopeField}:${str(record.label, "group")}`),
    scopeField,
    label: str(record.label, "Access"),
    summary: str(record.summary, ""),
    reason: str(record.reason, ""),
    reviewBand: str(record.review_band, "elevated"),
    selectionMode: str(record.selection_mode, "toggle"),
    suggestedIds: parseStringArray(record.suggested_ids),
    details: asRecords(record.details).map(parseAccessPlanDetail)
  };
}

function parseAccessPlan(value: unknown): AccessPlan {
  const record = asRecord(value);
  return {
    implicitAccess: asRecords(record.implicit_access).map(parseAccessPlanGroup),
    requestedAccess: asRecords(record.requested_access).map(parseAccessPlanGroup),
    suggestedActions: asRecords(record.suggested_actions).map((item) => ({
      name: str(item.name, ""),
      reason: str(item.reason, "")
    })),
    notes: parseStringArray(record.notes)
  };
}

function findMatchingModelProfile(
  agent: ProvisionedAgent,
  profiles: SavedModelProfile[]
): SavedModelProfile | null {
  return (
    profiles.find(
      (profile) =>
        profile.provider === agent.provider &&
        profile.model === agent.model &&
        (profile.baseUrl || "") === (agent.llmBaseUrl || "")
    ) ?? null
  );
}

function accessScopeLabels(
  scope: AccessScope,
  optionMaps: {
    mcp: Record<string, BuilderOption>;
    ssh: Record<string, BuilderOption>;
    customApi: Record<string, BuilderOption>;
    integration: Record<string, BuilderOption>;
    extensionPack: Record<string, BuilderOption>;
    channel: Record<string, BuilderOption>;
  }
): string[] {
  return [
    ...scope.approved_permission_ids.map((id) => formatPermissionLabel(id)),
    ...scope.mcp_server_ids.map((id) => optionMaps.mcp[id]?.label || id),
    ...scope.ssh_connection_names.map((id) => optionMaps.ssh[id]?.label || id),
    ...scope.custom_api_ids.map((id) => optionMaps.customApi[id]?.label || id),
    ...scope.integration_ids.map((id) => optionMaps.integration[id]?.label || id),
    ...scope.extension_pack_ids.map((id) => optionMaps.extensionPack[id]?.label || id),
    ...scope.channel_ids.map((id) => optionMaps.channel[id]?.label || id)
  ];
}

function accessScopeSummary(scope: AccessScope): string[] {
  const parts: string[] = [];
  if (scope.approved_permission_ids.length) {
    parts.push(`${scope.approved_permission_ids.length} permission${scope.approved_permission_ids.length === 1 ? "" : "s"}`);
  }
  if (scope.mcp_server_ids.length) parts.push(`${scope.mcp_server_ids.length} MCP`);
  if (scope.ssh_connection_names.length) parts.push(`${scope.ssh_connection_names.length} SSH`);
  if (scope.custom_api_ids.length) parts.push(`${scope.custom_api_ids.length} API`);
  if (scope.integration_ids.length) parts.push(`${scope.integration_ids.length} integration`);
  if (scope.extension_pack_ids.length) parts.push(`${scope.extension_pack_ids.length} pack`);
  if (scope.channel_ids.length) parts.push(`${scope.channel_ids.length} channel`);
  return parts;
}

function mergeAccessScopeOptions(options: BuilderOption[], selectedIds: string[]): BuilderOption[] {
  if (selectedIds.length === 0) return options;
  const knownIds = new Set(options.map((option) => option.id));
  const unavailableSelections = selectedIds
    .filter((id) => id && !knownIds.has(id))
    .map(
      (id): BuilderOption => ({
        id,
        label: id,
        helper: "Currently unavailable in Settings.",
        status: "unavailable",
        enabled: false
      })
    );
  return [...options, ...unavailableSelections];
}

function toSwarmRuns(data: unknown): SwarmRun[] {
  return pickRecords(data, "runs")
    .map((run) => ({
      id: str(run.id, ""),
      conversationId: str(run.conversation_id, ""),
      channel: str(run.channel, ""),
      request: str(run.request, "Delegated run"),
      status: normalizeLifecycleStatus(run.status),
      summary: str(run.summary, ""),
      startedAt: str(run.started_at, ""),
      updatedAt: str(run.updated_at, ""),
      completedAt: str(run.completed_at, ""),
      agentCount: Math.max(0, num(run.agent_count, 0)),
      agents: pickRecords(run.agents, "agents").map((agent) => ({
        id: str(agent.id, ""),
        agentName: str(agent.agent_name, "Agent"),
        agentRole: str(agent.agent_role, ""),
        modelName: str(agent.model_name, ""),
        task: str(agent.task, ""),
        status: normalizeLifecycleStatus(agent.status),
        summary: str(agent.summary, ""),
        latestUpdate: str(agent.latest_update, ""),
        isSpecialist: bool(agent.is_specialist),
        elapsedMs: num(agent.elapsed_ms, 0) || undefined
      }))
    }))
    .filter((run) => run.id)
    .sort((left, right) => {
      const leftTs = Date.parse(left.updatedAt || left.startedAt || "");
      const rightTs = Date.parse(right.updatedAt || right.startedAt || "");
      return (Number.isFinite(rightTs) ? rightTs : 0) - (Number.isFinite(leftTs) ? leftTs : 0);
    });
}

function SectionShell({
  eyebrow,
  title,
  detail,
  children
}: {
  eyebrow: string;
  title: string;
  detail: string;
  children: ReactNode;
}) {
  return (
    <Box
      sx={{
        p: { xs: 2, md: 2.35 },
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-255-255-255-070)",
        background:
          "linear-gradient(180deg, var(--ui-rgba-255-255-255-050) 0%, var(--ui-rgba-255-255-255-025) 100%)",
        boxShadow: "0 18px 40px var(--ui-rgba-7-16-32-220)"
      }}
    >
      <Stack spacing={1.4}>
        <Box>
          <Typography variant="overline" sx={{ letterSpacing: 0, color: "info.light" }}>
            {eyebrow}
          </Typography>
          <Typography variant="h6" sx={{ fontWeight: 800 }}>
            {title}
          </Typography>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              mt: 0.35,
              maxWidth: 860
            }}>
            {detail}
          </Typography>
        </Box>
        {children}
      </Stack>
    </Box>
  );
}

function RunCard({ run, live = false }: { run: SwarmRun; live?: boolean }) {
  const trackedAgents = Math.max(run.agentCount, run.agents.length);
  return (
    <Box
      sx={{
        p: 1.5,
        borderRadius: "8px",
        border: live
          ? "1px solid var(--ui-rgba-88-174-255-180)"
          : "1px solid var(--ui-rgba-255-255-255-070)",
        background: live
          ? "linear-gradient(180deg, var(--ui-rgba-88-174-255-100) 0%, var(--ui-rgba-255-255-255-030) 100%)"
          : "linear-gradient(180deg, var(--ui-rgba-255-255-255-040) 0%, var(--ui-rgba-255-255-255-020) 100%)"
      }}
    >
      <Stack spacing={1.2}>
        <Stack
          direction={{ xs: "column", md: "row" }}
          sx={{
            alignItems: { xs: "flex-start", md: "center" },
            justifyContent: "space-between",
            gap: 1
          }}>
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="body1" sx={{ fontWeight: 700 }}>
              {run.request}
            </Typography>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                display: "block",
                mt: 0.35
              }}>
              {run.summary || "Delegated run details available below."}
            </Typography>
          </Box>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            <Chip size="small" color={statusChipColor(run.status)} label={statusChipLabel(run.status)} />
            <Chip
              size="small"
              variant="outlined"
              label={`${trackedAgents} agent${trackedAgents === 1 ? "" : "s"}`}
            />
            {run.channel ? <Chip size="small" variant="outlined" label={run.channel} /> : null}
          </Stack>
        </Stack>

        <Stack direction="row" spacing={0.75} useFlexGap sx={{
          flexWrap: "wrap"
        }}>
          {run.startedAt ? (
            <Chip size="small" variant="outlined" label={`Started ${formatTimestamp(run.startedAt)}`} />
          ) : null}
          {run.completedAt ? (
            <Chip size="small" variant="outlined" label={`Finished ${formatTimestamp(run.completedAt)}`} />
          ) : null}
          {run.conversationId ? (
            <Chip size="small" variant="outlined" label={`Chat ${run.conversationId.slice(0, 8)}`} />
          ) : null}
        </Stack>

        <Stack spacing={0}>
          {run.agents.map((agent) => (
            <Box
              key={`${run.id}-${agent.id}`}
              sx={{ width: "100%", px: 0, py: 1.15, borderBottom: "1px solid", borderColor: "divider", transition: "background 0.15s ease", "&:hover": { background: "var(--ui-rgba-57-208-255-040)" } }}
            >
              <Stack spacing={0.4}>
                {/* Line 1: dot + agent name ... status right */}
                <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                  <Stack direction="row" sx={{ alignItems: "center", gap: 1, minWidth: 0 }}>
                    <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: statusDotColor(agent.status) }} />
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {agent.agentRole
                        ? `${agent.agentName} - ${agent.agentRole}`
                        : agent.agentName}
                    </Typography>
                  </Stack>
                  <Stack direction="row" sx={{ alignItems: "center", gap: 0.75, flexShrink: 0 }}>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>{statusChipLabel(agent.status)}</Typography>
                    {agent.elapsedMs ? (
                      <Typography variant="caption" sx={{ color: "text.secondary" }}>{formatElapsedMs(agent.elapsedMs)}</Typography>
                    ) : null}
                  </Stack>
                </Stack>
                {/* Line 2: specialist type, model */}
                <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                  {agent.modelName || (agent.isSpecialist ? "Specialist model" : "Auto agent")}
                </Typography>
                {/* Line 3: task / update */}
                {agent.task ? (
                  <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                    {agent.task}
                  </Typography>
                ) : null}
                <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                  {agent.latestUpdate || agent.summary || "No extra detail recorded."}
                </Typography>
              </Stack>
            </Box>
          ))}
        </Stack>
      </Stack>
    </Box>
  );
}

function RunHistoryList({ runs }: { runs: SwarmRun[] }) {
  const pageSize = 8;
  const [page, setPage] = useState(0);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const pageCount = Math.max(1, Math.ceil(runs.length / pageSize));
  const clampedPage = Math.min(page, Math.max(0, pageCount - 1));
  const pageRuns = useMemo(() => {
    const start = clampedPage * pageSize;
    return runs.slice(start, start + pageSize);
  }, [clampedPage, runs]);
  const selectedRun = pageRuns.find((run) => run.id === selectedRunId) ?? pageRuns[0] ?? null;

  useEffect(() => {
    if (page !== clampedPage) {
      setPage(clampedPage);
    }
  }, [clampedPage, page]);

  useEffect(() => {
    if (!selectedRunId || !pageRuns.some((run) => run.id === selectedRunId)) {
      setSelectedRunId(pageRuns[0]?.id ?? null);
    }
  }, [pageRuns, selectedRunId]);

  return (
    <Stack spacing={1.2}>
      <Stack spacing={0}>
        {pageRuns.map((run) => {
          const trackedAgents = Math.max(run.agentCount, run.agents.length);
          const isSelected = selectedRun?.id === run.id;
          return (
            <ButtonBase
              key={run.id}
              onClick={() => setSelectedRunId(run.id)}
              sx={{
                width: "100%",
                textAlign: "left",
                display: "block",
                px: 0,
                py: 1.15,
                borderBottom: "1px solid",
                borderColor: "divider",
                transition: "background 0.15s ease",
                "&:hover": { background: "var(--ui-rgba-57-208-255-040)" },
                ...(isSelected ? { background: "var(--ui-rgba-57-208-255-070)" } : {})
              }}
            >
              <Stack spacing={0.4}>
                {/* Line 1: dot + request ... status right */}
                <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                  <Stack direction="row" sx={{ alignItems: "center", gap: 1, minWidth: 0 }}>
                    <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: statusDotColor(run.status) }} />
                    <Typography variant="body2" sx={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {run.request}
                    </Typography>
                  </Stack>
                  <Stack direction="row" sx={{ alignItems: "center", gap: 0.75, flexShrink: 0 }}>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>{statusChipLabel(run.status)}</Typography>
                    <Button
                      size="small"
                      variant={isSelected ? "contained" : "outlined"}
                      onClick={(event) => {
                        event.stopPropagation();
                        setSelectedRunId(run.id);
                      }}
                    >
                      {isSelected ? "Viewing" : "View"}
                    </Button>
                  </Stack>
                </Stack>
                {/* Line 2: summary */}
                <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                  {run.summary || "Select this run to inspect delegated agent detail."}
                </Typography>
                {/* Line 3: metadata */}
                <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                  {trackedAgents} agent{trackedAgents === 1 ? "" : "s"} - {run.channel || "Workspace"} - Started {formatTimestamp(run.startedAt)} - {run.completedAt ? `Finished ${formatTimestamp(run.completedAt)}` : `Updated ${formatTimestamp(run.updatedAt)}`}
                </Typography>
              </Stack>
            </ButtonBase>
          );
        })}
      </Stack>

      <Stack
        direction={{ xs: "column", sm: "row" }}
        spacing={0.75}
        sx={{
          alignItems: { xs: "flex-start", sm: "center" },
          justifyContent: "space-between"
        }}
      >
        <Typography variant="caption" className="conversation-pagination-copy">
          {runs.length} run{runs.length === 1 ? "" : "s"}
        </Typography>
        <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", flexWrap: "wrap" }}>
          <Typography variant="caption" className="conversation-pagination-copy">
            Page {clampedPage + 1}/{pageCount}
          </Typography>
          <Button
            size="small"
            variant="outlined"
            onClick={() => setPage((prev) => Math.max(0, prev - 1))}
            disabled={clampedPage <= 0}
          >
            Prev
          </Button>
          <Button
            size="small"
            variant="outlined"
            onClick={() => setPage((prev) => Math.min(pageCount - 1, prev + 1))}
            disabled={clampedPage >= pageCount - 1}
          >
            Next
          </Button>
        </Stack>
      </Stack>

      {selectedRun ? (
        <Stack spacing={0.8}>
          <Typography variant="overline" sx={{ letterSpacing: 0, color: "info.light" }}>
            Run details
          </Typography>
          <RunCard run={selectedRun} />
        </Stack>
      ) : null}
    </Stack>
  );
}

function AccessScopeSelect({
  label,
  helperText,
  options,
  selectedIds,
  onChange
}: {
  label: string;
  helperText?: string;
  options: BuilderOption[];
  selectedIds: string[];
  onChange: (nextIds: string[]) => void;
}) {
  const mergedOptions = useMemo(() => mergeAccessScopeOptions(options, selectedIds), [options, selectedIds]);
  const selectedOptions = useMemo(
    () => mergedOptions.filter((option) => selectedIds.includes(option.id)),
    [mergedOptions, selectedIds]
  );
  const hasConfiguredOptions = options.length > 0;
  const hasSelections = selectedIds.length > 0;
  const helper = helperText?.trim() ?? "";
  const effectiveHelperText =
    !hasConfiguredOptions && hasSelections
      ? [helper, "Previously selected resources are currently unavailable in Settings."].filter(Boolean).join(" ")
      : helper || undefined;

  return (
    <Autocomplete
      multiple
      size="small"
      disableCloseOnSelect
      filterSelectedOptions
      disabled={!hasConfiguredOptions && !hasSelections}
      limitTags={3}
      options={mergedOptions}
      value={selectedOptions}
      onChange={(_, value) => onChange(value.map((item) => item.id))}
      isOptionEqualToValue={(option, value) => option.id === value.id}
      getOptionLabel={(option) => option.label}
      noOptionsText={hasSelections ? "No other configured options" : "Nothing available"}
      slotProps={{
        popper: {
          sx: {
            zIndex: (theme) => theme.zIndex.modal + 2
          }
        },
        paper: {
          sx: {
            mt: 0.75,
            borderRadius: "8px",
            border: "1px solid var(--ui-rgba-255-255-255-090)",
            background: "linear-gradient(180deg, var(--ui-rgba-8-14-28-980) 0%, var(--ui-rgba-6-10-20-980) 100%)",
            boxShadow: "0 24px 80px var(--ui-rgba-0-0-0-450)",
            "& .MuiAutocomplete-listbox": {
              p: 0.75,
              display: "grid",
              gap: 0.65,
              maxHeight: 280
            }
          }
        }
      }}
      renderInput={(params) => (
        <TextField
          {...params}
          label={label}
          helperText={effectiveHelperText}
          placeholder={
            hasConfiguredOptions
              ? "Select one or more"
              : hasSelections
                ? "Selected resources are currently unavailable"
                : "No enabled options available"
          }
        />
      )}
      renderOption={(props, option) => {
        const { key, ...optionProps } = props;
        return (
          <Box
            component="li"
            key={key}
            {...optionProps}
            sx={{
              display: "flex",
              alignItems: "flex-start",
              gap: 1,
              px: 1.15,
              py: 1,
              borderRadius: "8px",
              border: "1px solid var(--ui-rgba-255-255-255-060)",
              background: "var(--ui-rgba-255-255-255-020)",
              transition: "background 0.18s ease, border-color 0.18s ease",
              "&.Mui-focused": {
                background: "var(--ui-rgba-57-208-255-080)",
                borderColor: "var(--ui-rgba-57-208-255-220)"
              },
              '&[aria-selected="true"]': {
                background: "var(--ui-rgba-57-208-255-120)",
                borderColor: "var(--ui-rgba-57-208-255-280)"
              }
            }}
          >
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <Typography variant="body2" sx={{ fontWeight: 700, lineHeight: 1.3 }}>
                {option.label}
              </Typography>
              {option.helper ? (
                <Typography
                  variant="caption"
                  sx={{
                    color: "text.secondary",
                    display: "block",
                    mt: 0.2
                  }}>
                  {option.helper}
                </Typography>
              ) : null}
            </Box>
            {option.status ? (
              <Chip
                size="small"
                variant="outlined"
                label={formatBuilderStatus(option.status)}
                sx={{ height: 22, flexShrink: 0 }}
              />
            ) : null}
          </Box>
        );
      }}
      renderValue={(value, getItemProps) =>
        value.map((option, index) => (
          <Chip
            {...getItemProps({ index })}
            key={option.id}
            size="small"
            variant="outlined"
            label={option.label}
          />
        ))
      }
    />
  );
}

export function SwarmManager({ autoRefresh }: Props) {
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [editingAgentId, setEditingAgentId] = useState<string | null>(null);
  const [draft, setDraft] = useState<AgentDraft>(EMPTY_DRAFT);
  const [accessPlan, setAccessPlan] = useState<AccessPlan | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  const statusQ = useQuery({
    queryKey: ["swarm-status"],
    queryFn: () => api.rawGet("/swarm/status"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const agentsQ = useQuery({
    queryKey: ["swarm-agents"],
    queryFn: () => api.rawGet("/swarm/agents"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const configQ = useQuery({
    queryKey: ["swarm-config"],
    queryFn: () => api.rawGet("/swarm/config"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const modelsQ = useQuery({
    queryKey: ["models"],
    queryFn: () => api.rawGet("/models"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const builderQ = useQuery({
    queryKey: ["swarm-agent-builder-options"],
    queryFn: () => api.rawGet("/swarm/agents/builder/options"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });
  const delegationsQ = useQuery({
    queryKey: ["swarm-delegations"],
    queryFn: () => api.rawGet("/swarm/delegations?limit=all"),
    refetchInterval: autoRefresh ? REFRESH_MS : false
  });

  const status = asRecord(statusQ.data);
  const config = asRecord(configQ.data);
  const agents = toProvisionedAgents(agentsQ.data);
  const savedModelProfiles = useMemo(() => toSavedModelProfiles(modelsQ.data), [modelsQ.data]);
  const builderOptions = useMemo(
    () => filterBuilderOptions(toBuilderOptions(builderQ.data)),
    [builderQ.data]
  );
  const builderOptionsByScope = useMemo(
    () => ({
      mcp_server_ids: builderOptions.mcpServers,
      ssh_connection_names: builderOptions.sshConnections,
      custom_api_ids: builderOptions.customApis,
      integration_ids: builderOptions.integrations,
      extension_pack_ids: builderOptions.extensionPacks,
      channel_ids: builderOptions.channels
    }),
    [builderOptions]
  );
  const enabledModelProfiles = useMemo(
    () => savedModelProfiles.filter((profile) => profile.enabled),
    [savedModelProfiles]
  );
  const defaultModelProfile = enabledModelProfiles[0] ?? null;
  const selectedModelProfile =
    enabledModelProfiles.find((profile) => profile.id === draft.model_profile_id) ?? null;
  const customAgents = useMemo(() => agents.filter((agent) => !agent.isSystem), [agents]);
  const editingAgent = useMemo(
    () => customAgents.find((agent) => agent.id === editingAgentId) ?? null,
    [customAgents, editingAgentId]
  );
  const resourceAccessSections = useMemo(
    () =>
      [
        {
          field: "mcp_server_ids" as ResourceAccessScopeKey,
          label: "MCP servers",
          helperText: "Attach only the MCP servers this agent should see.",
          options: builderOptions.mcpServers,
          selectedIds: draft.access_scope.mcp_server_ids
        },
        {
          field: "ssh_connection_names" as ResourceAccessScopeKey,
          label: "SSH connections",
          helperText: "Grant remote execution only on the named SSH connections you select.",
          options: builderOptions.sshConnections,
          selectedIds: draft.access_scope.ssh_connection_names
        },
        {
          field: "custom_api_ids" as ResourceAccessScopeKey,
          label: "Custom APIs",
          helperText: "Attach imported API actions explicitly instead of exposing every configured API.",
          options: builderOptions.customApis,
          selectedIds: draft.access_scope.custom_api_ids
        },
        {
          field: "integration_ids" as ResourceAccessScopeKey,
          label: "Integrations",
          helperText: "Limit built-in integration actions like Google Workspace or Slack.",
          options: builderOptions.integrations,
          selectedIds: draft.access_scope.integration_ids
        },
        {
          field: "extension_pack_ids" as ResourceAccessScopeKey,
          label: "Extension packs",
          helperText: "Attach installed custom integrations that register runtime actions through the generic pack system.",
          options: builderOptions.extensionPacks,
          selectedIds: draft.access_scope.extension_pack_ids
        },
        {
          field: "channel_ids" as ResourceAccessScopeKey,
          label: "Messaging channels",
          helperText: "Only selected messaging channels can be attached for agent delivery or reporting flows.",
          options: builderOptions.channels,
          selectedIds: draft.access_scope.channel_ids
        }
      ].filter((section) => section.options.length > 0 || section.selectedIds.length > 0),
    [builderOptions, draft.access_scope]
  );
  const optionMaps = useMemo(
    () => ({
      mcp: buildOptionMap(builderOptions.mcpServers),
      ssh: buildOptionMap(builderOptions.sshConnections),
      customApi: buildOptionMap(builderOptions.customApis),
      integration: buildOptionMap(builderOptions.integrations),
      extensionPack: buildOptionMap(builderOptions.extensionPacks),
      channel: buildOptionMap(builderOptions.channels)
    }),
    [builderOptions]
  );
  const hiddenSystemCount = agents.length - customAgents.length;
  const activeRuns = toSwarmRuns({ runs: pickRecords(status.active_runs, "active_runs") });
  const recentRuns = toSwarmRuns(delegationsQ.data).filter(
    (run) => !activeRuns.some((active) => active.id === run.id)
  );
  const swarmEnabled = bool(status.enabled) || bool(config.enabled);
  const activeAgentCount = Math.max(0, num(status.active_agents, 0));
  const totalAgentCount = Math.max(customAgents.length, num(status.total_agents, 0) - hiddenSystemCount);
  const interruptedRuns = recentRuns.filter((run) => run.status === "interrupted").length;
  const failedRuns = recentRuns.filter((run) =>
    ["failed", "timed_out", "panicked"].includes(run.status)
  ).length;
  const queryError =
    statusQ.error || configQ.error || agentsQ.error || delegationsQ.error || modelsQ.error || builderQ.error;
  const accessPlanReview = useMemo(() => {
    if (!accessPlan) {
      return {
        implicit: [] as AccessPlanGroup[],
        requested: [] as AccessPlanGroup[],
        unavailable: [] as AccessPlanGroup[]
      };
    }
    const requested: AccessPlanGroup[] = [];
    const unavailable: AccessPlanGroup[] = [];
    accessPlan.requestedAccess.forEach((group) => {
      if (group.scopeField === "approved_permission_ids") {
        requested.push(group);
        return;
      }
      const options = builderOptionsByScope[group.scopeField as ResourceAccessScopeKey] ?? [];
      const optionIds = new Set(options.map((option) => option.id));
      const selectedIds = draft.access_scope[group.scopeField] as string[];
      const hasCurrentSelection = selectedIds.length > 0;
      const exactIdsMissing =
        group.selectionMode === "exact" &&
        group.suggestedIds.length > 0 &&
        group.suggestedIds.every((id) => !optionIds.has(id) && !selectedIds.includes(id));
      const noChoicesAvailable =
        group.selectionMode !== "exact" && options.length === 0 && !hasCurrentSelection;
      if (exactIdsMissing || noChoicesAvailable) {
        unavailable.push(group);
      } else {
        requested.push(group);
      }
    });
    return {
      implicit: accessPlan.implicitAccess,
      requested,
      unavailable
    };
  }, [accessPlan, builderOptionsByScope, draft.access_scope]);

  const saveAgent = useMutation({
    mutationFn: async () => {
      const resolvedProfile = selectedModelProfile;
      const payload = {
        name: draft.name.trim(),
        agent_type: draft.agent_type.trim(),
        llm_provider: resolvedProfile?.provider.trim() || draft.llm_provider.trim(),
        llm_model: resolvedProfile?.model.trim() || draft.llm_model.trim(),
        llm_base_url: resolvedProfile?.baseUrl.trim() || draft.llm_base_url.trim() || undefined,
        llm_api_key: draft.llm_api_key.trim() || undefined,
        capabilities: draft.capabilities
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
        system_prompt: draft.system_prompt.trim() || undefined,
        access_scope: cloneAccessScope(draft.access_scope)
      };
      return api.rawPost(editingAgentId ? `/swarm/agents/${editingAgentId}` : "/swarm/agents", payload);
    },
    onSuccess: async () => {
      setEditingAgentId(null);
      setCreateOpen(false);
      setAccessPlan(null);
      setDraft(applyModelProfileToDraft({ ...EMPTY_DRAFT, access_scope: emptyAccessScope() }, defaultModelProfile));
      setFormError(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["swarm-status"] }),
        queryClient.invalidateQueries({ queryKey: ["swarm-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["swarm-delegations"] })
      ]);
    }
  });
  const deleteAgent = useMutation({
    mutationFn: (id: string) => api.rawDelete(`/swarm/agents/${encodeURIComponent(id)}`),
    onSuccess: async (_, id) => {
      if (editingAgentId === id) {
        setEditingAgentId(null);
        setCreateOpen(false);
        setAccessPlan(null);
        setDraft(applyModelProfileToDraft({ ...EMPTY_DRAFT, access_scope: emptyAccessScope() }, defaultModelProfile));
      }
      setFormError(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["swarm-status"] }),
        queryClient.invalidateQueries({ queryKey: ["swarm-agents"] }),
        queryClient.invalidateQueries({ queryKey: ["swarm-delegations"] })
      ]);
    },
    onError: (error) => setFormError(errMessage(error))
  });
  const planAccess = useMutation({
    mutationFn: async () =>
      api.rawPost("/swarm/agents/access-plan", {
        description: draft.description.trim() || undefined,
        name: draft.name.trim(),
        agent_type: draft.agent_type.trim(),
        model_profile_id: draft.model_profile_id || undefined,
        capabilities: draft.capabilities
          .split(",")
          .map((value) => value.trim())
          .filter(Boolean),
        system_prompt: draft.system_prompt.trim(),
        access_scope: cloneAccessScope(draft.access_scope)
      }),
    onSuccess: (response) => {
      const nextPlan = parseAccessPlan(response);
      setAccessPlan(nextPlan);
      setDraft((prev) => {
        const nextDraft = {
          ...prev,
          access_scope: cloneAccessScope(prev.access_scope)
        };
        nextPlan.requestedAccess.forEach((group) => {
          if (group.scopeField === "approved_permission_ids" || group.selectionMode !== "exact") {
            return;
          }
          const current = nextDraft.access_scope[group.scopeField] as string[];
          if (current.length > 0) return;
          const options = builderOptionsByScope[group.scopeField as ResourceAccessScopeKey] ?? [];
          const optionIds = new Set(options.map((option) => option.id));
          const nextIds = group.suggestedIds.filter((id) => optionIds.has(id));
          if (nextIds.length > 0) {
            (nextDraft.access_scope[group.scopeField] as string[]) = uniqueStrings(nextIds);
          }
        });
        return nextDraft;
      });
    }
  });
  const generateDraft = useMutation({
    mutationFn: async () =>
      api.rawPost("/swarm/agents/draft", {
        description: draft.description.trim(),
        model_profile_id: draft.model_profile_id || undefined
      }),
    onSuccess: (response) => {
      const payload = asRecord(response);
      setAccessPlan(null);
      setDraft((prev) => ({
        ...prev,
        name: str(payload.name, prev.name),
        agent_type: str(payload.agent_type, prev.agent_type),
        capabilities: parseCapabilities(payload.capabilities).join(", "),
        system_prompt: str(payload.system_prompt, prev.system_prompt)
      }));
    }
  });

  const savedProfilesHelperText =
    enabledModelProfiles.length > 0
      ? `Choose from ${formatProfileCount(enabledModelProfiles.length)} saved in Settings > Models.`
      : savedModelProfiles.length > 0
        ? "All saved model profiles are disabled right now. Enable one in Settings > Models."
        : "Add a model in Settings > Models first, then pick it here.";

  function closeAgentDialog() {
    if (saveAgent.isPending || planAccess.isPending || deleteAgent.isPending) return;
    setCreateOpen(false);
    setAccessPlan(null);
    setEditingAgentId(null);
    setFormError(null);
    setDraft(applyModelProfileToDraft({ ...EMPTY_DRAFT, access_scope: emptyAccessScope() }, defaultModelProfile));
  }

  function openCreateAgentDialog() {
    setFormError(null);
    setAccessPlan(null);
    setEditingAgentId(null);
    setDraft(applyModelProfileToDraft({ ...EMPTY_DRAFT, access_scope: emptyAccessScope() }, defaultModelProfile));
    setCreateOpen(true);
  }

  function openEditAgentDialog(agent: ProvisionedAgent) {
    const matchingProfile =
      findMatchingModelProfile(agent, enabledModelProfiles) ?? findMatchingModelProfile(agent, savedModelProfiles);
    setFormError(null);
    setAccessPlan(null);
    setEditingAgentId(agent.id);
    setDraft({
      description: "",
      name: agent.name,
      agent_type: agent.agentType,
      model_profile_id: matchingProfile?.id ?? "",
      llm_provider: agent.provider,
      llm_model: agent.model,
      llm_base_url: agent.llmBaseUrl,
      llm_api_key: "",
      capabilities: agent.capabilities.join(", "),
      system_prompt: agent.systemPrompt,
      access_scope: cloneAccessScope(agent.accessScope)
    });
    setCreateOpen(true);
  }

  function updateAccessScope(field: ResourceAccessScopeKey, values: string[]) {
    setDraft((prev) => ({
      ...prev,
      access_scope: {
        ...prev.access_scope,
        [field]: uniqueStrings(values)
      }
    }));
  }

  function toggleApprovedPermission(permissionId: string, enabled: boolean) {
    const normalized = permissionId.trim().toLowerCase();
    if (!normalized) return;
    setDraft((prev) => ({
      ...prev,
      access_scope: {
        ...prev.access_scope,
        approved_permission_ids: enabled
          ? uniqueStrings([...prev.access_scope.approved_permission_ids, normalized])
          : prev.access_scope.approved_permission_ids.filter((value) => value !== normalized)
      }
    }));
  }

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Agent"
        title="Agents"
        description="Live delegated runs, specialist roster, and recent swarm history stay visible here. Chat and this view now share the same execution state instead of splitting live work from history."
        actions={
          <Stack
            direction="row"
            spacing={0.75}
            useFlexGap
            sx={{
              flexWrap: "wrap",
              alignItems: "center",
              p: 0.45,
              borderRadius: "8px",
              border: "1px solid var(--surface-border)",
              background: "var(--ui-rgba-255-255-255-020)",
              boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)"
            }}>
            <Button
              size="small"
              variant="contained"
              onClick={openCreateAgentDialog}
              sx={{
                minHeight: 32,
                px: 1.5,
                borderRadius: "8px",
                fontWeight: 700,
                textTransform: "none",
                boxShadow: "none"
              }}
            >
              Add agent
            </Button>
            <Chip
              size="small"
              color={swarmEnabled ? "success" : "default"}
              variant={swarmEnabled ? "filled" : "outlined"}
              label={swarmEnabled ? "Swarm enabled" : "Swarm disabled"}
              sx={{
                height: 32,
                borderRadius: "8px",
                "& .MuiChip-label": {
                  px: 1.25,
                  fontSize: "0.66rem",
                  fontWeight: 700,
                  letterSpacing: 0,
                  textTransform: "uppercase"
                }
              }}
            />
            <Chip
              size="small"
              variant="outlined"
              label={`${activeRuns.length} live run${activeRuns.length === 1 ? "" : "s"}`}
              sx={{
                height: 32,
                borderRadius: "8px",
                "& .MuiChip-label": {
                  px: 1.2,
                  fontSize: "0.66rem",
                  fontWeight: 700,
                  letterSpacing: 0,
                  textTransform: "uppercase"
                }
              }}
            />
          </Stack>
        }
      />
      <Box className="list-shell stat-strip">
        {[
          { label: "Active agents", value: activeAgentCount },
          { label: "Custom agents", value: totalAgentCount },
          { label: "Interrupted runs", value: interruptedRuns },
          { label: "Failed runs", value: failedRuns },
        ].map((s) => (
          <div key={s.label} className="stat-strip-item">
            <span className="stat-strip-label">{s.label}</span>
            <span className="stat-strip-value">{s.value}</span>
          </div>
        ))}
      </Box>
      {queryError ? <Alert severity="error">{errMessage(queryError)}</Alert> : null}
      {activeRuns.length > 0 ? (
        <SectionShell
          eyebrow="Live now"
          title="Delegated runs in progress"
          detail="Every active multi-agent run appears here with the same per-agent state shown in chat."
        >
          <Stack spacing={1.2}>
            {activeRuns.map((run) => (
              <RunCard key={run.id} run={run} live />
            ))}
          </Stack>
        </SectionShell>
      ) : null}
      {customAgents.length > 0 ? (
      <SectionShell
        eyebrow="Roster"
        title="Custom agents"
        detail="User-managed specialists stay visible here. Built-in system specialists remain available for delegation without cluttering the roster."
      >
        <Stack spacing={1.2}>
          <Stack
            direction={{ xs: "column", md: "row" }}
            sx={{
              alignItems: { xs: "flex-start", md: "center" },
              gap: 1
            }}>
            <Stack direction="row" spacing={0.75} useFlexGap sx={{
              flexWrap: "wrap"
            }}>
              <Chip size="small" variant="outlined" label={`${customAgents.length} custom`} />
              {hiddenSystemCount > 0 ? (
                <Chip size="small" variant="outlined" label={`${hiddenSystemCount} system hidden`} />
              ) : null}
            </Stack>
          </Stack>

        {customAgents.length > 0 ? (
          <Stack spacing={0}>
            {customAgents.map((agent) => (
              <Box
                key={agent.id}
                sx={{ width: "100%", px: 0, py: 1.15, borderBottom: "1px solid", borderColor: "divider", transition: "background 0.15s ease", "&:hover": { background: "var(--ui-rgba-57-208-255-040)" } }}
              >
                <Stack spacing={0.5}>
                  {/* Line 1: dot + agent name ... enabled/disabled right */}
                  <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", gap: 1 }}>
                    <Stack direction="row" sx={{ alignItems: "center", gap: 1, minWidth: 0 }}>
                      <Box sx={{ width: 7, height: 7, borderRadius: "50%", flexShrink: 0, background: statusDotColor(agent.enabled ? agent.status : "disabled") }} />
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        {agent.agentType
                          ? `${agent.displayName} - ${agent.agentType}`
                          : agent.displayName}
                      </Typography>
                    </Stack>
                    <Typography variant="caption" sx={{ color: "text.secondary", flexShrink: 0 }}>
                      {statusChipLabel(agent.enabled ? agent.status : "disabled")}
                    </Typography>
                  </Stack>
                  {/* Line 2: role, model, access scope summary */}
                  <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                    {agent.provider} / {agent.model}
                    {accessScopeSummary(agent.accessScope).length > 0
                      ? ` - ${accessScopeSummary(agent.accessScope).join(", ")}`
                      : " - No elevated access"}
                  </Typography>
                  {/* Line 3: capabilities */}
                  {agent.capabilities.length > 0 || agent.systemPrompt ? (
                    <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", pl: "15px" }}>
                      {agent.capabilities.slice(0, 5).map((capability) => (
                        <Chip
                          key={`${agent.id}-${capability}`}
                          size="small"
                          variant="outlined"
                          label={capability}
                          sx={{ height: 20 }}
                        />
                      ))}
                      {agent.systemPrompt ? (
                        <Chip size="small" variant="outlined" color="info" label="Prompt set" sx={{ height: 20 }} />
                      ) : null}
                    </Stack>
                  ) : null}
                  {/* Line 4: access scope labels */}
                  {accessScopeLabels(agent.accessScope, optionMaps).length > 0 ? (
                    <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap", pl: "15px" }}>
                      {accessScopeLabels(agent.accessScope, optionMaps)
                        .slice(0, 4)
                        .map((label) => (
                          <Chip key={`${agent.id}-${label}`} size="small" variant="outlined" label={label} sx={{ height: 20 }} />
                        ))}
                      {accessScopeLabels(agent.accessScope, optionMaps).length > 4 ? (
                        <Chip
                          size="small"
                          variant="outlined"
                          label={`+${accessScopeLabels(agent.accessScope, optionMaps).length - 4} more`}
                          sx={{ height: 20 }}
                        />
                      ) : null}
                    </Stack>
                  ) : null}
                  {/* Line 5: latest task */}
                  <Typography variant="caption" sx={{ color: "text.secondary", pl: "15px" }}>
                    {agent.lastTask || "No delegated task recorded yet."}
                    {(agent.lastUpdate || agent.lastSummary) ? ` - ${agent.lastUpdate || agent.lastSummary}` : ""}
                  </Typography>
                  {/* Line 6: timestamps + action buttons */}
                  <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", pl: "15px", gap: 1 }}>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      {agent.lastActivityAt ? `Last active ${formatTimestamp(agent.lastActivityAt)} - ` : ""}Created {formatTimestamp(agent.createdAt)}
                    </Typography>
                    <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexShrink: 0 }}>
                      <Button size="small" variant="outlined" onClick={() => openEditAgentDialog(agent)}>
                        Edit
                      </Button>
                      <Button
                        size="small"
                        color="error"
                        variant="outlined"
                        disabled={deleteAgent.isPending}
                        onClick={() => {
                          if (window.confirm(`Delete ${agent.displayName}?`)) {
                            deleteAgent.mutate(agent.id);
                          }
                        }}
                      >
                        Delete
                      </Button>
                    </Stack>
                  </Stack>
                </Stack>
              </Box>
            ))}
          </Stack>
        ) : null}
        </Stack>
      </SectionShell>
      ) : null}
      {recentRuns.length > 0 ? (
        <SectionShell
          eyebrow="History"
          title="Recent swarm runs"
          detail="Completed, interrupted, and failed runs stay here in a compact list. Select any run to inspect the delegated agent detail."
        >
          <RunHistoryList runs={recentRuns} />
        </SectionShell>
      ) : null}
      <Dialog open={createOpen} onClose={closeAgentDialog} maxWidth="md" fullWidth slotProps={{ paper: { sx: { borderRadius: "8px", border: "1px solid var(--surface-border)", background: "var(--surface-bg-elevated)", boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)" } } }}>
        <DialogTitle>{editingAgent ? "Edit custom agent" : "Add custom agent"}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.35}>
            <Alert severity="info">
              Define the agent and review requested access in the same form. Elevated access is derived from the drafted spec and live tool metadata below.
            </Alert>
            <TextField
              fullWidth
              size="small"
              multiline
              minRows={3}
              label="Describe the agent"
              value={draft.description}
              onChange={(event) => {
                setAccessPlan(null);
                setDraft((prev) => ({ ...prev, description: event.target.value }));
              }}
              placeholder="Example: Handles Slack follow-ups, summarizes urgent threads, drafts replies, and can query our Google Workspace docs."
              helperText="Optional for manual setup. Required only if you want AI to draft the name, role, capabilities, and system prompt."
            />
            <Stack
              direction={{ xs: "column", sm: "row" }}
              spacing={1}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "stretch", sm: "center" }
              }}>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                AI drafting never auto-saves. It only fills the editable fields below.
              </Typography>
              <Button
                variant="outlined"
                disabled={generateDraft.isPending || !draft.description.trim()}
                startIcon={generateDraft.isPending ? <CircularProgress size={14} color="inherit" /> : undefined}
                onClick={async () => {
                  setFormError(null);
                  try {
                    await generateDraft.mutateAsync();
                  } catch (error) {
                    setFormError(errMessage(error));
                  }
                }}
              >
                {generateDraft.isPending ? "Generating..." : "Generate draft"}
              </Button>
            </Stack>

            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Name"
                  value={draft.name}
                  onChange={(event) => {
                    setAccessPlan(null);
                    setDraft((prev) => ({ ...prev, name: event.target.value }));
                  }}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Role"
                  value={draft.agent_type}
                  onChange={(event) => {
                    setAccessPlan(null);
                    setDraft((prev) => ({ ...prev, agent_type: event.target.value }));
                  }}
                  placeholder="researcher, coder, support specialist, Slack triager"
                  helperText="Built-ins still work, but custom role labels are allowed."
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  select
                  size="small"
                  label="Model profile"
                  value={draft.model_profile_id}
                  onChange={(event) => {
                    setAccessPlan(null);
                    const nextProfile =
                      enabledModelProfiles.find((profile) => profile.id === event.target.value) ?? null;
                    setDraft((prev) => applyModelProfileToDraft(prev, nextProfile));
                  }}
                  helperText={savedProfilesHelperText}
                  disabled={enabledModelProfiles.length === 0}
                >
                  <MenuItem value="">
                    {editingAgent ? "Keep current model config" : "Select a saved model profile"}
                  </MenuItem>
                  {enabledModelProfiles.map((profile) => (
                    <MenuItem key={profile.id} value={profile.id}>
                      {profile.label || `${formatProfileRole(profile.role)} profile`}
                    </MenuItem>
                  ))}
                </TextField>
              </Grid2>
            </Grid2>

            {selectedModelProfile ? (
              <Box
                sx={{
                  px: 1.15,
                  py: 1,
                  borderRadius: "8px",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "linear-gradient(180deg, var(--ui-rgba-255-255-255-040) 0%, var(--ui-rgba-255-255-255-020) 100%)"
                }}
              >
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={0.75}
                  useFlexGap
                  sx={{
                    flexWrap: "wrap",
                    alignItems: { xs: "flex-start", sm: "center" }
                  }}>
                  <Typography variant="body2" sx={{ fontWeight: 700 }}>
                    {selectedModelProfile.label || `${formatProfileRole(selectedModelProfile.role)} profile`}
                  </Typography>
                  <Chip size="small" variant="outlined" label={formatProfileRole(selectedModelProfile.role)} />
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    Reuses your saved model setup automatically.
                  </Typography>
                </Stack>
              </Box>
            ) : draft.llm_provider && draft.llm_model ? (
              <Alert severity="warning">
                This agent is keeping its stored model config: {draft.llm_provider} / {draft.llm_model}
                {draft.llm_base_url ? ` / ${draft.llm_base_url}` : ""}. Pick a saved profile above if
                you want to replace it.
              </Alert>
            ) : null}

            <TextField
              fullWidth
              size="small"
              label="Capabilities"
              value={draft.capabilities}
              onChange={(event) => {
                setAccessPlan(null);
                setDraft((prev) => ({ ...prev, capabilities: event.target.value }));
              }}
              placeholder="debugging, code review, refactoring"
              helperText="Comma-separated skills shown on the agent card."
            />
            <TextField
              fullWidth
              size="small"
              multiline
              minRows={5}
              label="System prompt"
              value={draft.system_prompt}
              onChange={(event) => {
                setAccessPlan(null);
                setDraft((prev) => ({ ...prev, system_prompt: event.target.value }));
              }}
            />

            <Box
              sx={{
                borderRadius: "8px",
                border: "1px solid var(--ui-rgba-255-255-255-080)",
                background: "var(--ui-rgba-255-255-255-020)",
                px: 1.15,
                py: 1.05
              }}
            >
              <Stack spacing={1}>
                <Stack
                  direction={{ xs: "column", sm: "row" }}
                  spacing={1}
                  sx={{
                    justifyContent: "space-between",
                    alignItems: { xs: "stretch", sm: "center" }
                  }}
                >
                  <Box>
                    <Typography variant="subtitle2" sx={{ fontWeight: 800 }}>
                      Access review
                    </Typography>
                    <Typography variant="caption" sx={{ color: "text.secondary" }}>
                      Refresh this section to see exactly which permissions, integrations, and scoped resources this agent is asking for.
                    </Typography>
                  </Box>
                  <Button
                    size="small"
                    variant="outlined"
                    disabled={planAccess.isPending}
                    startIcon={planAccess.isPending ? <CircularProgress size={14} color="inherit" /> : undefined}
                    onClick={async () => {
                      const hasModelConfig = Boolean(
                        (selectedModelProfile?.provider || draft.llm_provider).trim() &&
                          (selectedModelProfile?.model || draft.llm_model).trim()
                      );
                      if (!draft.name.trim() || !draft.agent_type.trim() || !hasModelConfig) {
                        setFormError("Name, role, and model config are required.");
                        return;
                      }
                      setFormError(null);
                      try {
                        await planAccess.mutateAsync();
                      } catch (error) {
                        setFormError(errMessage(error));
                      }
                    }}
                  >
                    {planAccess.isPending ? "Reviewing..." : accessPlan ? "Refresh access review" : "Run access review"}
                  </Button>
                </Stack>

                {!accessPlan ? (
                  <Alert severity="info">
                    Access review has not run yet. Use the button above to surface approvals and resource scopes before creating the agent.
                  </Alert>
                ) : accessPlanReview.requested.length > 0 ? (
                  <Stack spacing={0.8}>
                    <Typography variant="subtitle2" sx={{ fontWeight: 800 }}>
                      Requested access
                    </Typography>

                    {accessPlanReview.requested
                      .filter((group) => group.scopeField === "approved_permission_ids")
                      .map((group) => {
                        const permissionId = (group.suggestedIds[0] || group.id).toLowerCase();
                        const checked = draft.access_scope.approved_permission_ids.includes(permissionId);
                        return (
                          <Box
                            key={group.id}
                            sx={{
                              px: 1,
                              py: 0.85,
                              borderRadius: "8px",
                              border: "1px solid var(--ui-rgba-255-255-255-080)",
                              background: "var(--ui-rgba-255-255-255-020)",
                              display: "flex",
                              alignItems: { xs: "stretch", sm: "center" },
                              justifyContent: "space-between",
                              gap: 1,
                              flexDirection: { xs: "column", sm: "row" }
                            }}
                          >
                            <Stack spacing={0.25} sx={{ minWidth: 0 }}>
                              <Typography variant="body2" sx={{ fontWeight: 700 }}>
                                {group.label}
                              </Typography>
                              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                                {group.summary || group.reason || "Review this permission before saving the agent."}
                              </Typography>
                            </Stack>
                            <Stack direction="row" spacing={0.75}>
                              <Button
                                size="small"
                                variant={checked ? "contained" : "outlined"}
                                onClick={() => toggleApprovedPermission(permissionId, true)}
                              >
                                Approve
                              </Button>
                              <Button
                                size="small"
                                color="inherit"
                                variant={checked ? "outlined" : "contained"}
                                onClick={() => toggleApprovedPermission(permissionId, false)}
                              >
                                Reject
                              </Button>
                            </Stack>
                          </Box>
                        );
                      })}

                    {accessPlanReview.requested
                      .filter((group) => group.scopeField !== "approved_permission_ids")
                      .map((group) => {
                        const section = resourceAccessSections.find((item) => item.field === group.scopeField);
                        if (!section) return null;
                        return (
                          <AccessScopeSelect
                            key={group.id}
                            label={group.label}
                            options={section.options}
                            selectedIds={draft.access_scope[group.scopeField] as string[]}
                            onChange={(nextIds) => updateAccessScope(group.scopeField as ResourceAccessScopeKey, nextIds)}
                          />
                        );
                      })}
                  </Stack>
                ) : (
                  <Alert severity="success">No approval needed for this agent.</Alert>
                )}

                {accessPlanReview.unavailable.length > 0 ? (
                  <Stack spacing={0.8}>
                    {accessPlanReview.unavailable.map((group) => (
                      <Alert key={group.id} severity="warning">
                        Configure {group.label} in Settings.
                      </Alert>
                    ))}
                  </Stack>
                ) : null}
              </Stack>
            </Box>

            {formError ? <Alert severity="error">{formError}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeAgentDialog}>Cancel</Button>
          <Button
            variant="contained"
            disabled={saveAgent.isPending}
            startIcon={saveAgent.isPending ? <CircularProgress size={14} color="inherit" /> : undefined}
            onClick={async () => {
              const hasModelConfig = Boolean(
                (selectedModelProfile?.provider || draft.llm_provider).trim() &&
                  (selectedModelProfile?.model || draft.llm_model).trim()
              );
              if (!draft.name.trim() || !draft.agent_type.trim() || !hasModelConfig) {
                setFormError("Name, role, and model config are required.");
                return;
              }
              if (!accessPlan) {
                setFormError(null);
                try {
                  await planAccess.mutateAsync();
                } catch (error) {
                  setFormError(errMessage(error));
                }
                return;
              }
              setFormError(null);
              try {
                await saveAgent.mutateAsync();
              } catch (error) {
                setFormError(errMessage(error));
              }
            }}
          >
            {saveAgent.isPending ? (editingAgent ? "Saving..." : "Creating...") : editingAgent ? "Save changes" : "Create agent"}
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
