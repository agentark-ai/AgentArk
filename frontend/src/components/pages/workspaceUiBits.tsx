import MoreVertIcon from "@mui/icons-material/MoreVert";
import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Menu,
  MenuItem,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import {
  formatUiDateOnly,
  formatUiDateTimeMeta,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import { asRecord, errMessage, num, pickRecords, str, type JsonRecord } from "./pageHelpers";

export function humanTs(raw: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(raw, { fallback: "-" });
}

export function formatBytes(value: unknown): string {
  const size = num(value, -1);
  if (size < 0) return "-";
  if (size < 1024) return `${Math.round(size)} B`;
  const kb = size / 1024;
  if (kb < 1024) return `${kb.toFixed(kb >= 100 ? 0 : 1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(mb >= 100 ? 0 : 1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(gb >= 100 ? 0 : 1)} GB`;
}

function looksLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  return trimmed.startsWith("http://") || trimmed.startsWith("https://");
}

function looksLikeUuid(value: string): boolean {
  const trimmed = value.trim();
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    trimmed,
  );
}

function looksLikeIsoTimestamp(value: string): boolean {
  const trimmed = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}T/.test(trimmed)) return false;
  const date = new Date(trimmed);
  return !Number.isNaN(date.getTime());
}

function looksLikeIsoDateOnly(value: string): boolean {
  const trimmed = value.trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) return false;
  const date = new Date(`${trimmed}T00:00:00`);
  return !Number.isNaN(date.getTime());
}

function formatTimestampForHumans(value: string): {
  label: string;
  tooltip: string;
} {
  const meta = formatUiDateTimeMeta(value, { fallback: value || "-" });
  return { label: meta.label, tooltip: meta.tip };
}

function boolLabelForKey(
  key: string,
  value: boolean,
): { label: string; color: "success" | "warning" | "default" } {
  const normalized = key.trim().toLowerCase();
  if (normalized.includes("enabled")) {
    return {
      label: value ? "Enabled" : "Disabled",
      color: value ? "success" : "warning",
    };
  }
  if (normalized.includes("active")) {
    return {
      label: value ? "Active" : "Inactive",
      color: value ? "success" : "warning",
    };
  }
  if (normalized.includes("connected")) {
    return {
      label: value ? "Connected" : "Not connected",
      color: value ? "success" : "warning",
    };
  }
  return { label: value ? "Yes" : "No", color: value ? "success" : "default" };
}

function formatCompactValue(value: unknown): { text: string; tooltip?: string } {
  if (value == null) return { text: "-" };
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (looksLikeIsoTimestamp(trimmed)) {
      const meta = formatUiDateTimeMeta(trimmed, { fallback: "-" });
      return { text: meta.label, tooltip: meta.tip };
    }
    if (looksLikeIsoDateOnly(trimmed)) {
      const text = formatUiDateOnly(trimmed, { fallback: "-" });
      const tooltip = formatUiDateOnly(trimmed, {
        fallback: "-",
        includeYear: true,
      });
      return { text, tooltip };
    }
    return { text: value };
  }
  if (typeof value === "number") {
    return { text: Number.isFinite(value) ? String(value) : "-" };
  }
  if (typeof value === "boolean") {
    return { text: value ? "true" : "false" };
  }
  if (Array.isArray(value)) {
    const items = value
      .slice(0, 5)
      .map((entry) =>
        typeof entry === "string"
          ? entry
          : typeof entry === "number"
            ? String(entry)
            : typeof entry === "boolean"
              ? entry
                ? "true"
                : "false"
              : "...",
      )
      .join(", ");
    const suffix = value.length > 5 ? ` +${value.length - 5} more` : "";
    return {
      text: items ? `${items}${suffix}` : `${value.length} items`,
      tooltip: items || undefined,
    };
  }
  if (typeof value === "object") {
    const record = asRecord(value);
    const title =
      str(record.title, "") ||
      str(record.name, "") ||
      str(record.label, "") ||
      str(record.description, "");
    const id = str(record.id, "");
    if (title) return { text: title, tooltip: id ? `ID: ${id}` : undefined };
    const scalars = Object.entries(record)
      .filter(
        ([, entry]) =>
          typeof entry === "string" ||
          typeof entry === "number" ||
          typeof entry === "boolean",
      )
      .slice(0, 4)
      .map(([recordKey, entry]) => {
        const text =
          typeof entry === "string" && entry.length > 30
            ? `${entry.slice(0, 30)}...`
            : String(entry);
        return `${recordKey}: ${text}`;
      });
    if (scalars.length > 0) {
      const keys = Object.keys(record);
      const more =
        keys.length > scalars.length
          ? ` (+${keys.length - scalars.length} fields)`
          : "";
      return {
        text: scalars.join(", ") + more,
        tooltip: `Fields: ${keys.join(", ")}`,
      };
    }
    const keys = Object.keys(record);
    return {
      text: keys.length ? `${keys.length} fields` : "-",
      tooltip: keys.length ? `Fields: ${keys.join(", ")}` : undefined,
    };
  }
  return { text: String(value) };
}

