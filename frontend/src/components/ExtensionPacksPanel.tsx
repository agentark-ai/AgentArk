import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import Grid2 from "@mui/material/Grid";
import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import type { ExtensionPackView } from "../types";

type ExtensionPackMode = "all" | "integrations" | "messaging" | "connectors" | "channels";
const EXTENSION_PACK_REFRESH_MS = 8000;

type ConnectionSecretField = {
  key: string;
  label: string;
  helperText?: string;
  multiline?: boolean;
  sensitive?: boolean;
  transport?: string;
  transportName?: string;
};

function packKindFilter(mode: ExtensionPackMode): string | undefined {
  if (mode === "messaging" || mode === "channels") return "messaging_channel";
  if (mode === "integrations" || mode === "connectors") return "integration";
  return undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function asRecord(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {};
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeSecretPath(value: unknown): string {
  const raw = stringValue(value);
  if (!raw) return "";
  const withoutPrefix = raw.replace(/^secret\s*[:.]\s*/i, "").trim();
  return /^[A-Za-z0-9_.-]+$/.test(withoutPrefix) ? withoutPrefix : "";
}

function appendUnique(values: string[], value: unknown) {
  const normalized = normalizeSecretPath(value);
  if (normalized && !values.includes(normalized)) {
    values.push(normalized);
  }
}

function collectSecretTemplatePaths(value: unknown, out: string[]) {
  if (typeof value === "string") {
    const patterns = [
      /\{\{\s*secret\.([A-Za-z0-9_.-]+)\s*\}\}/g,
      /\{\{\s*secret:([A-Za-z0-9_.-]+)\s*\}\}/g,
    ];
    for (const pattern of patterns) {
      for (const match of value.matchAll(pattern)) {
        appendUnique(out, match[1]);
      }
    }
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item) => collectSecretTemplatePaths(item, out));
    return;
  }
  if (isRecord(value)) {
    Object.values(value).forEach((item) => collectSecretTemplatePaths(item, out));
  }
}

function authModeFallbackSecrets(pack: ExtensionPackView): string[] {
  const mode = pack.manifest.auth.mode;
  if (mode === "basic") return ["username", "password"];
  if (mode === "api_key") {
    return [normalizeSecretPath(asRecord(pack.manifest.auth.metadata).secret_field) || "api_key"];
  }
  return [];
}

function inferredSecretSpecs(pack: ExtensionPackView): ConnectionSecretField[] {
  const fields: ConnectionSecretField[] = [];
  const addField = (
    key: unknown,
    details: Partial<ConnectionSecretField> = {},
  ) => {
    const normalized = normalizeSecretPath(key);
    if (!normalized) return;
    const existing = fields.find((field) => field.key === normalized);
    if (existing) {
      Object.entries(details).forEach(([detailKey, detailValue]) => {
        if (detailValue !== undefined && detailValue !== "") {
          (existing as Record<string, unknown>)[detailKey] = detailValue;
        }
      });
      return;
    }
    fields.push({
      key: normalized,
      label: secretFieldBaseLabel(normalized, details.transport),
      multiline: normalized.split(".").pop() === "allowed_numbers",
      sensitive: isSensitiveSecretField(normalized),
      ...details,
    });
  };

  const declared = (pack.manifest.auth.required_secrets || [])
    .map(normalizeSecretPath)
    .filter(Boolean);
  declared.forEach((key) => addField(key));

  const authMetadata = asRecord(pack.manifest.auth.metadata);
  addField(authMetadata.secret_field, {
    transport: stringValue(authMetadata.auth_binding_type),
    transportName:
      stringValue(authMetadata.auth_header) ||
      stringValue(authMetadata.auth_name),
  });

  for (const fallback of authModeFallbackSecrets(pack)) {
    addField(fallback, { transport: pack.manifest.auth.mode });
  }

  for (const feature of pack.manifest.features || []) {
    const config = asRecord(feature.binding?.config);
    const auth = asRecord(config.auth);
    const authType = stringValue(auth.type);
    if (authType === "basic") {
      addField("username", { transport: authType });
      addField("password", { transport: authType });
    } else {
      addField(auth.secret_path, {
        transport: authType,
        transportName: stringValue(auth.name),
      });
    }
    const templatedSecrets: string[] = [];
    collectSecretTemplatePaths(config, templatedSecrets);
    templatedSecrets.forEach((key) => addField(key, { transport: "template" }));
  }

  return fields;
}

function defaultSecretTemplate(pack: ExtensionPackView): string {
  const requiredSecrets = inferredSecretSpecs(pack).map((field) => field.key);
  if (requiredSecrets.length > 0) {
    const payload = Object.fromEntries(
      requiredSecrets.map((key) => [
        key,
        (key.split(".").filter(Boolean).pop() || key) === "allowed_numbers" ? [""] : "",
      ])
    );
    return JSON.stringify(payload, null, 2);
  }
  return "{}";
}

function secretFieldBaseLabel(key: string, transport?: string): string {
  const displayKey = key.split(".").filter(Boolean).pop() || key;
  const normalizedTransport = (transport || "").trim().toLowerCase();
  if (
    normalizedTransport === "bearer" &&
    ["api_key", "access_token", "token", "key"].includes(displayKey.toLowerCase())
  ) {
    return "API token";
  }
  switch (displayKey) {
    case "api_key":
      return "API key";
    case "access_token":
      return "Access token";
    case "client_id":
      return "Client ID";
    case "client_secret":
      return "Client secret";
    default:
      return titleize(displayKey);
  }
}

function secretFieldLabel(pack: ExtensionPackView, field: ConnectionSecretField): string {
  const baseLabel = secretFieldBaseLabel(field.key, field.transport);
  const packName = displayPackName(pack);
  if (!packName) return baseLabel;
  if (baseLabel.toLowerCase().startsWith(packName.toLowerCase())) {
    return baseLabel;
  }
  return `${packName} ${baseLabel}`;
}

