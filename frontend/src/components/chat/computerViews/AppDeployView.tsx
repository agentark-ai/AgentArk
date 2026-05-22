// App deploy view for the Computer pane.
// Renders the deployed app URL plus a list of files captured during the deploy.

import { useMemo, useState } from "react";
import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import OpenInNewRounded from "@mui/icons-material/OpenInNewRounded";
import ContentCopyRounded from "@mui/icons-material/ContentCopyRounded";
import InsertDriveFileRounded from "@mui/icons-material/InsertDriveFileRounded";
import ChevronRightRounded from "@mui/icons-material/ChevronRightRounded";

import type { ChatStepCard, ComputerPaneFile } from "../types";
import { firstSurfaceUri, surfacePayloads } from "../surface";

export interface AppDeployViewProps {
  card: ChatStepCard;
  onOpenFile?: (path: string) => void;
  workspaceFiles?: ComputerPaneFile[];
}

interface DeployFile {
  path: string;
  bytes?: number;
}

interface DeployPayload {
  appId?: string;
  url?: string;
  files: DeployFile[];
}

interface ServiceSummary {
  id?: string;
  title?: string;
  url?: string;
  status?: string;
  type?: string;
  running?: boolean;
  enabled?: boolean;
}

interface ServiceManagePayload {
  status?: string;
  message?: string;
  service?: ServiceSummary;
  services: ServiceSummary[];
  serviceCount?: number;
  query?: string;
  serviceId?: string;
}

interface ParsedPayload {
  deploy: DeployPayload;
  serviceManage?: ServiceManagePayload;
}

function safeParse(body: string): unknown {
  try {
    return JSON.parse(body);
  } catch {
    return null;
  }
}

function extractFirstJsonObject(body: string): unknown {
  const start = body.indexOf("{");
  if (start < 0) return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let idx = start; idx < body.length; idx += 1) {
    const ch = body[idx];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === "\\") {
        escaped = true;
      } else if (ch === "\"") {
        inString = false;
      }
      continue;
    }
    if (ch === "\"") {
      inString = true;
    } else if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) return safeParse(body.slice(start, idx + 1));
    }
  }
  return null;
}

function parseStructuredBody(body: string): unknown {
  return safeParse(body) || extractFirstJsonObject(body);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed || undefined;
}

function asNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const n = Number(value);
    if (Number.isFinite(n)) return n;
  }
  return undefined;
}

function isDeployFilePath(path: string): boolean {
  const normalized = (path || "").trim().replace(/\\/g, "/");
  if (!normalized) return false;
  if (/^https?:\/\//i.test(normalized)) return false;
  if (/^[\d.]+$/.test(normalized)) return false;
  if (normalized.includes("..") || normalized.startsWith("/")) return false;
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length === 0) return false;
  const base = parts[parts.length - 1] || "";
  if (!base || /[<>:"|?*]/.test(base)) return false;
  return parts.length > 1 || /\.[A-Za-z0-9]{1,12}$/.test(base) || base.startsWith(".");
}

function normalizeFiles(raw: unknown): DeployFile[] {
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    return Object.entries(raw as Record<string, unknown>)
      .filter(([path]) => isDeployFilePath(path))
      .map(([path, content]) => ({
        path,
        bytes: typeof content === "string" ? new Blob([content]).size : undefined,
      }));
  }
  if (!Array.isArray(raw)) return [];
  const out: DeployFile[] = [];
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const rec = entry as Record<string, unknown>;
    const path =
      (typeof rec.path === "string" && rec.path) ||
      (typeof rec.file === "string" && rec.file) ||
      (typeof rec.name === "string" && rec.name) ||
      "";
    if (!isDeployFilePath(path)) continue;
    const rawContent =
      typeof rec.content === "string"
        ? rec.content
        : typeof rec.text === "string"
          ? rec.text
          : typeof rec.body === "string"
            ? rec.body
            : "";
    const bytes = asNumber(rec.bytes) ?? asNumber(rec.size) ?? (rawContent ? new Blob([rawContent]).size : undefined);
    out.push({ path, bytes });
  }
  return out;
}

function mergeDeployFiles(
  primary: DeployFile[],
  workspaceFiles: ComputerPaneFile[],
): DeployFile[] {
  const merged = new Map<string, DeployFile>();
  for (const file of [...primary, ...workspaceFiles.map((entry) => ({
    path: entry.path,
    bytes: entry.content ? new Blob([entry.content]).size : undefined,
  }))]) {
    const key = file.path.trim();
    if (!isDeployFilePath(key)) continue;
    const existing = merged.get(key);
    merged.set(key, {
      path: key,
      bytes: file.bytes ?? existing?.bytes,
    });
  }
  return Array.from(merged.values());
}

