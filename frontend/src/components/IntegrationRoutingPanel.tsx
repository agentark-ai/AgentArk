import { Alert, Stack } from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { RoutingControlPanel } from "./RoutingControlPanel";

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
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

function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return normalized === "true" || normalized === "1" || normalized === "yes";
  }
  return false;
}

function pickRecords(payload: unknown, key: string): JsonRecord[] {
  const root = asRecord(payload);
  const value = root[key];
  return Array.isArray(value) ? value.filter(isRecord) : [];
}

function visibleSwarmRows(rows: JsonRecord[]): JsonRecord[] {
  return rows.filter((row) => !toBool(row.is_system));
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return str(asRecord(error).error, "Request failed");
}

export function IntegrationRoutingPanel({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const interval = autoRefresh ? 8000 : false;
  const [selectedRouteId, setSelectedRouteId] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);

  const routingQ = useQuery({
    queryKey: ["gateway-routing"],
    queryFn: () => api.rawGet("/integrations/routing"),
    refetchInterval: interval
  });
  const channelsQ = useQuery({
    queryKey: ["gateway-channels"],
    queryFn: () => api.rawGet("/gateway/channels"),
    refetchInterval: interval
  });
  const swarmQ = useQuery({
    queryKey: ["swarm-agents"],
    queryFn: () => api.rawGet("/swarm/agents"),
    refetchInterval: interval
  });

  const routeRows = pickRecords(routingQ.data, "rules");
  const groupRows = pickRecords(routingQ.data, "broadcast_groups");
  const channelRows = pickRecords(channelsQ.data, "channels");
  const swarmRows = visibleSwarmRows(pickRecords(asRecord(swarmQ.data), "agents"));

  useEffect(() => {
    if (routeRows.length === 0) {
      if (selectedRouteId !== null) setSelectedRouteId(null);
      return;
    }
    if (!selectedRouteId || !routeRows.some((row) => str(row.id) === selectedRouteId)) {
      setSelectedRouteId(str(routeRows[0]?.id));
    }
  }, [routeRows, selectedRouteId]);

  const routes = useMemo(
    () =>
      routeRows.map((row) => {
        const matchKind = str(row.match_kind, "all");
        const matchValue = str(row.match_value);
        return {
          id: str(row.id),
          name: str(row.name, "Route"),
          match: matchValue ? `${matchKind}:${matchValue}` : matchKind,
          scope: str(row.conversation_scope, "per_channel"),
          route_to:
            str(row.target_kind) && str(row.target_value)
              ? `${str(row.target_kind)}:${str(row.target_value)}`
              : str(row.target_value),
          channel: str(row.channel_id),
          agent: str(row.agent_id),
          broadcast_group: str(row.broadcast_group_id),
          enabled: toBool(row.enabled),
          priority: num(row.priority, 0),
          last_matched_at: str(row.updated_at),
          description: str(row.notes)
        };
      }),
    [routeRows]
  );

  const targets = useMemo(() => {
    const channelTargets = channelRows.map((row) => ({
      id: `channel:${str(row.id)}`,
      label: `${str(row.name, "Channel")} channel`,
      kind: "channel",
      detail: str(row.description)
    }));
    const agentTargets = swarmRows.map((row) => ({
      id: `agent:${str(row.id)}`,
      label: `${str(row.name, "Agent")} agent`,
      kind: "agent",
      detail: str(row.agent_type)
    }));
    return [...channelTargets, ...agentTargets];
  }, [channelRows, swarmRows]);

  const broadcastGroups = useMemo(
    () =>
      groupRows.map((row) => ({
        id: str(row.id),
        name: str(row.name, "Broadcast group"),
        members: Array.isArray(row.targets)
          ? row.targets.map((value) => str(value)).filter(Boolean)
          : [],
        description: str(row.description)
      })),
    [groupRows]
  );

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["gateway-routing"] }),
      queryClient.invalidateQueries({ queryKey: ["gateway-channels"] })
    ]);
  };

  const parseMatch = (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return { match_kind: "all", match_value: "" };
    const idx = trimmed.indexOf(":");
    if (idx < 0) return { match_kind: "contains", match_value: trimmed };
    return {
      match_kind: trimmed.slice(0, idx).trim() || "all",
      match_value: trimmed.slice(idx + 1).trim()
    };
  };

  const parseTarget = (input: string) => {
    const trimmed = input.trim();
    if (!trimmed) return { target_kind: "agent", target_value: "" };
    const idx = trimmed.indexOf(":");
    if (idx < 0) return { target_kind: "agent", target_value: trimmed };
    return {
      target_kind: trimmed.slice(0, idx).trim() || "agent",
      target_value: trimmed.slice(idx + 1).trim()
    };
  };

  const saveRoute = useMutation({
    mutationFn: async (partial: Record<string, unknown>) => {
      const routeId = str(partial.id);
      const existing = routeRows.find((row) => str(row.id) === routeId);
      const merged = { ...asRecord(existing), ...partial };
      const match = parseMatch(str(merged.match));
      const target = parseTarget(str(merged.route_to, str(merged.target_value)));
      const payloadForSave = {
        name: str(merged.name, "Route"),
        enabled: toBool(merged.enabled ?? true),
        priority: num(merged.priority, 0),
        channel_id: str(merged.channel, str(merged.channel_id)) || undefined,
        account_id: str(merged.account_id) || undefined,
        match_kind: match.match_kind,
        match_value: match.match_value,
        target_kind: target.target_kind,
        target_value: target.target_value,
        agent_id: str(merged.agent, str(merged.agent_id)) || undefined,
        conversation_scope: str(merged.scope, str(merged.conversation_scope, "per_channel")),
        broadcast_group_id: str(merged.broadcast_group, str(merged.broadcast_group_id)) || undefined,
        notes: str(merged.description, str(merged.notes)) || undefined
      };
      if (routeId) {
        await api.rawPut(`/integrations/routing/rules/${encodeURIComponent(routeId)}`, payloadForSave);
      } else {
        await api.rawPost("/integrations/routing/rules", payloadForSave);
      }
    },
    onSuccess: async (_, partial) => {
      await refresh();
      setNotice({ kind: "success", text: str(partial.id) ? "Route updated." : "Route created." });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const deleteRoute = useMutation({
    mutationFn: (routeId: string) => api.rawDelete(`/integrations/routing/rules/${encodeURIComponent(routeId)}`),
    onSuccess: async () => {
      await refresh();
      setNotice({ kind: "success", text: "Route deleted." });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  const createGroup = useMutation({
    mutationFn: (name: string) => api.rawPost("/integrations/routing/broadcast-groups", { name, targets: [] }),
    onSuccess: async () => {
      await refresh();
      setNotice({ kind: "success", text: "Broadcast group created." });
    },
    onError: (error) => setNotice({ kind: "error", text: errMessage(error) })
  });

  return (
    <Stack spacing={1.25}>
      {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
      {routingQ.error ? <Alert severity="error">{errMessage(routingQ.error)}</Alert> : null}
      <RoutingControlPanel
        routes={routes}
        targets={targets}
        broadcastGroups={broadcastGroups}
        selectedRouteId={selectedRouteId}
        onSelectRoute={setSelectedRouteId}
        onToggleRoute={(routeId, enabled) => {
          const existing = routeRows.find((row) => str(row.id) === routeId);
          if (!existing) return Promise.resolve();
          return saveRoute.mutateAsync({ ...existing, id: routeId, enabled });
        }}
        onSaveRoute={(route) => saveRoute.mutateAsync(route as Record<string, unknown>)}
        onDeleteRoute={async (routeId) => {
          await deleteRoute.mutateAsync(routeId);
        }}
        onCreateGroup={async (name) => {
          await createGroup.mutateAsync(name);
        }}
      />
    </Stack>
  );
}