export type RowMenuAction = {
  label: string;
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  tone?: "default" | "warning" | "error";
  divider?: boolean;
};

export function RowOpsMenu({
  actions,
  ariaLabel = "Row actions",
}: {
  actions: RowMenuAction[];
  ariaLabel?: string;
}) {
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null);
  const open = Boolean(anchorEl);
  const closeMenu = () => setAnchorEl(null);
  return (
    <>
      <IconButton
        size="small"
        aria-label={ariaLabel}
        onClick={(event) => setAnchorEl(event.currentTarget)}
      >
        <MoreVertIcon fontSize="small" />
      </IconButton>
      <Menu anchorEl={anchorEl} open={open} onClose={closeMenu}>
        {actions.map((action, index) => (
          <MenuItem
            key={`${action.label}-${index}`}
            divider={action.divider}
            disabled={action.disabled}
            onClick={() => {
              closeMenu();
              if (action.disabled) return;
              void action.onClick();
            }}
            sx={
              action.tone === "error"
                ? { color: "error.main" }
                : action.tone === "warning"
                  ? { color: "warning.main" }
                  : undefined
            }
          >
            {action.label}
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

export function KeyValuePanel({
  title,
  data,
  emptyLabel,
  maxRows,
}: {
  title: string;
  data: JsonRecord;
  emptyLabel?: string;
  maxRows?: number;
}) {
  const entries = Object.entries(data || {});
  const shown = entries.slice(0, maxRows ?? 14);
  return (
    <Box
      sx={{
        borderRadius: "8px",
        border: "1px solid var(--ui-rgba-255-255-255-080)",
        background: "var(--ui-rgba-255-255-255-025)",
        p: 1.25,
      }}
    >
      <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
        {title}
      </Typography>
      <Stack spacing={0} sx={{ mt: 0.9 }}>
        {shown.length === 0 ? (
          <Typography variant="body2" sx={{ color: "text.secondary" }}>
            {emptyLabel || "No details available."}
          </Typography>
        ) : (
          shown.map(([key, value], index) => {
            const compactValue = formatCompactValue(value);
            const keyLower = key.toLowerCase();
            const renderValue = () => {
              if (typeof value === "string" && looksLikeUrl(value)) {
                const trimmed = value.trim();
                const label =
                  trimmed.length > 54 ? `${trimmed.slice(0, 54)}...` : trimmed;
                return (
                  <Typography
                    variant="body2"
                    sx={{ wordBreak: "break-all" }}
                    title={trimmed}
                  >
                    <a
                      href={trimmed}
                      target="_blank"
                      rel="noreferrer"
                      style={{ color: "inherit", textDecoration: "underline" }}
                    >
                      {label}
                    </a>
                  </Typography>
                );
              }
              if (
                typeof value === "string" &&
                (looksLikeIsoTimestamp(value) ||
                  looksLikeIsoDateOnly(value) ||
                  keyLower.endsWith("_at") ||
                  keyLower.endsWith("_date") ||
                  keyLower.includes("timestamp"))
              ) {
                const timestamp =
                  looksLikeIsoDateOnly(value) || keyLower.endsWith("_date")
                    ? {
                        label: formatUiDateOnly(value, { fallback: "-" }),
                        tooltip: formatUiDateOnly(value, {
                          fallback: "-",
                          includeYear: true,
                        }),
                      }
                    : formatTimestampForHumans(value);
                return (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={timestamp.label}
                    title={timestamp.tooltip}
                  />
                );
              }
              if (typeof value === "boolean") {
                const boolLabel = boolLabelForKey(key, value);
                return (
                  <Chip
                    size="small"
                    label={boolLabel.label}
                    color={boolLabel.color}
                    variant={value ? "filled" : "outlined"}
                  />
                );
              }
              if (typeof value === "number" && Number.isFinite(value)) {
                if (keyLower.includes("ms") || keyLower.includes("duration")) {
                  return (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={`${Math.round(value)} ms`}
                    />
                  );
                }
                if (
                  keyLower.includes("count") ||
                  keyLower.includes("total") ||
                  keyLower.includes("remaining")
                ) {
                  return (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={String(value)}
                    />
                  );
                }
              }
              if (
                typeof value === "string" &&
                (looksLikeUuid(value) ||
                  keyLower.endsWith("_id") ||
                  keyLower === "id")
              ) {
                const trimmed = value.trim();
                const label =
                  trimmed.length > 22
                    ? `${trimmed.slice(0, 8)}...${trimmed.slice(-6)}`
                    : trimmed;
                return (
                  <Chip
                    size="small"
                    variant="outlined"
                    label={label}
                    title={trimmed}
                    onClick={async () => {
                      try {
                        await navigator.clipboard.writeText(trimmed);
                      } catch {
                        // Ignore clipboard failures.
                      }
                    }}
                    sx={{ cursor: "pointer" }}
                  />
                );
              }
              return (
                <Typography
                  variant="body2"
                  sx={{
                    minWidth: 0,
                    flex: "1 1 auto",
                    wordBreak: "break-word",
                  }}
                  title={compactValue.tooltip || ""}
                >
                  {compactValue.text}
                </Typography>
              );
            };
            return (
              <Box
                key={key}
                sx={{
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    md: "160px minmax(0, 1fr)",
                  },
                  gap: { xs: 0.35, md: 1.1 },
                  py: 0.9,
                  borderTop:
                    index === 0 ? "none" : "1px solid var(--ui-rgba-255-255-255-060)",
                }}
              >
                <Typography
                  variant="caption"
                  sx={{
                    color: "var(--ui-rgba-188-198-212-680)",
                    minWidth: 0,
                  }}
                >
                  {key}
                </Typography>
                {renderValue()}
              </Box>
            );
          })
        )}
        {entries.length > shown.length ? (
          <Typography variant="caption" sx={{ color: "text.secondary", pt: 0.9 }}>
            {entries.length - shown.length} more field(s) not shown.
          </Typography>
        ) : null}
      </Stack>
    </Box>
  );
}

export function DataTable({
  rows,
  columns,
}: {
  rows: JsonRecord[];
  columns: string[];
}) {
  return (
    <TableContainer className="table-shell">
      <Table size="small">
        <TableHead>
          <TableRow>
            {columns.map((column) => (
              <TableCell key={column}>{column}</TableCell>
            ))}
          </TableRow>
        </TableHead>
        <TableBody>
          {rows.map((row, rowIndex) => (
            <TableRow key={`row-${rowIndex}`}>
              {columns.map((column) => (
                <TableCell key={`${rowIndex}-${column}`}>
                  {str(row[column], "-")}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </TableContainer>
  );
}

export function QueryTable({
  title,
  path,
  arrayKey,
  columns,
  autoRefresh,
  emptyLabel,
  queryKey,
  pageSize,
}: {
  title: string;
  path: string;
  arrayKey: string;
  columns: string[];
  autoRefresh: boolean;
  emptyLabel: string;
  queryKey: string;
  pageSize?: number;
}) {
  const [page, setPage] = useState(0);
  const offset = pageSize ? page * pageSize : 0;
  const queryPath = useMemo(() => {
    if (!pageSize) return path;
    const [pathname, rawSearch = ""] = path.split("?");
    const params = new URLSearchParams(rawSearch);
    params.set("limit", String(pageSize));
    params.set("offset", String(offset));
    const search = params.toString();
    return search ? `${pathname}?${search}` : pathname;
  }, [offset, pageSize, path]);
  const query = useQuery({
    queryKey: [queryKey, queryPath],
    queryFn: () => api.rawGet(queryPath),
    refetchInterval: autoRefresh ? 8000 : false,
  });

  const payload = asRecord(query.data);
  const rows = pickRecords(payload, arrayKey);
  const totalRows = pageSize
    ? Math.max(0, num(payload.total, rows.length))
    : rows.length;
  const effectiveLimit = pageSize
    ? Math.max(1, num(payload.limit, pageSize))
    : Math.max(1, rows.length || 1);
  const pageCount = pageSize
    ? Math.max(1, Math.ceil(totalRows / effectiveLimit))
    : 1;
  const pageLabel = `${Math.min(page + 1, pageCount)}/${pageCount}`;

  useEffect(() => {
    if (!pageSize) return;
    const maxPage = Math.max(0, pageCount - 1);
    if (page > maxPage) {
      setPage(maxPage);
    }
  }, [page, pageCount, pageSize]);

  return (
    <Box className="list-shell">
      <Typography variant="h6" sx={{ mb: 1 }}>
        {title}
      </Typography>
      {query.error ? (
        <Alert severity="error">{errMessage(query.error)}</Alert>
      ) : rows.length === 0 ? (
        <Typography variant="body2" sx={{ color: "text.secondary" }}>
          {emptyLabel}
        </Typography>
      ) : (
        <>
          <DataTable rows={rows} columns={columns} />
          {pageSize ? (
            <Stack
              direction="row"
              spacing={0.75}
              sx={{
                alignItems: "center",
                justifyContent: "space-between",
                mt: 1,
              }}
            >
              <Typography variant="caption" className="conversation-pagination-copy">
                {totalRows} item{totalRows === 1 ? "" : "s"}
              </Typography>
              <Stack direction="row" spacing={0.75} sx={{ alignItems: "center" }}>
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => setPage((previous) => Math.max(0, previous - 1))}
                  disabled={page <= 0}
                >
                  Prev
                </Button>
                <Typography
                  variant="caption"
                  className="conversation-page-indicator"
                >
                  {pageLabel}
                </Typography>
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() =>
                    setPage((previous) =>
                      Math.min(Math.max(0, pageCount - 1), previous + 1),
                    )
                  }
                  disabled={page >= pageCount - 1}
                >
                  Next
                </Button>
              </Stack>
            </Stack>
          ) : null}
        </>
      )}
    </Box>
  );
}