function isSensitiveSecretField(key: string): boolean {
  const normalized = key.trim().toLowerCase().split(".").pop() || "";
  return (
    normalized.includes("token") ||
    normalized.includes("secret") ||
    normalized.includes("password") ||
    normalized.includes("api_key") ||
    normalized.endsWith("_key") ||
    normalized === "key"
  );
}

function connectionSecretFields(pack: ExtensionPackView): ConnectionSecretField[] {
  return inferredSecretSpecs(pack).map((field) => ({
    ...field,
    label: secretFieldLabel(pack, field),
    helperText:
      field.helperText ||
      [
        `Stored encrypted for this connection as ${field.key}; no storage key name is required.`,
        field.transport === "bearer"
          ? "Sent as a bearer token."
          : field.transport === "header" && field.transportName
            ? `Sent in the ${field.transportName} header.`
            : field.transport === "query" && field.transportName
              ? `Sent as the ${field.transportName} query parameter.`
              : field.transport === "basic"
                ? "Used for HTTP Basic auth."
                : "No storage key name is required.",
      ].join(" ")
  }));
}

function defaultSecretValues(pack: ExtensionPackView): Record<string, string> {
  return Object.fromEntries(connectionSecretFields(pack).map((field) => [field.key, ""]));
}

function assignStructuredSecretValue(
  target: Record<string, unknown>,
  path: string,
  value: unknown,
) {
  const parts = path
    .split(".")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) return;
  let current = target;
  for (const part of parts.slice(0, -1)) {
    const existing = current[part];
    if (!isRecord(existing)) {
      current[part] = {};
    }
    current = current[part] as Record<string, unknown>;
  }
  current[parts[parts.length - 1]] = value;
}

function buildStructuredSecretPayload(values: Record<string, string>): Record<string, unknown> {
  const payload: Record<string, unknown> = {};
  Object.entries(values).forEach(([key, value]) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    const leaf = key.split(".").filter(Boolean).pop() || key;
    const structuredValue =
      leaf === "allowed_numbers"
        ? trimmed
            .split(/[\r\n,]+/)
            .map((entry) => entry.trim())
            .filter(Boolean)
        : trimmed;
    assignStructuredSecretValue(payload, key, structuredValue);
  });
  return payload;
}

function packNeedsManualSecret(pack: ExtensionPackView): boolean {
  const mode = pack.manifest.auth.mode;
  return (
    connectionSecretFields(pack).length > 0 ||
    (mode !== "none" && mode !== "oauth2_external") ||
    pack.needs_auth
  );
}

function hasStructuredSecretValue(value: unknown): boolean {
  if (typeof value === "string") return value.trim().length > 0;
  if (typeof value === "number" || typeof value === "boolean") return true;
  if (Array.isArray(value)) return value.some(hasStructuredSecretValue);
  if (isRecord(value)) return Object.values(value).some(hasStructuredSecretValue);
  return false;
}

function runtimeStatusLabel(status: string): string {
  return status.replace(/_/g, " ");
}

function titleize(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  return trimmed
    .split(/[_\-\s:.]+/)
    .filter(Boolean)
    .map((part) => {
      const lower = part.toLowerCase();
      if (lower === "api") return "API";
      if (lower === "oauth") return "OAuth";
      if (lower === "url") return "URL";
      if (lower === "uri") return "URI";
      if (lower === "cli") return "CLI";
      if (lower === "id") return "ID";
      return lower.charAt(0).toUpperCase() + lower.slice(1);
    })
    .join(" ");
}

function formatBadgeLabel(value: string): string {
  return titleize(value.replace(/_/g, " "));
}

function displayPackName(pack: ExtensionPackView): string {
  const raw = pack.manifest.name?.trim() || pack.manifest.id;
  if (/[A-Z]/.test(raw)) return raw;
  return titleize(raw);
}

function packIconColor(id: string): string {
  const palette = [
    "#78F2B0",
    "#D8AD78",
    "#C8D8C9",
    "#B7A7FF",
    "#FFB020",
    "#E57373",
    "#81C784",
    "#E6D6C0",
  ];
  const seed = Array.from(id).reduce((sum, ch) => sum + ch.charCodeAt(0), 0);
  return palette[seed % palette.length];
}

function PackIcon({ pack, size = 22 }: { pack: ExtensionPackView; size?: number }) {
  const name = displayPackName(pack);
  const color = packIconColor(pack.manifest.id);
  const letter = name.charAt(0).toUpperCase() || "?";
  return (
    <Box
      component="span"
      sx={{
        width: size,
        height: size,
        borderRadius: "6px",
        background: color,
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
        fontSize: size * 0.5,
        fontWeight: 800,
        color: "#fff",
        lineHeight: 1,
      }}
    >
      {letter}
    </Box>
  );
}

function packCardStatusLine(pack: ExtensionPackView, runtimeReady: boolean): string {
  const detail =
    pack.status_detail?.trim() ||
    pack.runtime_detail?.trim() ||
    pack.verification_detail?.trim() ||
    "";
  if (detail) return detail;
  if (pack.status === "error") {
    return "Review setup before the agent relies on this integration.";
  }
  if (pack.manifest.draft && pack.enabled && runtimeReady && !pack.needs_auth) {
    return "Draft pack. Review bindings before using it in production workflows.";
  }
  return "";
}

function packStatusLabel(
  pack: ExtensionPackView,
  runtimeReady: boolean,
  installedPack: boolean,
): string | null {
  if (!installedPack) {
    return pack.manifest.draft ? "Draft" : "Available";
  }
  if (!pack.enabled) {
    return "Disabled";
  }
  if (pack.runtime_required && !runtimeReady) {
    return "Runtime missing";
  }
  if (pack.status === "error") {
    return "Needs attention";
  }
  if (pack.needs_auth || pack.status === "needs_auth") {
    return "Needs setup";
  }
  if (pack.manifest.draft || pack.status === "draft") {
    return "Draft";
  }
  if (pack.status === "connected" || pack.status === "ready") {
    return "Ready";
  }
  const fallback = formatBadgeLabel(pack.status);
  return fallback || "Ready";
}

function packHasConfiguredLook(pack: ExtensionPackView, runtimeReady: boolean): boolean {
  return pack.enabled && !pack.needs_auth && runtimeReady && pack.status !== "error";
}