function normalizeService(value: unknown): ServiceSummary | undefined {
  const rec = asRecord(value);
  if (!rec) return undefined;
  const id = asString(rec.id) || asString(rec.app_id) || asString(rec.service_id);
  const title = asString(rec.title) || asString(rec.name);
  const url = asString(rec.url) || asString(rec.access_url);
  const status = asString(rec.status);
  const type = asString(rec.type) || asString(rec.kind);
  const running = typeof rec.running === "boolean" ? rec.running : undefined;
  const enabled = typeof rec.enabled === "boolean" ? rec.enabled : undefined;
  if (!id && !title && !url && !status) return undefined;
  return { id, title, url, status, type, running, enabled };
}

function parseServiceManagePayload(parsed: unknown): ServiceManagePayload | undefined {
  const rec = asRecord(parsed);
  if (!rec) return undefined;
  const data = asRecord(rec.data);
  const candidates = data ? [data, rec] : [rec];
  for (const candidate of candidates) {
    const tool = asString(candidate.tool) || asString(rec.tool);
    const hasServiceShape =
      Boolean(candidate.service) ||
      Array.isArray(candidate.services) ||
      candidate.service_count !== undefined ||
      candidate.service_id !== undefined;
    if (tool !== "service_manage" && !hasServiceShape) continue;
    const services = Array.isArray(candidate.services)
      ? candidate.services.map(normalizeService).filter((item): item is ServiceSummary => Boolean(item))
      : [];
    const service = normalizeService(candidate.service);
    const serviceCount = asNumber(candidate.service_count) ?? services.length + (service ? 1 : 0);
    return {
      status: asString(candidate.status) || asString(rec.status),
      message: asString(candidate.message) || asString(rec.detail) || asString(candidate.detail),
      service,
      services,
      serviceCount,
      query: asString(candidate.query),
      serviceId: asString(candidate.service_id),
    };
  }
  return undefined;
}

function parsePayload(card: ChatStepCard): ParsedPayload {
  const surfaceUrl = firstSurfaceUri(card);
  const surfaceFiles = normalizeFiles(
    surfacePayloads(card)
      .map((item) => {
        if (item.path) {
          return {
            path: item.path,
            content: item.text || (typeof item.json === "string" ? item.json : ""),
          };
        }
        return null;
      })
      .filter(Boolean),
  );
  const body =
    card.payloadView?.body ||
    card.rawDetailFull ||
    card.detailFull ||
    card.detail ||
    card.summary ||
    "";
  const parsed = parseStructuredBody(body);
  let appId: string | undefined;
  let url: string | undefined = surfaceUrl || undefined;
  let files: DeployFile[] = surfaceFiles;
  const serviceManage = parseServiceManagePayload(parsed);
  if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
    const rec = parsed as Record<string, unknown>;
    const data = asRecord(rec.data);
    const source = data || rec;
    if (typeof source.app_id === "string") appId = source.app_id;
    if (typeof source.url === "string") url = source.url;
    else if (typeof source.app_url === "string") url = source.app_url;
    else if (typeof source.dashboard_url === "string") url = source.dashboard_url;
    files = normalizeFiles(source.files);
    if (files.length === 0) files = normalizeFiles(source.sources);
    if (files.length === 0 && Array.isArray(source.file_names)) {
      files = (source.file_names as unknown[])
        .filter((value): value is string => typeof value === "string")
        .filter(isDeployFilePath)
        .map((path) => ({ path }));
    }
  }
  return { deploy: { appId, url, files }, serviceManage };
}

function formatKb(bytes?: number): string | null {
  if (bytes === undefined || bytes < 0) return null;
  const kb = bytes / 1024;
  if (kb < 0.1) return `${bytes} B`;
  return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
}