function appendWarning(message: string, warning?: string | null): string {
  const detail = typeof warning === "string" ? warning.trim() : "";
  return detail ? `${message} ${detail}` : message;
}

function runtimeResultMessage(payload: { result?: Record<string, unknown>; warning?: string | null }): string {
  const detail = typeof payload.result?.detail === "string" ? payload.result.detail : "";
  if (detail.trim()) {
    return appendWarning(detail, payload.warning);
  }
  const status = typeof payload.result?.status === "string" ? payload.result.status : "ok";
  return appendWarning(runtimeStatusLabel(status), payload.warning);
}

function isBuiltinPack(pack: ExtensionPackView): boolean {
  const metadata = pack.manifest.metadata || {};
  const authMetadata = pack.manifest.auth.metadata || {};
  return (
    typeof metadata.builtin_integration_id === "string" ||
    typeof authMetadata.builtin_integration_id === "string" ||
    pack.source_kind === "bundled_registry"
  );
}

export function ExtensionPacksPanel({
  mode = "all",
  autoRefresh = false,
}: {
  mode?: ExtensionPackMode;
  autoRefresh?: boolean;
}) {
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [notice, setNotice] = useState<{ kind: "success" | "error"; text: string } | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [linkDialogOpen, setLinkDialogOpen] = useState(false);
  const [uploadDialogOpen, setUploadDialogOpen] = useState(false);
  const [scaffoldDialogOpen, setScaffoldDialogOpen] = useState(false);
  const [connectPack, setConnectPack] = useState<ExtensionPackView | null>(null);
  const [eventsPack, setEventsPack] = useState<ExtensionPackView | null>(null);
  const [linkUrl, setLinkUrl] = useState("");
  const [sourcePath, setSourcePath] = useState("");
  const [linkTrustUnverified, setLinkTrustUnverified] = useState(false);
  const [uploadFile, setUploadFile] = useState<File | null>(null);
  const [uploadTrustUnverified, setUploadTrustUnverified] = useState(false);
  const [scaffoldName, setScaffoldName] = useState("");
  const [scaffoldKind, setScaffoldKind] = useState(mode === "messaging" || mode === "channels" ? "messaging_channel" : "integration");
  const [scaffoldFeatures, setScaffoldFeatures] = useState("");
  const [scaffoldDocsUrl, setScaffoldDocsUrl] = useState("");
  const [scaffoldOpenapiUrl, setScaffoldOpenapiUrl] = useState("");
  const [scaffoldOpenapiText, setScaffoldOpenapiText] = useState("");
  const [scaffoldCurlText, setScaffoldCurlText] = useState("");
  const [connectionName, setConnectionName] = useState("Default connection");
  const [connectionSecretValues, setConnectionSecretValues] = useState<Record<string, string>>({});
  const [connectionSecretJson, setConnectionSecretJson] = useState("{}");
  const [connectError, setConnectError] = useState<string | null>(null);
  const [selectedConnectionId, setSelectedConnectionId] = useState<string | null>(null);
  const [packMenuAnchor, setPackMenuAnchor] = useState<HTMLElement | null>(null);
  const [packMenuTarget, setPackMenuTarget] = useState<ExtensionPackView | null>(null);
  const actionButtonSx = {
    minWidth: 0,
    width: "auto",
    maxWidth: "fit-content",
    alignSelf: "flex-start",
    flex: "0 0 auto",
    whiteSpace: "nowrap",
  } as const;
  const tagChipSx = {
    height: 22,
    borderRadius: 1,
    background: "var(--ui-rgba-14-25-43-950)",
    border: "1px solid var(--ui-rgba-112-153-201-180)",
    color: "var(--ui-rgba-198-214-235-820)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.63rem",
      fontWeight: 700,
      letterSpacing: 0,
      textTransform: "uppercase",
    },
  } as const;
  const statusChipSx = {
    height: 22,
    borderRadius: 1,
    background: "var(--ui-rgba-14-25-43-920)",
    border: "1px solid var(--ui-rgba-112-153-201-160)",
    color: "var(--ui-rgba-173-192-214-900)",
    "& .MuiChip-label": {
      px: 1,
      fontSize: "0.64rem",
      fontWeight: 700,
      letterSpacing: 0,
      textTransform: "uppercase",
    },
  } as const;

  const kind = packKindFilter(mode);
  const connectSecretFields = useMemo(
    () => (connectPack ? connectionSecretFields(connectPack) : []),
    [connectPack]
  );
  const connectNeedsManualSecret = useMemo(
    () => (connectPack ? packNeedsManualSecret(connectPack) : false),
    [connectPack]
  );
  const packsQ = useQuery({
    queryKey: ["extension-packs", kind || "all", search],
    queryFn: () =>
      api.getExtensionPacks({
        query: search.trim() || undefined,
        kind
      }),
    refetchInterval: autoRefresh ? EXTENSION_PACK_REFRESH_MS : false,
  });

  const installed = useMemo(() => {
    const items = packsQ.data?.installed || [];
    if (mode !== "integrations" && mode !== "connectors") return items;
    return items.filter((pack) => !isBuiltinPack(pack));
  }, [mode, packsQ.data?.installed]);
  const catalog = useMemo(() => {
    const items = packsQ.data?.catalog || [];
    if (mode !== "integrations" && mode !== "connectors") return items;
    return items.filter((pack) => !isBuiltinPack(pack));
  }, [mode, packsQ.data?.catalog]);
  const emptyStateVisible =
    !packsQ.isLoading && !packsQ.isFetching && installed.length === 0 && catalog.length === 0;
  const connectDetailQ = useQuery({
    queryKey: ["extension-pack-detail", connectPack?.manifest.id],
    enabled: !!connectPack,
    queryFn: () => api.getExtensionPack(connectPack!.manifest.id),
    refetchInterval: autoRefresh && connectPack ? EXTENSION_PACK_REFRESH_MS : false,
  });
  const eventsQ = useQuery({
    queryKey: ["extension-pack-events", eventsPack?.manifest.id],
    enabled: !!eventsPack,
    queryFn: () => api.getExtensionPackEvents(eventsPack!.manifest.id, 25)
  });

  const installMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.installExtensionPack(payload),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning(`${payload.pack.manifest.name} installed.`, payload.warning)
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });
  const uploadMutation = useMutation({
    mutationFn: (formData: FormData) => api.uploadExtensionPack(formData),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning(`${payload.pack.manifest.name} uploaded and installed.`, payload.warning)
      });
      setUploadDialogOpen(false);
      setUploadFile(null);
      setUploadTrustUnverified(false);
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const scaffoldMutation = useMutation({
    mutationFn: (payload: Record<string, unknown>) => api.scaffoldExtensionPack(payload),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning(
          `${payload.pack.manifest.name} scaffolded as an unverified draft pack.`,
          payload.warning
        )
      });
      setScaffoldDialogOpen(false);
      setScaffoldName("");
      setScaffoldFeatures("");
      setScaffoldDocsUrl("");
      setScaffoldOpenapiUrl("");
      setScaffoldOpenapiText("");
      setScaffoldCurlText("");
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const connectionMutation = useMutation({
    mutationFn: (payload: { packId: string; body: Record<string, unknown> }) =>
      api.upsertExtensionPackConnection(payload.packId, payload.body),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning("Connection saved.", payload.warning)
      });
      setConnectPack(null);
      setConnectError(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["extension-packs"] }),
        queryClient.invalidateQueries({ queryKey: ["extension-pack-detail"] })
      ]);
    },
    onError: (error: Error) => setConnectError(error.message)
  });

  const enableMutation = useMutation({
    mutationFn: (payload: { packId: string; enabled: boolean }) =>
      api.setExtensionPackEnabled(payload.packId, payload.enabled),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning(
          `${payload.pack.manifest.name} ${payload.pack.enabled ? "enabled" : "disabled"}.`,
          payload.warning
        )
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });
  const runtimeMutation = useMutation({
    mutationFn: async (payload: {
      packId: string;
      operation: "install" | "verify" | "update" | "uninstall";
    }) => {
      switch (payload.operation) {
        case "install":
          return api.installExtensionPackRuntime(payload.packId);
        case "verify":
          return api.verifyExtensionPackRuntime(payload.packId);
        case "update":
          return api.updateExtensionPackRuntime(payload.packId);
        case "uninstall":
          return api.uninstallExtensionPackRuntime(payload.packId);
      }
    },
    onSuccess: async (payload, variables) => {
      setNotice({
        kind: variables.operation === "verify" ? "success" : "success",
        text: runtimeResultMessage(payload)
      });
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["extension-packs"] }),
        queryClient.invalidateQueries({ queryKey: ["extension-pack-detail"] })
      ]);
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  useEffect(() => {
    if (!connectPack) return;
    const preferred =
      connectDetailQ.data?.connections?.find((item) => item.state === "ready") ||
      connectDetailQ.data?.connections?.[0];
    if (!preferred) return;
    if (!selectedConnectionId) {
      setSelectedConnectionId(preferred.connection.id);
      setConnectionName(preferred.connection.name || "Default connection");
    }
  }, [connectDetailQ.data, connectPack, selectedConnectionId]);

  const deleteMutation = useMutation({
    mutationFn: (packId: string) => api.deleteExtensionPack(packId, { remove_connections: true }),
    onSuccess: async (payload) => {
      setNotice({
        kind: "success",
        text: appendWarning("Pack deleted.", payload.warning)
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
      await queryClient.invalidateQueries({ queryKey: ["extension-pack-detail"] });
    },
    onError: (error: Error) => setNotice({ kind: "error", text: error.message })
  });

  const sectionTitle = useMemo(() => {
    if (mode === "messaging" || mode === "channels") return "Generic Channel Packs";
    if (mode === "integrations" || mode === "connectors") return "Extension Pack Integrations";
    return "Generic Packs";
  }, [mode]);

  const sectionSubtitle = useMemo(() => {
    if (mode === "messaging" || mode === "channels") {
      return "Search installed packs, bundled defaults, upload a bundle, or scaffold a new messaging channel pack.";
    }
    if (mode === "integrations" || mode === "connectors") {
      return "Install and manage manifest-based integration packs. Custom API integrations are managed separately above.";
    }
    return "Search installed packs, bundled defaults, upload a bundle, or scaffold from OpenAPI/cURL when nothing exists yet.";
  }, [mode]);

  const emptyStateMessage = useMemo(() => {
    if (mode === "messaging" || mode === "channels") {
      return "No channel pack matched this search. Ask for a link or local path, upload a manifest/bundle, or scaffold a draft channel pack.";
    }
    if (mode === "integrations" || mode === "connectors") {
      return "No extension pack matched this search yet. Add one from a link, upload, or scaffold a draft.";
    }
    return "No pack matched this search. Ask for a link or local path, upload a manifest/bundle, or scaffold a draft pack from docs/OpenAPI/cURL.";
  }, [mode]);

  const addButtonLabel = useMemo(() => {
    if (mode === "messaging" || mode === "channels") return "Add Channel Pack";
    if (mode === "integrations" || mode === "connectors") return "Add Extension Pack";
    return "Add Pack";
  }, [mode]);

  const addDialogTitle = useMemo(() => {
    if (mode === "messaging" || mode === "channels") return "Add channel pack";
    if (mode === "integrations" || mode === "connectors") return "Add extension pack";
    return "Add pack";
  }, [mode]);

  async function openOauthConnect(pack: ExtensionPackView) {
    try {
      const payload = await api.getExtensionPackConnectUrl(pack.manifest.id);
      window.open(payload.url, "_blank", "noopener,noreferrer");
      setNotice({
        kind: "success",
        text: `Opened ${pack.manifest.name} sign-in in a new tab.`
      });
      void queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    } catch (error) {
      setNotice({
        kind: "error",
        text: error instanceof Error ? error.message : "Failed to open the connect URL."
      });
    }
  }

  async function testPack(pack: ExtensionPackView) {
    try {
      const detail = await api.getExtensionPack(pack.manifest.id);
      const connectionId =
        detail.connections.find((item) => item.state === "ready")?.connection.id ||
        detail.connections[0]?.connection.id;
      if (!connectionId) throw new Error("No saved connection was found for this pack.");
      const result = await api.testExtensionPackConnection(pack.manifest.id, connectionId);
      const status = String(result.result.status || "ok");
      setNotice({
        kind: status === "ok" ? "success" : "error",
        text:
          String(result.result.message || "") ||
          `${pack.manifest.name} test finished with status ${status}.`
      });
      await queryClient.invalidateQueries({ queryKey: ["extension-packs"] });
    } catch (error) {
      setNotice({
        kind: "error",
        text: error instanceof Error ? error.message : "Pack test failed."
      });
    }
  }

  function openConnectDialog(pack: ExtensionPackView) {
    setConnectPack(pack);
    setSelectedConnectionId(null);
    setConnectionName("Default connection");
    setConnectionSecretValues(defaultSecretValues(pack));
    setConnectionSecretJson(defaultSecretTemplate(pack));
    setConnectError(null);
  }

  function openPackMenu(event: MouseEvent<HTMLElement>, pack: ExtensionPackView) {
    event.stopPropagation();
    setPackMenuAnchor(event.currentTarget);
    setPackMenuTarget(pack);
  }

  function closePackMenu() {
    setPackMenuAnchor(null);
    setPackMenuTarget(null);
  }

  function renderPackCard(pack: ExtensionPackView, installedPack: boolean) {
    const runtimeReady = !pack.runtime_required || pack.runtime_status === "ready";
    const packName = displayPackName(pack);
    const statusLine = packCardStatusLine(pack, runtimeReady);
    const statusLabel = packStatusLabel(pack, runtimeReady, installedPack);
    const configuredLook = installedPack
      ? packHasConfiguredLook(pack, runtimeReady)
      : false;
    const primaryLabel = installedPack
      ? pack.enabled
        ? "Disable"
        : "Enable"
      : "Install";
    return (
      <Box
        key={`${installedPack ? "installed" : "catalog"}-${pack.manifest.id}`}
        sx={{
          height: "100%",
          p: 1.5,
          borderRadius: 1.5,
          border: configuredLook
            ? "1px solid var(--ui-rgba-64-196-255-240)"
            : "1px solid var(--ui-rgba-112-153-201-160)",
          background: configuredLook
            ? "var(--ui-rgba-8-24-42-560)"
            : "var(--ui-rgba-7-17-32-600)",
          transition: "border-color 0.15s, background 0.15s, box-shadow 0.15s",
          "&:hover": {
            borderColor: configuredLook
              ? "var(--ui-rgba-109-226-255-340)"
              : "var(--ui-rgba-148-181-220-240)",
            background: configuredLook
              ? "var(--ui-rgba-9-28-48-660)"
              : "var(--ui-rgba-9-21-39-720)",
            boxShadow: "0 8px 24px var(--ui-rgba-0-0-0-180)",
          },
        }}
      >
        <Stack spacing={1.1} sx={{ height: "100%", justifyContent: "space-between" }}>
          <Box>
          <Stack
            direction="row"
            spacing={0.9}
            sx={{
              alignItems: "center",
              mb: 0.75,
              justifyContent: "space-between",
            }}
          >
            <Stack direction="row" spacing={0.9} sx={{ alignItems: "center", minWidth: 0 }}>
              <PackIcon pack={pack} size={20} />
              <Typography variant="subtitle2" noWrap sx={{ fontWeight: 700 }}>
                {packName}
              </Typography>
            </Stack>
            {installedPack ? (
              <IconButton
                size="small"
                onClick={(event) => openPackMenu(event, pack)}
                aria-label={`More actions for ${packName}`}
                sx={{ color: "text.secondary", flexShrink: 0 }}
              >
                <MoreVertIcon fontSize="small" />
              </IconButton>
            ) : null}
          </Stack>
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
              lineHeight: 1.45,
              display: "-webkit-box",
              WebkitLineClamp: 2,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
            }}
          >
            {pack.manifest.description || "No description provided."}
          </Typography>
          {statusLine ? (
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                lineHeight: 1.45,
                mt: 0.75,
                display: "-webkit-box",
                WebkitLineClamp: 2,
                WebkitBoxOrient: "vertical",
                overflow: "hidden",
              }}
            >
              {statusLine}
            </Typography>
          ) : null}
          </Box>
          <Stack direction="row" spacing={0.75} sx={{ alignItems: "center", justifyContent: "space-between" }}>
            <Stack direction="row" spacing={0.5} useFlexGap sx={{ flexWrap: "wrap" }}>
              <Chip size="small" label={isBuiltinPack(pack) ? "Bundled" : "Custom"} sx={tagChipSx} />
              {statusLabel ? <Chip size="small" label={statusLabel} sx={statusChipSx} /> : null}
            </Stack>
            {!installedPack ? (
              <Button
                size="small"
                variant="contained"
                sx={actionButtonSx}
                onClick={() => installMutation.mutate({ pack_id: pack.manifest.id })}
                disabled={installMutation.isPending}
              >
                {primaryLabel}
              </Button>
            ) : (
                <Button
                  size="small"
                  variant={pack.enabled ? "outlined" : "contained"}
                  sx={actionButtonSx}
                  onClick={() =>
                    enableMutation.mutate({
                      packId: pack.manifest.id,
                      enabled: !pack.enabled,
                    })
                  }
                  disabled={enableMutation.isPending}
                >
                  {primaryLabel}
                </Button>
            )}
          </Stack>
        </Stack>
      </Box>
    );
  }

  return (
    <Stack spacing={1.5}>
      <Box className="list-shell" sx={{ p: 1.5 }}>
        <Stack spacing={1.2}>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              justifyContent: "space-between",
              alignItems: { xs: "stretch", sm: "center" }
            }}>
            <Box>
              <Typography variant="subtitle2">{sectionTitle}</Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {sectionSubtitle}
              </Typography>
            </Box>
            <Button variant="contained" sx={{ minWidth: 0, width: "auto", maxWidth: "fit-content", alignSelf: "flex-start", flex: "0 0 auto", whiteSpace: "nowrap" }} onClick={() => setAddDialogOpen(true)}>
              {addButtonLabel}
            </Button>
          </Stack>
          <TextField
            size="small"
            label="Search packs"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="microsoft, slack, notion, clickup..."
          />
          {notice ? <Alert severity={notice.kind}>{notice.text}</Alert> : null}
          {packsQ.error ? (
            <Alert severity="error">
              {packsQ.error instanceof Error ? packsQ.error.message : "Failed to load extension packs."}
            </Alert>
          ) : null}
          {emptyStateVisible ? (
            <Alert severity="info">
              {emptyStateMessage}
            </Alert>
          ) : null}
          {packsQ.data?.not_found ? (
            <Stack spacing={0.75}>
              {packsQ.data.next_steps.map((step) => (
                <Typography key={step} variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  {step}
                </Typography>
              ))}
            </Stack>
          ) : null}
          {installed.length > 0 ? (
            <Stack spacing={1}>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Installed
              </Typography>
              <Grid2 container spacing={1.25}>
                {installed.map((pack) => (
                  <Grid2 key={`installed-${pack.manifest.id}`} size={{ xs: 12, md: 6, xl: 4 }}>
                    {renderPackCard(pack, true)}
                  </Grid2>
                ))}
              </Grid2>
            </Stack>
          ) : null}
          {catalog.length > 0 ? (
            <Stack spacing={1}>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Catalog
              </Typography>
              <Grid2 container spacing={1.25}>
                {catalog.map((pack) => (
                  <Grid2 key={`catalog-${pack.manifest.id}`} size={{ xs: 12, md: 6, xl: 4 }}>
                    {renderPackCard(pack, false)}
                  </Grid2>
                ))}
              </Grid2>
            </Stack>
          ) : null}
        </Stack>
      </Box>
      <Menu
        anchorEl={packMenuAnchor}
        open={!!packMenuAnchor && !!packMenuTarget}
        onClose={closePackMenu}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
      >
        {packMenuTarget ? (
          [
            <MenuItem key="status" disabled>
              Status: {formatBadgeLabel(packMenuTarget.status)}
            </MenuItem>,
            (packMenuTarget.needs_auth || packMenuTarget.supports_connect_url) ? (
              <MenuItem
                key="setup"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  if (pack.supports_connect_url) {
                    void openOauthConnect(pack);
                  } else {
                    openConnectDialog(pack);
                  }
                }}
              >
                Setup
              </MenuItem>
            ) : null,
            <MenuItem
              key="test"
              disabled={!(!packMenuTarget.runtime_required || packMenuTarget.runtime_status === "ready")}
              onClick={() => {
                const pack = packMenuTarget;
                closePackMenu();
                if (!pack) return;
                void testPack(pack);
              }}
            >
              Test
            </MenuItem>,
            packMenuTarget.supports_webhook ? (
              <MenuItem
                key="events"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  setEventsPack(pack);
                }}
              >
                Recent runs
              </MenuItem>
            ) : null,
            packMenuTarget.runtime_required ? (
              <MenuItem
                key="runtime-install"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  runtimeMutation.mutate({
                    packId: pack.manifest.id,
                    operation: "install",
                  });
                }}
              >
                {packMenuTarget.runtime_status === "ready" ? "Reinstall runtime" : "Install runtime"}
              </MenuItem>
            ) : null,
            packMenuTarget.runtime_required ? (
              <MenuItem
                key="runtime-verify"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  runtimeMutation.mutate({
                    packId: pack.manifest.id,
                    operation: "verify",
                  });
                }}
              >
                Verify runtime
              </MenuItem>
            ) : null,
            packMenuTarget.runtime_required ? (
              <MenuItem
                key="runtime-update"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  runtimeMutation.mutate({
                    packId: pack.manifest.id,
                    operation: "update",
                  });
                }}
              >
                Update runtime
              </MenuItem>
            ) : null,
            packMenuTarget.runtime_required ? (
              <MenuItem
                key="runtime-uninstall"
                disabled={packMenuTarget.runtime_status === "missing"}
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  runtimeMutation.mutate({
                    packId: pack.manifest.id,
                    operation: "uninstall",
                  });
                }}
              >
                Uninstall runtime
              </MenuItem>
            ) : null,
            !isBuiltinPack(packMenuTarget) ? (
              <MenuItem
                key="delete"
                onClick={() => {
                  const pack = packMenuTarget;
                  closePackMenu();
                  if (!pack) return;
                  if (
                    !window.confirm(
                      `Delete ${displayPackName(pack)}? This removes saved connections, credentials, auth profiles, events, and local runtime files for this pack.`
                    )
                  ) {
                    return;
                  }
                  deleteMutation.mutate(pack.manifest.id);
                }}
              >
                Delete
              </MenuItem>
            ) : null,
          ].filter(Boolean)
        ) : null}
      </Menu>
      <Dialog open={addDialogOpen} onClose={() => setAddDialogOpen(false)} maxWidth="xs" fullWidth>
        <DialogTitle>{addDialogTitle}</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.25}>
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              Choose how you want to add this integration.
            </Typography>
            <Button
              variant="outlined"
              onClick={() => {
                setAddDialogOpen(false);
                setLinkDialogOpen(true);
              }}
            >
              Link or path
            </Button>
            <Button
              variant="outlined"
              onClick={() => {
                setAddDialogOpen(false);
                setUploadDialogOpen(true);
              }}
            >
              Upload bundle
            </Button>
            <Button
              variant="outlined"
              onClick={() => {
                setAddDialogOpen(false);
                setScaffoldDialogOpen(true);
              }}
            >
              Scaffold draft
            </Button>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setAddDialogOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog open={linkDialogOpen} onClose={() => setLinkDialogOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Install pack from link or local path</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Use this when you already have a manifest URL, raw manifest path, or local bundle path. Non-bundled sources install as unverified packs unless publisher verification succeeds.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Manifest URL"
              value={linkUrl}
              onChange={(event) => setLinkUrl(event.target.value)}
              placeholder="https://example.com/pack.json"
            />
            <TextField
              fullWidth
              size="small"
              label="Local manifest or bundle path"
              value={sourcePath}
              onChange={(event) => setSourcePath(event.target.value)}
              placeholder="C:\\packs\\clickup-pack.zip"
            />
            <Alert severity="warning">
              Unsigned packs are blocked unless you explicitly accept unverified code, bindings, and runtime commands.
            </Alert>
            <FormControlLabel
              control={
                <Checkbox
                  checked={linkTrustUnverified}
                  onChange={(event) => setLinkTrustUnverified(event.target.checked)}
                />
              }
              label="Install without publisher verification"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setLinkDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() =>
              installMutation.mutate(
                {
                  source_url: linkUrl.trim() || undefined,
                  source_path: sourcePath.trim() || undefined,
                  trust_unverified: linkTrustUnverified
                },
                {
                  onSuccess: () => {
                    setLinkDialogOpen(false);
                    setLinkUrl("");
                    setSourcePath("");
                    setLinkTrustUnverified(false);
                  }
                }
              )
            }
            disabled={installMutation.isPending || (!linkUrl.trim() && !sourcePath.trim())}
          >
            Install
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={uploadDialogOpen} onClose={() => setUploadDialogOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>Upload manifest or bundle</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Upload a manifest JSON/YAML file or a zip bundle containing one of the expected manifest names.
            </Typography>
            <Alert severity="warning">
              Review local CLI, runtime installer, HTTP bindings, and secret usage before installing unsigned packs.
            </Alert>
            <Button variant="outlined" component="label">
              {uploadFile ? uploadFile.name : "Choose file"}
              <input
                hidden
                type="file"
                accept=".json,.yaml,.yml,.zip"
                onChange={(event) => setUploadFile(event.target.files?.[0] ?? null)}
              />
            </Button>
            <FormControlLabel
              control={
                <Checkbox
                  checked={uploadTrustUnverified}
                  onChange={(event) => setUploadTrustUnverified(event.target.checked)}
                />
              }
              label="Install upload without publisher verification"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setUploadDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            disabled={uploadMutation.isPending || !uploadFile}
            onClick={() => {
              if (!uploadFile) return;
              const formData = new FormData();
              formData.append("file", uploadFile);
              formData.append("trust_unverified", String(uploadTrustUnverified));
              uploadMutation.mutate(formData);
            }}
          >
            Upload
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={scaffoldDialogOpen}
        onClose={() => setScaffoldDialogOpen(false)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Scaffold draft pack</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Draft packs are local and unverified by default. Start read-only when possible, then replace placeholder bindings after review.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Service name"
              value={scaffoldName}
              onChange={(event) => setScaffoldName(event.target.value)}
              placeholder="ClickUp"
            />
            <TextField
              fullWidth
              size="small"
              label="Pack kind"
              value={scaffoldKind}
              onChange={(event) => setScaffoldKind(event.target.value)}
              placeholder="integration"
            />
            <TextField
              fullWidth
              size="small"
              label="Desired features"
              value={scaffoldFeatures}
              onChange={(event) => setScaffoldFeatures(event.target.value)}
              placeholder="tasks.list, tasks.get, tasks.update"
              helperText="Comma-separated canonical or experimental feature IDs."
            />
            <TextField
              fullWidth
              size="small"
              label="Docs URL"
              value={scaffoldDocsUrl}
              onChange={(event) => setScaffoldDocsUrl(event.target.value)}
            />
            <Divider flexItem />
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Optional import source. If you provide an OpenAPI URL, OpenAPI text, or a sample curl command, the draft pack will be generated with executable HTTP bindings instead of placeholder bindings.
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="OpenAPI URL"
              value={scaffoldOpenapiUrl}
              onChange={(event) => setScaffoldOpenapiUrl(event.target.value)}
              placeholder="https://api.example.com/openapi.json"
            />
            <TextField
              fullWidth
              multiline
              minRows={4}
              size="small"
              label="OpenAPI text"
              value={scaffoldOpenapiText}
              onChange={(event) => setScaffoldOpenapiText(event.target.value)}
              placeholder='{"openapi":"3.0.0", ...}'
            />
            <TextField
              fullWidth
              multiline
              minRows={3}
              size="small"
              label="Sample curl command"
              value={scaffoldCurlText}
              onChange={(event) => setScaffoldCurlText(event.target.value)}
              placeholder="curl https://api.example.com/v1/items -H 'Authorization: Bearer ...'"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setScaffoldDialogOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() =>
              scaffoldMutation.mutate({
                name: scaffoldName.trim(),
                kind: scaffoldKind.trim(),
                docs_url: scaffoldDocsUrl.trim() || undefined,
                openapi_url: scaffoldOpenapiUrl.trim() || undefined,
                openapi_text: scaffoldOpenapiText.trim() || undefined,
                curl_text: scaffoldCurlText.trim() || undefined,
                desired_features: scaffoldFeatures
                  .split(",")
                  .map((value) => value.trim())
                  .filter(Boolean),
                binding_kind: "unsupported"
              })
            }
            disabled={scaffoldMutation.isPending || !scaffoldName.trim()}
          >
            Create draft
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={!!connectPack} onClose={() => setConnectPack(null)} maxWidth="sm" fullWidth>
        <DialogTitle>{connectPack ? `Connect ${connectPack.manifest.name}` : "Connect pack"}</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.5}>
            {connectError ? <Alert severity="error">{connectError}</Alert> : null}
            {connectDetailQ.data?.connections?.length ? (
              <Stack spacing={1}>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  Existing connections
                </Typography>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  {connectDetailQ.data.connections.map((item) => (
                    <Button
                      key={item.connection.id}
                      size="small"
                      variant={selectedConnectionId === item.connection.id ? "contained" : "outlined"}
                      onClick={() => {
                        setSelectedConnectionId(item.connection.id);
                        setConnectionName(item.connection.name || "Default connection");
                        setConnectionSecretValues(defaultSecretValues(connectPack!));
                        setConnectionSecretJson(defaultSecretTemplate(connectPack!));
                      }}
                    >
                      {item.connection.name || item.connection.id}
                    </Button>
                  ))}
                  <Button
                    size="small"
                    variant={selectedConnectionId ? "outlined" : "contained"}
                    onClick={() => {
                      setSelectedConnectionId(null);
                      setConnectionName("Default connection");
                      setConnectionSecretValues(defaultSecretValues(connectPack!));
                      setConnectionSecretJson(defaultSecretTemplate(connectPack!));
                    }}
                  >
                    New connection
                  </Button>
                </Stack>
              </Stack>
            ) : null}
            <Alert severity="info" sx={{ borderRadius: 1 }}>
              Never paste secrets in normal chat. Use this secure setup form.
            </Alert>
            <TextField
              fullWidth
              size="small"
              label="Connection name"
              value={connectionName}
              onChange={(event) => setConnectionName(event.target.value)}
              helperText="A connection is the saved credential profile for this integration. The default connection is used automatically unless a workflow selects another one."
            />
            {connectSecretFields.length > 0 ? (
              <Stack spacing={1}>
                {connectSecretFields.map((field) => (
                  <TextField
                    key={field.key}
                    fullWidth
                    size="small"
                    type={field.sensitive && !field.multiline ? "password" : "text"}
                    label={field.label}
                    value={connectionSecretValues[field.key] || ""}
                    helperText={field.helperText}
                    multiline={field.multiline}
                    minRows={field.multiline ? 3 : undefined}
                    onChange={(event) =>
                      setConnectionSecretValues((current) => ({
                        ...current,
                        [field.key]: event.target.value
                      }))
                    }
                  />
                ))}
              </Stack>
            ) : connectNeedsManualSecret ? (
              <>
                <Typography variant="caption" sx={{
                  color: "text.secondary"
                }}>
                  This pack declares credential requirements that do not map to simple fields. Use advanced JSON for this setup.
                </Typography>
                <TextField
                  fullWidth
                  multiline
                  minRows={6}
                  size="small"
                  label="Secret JSON"
                  value={connectionSecretJson}
                  onChange={(event) => setConnectionSecretJson(event.target.value)}
                />
              </>
            ) : (
              <Alert severity="info" sx={{ borderRadius: 1 }}>
                This pack does not require credentials for this connection.
              </Alert>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConnectPack(null)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => {
              if (!connectPack) return;
              if (connectSecretFields.length > 0) {
                const defaultValues = defaultSecretValues(connectPack);
                const hasSecretEdits = connectSecretFields.some(
                  (field) =>
                    (connectionSecretValues[field.key] || "") !== (defaultValues[field.key] || "")
                );
                const missingFields = connectSecretFields
                  .filter((field) => !(connectionSecretValues[field.key] || "").trim())
                  .map((field) => field.label);
                if ((!selectedConnectionId || hasSecretEdits) && missingFields.length > 0) {
                  setConnectError(`Enter ${missingFields.join(", ")} using the secure form.`);
                  return;
                }
                connectionMutation.mutate({
                  packId: connectPack.manifest.id,
                  body: {
                    connection_id: selectedConnectionId || undefined,
                    name: connectionName.trim() || "Default connection",
                    secret: hasSecretEdits || !selectedConnectionId
                      ? buildStructuredSecretPayload(connectionSecretValues)
                      : undefined
                  }
                });
                return;
              }
              try {
                const parsedSecret = JSON.parse(connectionSecretJson);
                const hasSecret = hasStructuredSecretValue(parsedSecret);
                if (connectNeedsManualSecret && !selectedConnectionId && !hasSecret) {
                  setConnectError("Enter the required credentials using advanced JSON.");
                  return;
                }
                connectionMutation.mutate({
                  packId: connectPack.manifest.id,
                  body: {
                    connection_id: selectedConnectionId || undefined,
                    name: connectionName.trim() || "Default connection",
                    secret: hasSecret ? parsedSecret : undefined
                  }
                });
              } catch {
                setConnectError("Secret JSON is invalid.");
              }
            }}
            disabled={connectionMutation.isPending}
          >
            Save connection
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={!!eventsPack} onClose={() => setEventsPack(null)} maxWidth="md" fullWidth>
        <DialogTitle>{eventsPack ? `${eventsPack.manifest.name} inbound events` : "Inbound events"}</DialogTitle>
        <DialogContent sx={{ pt: 1 }}>
          <Stack spacing={1.25}>
            {eventsPack?.webhook_path ? (
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                  wordBreak: "break-all"
                }}>
                {`${window.location.origin}${eventsPack.webhook_path}`}
              </Typography>
            ) : null}
            {eventsQ.error ? (
              <Alert severity="error">
                {eventsQ.error instanceof Error ? eventsQ.error.message : "Failed to load pack events."}
              </Alert>
            ) : null}
            {!eventsQ.data?.items?.length ? (
              <Alert severity="info">No inbound events recorded for this pack yet.</Alert>
            ) : null}
            {eventsQ.data?.items?.map((event) => (
              <Box
                key={event.id}
                sx={{
                  p: 1.2,
                  borderRadius: "8px",
                  border: "1px solid var(--ui-rgba-255-255-255-080)",
                  background: "var(--ui-rgba-255-255-255-020)"
                }}
              >
                <Stack spacing={0.75}>
                  <Stack direction="row" spacing={0.75} useFlexGap sx={{
                    flexWrap: "wrap"
                  }}>
                    <Chip size="small" label={event.event_type} variant="outlined" />
                    <Chip size="small" label={event.status} variant="outlined" />
                    <Chip size="small" label={event.transport} variant="outlined" />
                  </Stack>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    {formatUiDateTime(event.received_at, {
                      fallback: event.received_at || "-",
                      includeSeconds: true,
                      includeYear: true,
                    })}
                  </Typography>
                  {event.outcome ? (
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {event.outcome}
                    </Typography>
                  ) : null}
                  {event.response_preview ? (
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {event.response_preview}
                    </Typography>
                  ) : null}
                </Stack>
              </Box>
            ))}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setEventsPack(null)}>Close</Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
}