function ServiceManageView({
  card,
  payload,
}: {
  card: ChatStepCard;
  payload: ServiceManagePayload;
}) {
  const [copiedUrl, setCopiedUrl] = useState<string | null>(null);
  const rows = payload.service
    ? [payload.service]
    : payload.services;
  const title = card.label || "Service manage";
  const status = payload.status || (rows.length > 0 ? "ok" : "empty");
  const statusClass = status.replace(/[^a-z0-9_-]/gi, "_");
  const copyUrl = (url?: string) => {
    if (!url) return;
    void navigator.clipboard.writeText(url).then(() => {
      setCopiedUrl(url);
      window.setTimeout(() => setCopiedUrl(null), 1500);
    });
  };

  return (
    <Box className="cview cview-deploy cview-service-manage">
      <Box className="cview-deploy-head">
        <Box className="cview-deploy-title">
          <Typography component="span" variant="subtitle2">{title}</Typography>
          <Box component="span" className={`cview-service-status cview-service-status-${statusClass}`}>
            {status}
          </Box>
        </Box>
      </Box>
      {payload.message ? (
        <Typography variant="body2" className="cview-service-message">
          {payload.message}
        </Typography>
      ) : null}
      <Box className="cview-deploy-files-head">
        <Typography component="span" variant="caption">Managed services</Typography>
        <Typography component="span" variant="caption">{payload.serviceCount ?? rows.length}</Typography>
      </Box>
      {rows.length === 0 ? (
        <Typography variant="body2" className="cview-service-empty">
          No matching managed app or service is registered.
        </Typography>
      ) : (
        <Box className="cview-service-list" role="list">
          {rows.slice(0, 8).map((service, idx) => {
            const label = service.title || service.id || "Managed service";
            return (
              <Box className="cview-service-row" role="listitem" key={`${service.id || label}-${idx}`}>
                <Box className="cview-service-row-main">
                  <Typography component="span" variant="body2" title={label}>
                    {label}
                  </Typography>
                  <span className="cview-service-row-meta">
                    {[service.id, service.type, service.status].filter(Boolean).join(" · ")}
                  </span>
                </Box>
                <Box className="cview-deploy-actions">
                  <Tooltip title="Open in new tab">
                    <span>
                      <IconButton
                        size="small"
                        disabled={!service.url}
                        onClick={() => service.url && window.open(service.url, "_blank", "noopener,noreferrer")}
                        aria-label="Open managed service"
                      >
                        <OpenInNewRounded fontSize="small" />
                      </IconButton>
                    </span>
                  </Tooltip>
                  <Tooltip title={copiedUrl === service.url ? "Copied" : "Copy URL"}>
                    <span>
                      <IconButton
                        size="small"
                        disabled={!service.url}
                        onClick={() => copyUrl(service.url)}
                        aria-label="Copy managed service URL"
                      >
                        <ContentCopyRounded fontSize="small" />
                      </IconButton>
                    </span>
                  </Tooltip>
                </Box>
              </Box>
            );
          })}
        </Box>
      )}
    </Box>
  );
}

export function AppDeployView({
  card,
  onOpenFile,
  workspaceFiles = [],
}: AppDeployViewProps) {
  const parsed = useMemo(() => parsePayload(card), [card]);
  const payload = parsed.deploy;
  const files = useMemo(
    () => mergeDeployFiles(payload.files, workspaceFiles),
    [payload.files, workspaceFiles],
  );
  const [copied, setCopied] = useState(false);

  const title = card.label || payload.appId || "App deploy";
  const handleCopy = () => {
    if (!payload.url) return;
    void navigator.clipboard.writeText(payload.url).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  if (parsed.serviceManage) {
    return <ServiceManageView card={card} payload={parsed.serviceManage} />;
  }

  return (
    <Box className="cview cview-deploy">
      <Box className="cview-deploy-head">
        <Box className="cview-deploy-title">
          <Typography component="span" variant="subtitle2">{title}</Typography>
          <Box component="span" className="cview-deploy-meta">{card.kind}</Box>
        </Box>
        <Box className="cview-deploy-actions">
          <Tooltip title="Open in new tab">
            <span>
              <IconButton
                size="small"
                disabled={!payload.url}
                onClick={() => payload.url && window.open(payload.url, "_blank", "noopener,noreferrer")}
                aria-label="Open deployed app"
              >
                <OpenInNewRounded fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
          <Tooltip title={copied ? "Copied" : "Copy URL"}>
            <span>
              <IconButton
                size="small"
                disabled={!payload.url}
                onClick={handleCopy}
                aria-label="Copy deploy URL"
              >
                <ContentCopyRounded fontSize="small" />
              </IconButton>
            </span>
          </Tooltip>
        </Box>
      </Box>

      {payload.url ? (
        <a
          className="cview-deploy-url"
          href={payload.url}
          target="_blank"
          rel="noopener noreferrer"
        >
          {payload.url}
        </a>
      ) : (
        <Typography variant="body2" className="cview-deploy-url">
          Deploy URL not yet available.
        </Typography>
      )}

      <Box className="cview-deploy-files-head">
        <Typography component="span" variant="caption">Files</Typography>
        <Typography component="span" variant="caption">{files.length}</Typography>
      </Box>
      {files.length === 0 ? (
        <Typography variant="body2">No files captured for this deploy.</Typography>
      ) : (
        <Box className="cview-deploy-files" role="list">
          {files.map((file, idx) => {
            const size = formatKb(file.bytes);
            const open = () => onOpenFile?.(file.path);
            return (
              <Box
                key={`${file.path}-${idx}`}
                className="cview-deploy-file"
                role="listitem"
                onClick={open}
              >
                <InsertDriveFileRounded
                  fontSize="small"
                  className="cview-deploy-file-icon"
                />
                <span className="cview-deploy-file-path" title={file.path}>
                  {file.path}
                </span>
                {size ? (
                  <span className="cview-deploy-file-size">{size}</span>
                ) : null}
                <IconButton
                  size="small"
                  className="cview-deploy-file-open"
                  aria-label={`Open ${file.path}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    open();
                  }}
                >
                  <ChevronRightRounded fontSize="small" />
                </IconButton>
              </Box>
            );
          })}
        </Box>
      )}
    </Box>
  );
}

export default AppDeployView;
