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
  Switch,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type MouseEvent, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api/client";
import { humanizeMachineLabel, humanizeStatusLabel } from "../../lib/displayLabels";
import type { SkillImportResponse, SkillTestResponse } from "../../types";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  toBool,
  type JsonRecord,
} from "./pageHelpers";
import {
  asRecords,
  boolText,
  DEVELOPER_MODE_EVENT,
  dedupeStrings,
  getDeveloperModeEnabled,
  REFRESH_MS,
} from "./workspaceCore";
import {
  buildSkillMdFromForm,
  computeImportRiskSummary,
  defaultSkillEditorForm,
  explainImportFinding,
  extractActionMdFromModelOutput,
  extractFirstUrl,
  importFindingCategoryLabel,
  IMPORT_RISK_POLICY,
  IMPORT_SECURITY_FORCE_RISK_THRESHOLD,
  inferHookTriggerFromInstruction,
  inferTaskCronFromInstruction,
  isHookAttachedToAction,
  isHookRecordAttachedToAction,
  isRunOnceInstruction,
  isValidActionName,
  normalizeActionName,
  parseSkillEditorForm,
  sanitizeHookName,
  type HookTriggerValue,
  type SkillEditorForm,
  type SkillImportSummary,
} from "./skillsPageHelpers";
import { DataTable, RowOpsMenu } from "./workspaceUiBits";

const WORKSPACE_HEADER_ACTION_GROUP_SX = {
  p: 0.45,
  borderRadius: "8px",
  border: "1px solid var(--surface-border)",
  background: "var(--ui-rgba-255-255-255-020)",
  boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)",
} as const;
const WORKSPACE_HEADER_PRIMARY_BUTTON_SX = {
  minHeight: 32,
  px: 1.5,
  borderRadius: "8px",
  fontWeight: 700,
  textTransform: "none",
  boxShadow: "none",
} as const;
type ImportCallback = (summary: SkillImportSummary) => Promise<void> | void;

function skillImportNeedsAttention(
  result: SkillImportResponse | null | undefined,
): boolean {
  if (!result) return false;
  if (result.status === "blocked") return true;
  const missing = result.secrets?.missing_env || [];
  return result.status === "needs_secrets" || missing.length > 0;
}

function dedupeSkillRecords(records: JsonRecord[]): JsonRecord[] {
  const byNameAndSource = new Map<string, JsonRecord>();
  const unnamed: JsonRecord[] = [];
  for (const record of records) {
    const name = str(record.name, "").trim().toLowerCase();
    if (!name) {
      unnamed.push(record);
      continue;
    }
    const source = str(record.source, "").trim().toLowerCase();
    const key = `${source}:${name}`;
    const existing = byNameAndSource.get(key);
    if (!existing) {
      byNameAndSource.set(key, record);
      continue;
    }
    const existingImportedAt = Date.parse(str(existing.imported_at, ""));
    const importedAt = Date.parse(str(record.imported_at, ""));
    const existingTime = Number.isFinite(existingImportedAt)
      ? existingImportedAt
      : 0;
    const nextTime = Number.isFinite(importedAt) ? importedAt : 0;
    if (nextTime > existingTime) byNameAndSource.set(key, record);
  }
  return [...byNameAndSource.values(), ...unnamed];
}

function stringsFromArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => (typeof entry === "string" ? entry.trim() : ""))
    .filter(Boolean);
}

function skillRequiredInputNames(
  skill: JsonRecord,
  override?: string[],
): string[] {
  const required =
    override && override.length > 0
      ? override
      : stringsFromArray(asRecord(skill.input_schema).required);
  return dedupeStrings(required.map((field) => field.trim()).filter(Boolean));
}

function skillInputProperty(skill: JsonRecord, field: string): JsonRecord {
  const schema = asRecord(skill.input_schema);
  const properties = asRecord(schema.properties);
  return asRecord(properties[field]);
}

function skillInputLabel(field: string): string {
  const words = field
    .split(/[_\-\s]+/)
    .map((part) => part.trim())
    .filter(Boolean);
  if (words.length === 0) return "Input";
  return words
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function skillInputDescription(skill: JsonRecord, field: string): string {
  return str(
    skillInputProperty(skill, field).description,
    `Value for ${skillInputLabel(field)}`,
  );
}

function skillTestOutputText(out: SkillTestResponse): string {
  if (typeof out.output === "string" && out.output.trim()) {
    return out.output.trim();
  }
  if (out.output != null) {
    try {
      return JSON.stringify(out.output, null, 2);
    } catch {
      return String(out.output);
    }
  }
  return str(out.message, "").trim();
}

type SkillTestRunPhase =
  | "checking"
  | "waiting_input"
  | "running"
  | "completed"
  | "error"
  | "cancelled";

type SkillTestRunDialog = {
  name: string;
  skill: JsonRecord;
  phase: SkillTestRunPhase;
  message: string;
  output: string;
  inputFields: string[];
  inputValues: Record<string, string>;
  inputError: string | null;
};

type SkillTestResultTone = "info" | "error";

function isSkillTestRunActive(phase: SkillTestRunPhase | undefined): boolean {
  return phase === "checking" || phase === "running";
}

function isAbortError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const maybeNamed = error as { name?: unknown };
  return maybeNamed.name === "AbortError";
}

function initialSkillTestInputValues(
  fields: string[],
  previous: Record<string, string> = {},
): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of fields) {
    values[field] = previous[field] || "";
  }
  return values;
}

function createSkillTestRunId(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return cryptoApi.randomUUID();
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function skillTestCompletionMessage(out: SkillTestResponse): string {
  if (out.status === "ok") {
    return out.mode === "workflow"
      ? "Workflow test completed."
      : "Skill test completed.";
  }
  return out.message || out.error || "Test returned.";
}

type BulkImportItem = {
  url: string;
  selected: boolean;
  analyzed: boolean;
  status?: string;
  discovered: BulkImportDiscoveredSkill[];
};

type BulkImportDiscoveredSkill = {
  key: string;
  parentUrl: string;
  url: string;
  name: string;
  selected: boolean;
  status?: string;
  preview?: SkillImportResponse;
  importResult?: SkillImportResponse;
  error?: string;
};

type SkillMarketplaceForm = {
  id: string;
  name: string;
  url: string;
  enabled: boolean;
};

type SkillMarketplaceInstallerRow = JsonRecord & {
  marketplace_id: string;
  marketplace_name: string;
  marketplace_enabled: boolean;
  _key: string;
};

const EMPTY_SKILL_MARKETPLACE_FORM: SkillMarketplaceForm = {
  id: "",
  name: "",
  url: "",
  enabled: true,
};

function BulkImportDialog({
  open,
  onClose,
  onImported,
  onAfterImport,
  initialUrls = [],
  sourceLabel,
}: {
  open: boolean;
  onClose: () => void;
  onImported?: ImportCallback;
  onAfterImport?: (
    name: string,
    importResult: SkillImportResponse,
  ) => Promise<void>;
  initialUrls?: string[];
  sourceLabel?: string;
}) {
  const [urlsText, setUrlsText] = useState("");
  const [items, setItems] = useState<BulkImportItem[]>([]);
  const [analyzing, setAnalyzing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [force, setForce] = useState(false);
  const [model, setModel] = useState("");
  const [analysisDone, setAnalysisDone] = useState(false);

  const parseUrlsFromText = (text: string): string[] => {
    const urls = text
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l.length > 0 && !l.startsWith("#"))
      .map((u) => {
        // Auto-fix common GitHub mistake: /blob/ -> /tree/ for folder URLs
        if (
          u.includes("github.com/") &&
          u.includes("/blob/") &&
          !u.match(/\.\w+$/)
        ) {
          return u.replace("/blob/", "/tree/");
        }
        return u;
      });
    const uniq: string[] = [];
    for (const u of urls) {
      if (!uniq.includes(u)) uniq.push(u);
    }
    return uniq;
  };

  const buildItemsFromUrls = (urls: string[]): BulkImportItem[] =>
    urls.map((url) => ({
      url,
      selected: true,
      analyzed: false,
      discovered: [],
    }));

  const requiresForceForResult = (
    result: SkillImportResponse | undefined,
  ): boolean => {
    if (!result) return false;
    const risk = computeImportRiskSummary(result.security);
    const blocked =
      toBool(result.security?.blocked) || result.status === "blocked";
    return blocked || risk.score10 >= IMPORT_SECURITY_FORCE_RISK_THRESHOLD;
  };

  const normalizeDiscoveredSkills = (
    sourceUrl: string,
    previewResult: SkillImportResponse,
  ): BulkImportDiscoveredSkill[] => {
    const importedChildren = Array.isArray(previewResult.imported)
      ? previewResult.imported
      : [];
    if (importedChildren.length > 0) {
      return importedChildren.map((entry, idx) => {
        const childResult = entry?.result;
        const childUrl = str(entry?.url, sourceUrl);
        const childName = str(childResult?.name, `skill-${idx + 1}`);
        const needsForce = requiresForceForResult(childResult);
        return {
          key: `${sourceUrl}::${childUrl}::${idx}`,
          parentUrl: sourceUrl,
          url: childUrl,
          name: childName,
          selected: force || !needsForce,
          status:
            str(childResult?.message, "") ||
            (childResult?.status === "blocked"
              ? "Blocked by security verification"
              : "Preview ready"),
          preview: childResult,
        };
      });
    }

    const singleNeedsForce = requiresForceForResult(previewResult);
    return [
      {
        key: `${sourceUrl}::single`,
        parentUrl: sourceUrl,
        url: sourceUrl,
        name: str(previewResult.name, "imported-skill"),
        selected: force || !singleNeedsForce,
        status:
          str(previewResult.message, "") ||
          (previewResult.status === "blocked"
            ? "Blocked by security verification"
            : "Preview ready"),
        preview: previewResult,
      },
    ];
  };

  const buildItemsFromText = (text = urlsText): BulkImportItem[] =>
    buildItemsFromUrls(parseUrlsFromText(text));

  const selectedDiscoveredSkills: BulkImportDiscoveredSkill[] = items.flatMap(
    (item) =>
      item.selected ? item.discovered.filter((skill) => skill.selected) : [],
  );
  const selectedSkillCount = selectedDiscoveredSkills.length;
  const riskySelectedCount = selectedDiscoveredSkills.filter((skill) =>
    requiresForceForResult(skill.preview),
  ).length;

  const initialUrlsKey = useMemo(
    () => initialUrls.map((url) => url.trim()).filter(Boolean).join("\n"),
    [initialUrls],
  );

  useEffect(() => {
    if (!open) {
      setError(null);
      setAnalyzing(false);
      setImporting(false);
      setAnalysisDone(false);
      return;
    }
    setUrlsText(initialUrlsKey);
    setItems(initialUrlsKey ? buildItemsFromText(initialUrlsKey) : []);
    setAnalyzing(false);
    setImporting(false);
    setAnalysisDone(false);
    setError(null);
    setForce(false);
    setModel("");
  }, [open, initialUrlsKey]);

  const updateDiscoveredSkill = (
    parentUrl: string,
    skillKey: string,
    patch: Partial<BulkImportDiscoveredSkill>,
  ) => {
    setItems((prev) =>
      prev.map((item) => {
        if (item.url !== parentUrl) return item;
        return {
          ...item,
          discovered: item.discovered.map((skill) =>
            skill.key === skillKey ? { ...skill, ...patch } : skill,
          ),
        };
      }),
    );
  };

  const handleAnalyzeSelected = async () => {
    setError(null);
    setAnalysisDone(false);
    const effectiveItems = items.length > 0 ? items : buildItemsFromText();
    if (!effectiveItems.length) {
      setError("Paste at least one import URL before analyzing.");
      return;
    }
    const toAnalyze = effectiveItems.filter((item) => item.selected);
    if (!toAnalyze.length) {
      setError("Select at least one URL to analyze.");
      return;
    }
    if (items.length === 0) setItems(effectiveItems);

    setAnalyzing(true);
    for (const item of toAnalyze) {
      setItems((prev) =>
        prev.map((x) =>
          x.url === item.url
            ? { ...x, status: "Analyzing security...", analyzed: false }
            : x,
        ),
      );
      try {
        const preview = await api.importSkill({
          url: item.url,
          force,
          model: model.trim() || undefined,
          preview_only: true,
        });
        const discovered = normalizeDiscoveredSkills(item.url, preview);
        setItems((prev) =>
          prev.map((x) =>
            x.url === item.url
              ? {
                  ...x,
                  analyzed: true,
                  status:
                    preview.message ||
                    `Previewed ${discovered.length} skill${discovered.length === 1 ? "" : "s"}.`,
                  discovered,
                }
              : x,
          ),
        );
      } catch (err) {
        const message = `Error: ${errMessage(err)}`;
        setItems((prev) =>
          prev.map((x) =>
            x.url === item.url
              ? {
                  ...x,
                  analyzed: true,
                  status: message,
                  discovered: [],
                }
              : x,
          ),
        );
      }
    }
    setAnalyzing(false);
    setAnalysisDone(true);
  };

  const handleImportSelected = async () => {
    setError(null);
    if (!analysisDone) {
      setError(
        "Analyze selected URLs first so you can review security and choose which skills to import.",
      );
      return;
    }
    if (!selectedSkillCount) {
      setError("Select at least one skill to import.");
      return;
    }
    if (!force && riskySelectedCount > 0) {
      setError(
        `Selected set includes ${riskySelectedCount} risky skill(s). Enable override or deselect them before importing.`,
      );
      return;
    }

    const selectedByParent = new Map<string, BulkImportDiscoveredSkill[]>();
    for (const skill of selectedDiscoveredSkills) {
      const bucket = selectedByParent.get(skill.parentUrl) || [];
      bucket.push(skill);
      selectedByParent.set(skill.parentUrl, bucket);
    }

    setImporting(true);
    for (const [parentUrl, selectedSkills] of selectedByParent.entries()) {
      for (const skill of selectedSkills) {
        updateDiscoveredSkill(skill.parentUrl, skill.key, {
          status: "Importing...",
          error: undefined,
        });
      }
      try {
        const result = await api.importSkill({
          url: parentUrl,
          force,
          model: model.trim() || undefined,
          selected_urls: selectedSkills.map((skill) => skill.url),
        });

        const importedEntries = Array.isArray(result.imported)
          ? result.imported
          : [];
        const importedByUrl = new Map<string, SkillImportResponse>();
        for (const entry of importedEntries) {
          const childResult = entry?.result;
          const childUrl = str(entry?.url, "").trim();
          if (!childResult || !childUrl) continue;
          importedByUrl.set(childUrl, childResult);
        }

        if (importedEntries.length > 0) {
          for (const skill of selectedSkills) {
            const childResult = importedByUrl.get(skill.url);
            if (!childResult) {
              updateDiscoveredSkill(skill.parentUrl, skill.key, {
                status:
                  "Error: selected skill was not returned by bulk import response.",
              });
              continue;
            }
            let childMessage =
              childResult.message || `Imported ${childResult.name}`;
            if (childResult.status === "blocked") {
              childMessage =
                childResult.message ||
                "Blocked by security verification (enable override and retry).";
            } else if (childResult.status === "needs_secrets") {
              childMessage =
                childResult.message ||
                `Imported ${childResult.name} (disabled until secrets are configured)`;
            }

            updateDiscoveredSkill(skill.parentUrl, skill.key, {
              status: childMessage,
              importResult: childResult,
            });
            await onAfterImport?.(childResult.name, childResult);
            await onImported?.({ result: childResult, message: childMessage });
          }
        } else {
          let statusMessage = result.message || `Imported ${result.name}`;
          if (result.status === "blocked") {
            statusMessage =
              result.message ||
              "Blocked by security verification (enable override and retry).";
          } else if (result.status === "needs_secrets") {
            statusMessage =
              result.message ||
              `Imported ${result.name} (disabled until secrets are configured)`;
          }
          for (const skill of selectedSkills) {
            updateDiscoveredSkill(skill.parentUrl, skill.key, {
              status: statusMessage,
              importResult: result,
            });
          }
          await onAfterImport?.(result.name, result);
          await onImported?.({ result, message: statusMessage });
        }
      } catch (err) {
        const message = `Error: ${errMessage(err)}`;
        for (const skill of selectedSkills) {
          updateDiscoveredSkill(skill.parentUrl, skill.key, {
            status: message,
            error: message,
          });
        }
      }
    }
    setImporting(false);
    onClose();
  };

  const titleCaseWords = (value: string): string =>
    value
      .split(/[\s_-]+/)
      .filter(Boolean)
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");

  const compactStatusToken = (value: string): string =>
    value.toLowerCase().replace(/[^a-z0-9]+/g, "");

  const buildBulkSkillStatus = (
    skill: BulkImportDiscoveredSkill,
  ): {
    label: string;
    color: "default" | "success" | "warning" | "error" | "info";
    detail: string;
  } => {
    const rawStatus = str(skill.status, "").trim();
    const rawLower = rawStatus.toLowerCase();
    const result = skill.importResult || skill.preview;
    const blocked =
      toBool(result?.security?.blocked) || result?.status === "blocked";

    if (rawLower === "importing...") {
      return { label: "Importing", color: "info", detail: "" };
    }
    if (skill.error || rawLower.startsWith("error")) {
      return {
        label: "Import failed",
        color: "error",
        detail: rawStatus.replace(/^error:\s*/i, "").trim() || "Import failed.",
      };
    }
    if (blocked) {
      return {
        label: "Blocked",
        color: "error",
        detail:
          rawStatus ||
          str(result?.message, "Blocked by security verification."),
      };
    }
    if (result?.status === "needs_secrets") {
      return {
        label: "Needs secrets",
        color: "warning",
        detail:
          rawStatus ||
          str(
            result?.message,
            "Imported template is disabled until required secrets are configured.",
          ),
      };
    }
    if (skill.importResult && result?.status === "ok") {
      return {
        label: "Imported",
        color: "success",
        detail:
          rawStatus ||
          str(result?.message, `Imported ${str(result?.name, "skill")}.`),
      };
    }
    if (skill.preview) {
      return {
        label: "Preview ready",
        color: "info",
        detail:
          rawStatus ||
          str(
            result?.message,
            "Preview completed. Select and import when ready.",
          ),
      };
    }
    return {
      label: "Pending",
      color: "default",
      detail: rawStatus || "Waiting for analysis.",
    };
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Bulk Import</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1.25}>
          {error ? <Alert severity="error">{error}</Alert> : null}
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
            }}
          >
            {sourceLabel
              ? `Reviewing installer URLs from ${sourceLabel}. Run Analyze to review discovered skills and security before any import.`
              : "Paste one or more skill URLs (one per line). Then run Analyze to review discovered skills and security before any import."}
          </Typography>
          <Alert
            severity="info"
            variant="outlined"
            sx={{ py: 0.25, "& .MuiAlert-message": { fontSize: "0.75rem" } }}
          >
            Getting 403 errors? GitHub rate-limits unauthenticated requests. Go
            to Settings &gt; Integrations &gt; GitHub and add a Personal Access
            Token for higher limits.
          </Alert>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              whiteSpace: "pre-line",
            }}
          >
            {`Examples:
https://github.com/org/repo/tree/main/skills
https://raw.githubusercontent.com/org/repo/main/skills/my-skill/SKILL.md
https://raw.githubusercontent.com/org/repo/main/skills/another-skill/SKILL.md`}
          </Typography>
          <TextField
            fullWidth
            multiline
            minRows={3}
            maxRows={8}
            label="Import URLs"
            value={urlsText}
            onChange={(e) => {
              setUrlsText(e.target.value);
              setItems([]);
              setAnalysisDone(false);
              setError(null);
            }}
            placeholder={
              "https://github.com/org/repo/tree/main/skills\nhttps://raw.githubusercontent.com/org/repo/main/skills/my-skill/SKILL.md"
            }
          />
          <TextField
            fullWidth
            size="small"
            label="Model override (optional)"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
          <FormControlLabel
            control={
              <Switch
                checked={force}
                onChange={(e) => setForce(e.target.checked)}
              />
            }
            label="Override warnings (import anyway)"
          />
          {analysisDone ? (
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Selected for import: {selectedSkillCount} skill
              {selectedSkillCount === 1 ? "" : "s"}.
            </Typography>
          ) : null}
          {!force && riskySelectedCount > 0 ? (
            <Alert severity="warning">
              {riskySelectedCount} selected skill
              {riskySelectedCount === 1 ? "" : "s"} exceed the risk threshold (
              {IMPORT_SECURITY_FORCE_RISK_THRESHOLD}/10) or are blocked. Enable
              override or deselect those entries.
            </Alert>
          ) : null}
          {items.length > 0 ? (
            <Stack spacing={1}>
              {items.map((it) => (
                <Box key={it.url} className="bulk-import-source-card">
                  <Stack
                    direction="row"
                    spacing={1}
                    className="bulk-import-source-header"
                    sx={{
                      alignItems: "flex-start",
                    }}
                  >
                    <Checkbox
                      size="small"
                      checked={it.selected}
                      onChange={(event) => {
                        const checked = event.target.checked;
                        setItems((prev) =>
                          prev.map((item) =>
                            item.url === it.url
                              ? { ...item, selected: checked }
                              : item,
                          ),
                        );
                      }}
                      disabled={analyzing || importing}
                    />
                    <Box sx={{ flex: 1, minWidth: 0 }}>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          display: "block",
                          mb: 0.25,
                        }}
                      >
                        Source URL
                      </Typography>
                      <Typography
                        variant="body2"
                        className="bulk-import-source-url"
                      >
                        {it.url}
                      </Typography>
                    </Box>
                  </Stack>
                  <Typography
                    variant="caption"
                    color={
                      it.status?.startsWith("Error")
                        ? "error"
                        : "text.secondary"
                    }
                    sx={{ display: "block", mt: 0.5 }}
                  >
                    {it.status || "Pending"}
                  </Typography>

                  {it.analyzed && it.discovered.length > 0 ? (
                    <TableContainer
                      className="table-shell bulk-import-table-shell"
                      sx={{ mt: 1 }}
                    >
                      <Table size="small" className="bulk-import-table">
                        <TableHead>
                          <TableRow>
                            <TableCell
                              padding="checkbox"
                              className="bulk-import-col-select"
                            >
                              Import
                            </TableCell>
                            <TableCell className="bulk-import-col-skill">
                              Skill
                            </TableCell>
                            <TableCell className="bulk-import-col-source">
                              Skill URL
                            </TableCell>
                            <TableCell className="bulk-import-col-risk">
                              Risk
                            </TableCell>
                            <TableCell className="bulk-import-col-security">
                              Security
                            </TableCell>
                            <TableCell className="bulk-import-col-findings">
                              Findings
                            </TableCell>
                          </TableRow>
                        </TableHead>
                        <TableBody>
                          {it.discovered.map((skill) => {
                            const result = skill.importResult || skill.preview;
                            const risk = computeImportRiskSummary(
                              result?.security,
                            );
                            const findingsCount = risk.totalFindings;
                            const blocked =
                              toBool(result?.security?.blocked) ||
                              result?.status === "blocked";
                            const threatRaw = str(
                              result?.security?.threat_level,
                              "",
                            )
                              .trim()
                              .toLowerCase();
                            const threatLabel = threatRaw
                              ? titleCaseWords(threatRaw)
                              : "Unknown";
                            const threatColor =
                              threatRaw === "malicious"
                                ? "error"
                                : threatRaw === "suspicious" ||
                                    threatRaw === "elevated"
                                  ? "warning"
                                  : threatRaw
                                    ? "success"
                                    : "default";
                            const securityLabel = blocked
                              ? "Blocked"
                              : risk.band === "secure"
                                ? "Clean"
                                : risk.bandLabel;
                            const securityColor = blocked
                              ? "error"
                              : risk.chipColor;
                            const statusMeta = buildBulkSkillStatus(skill);
                            const statusDetail =
                              compactStatusToken(statusMeta.label) ===
                              compactStatusToken(statusMeta.detail)
                                ? ""
                                : statusMeta.detail;
                            return (
                              <TableRow key={skill.key}>
                                <TableCell padding="checkbox">
                                  <Checkbox
                                    size="small"
                                    checked={skill.selected}
                                    onChange={(event) => {
                                      const checked = event.target.checked;
                                      setItems((prev) =>
                                        prev.map((item) => {
                                          if (item.url !== it.url) return item;
                                          return {
                                            ...item,
                                            discovered: item.discovered.map(
                                              (entry) =>
                                                entry.key === skill.key
                                                  ? {
                                                      ...entry,
                                                      selected: checked,
                                                    }
                                                  : entry,
                                            ),
                                          };
                                        }),
                                      );
                                    }}
                                    disabled={
                                      !it.selected || analyzing || importing
                                    }
                                  />
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="body2"
                                    className="bulk-import-wrap"
                                  >
                                    {skill.name}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    className="bulk-import-wrap"
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    {skill.url}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color:
                                        risk.chipColor === "success"
                                          ? "var(--ui-rgba-15-240-179-800)"
                                          : risk.chipColor === "error"
                                            ? "#ff5f57"
                                            : risk.chipColor === "warning"
                                              ? "#febc2e"
                                              : "var(--ui-rgba-180-220-200-600)",
                                    }}
                                  >
                                    {risk.score10.toFixed(1)}/10 *{" "}
                                    {risk.bandLabel}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color:
                                        threatColor === "success"
                                          ? "var(--ui-rgba-15-240-179-800)"
                                          : threatColor === "error"
                                            ? "#ff5f57"
                                            : threatColor === "warning"
                                              ? "#febc2e"
                                              : "var(--ui-rgba-180-220-200-600)",
                                    }}
                                  >
                                    {threatLabel} * {securityLabel}
                                  </Typography>
                                </TableCell>
                                <TableCell>
                                  <Typography
                                    variant="caption"
                                    color={
                                      findingsCount > 0
                                        ? "text.primary"
                                        : "text.secondary"
                                    }
                                  >
                                    {findingsCount === 0
                                      ? "None"
                                      : `${findingsCount}${risk.contextualFindings > 0 ? ` (${risk.contextualFindings} contextual)` : ""}`}
                                  </Typography>
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </TableContainer>
                  ) : null}
                </Box>
              ))}
            </Stack>
          ) : null}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
        <Button
          variant="outlined"
          disabled={importing || analyzing || !urlsText.trim()}
          onClick={handleAnalyzeSelected}
        >
          {analyzing ? "Analyzing..." : "Analyze"}
        </Button>
        <Button
          variant="contained"
          disabled={
            importing ||
            analyzing ||
            !analysisDone ||
            selectedSkillCount === 0 ||
            (!force && riskySelectedCount > 0)
          }
          onClick={handleImportSelected}
        >
          {importing
            ? "Importing..."
            : `Import Selected (${selectedSkillCount})`}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function ImportUrlDialog({
  open,
  onClose,
  onImported,
  onAfterImport,
}: {
  open: boolean;
  onClose: () => void;
  onImported?: ImportCallback;
  onAfterImport?: (
    name: string,
    importResult: SkillImportResponse,
  ) => Promise<void>;
}) {
  const [url, setUrl] = useState("");
  const [model, setModel] = useState("");
  const [force, setForce] = useState(false);
  const [loading, setLoading] = useState(false);
  const [previewReady, setPreviewReady] = useState(false);
  const [importCommitted, setImportCommitted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [importResult, setImportResult] = useState<SkillImportResponse | null>(
    null,
  );
  const [secretDrafts, setSecretDrafts] = useState<
    Record<string, { storeAs: string; value: string; useBuiltin: boolean }>
  >({});
  const [savingSecrets, setSavingSecrets] = useState(false);
  const [secretsSaved, setSecretsSaved] = useState(false);
  const importRisk = useMemo(
    () => computeImportRiskSummary(importResult?.security),
    [importResult],
  );
  const securityBlocked = toBool(importResult?.security?.blocked);
  const importRequiresForce =
    previewReady &&
    !force &&
    (securityBlocked ||
      importRisk.score10 >= IMPORT_SECURITY_FORCE_RISK_THRESHOLD);

  const resetDialogState = (clearInputs = false) => {
    if (clearInputs) {
      setUrl("");
      setModel("");
      setForce(false);
    }
    setError(null);
    setInfo(null);
    setImportResult(null);
    setPreviewReady(false);
    setImportCommitted(false);
    setSecretDrafts({});
    setSavingSecrets(false);
    setSecretsSaved(false);
  };

  const buildSecretDraftsFromResult = (result: SkillImportResponse) => {
    const required = result.secrets?.required_env || [];
    const bindings = result.secrets?.bindings || {};
    const drafts: Record<
      string,
      { storeAs: string; value: string; useBuiltin: boolean }
    > = {};
    for (const env of required) {
      const binding = bindings[env];
      drafts[env] = {
        storeAs: binding && binding !== "builtin" ? binding : env,
        value: "",
        useBuiltin: binding === "builtin",
      };
    }
    setSecretDrafts(drafts);
  };

  const runImport = async (previewOnly: boolean) => {
    if (!url.trim()) return;
    setLoading(true);
    setError(null);
    setInfo(null);
    setSecretsSaved(false);
    let closeAfterSuccess = false;
    if (previewOnly) {
      setImportCommitted(false);
    }
    try {
      const result = await api.importSkill({
        url: url.trim(),
        model: model.trim() || undefined,
        force,
        preview_only: previewOnly,
      });

      setImportResult(result);
      buildSecretDraftsFromResult(result);

      let message =
        result.message ||
        (previewOnly
          ? `Preview ready for ${result.name}`
          : `Imported ${result.name}`);
      if (result.status === "blocked") {
        message =
          result.message ||
          "Blocked by security verification. Enable override to continue.";
      } else if (!previewOnly && result.status === "needs_secrets") {
        message =
          result.message ||
          `Imported ${result.name} (disabled until secrets are configured)`;
      }
      setInfo(message);

      if (previewOnly) {
        setPreviewReady(true);
        return;
      }

      setPreviewReady(false);
      setImportCommitted(true);

      // Auto-save secrets if the user pre-filled them during preview
      const requiredEnvs = result.secrets?.required_env || [];
      let autoSavedAllRequiredSecrets = false;
      if (result.name && requiredEnvs.length > 0) {
        const filledSecrets = requiredEnvs
          .map((env) => {
            const d = secretDrafts[env] || {
              storeAs: env,
              value: "",
              useBuiltin: false,
            };
            if (d.useBuiltin) return { env, store_as: "builtin" as const };
            const storeAs = (d.storeAs || env).trim();
            const value = (d.value || "").trim();
            return value ? { env, store_as: storeAs, value } : null;
          })
          .filter(Boolean);
        if (filledSecrets.length > 0) {
          try {
            const secretsOut = await api.setSkillSecrets(result.name, {
              secrets: filledSecrets as {
                env: string;
                store_as: string;
                value?: string;
              }[],
            });
            if ((secretsOut.missing_env || []).length === 0) {
              autoSavedAllRequiredSecrets = true;
              setSecretsSaved(true);
              setInfo(`Imported ${result.name} - secrets saved automatically.`);
            }
          } catch {
            /* silent - user can still save manually */
          }
        }
      }

      const importedChildren = Array.isArray(result.imported)
        ? result.imported
        : [];
      if (importedChildren.length > 0) {
        for (const child of importedChildren) {
          const childResult = child?.result;
          if (!childResult?.name) continue;
          const childMessage =
            childResult.message ||
            (childResult.status === "needs_secrets"
              ? `Imported ${childResult.name} (disabled until secrets are configured)`
              : `Imported ${childResult.name}`);
          await onAfterImport?.(childResult.name, childResult);
          await onImported?.({ result: childResult, message: childMessage });
        }
      } else {
        await onAfterImport?.(result.name, result);
        await onImported?.({ result, message });
      }

      const childNeedsAttention = importedChildren.some((child) =>
        skillImportNeedsAttention(child?.result),
      );
      const resultNeedsAttention =
        skillImportNeedsAttention(result) && !autoSavedAllRequiredSecrets;
      closeAfterSuccess =
        result.status !== "blocked" &&
        !resultNeedsAttention &&
        !childNeedsAttention;
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setLoading(false);
      if (closeAfterSuccess) {
        resetDialogState(true);
        onClose();
      }
    }
  };

  const handleAnalyze = async () => runImport(true);
  const handleImport = async () => runImport(false);

  const handleSaveSecrets = async () => {
    if (!importResult?.name) return;
    if (!importCommitted) {
      setError("Import template first, then save secrets.");
      return;
    }
    const required = importResult.secrets?.required_env || [];
    if (required.length === 0) return;
    setSavingSecrets(true);
    setError(null);
    try {
      const payload = required.map((env) => {
        const d = secretDrafts[env] || {
          storeAs: env,
          value: "",
          useBuiltin: false,
        };
        if (d.useBuiltin) return { env, store_as: "builtin" };
        const storeAs = (d.storeAs || env).trim();
        const value = (d.value || "").trim();
        return value
          ? { env, store_as: storeAs, value }
          : { env, store_as: storeAs };
      });
      const secretsOut = await api.setSkillSecrets(importResult.name, {
        secrets: payload,
      });
      if ((secretsOut.missing_env || []).length > 0) {
        setError(
          `Some keys are still missing: ${secretsOut.missing_env.join(", ")}`,
        );
      } else {
        setSecretsSaved(true);
        setInfo(
          "Secrets saved. The skill remains disabled until you manually enable it in Skills.",
        );
      }
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setSavingSecrets(false);
    }
  };

  const handleClose = () => {
    if (loading) return;
    resetDialogState();
    onClose();
  };

  return (
    <Dialog open={open} onClose={handleClose} maxWidth="sm" fullWidth>
      <DialogTitle>Import from URL</DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1}>
          {error && <Alert severity="error">{error}</Alert>}
          {info && <Alert severity="info">{info}</Alert>}
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
            }}
          >
            Supports direct SKILL.md links plus GitHub-hosted skill sources.
          </Typography>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              whiteSpace: "pre-line",
            }}
          >
            {`Examples:
1. https://github.com/org/repo/tree/main/skills/market-analysis
2. https://raw.githubusercontent.com/org/repo/main/skills/market-analysis/SKILL.md
3. https://raw.githubusercontent.com/org/repo/main/skills/self-improving-agent/SKILL.md`}
          </Typography>
          <TextField
            fullWidth
            size="small"
            label="Import URL"
            value={url}
            onChange={(event) => {
              setUrl(event.target.value);
              setPreviewReady(false);
              setImportCommitted(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
          />
          <TextField
            fullWidth
            size="small"
            label="Model override (optional)"
            value={model}
            onChange={(event) => {
              setModel(event.target.value);
              setPreviewReady(false);
              setImportCommitted(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
          />
          <FormControlLabel
            control={
              <Switch
                checked={force}
                onChange={(event) => setForce(event.target.checked)}
              />
            }
            label="Override all warnings (import anyway)"
          />
          {importResult?.security
            ? (() => {
                const riskColor =
                  importRisk.score10 >= IMPORT_RISK_POLICY.reviewBandMax
                    ? "#ff5f57"
                    : importRisk.score10 >= IMPORT_RISK_POLICY.secureBandMax
                      ? "#febc2e"
                      : "#0ff0b3";
                const riskBg =
                  importRisk.score10 >= IMPORT_RISK_POLICY.reviewBandMax
                    ? "var(--ui-rgba-255-95-87-060)"
                    : importRisk.score10 >= IMPORT_RISK_POLICY.secureBandMax
                      ? "var(--ui-rgba-254-188-46-060)"
                      : "var(--ui-rgba-15-240-179-040)";
                const riskEmoji =
                  importRisk.score10 >= IMPORT_RISK_POLICY.reviewBandMax
                    ? "High Risk"
                    : importRisk.score10 >= IMPORT_RISK_POLICY.secureBandMax
                      ? "Needs Review"
                      : "Safe";
                const semanticCapabilities = Array.isArray(
                  importResult.security.capabilities,
                )
                  ? importResult.security.capabilities
                  : [];
                const capabilityLabels = semanticCapabilities
                  .map((rawCapability) => {
                    const capability = asRecord(rawCapability);
                    const kind = str(capability.kind, "").trim();
                    const target = str(capability.target, "").trim();
                    return kind
                      ? target
                        ? `${kind}:${target}`
                        : kind
                      : "";
                  })
                  .filter(Boolean);
                const matchedRules = Array.isArray(
                  importResult.security.matched_rules,
                )
                  ? importResult.security.matched_rules
                  : [];
                const reviewModel = str(
                  importResult.security.review_model,
                  "",
                ).trim();
                const reviewSummary = str(
                  importResult.security.review_summary,
                  "",
                ).trim();
                return (
                  <Box
                    sx={{
                      mt: 1,
                      border: `1px solid ${riskColor}22`,
                      borderRadius: "8px",
                      background: riskBg,
                      overflow: "hidden",
                    }}
                  >
                    <Box
                      sx={{
                        px: 1.5,
                        py: 1,
                        display: "flex",
                        alignItems: "center",
                        gap: 1,
                        borderBottom: `1px solid ${riskColor}15`,
                      }}
                    >
                      <Box
                        sx={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          background: riskColor,
                          boxShadow: `0 0 6px ${riskColor}60`,
                          flexShrink: 0,
                        }}
                      />
                      <Typography
                        variant="subtitle2"
                        sx={{
                          fontSize: "12px",
                          fontWeight: 700,
                          color: riskColor,
                        }}
                      >
                        {riskEmoji} - {importRisk.score10.toFixed(1)}/10
                      </Typography>
                      <Box sx={{ flex: 1 }} />
                      {securityBlocked ? (
                        <Typography
                          variant="caption"
                          sx={{ color: "#ff5f57", fontWeight: 600 }}
                        >
                          BLOCKED
                        </Typography>
                      ) : importRequiresForce ? (
                        <Typography
                          variant="caption"
                          sx={{ color: "#febc2e", fontWeight: 600 }}
                        >
                          OVERRIDE REQUIRED
                        </Typography>
                      ) : null}
                    </Box>
                    <Box sx={{ px: 1.5, py: 1 }}>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "var(--ui-rgba-200-230-210-550)",
                          display: "block",
                          mb: 0.5,
                        }}
                      >
                        {(() => {
                          const raw = str(
                            importResult.security.threat_level,
                            "unknown",
                          );
                          const ctxRatio =
                            importRisk.totalFindings > 0
                              ? importRisk.contextualFindings /
                                importRisk.totalFindings
                              : 0;
                          const display =
                            raw.toLowerCase() === "malicious" &&
                            ctxRatio >= IMPORT_RISK_POLICY.contextualStrongRatio
                              ? "Standard integration patterns"
                              : `Threat level: ${raw}`;
                          return display;
                        })()}
                        {importRisk.totalFindings > 0
                          ? ` * ${importRisk.totalFindings} signal${importRisk.totalFindings === 1 ? "" : "s"} found`
                          : " * No signals"}
                        {importRisk.contextualFindings > 0
                          ? ` (${importRisk.contextualFindings} likely safe - common in integrations)`
                          : ""}
                      </Typography>
                      {reviewModel || reviewSummary || capabilityLabels.length ? (
                        <Box sx={{ mb: 0.75 }}>
                          {reviewModel ? (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "var(--ui-rgba-200-230-210-600)",
                                display: "block",
                                lineHeight: 1.35,
                              }}
                            >
                              Semantic review model: {reviewModel}
                            </Typography>
                          ) : null}
                          {reviewSummary ? (
                            <Typography
                              variant="caption"
                              sx={{
                                color: "var(--ui-rgba-200-230-210-600)",
                                display: "block",
                                lineHeight: 1.35,
                              }}
                            >
                              {reviewSummary}
                            </Typography>
                          ) : null}
                          {capabilityLabels.length ? (
                            <Stack
                              direction="row"
                              spacing={0.5}
                              useFlexGap
                              sx={{ flexWrap: "wrap", mt: 0.5 }}
                            >
                              {capabilityLabels.slice(0, 12).map((label) => (
                                <Chip
                                  key={label}
                                  size="small"
                                  label={label}
                                  sx={{
                                    height: 20,
                                    maxWidth: 220,
                                    borderRadius: "6px",
                                    color: "var(--ui-rgba-230-245-235-780)",
                                    background: "var(--ui-rgba-255-255-255-050)",
                                    "& .MuiChip-label": {
                                      overflow: "hidden",
                                      textOverflow: "ellipsis",
                                    },
                                  }}
                                />
                              ))}
                            </Stack>
                          ) : null}
                        </Box>
                      ) : null}
                      {matchedRules.length ? (
                        <Stack spacing={0.25} sx={{ mb: 0.75 }}>
                          {matchedRules.slice(0, 5).map((rawRule, idx) => {
                            const rule = asRecord(rawRule);
                            const effect = str(rule.effect, "warn");
                            const ruleId = str(rule.id, `rule-${idx}`);
                            const message = str(rule.message, ruleId);
                            return (
                              <Typography
                                key={`${ruleId}-${idx}`}
                                variant="caption"
                                sx={{
                                  color:
                                    effect.toLowerCase() === "block"
                                      ? "#ff5f57"
                                      : "#febc2e",
                                  display: "block",
                                  lineHeight: 1.35,
                                }}
                              >
                                Policy {effect}: {message}
                              </Typography>
                            );
                          })}
                        </Stack>
                      ) : null}
                      {securityBlocked || importRequiresForce ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "var(--ui-rgba-230-245-235-720)",
                            display: "block",
                            mb: 0.75,
                            lineHeight: 1.35,
                          }}
                        >
                          Review the listed line before importing. Override only
                          when you trust the source and the flagged behavior is
                          expected.
                        </Typography>
                      ) : null}
                      {Array.isArray(importResult.security.findings) &&
                      importResult.security.findings.length > 0 ? (
                        <Stack spacing={0.3} sx={{ mt: 0.75 }}>
                          {(importResult.security.findings as unknown[])
                            .slice(0, 15)
                            .map((rawFinding, idx) => {
                              const f = asRecord(rawFinding);
                              const sev = num(f.severity, 0);
                              const sevColor =
                                sev >= IMPORT_RISK_POLICY.findingHighSeverity
                                  ? "#ff5f57"
                                  : sev >=
                                      IMPORT_RISK_POLICY.findingMediumSeverity
                                    ? "#febc2e"
                                    : "#0ff0b3";
                              const sevLabel =
                                sev >= IMPORT_RISK_POLICY.findingHighSeverity
                                  ? "HIGH"
                                  : sev >=
                                      IMPORT_RISK_POLICY.findingMediumSeverity
                                    ? "MED"
                                    : "LOW";
                              const cat = str(f.category, "");
                              const humanCat = importFindingCategoryLabel(
                                cat,
                                f,
                              );
                              const matchedText = str(
                                f.matched_text,
                                "",
                              ).trim();
                              const findingFile = str(f.file, "").trim();
                              const findingLine = num(f.line, -1);
                              const explanation = explainImportFinding(f);
                              return (
                                <Box
                                  key={`${idx}-${cat}`}
                                  sx={{
                                    display: "flex",
                                    gap: 1,
                                    py: 0.45,
                                    alignItems: "flex-start",
                                  }}
                                >
                                  <Typography
                                    sx={{
                                      fontSize: "9.5px",
                                      fontWeight: 700,
                                      color: sevColor,
                                      minWidth: "30px",
                                      flexShrink: 0,
                                    }}
                                  >
                                    {sevLabel}
                                  </Typography>
                                  <Box sx={{ flex: 1, minWidth: 0 }}>
                                    <Typography
                                      sx={{
                                        fontSize: "10.5px",
                                        color: "var(--ui-rgba-200-230-210-760)",
                                        lineHeight: 1.3,
                                      }}
                                    >
                                      {humanCat}
                                      {findingFile
                                        ? ` in ${findingFile}`
                                        : ""}
                                      {findingLine >= 0
                                        ? ` at line ${findingLine}`
                                        : ""}
                                      {matchedText ? (
                                        <span
                                          style={{
                                            color: "var(--ui-rgba-130-170-160-480)",
                                          }}
                                        >{` - ${matchedText.slice(0, 80)}`}</span>
                                      ) : null}
                                    </Typography>
                                    <Typography
                                      sx={{
                                        fontSize: "10px",
                                        color: "var(--ui-rgba-200-230-210-550)",
                                        lineHeight: 1.35,
                                      }}
                                    >
                                      {explanation}
                                    </Typography>
                                  </Box>
                                </Box>
                              );
                            })}
                        </Stack>
                      ) : (
                        <Typography variant="caption" sx={{ color: "#0ff0b3" }}>
                          No signals detected - looks safe.
                        </Typography>
                      )}
                    </Box>
                  </Box>
                );
              })()
            : null}
          {Array.isArray(importResult?.imported) &&
          importResult.imported.length > 0 ? (
            <Box className="term-shell" sx={{ mt: 1, p: 0 }}>
              <Box
                sx={{
                  px: 1.5,
                  py: 0.75,
                  borderBottom: "1px solid var(--ui-rgba-57-208-255-060)",
                }}
              >
                <Typography
                  sx={{
                    fontFamily: "inherit",
                    fontSize: "10.5px",
                    fontWeight: 700,
                    letterSpacing: 0,
                    textTransform: "uppercase",
                    color: "var(--ui-rgba-57-208-255-350)",
                  }}
                >
                  Per-Skill Analysis
                </Typography>
              </Box>
              <Stack spacing={0} sx={{ px: 1.5, py: 0.5 }}>
                {importResult.imported.map((entry, idx) => {
                  const child = entry?.result;
                  const sec = child?.security;
                  const findingsCount = Array.isArray(sec?.findings)
                    ? sec?.findings.length
                    : 0;
                  const childRisk = computeImportRiskSummary(sec);
                  const riskColor =
                    childRisk.score10 >= IMPORT_RISK_POLICY.reviewBandMax
                      ? "#ff5f57"
                      : childRisk.score10 >= IMPORT_RISK_POLICY.secureBandMax
                        ? "#febc2e"
                        : "var(--ui-rgba-15-240-179-700)";
                  return (
                    <Box
                      key={`${entry?.url || child?.name || idx}-${idx}`}
                      sx={{
                        display: "flex",
                        gap: 1.5,
                        py: 0.4,
                        borderBottom: "1px solid var(--ui-rgba-57-208-255-040)",
                        alignItems: "baseline",
                      }}
                    >
                      <Typography
                        sx={{
                          fontFamily: "inherit",
                          fontSize: "10.5px",
                          color: "var(--ui-rgba-200-230-210-800)",
                          minWidth: "120px",
                          flexShrink: 0,
                        }}
                      >
                        {child?.name || "-"}
                      </Typography>
                      <Typography
                        sx={{
                          fontFamily: "inherit",
                          fontSize: "10px",
                          color: riskColor,
                          fontWeight: 600,
                          minWidth: "55px",
                          flexShrink: 0,
                        }}
                      >
                        {childRisk.score10.toFixed(1)}/10
                      </Typography>
                      <Typography
                        sx={{
                          fontFamily: "inherit",
                          fontSize: "10px",
                          color: "var(--ui-rgba-180-220-200-500)",
                          flex: 1,
                        }}
                      >
                        {str(sec?.threat_level, "-")} * {findingsCount} signals
                      </Typography>
                    </Box>
                  );
                })}
              </Stack>
              <Stack spacing={0.75} sx={{ mt: 1 }}>
                {importResult.imported.map((entry, idx) => {
                  const child = entry?.result;
                  if (!child) return null;
                  const sec = child.security;
                  const warnings = Array.isArray(sec?.warnings)
                    ? sec?.warnings
                    : [];
                  const findings = Array.isArray(sec?.findings)
                    ? sec?.findings
                    : [];
                  if (warnings.length === 0 && findings.length === 0)
                    return null;
                  return (
                    <Box
                      key={`skill-sec-${entry?.url || child?.name || idx}-${idx}`}
                      sx={{
                        border: "1px solid var(--ui-rgba-108-156-212-180)",
                        borderRadius: 1,
                        p: 1,
                      }}
                    >
                      <Typography
                        variant="caption"
                        sx={{ display: "block", mb: 0.5 }}
                      >
                        {child.name || "-"} details
                      </Typography>
                      {warnings.length > 0 ? (
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            display: "block",
                          }}
                        >
                          Warnings: {warnings.slice(0, 3).join(" | ")}
                        </Typography>
                      ) : null}
                      {findings.length > 0 ? (
                        <Stack spacing={0.25} sx={{ mt: 0.5 }}>
                          {findings.slice(0, 3).map((rawFinding, fidx) => {
                            const f = asRecord(rawFinding);
                            return (
                              <Typography
                                key={`finding-${fidx}-${str(f.category, "")}`}
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                  display: "block",
                                }}
                              >
                                [{str(f.category, "-")}] line{" "}
                                {num(f.line, -1) >= 0 ? num(f.line) : "-"}:{" "}
                                {str(f.description, "-").slice(0, 180)}
                              </Typography>
                            );
                          })}
                        </Stack>
                      ) : null}
                    </Box>
                  );
                })}
              </Stack>
              {Array.isArray(importResult.failed) &&
              importResult.failed.length > 0 ? (
                <Alert severity="warning" sx={{ mt: 1 }}>
                  Failed imports: {importResult.failed.length}
                </Alert>
              ) : null}
            </Box>
          ) : null}
          {(importResult?.secrets?.required_env || []).length > 0 ? (
            <Box sx={{ mt: 1 }}>
              <Typography
                variant="subtitle2"
                sx={{
                  mb: 1,
                }}
              >
                Required credentials
              </Typography>
              {!importCommitted ? (
                <Box
                  sx={{
                    mb: 1,
                    p: 1,
                    borderRadius: "6px",
                    background: "var(--ui-rgba-254-188-46-060)",
                    border: "1px solid var(--ui-rgba-254-188-46-150)",
                  }}
                >
                  <Typography
                    variant="caption"
                    sx={{ color: "#febc2e", fontWeight: 600 }}
                  >
                    Fill in credentials below, then click "Import Template" to
                    save them.
                  </Typography>
                </Box>
              ) : null}
              <Stack spacing={1}>
                {(importResult?.secrets?.required_env || []).map((env) => {
                  const d = secretDrafts[env] || {
                    storeAs: env,
                    value: "",
                    useBuiltin: false,
                  };
                  const missing = (
                    importResult?.secrets?.missing_env || []
                  ).includes(env);
                  return (
                    <Box
                      key={env}
                      sx={{
                        border: "1px solid var(--ui-rgba-108-156-212-180)",
                        borderRadius: 1,
                        p: 1,
                      }}
                    >
                      <Stack
                        direction="row"
                        sx={{
                          justifyContent: "space-between",
                          alignItems: "center",
                        }}
                      >
                        <Typography
                          variant="body2"
                          sx={{
                            fontWeight: 700,
                          }}
                        >
                          {env}
                        </Typography>
                        <Chip
                          size="small"
                          color={missing ? "warning" : "success"}
                          label={missing ? "missing" : "configured"}
                        />
                      </Stack>
                      <Stack
                        direction={{ xs: "column", md: "row" }}
                        spacing={1}
                        sx={{
                          mt: 1,
                        }}
                      >
                        <TextField
                          fullWidth
                          size="small"
                          label="Store as"
                          value={d.storeAs}
                          disabled={d.useBuiltin}
                          onChange={(e) =>
                            setSecretDrafts((prev) => ({
                              ...prev,
                              [env]: { ...d, storeAs: e.target.value },
                            }))
                          }
                        />
                        <TextField
                          fullWidth
                          size="small"
                          type="password"
                          label="Value (optional)"
                          value={d.value}
                          disabled={d.useBuiltin}
                          onChange={(e) =>
                            setSecretDrafts((prev) => ({
                              ...prev,
                              [env]: { ...d, value: e.target.value },
                            }))
                          }
                        />
                      </Stack>
                      <FormControlLabel
                        control={
                          <Switch
                            checked={d.useBuiltin}
                            onChange={(e) =>
                              setSecretDrafts((prev) => ({
                                ...prev,
                                [env]: { ...d, useBuiltin: e.target.checked },
                              }))
                            }
                          />
                        }
                        label="Use builtin provider key"
                      />
                    </Box>
                  );
                })}
                <Button
                  variant="outlined"
                  disabled={savingSecrets || secretsSaved || !importCommitted}
                  onClick={handleSaveSecrets}
                >
                  {savingSecrets
                    ? "Saving..."
                    : secretsSaved
                      ? "Secrets saved"
                      : !importCommitted
                        ? "Import template first"
                        : "Save secrets"}
                </Button>
              </Stack>
            </Box>
          ) : null}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={handleClose} disabled={loading}>
          Close
        </Button>
        <Button
          variant="contained"
          disabled={loading || !url.trim() || importRequiresForce}
          onClick={previewReady ? handleImport : handleAnalyze}
        >
          {loading
            ? previewReady
              ? "Importing..."
              : "Analyzing..."
            : previewReady
              ? "Import Template"
              : "Analyze Template"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function SkillSecretsDialog({
  open,
  skillName,
  onClose,
}: {
  open: boolean;
  skillName: string | null;
  onClose: () => void;
}) {
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [secrets, setSecrets] = useState<{
    required_env: string[];
    missing_env: string[];
    bindings: Record<string, string>;
  } | null>(null);
  const [drafts, setDrafts] = useState<
    Record<string, { storeAs: string; value: string; useBuiltin: boolean }>
  >({});

  useEffect(() => {
    if (!open || !skillName) return;
    setLoading(true);
    setError(null);
    setInfo(null);
    setSecrets(null);
    setDrafts({});
    api
      .getSkillSecrets(skillName)
      .then((out) => {
        setSecrets(out);
        const next: Record<
          string,
          { storeAs: string; value: string; useBuiltin: boolean }
        > = {};
        for (const env of out.required_env || []) {
          const binding = (out.bindings || {})[env];
          next[env] = {
            storeAs: binding && binding !== "builtin" ? binding : env,
            value: "",
            useBuiltin: binding === "builtin",
          };
        }
        setDrafts(next);
      })
      .catch((err) => setError(errMessage(err)))
      .finally(() => setLoading(false));
  }, [open, skillName]);

  const save = async () => {
    if (!skillName || !secrets) return;
    setSaving(true);
    setError(null);
    setInfo(null);
    try {
      const payload = (secrets.required_env || []).map((env) => {
        const d = drafts[env] || { storeAs: env, value: "", useBuiltin: false };
        if (d.useBuiltin) return { env, store_as: "builtin" };
        const storeAs = (d.storeAs || env).trim();
        const value = (d.value || "").trim();
        return value
          ? { env, store_as: storeAs, value }
          : { env, store_as: storeAs };
      });
      const out = await api.setSkillSecrets(skillName, { secrets: payload });
      setSecrets(out);
      if ((out.missing_env || []).length > 0) {
        setError(`Some keys are still missing: ${out.missing_env.join(", ")}`);
      } else {
        setInfo(
          "Secrets saved. The skill remains disabled until you manually enable it in Skills.",
        );
      }
    } catch (err) {
      setError(errMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Secrets: {skillName || ""}</DialogTitle>
      <DialogContent dividers>
        <Typography
          variant="caption"
          sx={{
            color: "text.secondary",
            display: "block",
            mb: 1,
          }}
        >
          Secrets are private API keys or tokens used by this skill at runtime.
        </Typography>
        {loading ? (
          <Typography
            variant="body2"
            sx={{
              color: "text.secondary",
            }}
          >
            Loading...
          </Typography>
        ) : null}
        {error ? <Alert severity="error">{error}</Alert> : null}
        {info ? <Alert severity="info">{info}</Alert> : null}
        {!loading && secrets ? (
          <Stack spacing={1.25}>
            {(secrets.required_env || []).length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No required credentials detected for this skill.
              </Typography>
            ) : (
              (secrets.required_env || []).map((env) => {
                const d = drafts[env] || {
                  storeAs: env,
                  value: "",
                  useBuiltin: false,
                };
                const missing = (secrets.missing_env || []).includes(env);
                return (
                  <Box
                    key={env}
                    sx={{
                      border: "1px solid var(--ui-rgba-108-156-212-180)",
                      borderRadius: 1,
                      p: 1,
                    }}
                  >
                    <Stack
                      direction="row"
                      sx={{
                        justifyContent: "space-between",
                        alignItems: "center",
                      }}
                    >
                      <Typography
                        variant="body2"
                        sx={{
                          fontWeight: 700,
                        }}
                      >
                        {env}
                      </Typography>
                      <Chip
                        size="small"
                        color={missing ? "warning" : "success"}
                        label={missing ? "missing" : "configured"}
                      />
                    </Stack>
                    <Stack
                      direction={{ xs: "column", md: "row" }}
                      spacing={1}
                      sx={{
                        mt: 1,
                      }}
                    >
                      <TextField
                        fullWidth
                        size="small"
                        label="Store as"
                        value={d.storeAs}
                        disabled={d.useBuiltin}
                        onChange={(e) =>
                          setDrafts((prev) => ({
                            ...prev,
                            [env]: { ...d, storeAs: e.target.value },
                          }))
                        }
                      />
                      <TextField
                        fullWidth
                        size="small"
                        type="password"
                        label="Value (optional)"
                        value={d.value}
                        disabled={d.useBuiltin}
                        onChange={(e) =>
                          setDrafts((prev) => ({
                            ...prev,
                            [env]: { ...d, value: e.target.value },
                          }))
                        }
                      />
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={d.useBuiltin}
                          onChange={(e) =>
                            setDrafts((prev) => ({
                              ...prev,
                              [env]: { ...d, useBuiltin: e.target.checked },
                            }))
                          }
                        />
                      }
                      label="Use builtin provider key"
                    />
                  </Box>
                );
              })
            )}
          </Stack>
        ) : null}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
        <Button
          variant="contained"
          onClick={save}
          disabled={
            saving ||
            loading ||
            !secrets ||
            (secrets.required_env || []).length === 0
          }
        >
          {saving ? "Saving..." : "Save"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function QueryTable({
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
  const q = useQuery({
    queryKey: [queryKey, queryPath],
    queryFn: () => api.rawGet(queryPath),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const payload = asRecord(q.data);
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
      <Typography
        variant="h6"
        sx={{
          mb: 1,
        }}
      >
        {title}
      </Typography>
      {q.error ? (
        <Alert severity="error">{errMessage(q.error)}</Alert>
      ) : rows.length === 0 ? (
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
          }}
        >
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
              <Typography
                variant="caption"
                className="conversation-pagination-copy"
              >
                {totalRows} item{totalRows === 1 ? "" : "s"}
              </Typography>
              <Stack
                direction="row"
                spacing={0.75}
                sx={{
                  alignItems: "center",
                }}
              >
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => setPage((prev) => Math.max(0, prev - 1))}
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
                    setPage((prev) =>
                      Math.min(Math.max(0, pageCount - 1), prev + 1),
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

export default function SkillsPage({ autoRefresh }: { autoRefresh: boolean }) {
  const queryClient = useQueryClient();
  const [lastImport, setLastImport] = useState<SkillImportSummary | null>(null);
  const [testResults, setTestResults] = useState<Record<string, string>>({});
  const [testResultTones, setTestResultTones] = useState<
    Record<string, SkillTestResultTone>
  >({});
  const [testRunDialog, setTestRunDialog] =
    useState<SkillTestRunDialog | null>(null);
  const skillTestAbortRef = useRef<AbortController | null>(null);
  const activeSkillTestRunIdRef = useRef<string | null>(null);
  const skillTestRunSeqRef = useRef(0);
  const [skillMenuAnchor, setSkillMenuAnchor] = useState<{
    el: HTMLElement;
    name: string;
  } | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [bulkOpen, setBulkOpen] = useState(false);
  const [bulkInitialUrls, setBulkInitialUrls] = useState<string[]>([]);
  const [bulkSourceLabel, setBulkSourceLabel] = useState<string | undefined>(
    undefined,
  );
  const [marketplaceDialogOpen, setMarketplaceDialogOpen] = useState(false);
  const [marketplaceEditingId, setMarketplaceEditingId] = useState<
    string | null
  >(null);
  const [marketplaceForm, setMarketplaceForm] =
    useState<SkillMarketplaceForm>(EMPTY_SKILL_MARKETPLACE_FORM);
  const [marketplaceError, setMarketplaceError] = useState<string | null>(null);
  const [marketplaceSearch, setMarketplaceSearch] = useState("");
  const [selectedMarketplaceInstallerKeys, setSelectedMarketplaceInstallerKeys] =
    useState<string[]>([]);
  const [editOpen, setEditOpen] = useState(false);
  const [editTargetName, setEditTargetName] = useState<string | null>(null);
  const [developerModeEnabled, setDeveloperModeEnabledState] = useState(
    getDeveloperModeEnabled,
  );
  const [editForm, setEditForm] = useState<SkillEditorForm>(
    defaultSkillEditorForm(),
  );
  const [editContent, setEditContent] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const [editLoading, setEditLoading] = useState(false);
  const [createWizardEnabled, setCreateWizardEnabled] = useState(true);
  const [createWizardStep, setCreateWizardStep] = useState(0);
  const [editAttachHook, setEditAttachHook] = useState(false);
  const [editHookInstruction, setEditHookInstruction] = useState("");
  const [editHookTrigger, setEditHookTrigger] =
    useState<HookTriggerValue>("on_error");
  const [editHookUrl, setEditHookUrl] = useState("");
  const [editAttachTask, setEditAttachTask] = useState(false);
  const [editTaskInstruction, setEditTaskInstruction] = useState("");
  const [editTaskCron, setEditTaskCron] = useState("");
  const [aiCreateOpen, setAiCreateOpen] = useState(false);
  const [aiPrompt, setAiPrompt] = useState("");
  const [aiNameHint, setAiNameHint] = useState("");
  const [aiError, setAiError] = useState<string | null>(null);
  const [skillsTab, setSkillsTab] = useState<"manage" | "system">("manage");
  const [skillSearch, setSkillSearch] = useState("");
  const [skillSort, setSkillSort] = useState<"name" | "imported">("name");
  const [secretsName, setSecretsName] = useState<string | null>(null);
  const [hooksOpen, setHooksOpen] = useState(false);
  const [hooksTargetAction, setHooksTargetAction] = useState<string | null>(
    null,
  );
  const [hookInstruction, setHookInstruction] = useState("");
  const [hookName, setHookName] = useState("");
  const [hookTrigger, setHookTrigger] =
    useState<HookTriggerValue>("post_action");
  const [hookUrl, setHookUrl] = useState("");
  const [hookError, setHookError] = useState<string | null>(null);
  const editRawMode = developerModeEnabled;

  useEffect(() => {
    const refreshDeveloperMode = () =>
      setDeveloperModeEnabledState(getDeveloperModeEnabled());
    window.addEventListener(
      DEVELOPER_MODE_EVENT,
      refreshDeveloperMode as EventListener,
    );
    window.addEventListener("storage", refreshDeveloperMode);
    return () => {
      window.removeEventListener(
        DEVELOPER_MODE_EVENT,
        refreshDeveloperMode as EventListener,
      );
      window.removeEventListener("storage", refreshDeveloperMode);
    };
  }, []);

  const skillsQ = useQuery({
    queryKey: ["skills-manager"],
    queryFn: () => api.rawGet("/skills"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const marketplacesQ = useQuery({
    queryKey: ["skills-marketplaces"],
    queryFn: () => api.rawGet("/skills/marketplaces"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const hooksQ = useQuery({
    queryKey: ["skills-hooks"],
    queryFn: () => api.rawGet("/hooks"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const hookRunsQ = useQuery({
    queryKey: ["skills-hook-runs"],
    queryFn: () => api.rawGet("/hooks/runs?limit=200"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const skills = dedupeSkillRecords(pickRecords(skillsQ.data, "skills"));
  const marketplaces = useMemo(
    () => asRecords(asRecord(marketplacesQ.data).marketplaces),
    [marketplacesQ.data],
  );
  const hooks = asRecords(hooksQ.data);
  const hookRuns = asRecords(hookRunsQ.data);
  const activeTestName =
    testRunDialog &&
    (isSkillTestRunActive(testRunDialog.phase) ||
      testRunDialog.phase === "waiting_input")
      ? testRunDialog.name
      : null;

  useEffect(() => {
    return () => {
      const runId = activeSkillTestRunIdRef.current;
      if (runId) {
        void api.cancelSkillTest(runId).catch(() => undefined);
      }
      skillTestAbortRef.current?.abort();
      skillTestAbortRef.current = null;
      activeSkillTestRunIdRef.current = null;
    };
  }, []);

  const setSkillTestStatus = (
    name: string,
    message: string,
    tone: SkillTestResultTone = "info",
  ) => {
    setTestResults((prev) => ({ ...prev, [name]: message }));
    setTestResultTones((prev) => ({ ...prev, [name]: tone }));
  };

  const updateSkillTestDialog = (
    name: string,
    updater: (current: SkillTestRunDialog) => SkillTestRunDialog,
  ) => {
    setTestRunDialog((current) => {
      if (!current || current.name !== name) return current;
      return updater(current);
    });
  };

  const handleImported = async (summary: SkillImportSummary) => {
    setLastImport(summary);
    await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
  };
  const afterImport = async () => {
    await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
  };

  const marketplaceInstallers = useMemo<SkillMarketplaceInstallerRow[]>(() => {
    return marketplaces.flatMap((marketplace) => {
      const marketplaceId = str(marketplace.id, "").trim();
      const marketplaceName = str(marketplace.name, marketplaceId).trim();
      return asRecords(marketplace.installers).map((installer, idx) => {
        const installUrl = str(installer.install_url, "").trim();
        const installerId = str(installer.id, "").trim() || `installer-${idx}`;
        return {
          ...installer,
          marketplace_id: marketplaceId,
          marketplace_name: marketplaceName,
          marketplace_enabled: marketplace.enabled !== false,
          _key: `${marketplaceId}::${installerId}::${installUrl}`,
        } as SkillMarketplaceInstallerRow;
      });
    });
  }, [marketplaces]);
  const marketplaceInstallerKeySet = useMemo(
    () => new Set(selectedMarketplaceInstallerKeys),
    [selectedMarketplaceInstallerKeys],
  );
  const filteredMarketplaceInstallers = useMemo(() => {
    const query = marketplaceSearch.trim().toLowerCase();
    if (!query) return marketplaceInstallers;
    return marketplaceInstallers.filter((installer) => {
      const haystack = [
        str(installer.name),
        str(installer.description),
        str(installer.category),
        str(installer.marketplace_name),
        str(installer.install_url),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [marketplaceInstallers, marketplaceSearch]);
  const selectedMarketplaceInstallerUrls = marketplaceInstallers
    .filter(
      (installer) =>
        installer.marketplace_enabled !== false &&
        marketplaceInstallerKeySet.has(str(installer._key)),
    )
    .map((installer) => str(installer.install_url, "").trim())
    .filter(Boolean);

  useEffect(() => {
    const availableKeys = new Set(
      marketplaceInstallers
        .filter(
          (installer) =>
            installer.marketplace_enabled !== false &&
            str(installer.install_url, "").trim(),
        )
        .map((installer) => str(installer._key, "")),
    );
    setSelectedMarketplaceInstallerKeys((prev) =>
      {
        const next = prev.filter((key) => availableKeys.has(key));
        if (
          next.length === prev.length &&
          next.every((key, idx) => key === prev[idx])
        ) {
          return prev;
        }
        return next;
      },
    );
  }, [marketplaceInstallers]);

  const openBulkImportWithUrls = (urls: string[], label?: string) => {
    const uniqueUrls = dedupeStrings(
      urls.map((url) => url.trim()).filter(Boolean),
    );
    if (!uniqueUrls.length) {
      setLastImport({
        result: {
          status: "error",
          name: "marketplace",
          message: "No installable marketplace URLs selected.",
        },
        message: "No installable marketplace URLs selected.",
      });
      return;
    }
    setBulkInitialUrls(uniqueUrls);
    setBulkSourceLabel(label);
    setBulkOpen(true);
  };

  const openMarketplaceDialog = (marketplace?: JsonRecord) => {
    setMarketplaceError(null);
    if (marketplace) {
      setMarketplaceEditingId(str(marketplace.id, "").trim());
      setMarketplaceForm({
        id: str(marketplace.id, ""),
        name: str(marketplace.name, ""),
        url: str(marketplace.url, ""),
        enabled: marketplace.enabled !== false,
      });
    } else {
      setMarketplaceEditingId(null);
      setMarketplaceForm(EMPTY_SKILL_MARKETPLACE_FORM);
    }
    setMarketplaceDialogOpen(true);
  };

  const closeMarketplaceDialog = () => {
    setMarketplaceDialogOpen(false);
    setMarketplaceEditingId(null);
    setMarketplaceForm(EMPTY_SKILL_MARKETPLACE_FORM);
    setMarketplaceError(null);
  };

  const createMarketplaceMutation = useMutation({
    mutationFn: (payload: SkillMarketplaceForm) =>
      api.rawPost("/skills/marketplaces", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-marketplaces"] });
    },
  });
  const updateMarketplaceMutation = useMutation({
    mutationFn: (payload: SkillMarketplaceForm) =>
      api.rawPut(
        `/skills/marketplaces/${encodeURIComponent(marketplaceEditingId || payload.id)}`,
        payload,
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-marketplaces"] });
    },
  });
  const deleteMarketplaceMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/skills/marketplaces/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-marketplaces"] });
    },
  });
  const refreshMarketplaceMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawPost(`/skills/marketplaces/${encodeURIComponent(id)}/refresh`, {}),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-marketplaces"] });
    },
  });

  const saveMarketplace = async () => {
    setMarketplaceError(null);
    const payload = {
      ...marketplaceForm,
      id: marketplaceForm.id.trim(),
      name: marketplaceForm.name.trim(),
      url: marketplaceForm.url.trim(),
    };
    if (!payload.url) {
      setMarketplaceError("Marketplace URL is required.");
      return;
    }
    try {
      if (marketplaceEditingId) {
        await updateMarketplaceMutation.mutateAsync(payload);
      } else {
        await createMarketplaceMutation.mutateAsync(payload);
      }
      closeMarketplaceDialog();
    } catch (err) {
      setMarketplaceError(errMessage(err));
    }
  };

  const setEnabledMutation = useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) =>
      api.setSkillEnabled(name, enabled),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
    },
  });

  const runSkillTestRequest = async (
    skill: JsonRecord,
    argumentsPayload?: Record<string, unknown>,
  ) => {
    const name = str(skill.name, "").trim();
    if (!name) return;

    const runSeq = ++skillTestRunSeqRef.current;
    const previousRunId = activeSkillTestRunIdRef.current;
    if (previousRunId) {
      void api.cancelSkillTest(previousRunId).catch(() => undefined);
    }
    skillTestAbortRef.current?.abort();
    const runId = createSkillTestRunId();
    activeSkillTestRunIdRef.current = runId;
    const abortController = new AbortController();
    skillTestAbortRef.current = abortController;

    setTestRunDialog({
      name,
      skill,
      phase: "running",
      message: "Running skill test...",
      output: "",
      inputFields: [],
      inputValues: {},
      inputError: null,
    });
    setSkillTestStatus(name, "Running skill test...");

    try {
      const out = await api.testSkill(
        name,
        argumentsPayload,
        {
          signal: abortController.signal,
        },
        runId,
      );
      if (
        runSeq !== skillTestRunSeqRef.current ||
        abortController.signal.aborted
      ) {
        return;
      }
      skillTestAbortRef.current = null;
      activeSkillTestRunIdRef.current = null;

      if (out.status === "needs_input") {
        const fields = stringsFromArray(out.required_inputs).length
          ? stringsFromArray(out.required_inputs)
          : stringsFromArray(out.missing_inputs);
        if (fields.length > 0) {
          setTestRunDialog({
            name,
            skill,
            phase: "waiting_input",
            message:
              out.message || "This skill needs input before the test can run.",
            output: "",
            inputFields: fields,
            inputValues: initialSkillTestInputValues(fields),
            inputError: null,
          });
          setSkillTestStatus(name, "Waiting for test input.");
          return;
        }
      }

      if (out.status === "ok") {
        const output = skillTestOutputText(out) || "No output returned.";
        const message = skillTestCompletionMessage(out);
        setTestRunDialog({
          name,
          skill,
          phase: "completed",
          message,
          output,
          inputFields: [],
          inputValues: {},
          inputError: null,
        });
        setSkillTestStatus(name, message);
        return;
      }

      const message = skillTestCompletionMessage(out);
      setTestRunDialog({
        name,
        skill,
        phase: "error",
        message,
        output: skillTestOutputText(out),
        inputFields: [],
        inputValues: {},
        inputError: null,
      });
      setSkillTestStatus(name, message, "error");
    } catch (err) {
      if (runSeq !== skillTestRunSeqRef.current) return;
      skillTestAbortRef.current = null;
      activeSkillTestRunIdRef.current = null;
      if (isAbortError(err)) {
        setSkillTestStatus(name, "Skill test cancelled.");
        updateSkillTestDialog(name, (current) => ({
          ...current,
          phase: "cancelled",
          message: "Skill test cancelled.",
        }));
        return;
      }
      const message = errMessage(err);
      setTestRunDialog({
        name,
        skill,
        phase: "error",
        message,
        output: "",
        inputFields: [],
        inputValues: {},
        inputError: null,
      });
      setSkillTestStatus(name, message, "error");
    }
  };

  const deleteSkillMutation = useMutation({
    mutationFn: (name: string) => api.deleteSkill(name),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
    },
  });

  const addHookMutation = useMutation({
    mutationFn: (payload: {
      name: string;
      trigger: HookTriggerValue;
      hook_type: string;
      url?: string;
      action_name?: string;
    }) => api.rawPost("/hooks", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-hooks"] });
      await queryClient.invalidateQueries({ queryKey: ["skills-hook-runs"] });
    },
  });

  const removeHookMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/hooks/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["skills-hooks"] });
      await queryClient.invalidateQueries({ queryKey: ["skills-hook-runs"] });
    },
  });

  const hooksForSelectedAction = hooksTargetAction
    ? hooks.filter((h) => isHookRecordAttachedToAction(h, hooksTargetAction))
    : hooks;
  const skillSearchFilter = (a: JsonRecord) => {
    if (!skillSearch.trim()) return true;
    const q = skillSearch.toLowerCase();
    return (
      str(a.name, "").toLowerCase().includes(q) ||
      str(a.description, "").toLowerCase().includes(q)
    );
  };
  const skillSortFn = (a: JsonRecord, b: JsonRecord) => {
    if (skillSort === "imported") {
      const ta = str(a.imported_at, "");
      const tb = str(b.imported_at, "");
      if (tb && ta) return tb.localeCompare(ta);
      if (tb) return 1;
      if (ta) return -1;
      return str(a.name, "").localeCompare(str(b.name, ""));
    }
    return str(a.name, "").localeCompare(str(b.name, ""));
  };
  const systemSkills = skills
    .filter((a) => str(a.source).toLowerCase() === "system")
    .filter(skillSearchFilter)
    .sort(skillSortFn);
  const customSkills = skills
    .filter((a) => str(a.source).toLowerCase() === "custom")
    .filter(skillSearchFilter)
    .sort(skillSortFn);
  const availableToolNames = dedupeStrings(
    systemSkills.map((a) => str(a.name, "").trim()).filter(Boolean),
  );
  const allSkillNames = dedupeStrings(
    skills.map((a) => str(a.name, "").trim()).filter(Boolean),
  );
  const hookLastRunById = useMemo(() => {
    const map: Record<string, JsonRecord> = {};
    for (const run of hookRuns) {
      const id = str(run.hook_id, "");
      if (!id || map[id]) continue;
      map[id] = run;
    }
    return map;
  }, [hookRuns]);

  const closeEditor = () => {
    setEditOpen(false);
    setEditTargetName(null);
    setEditForm(defaultSkillEditorForm());
    setEditContent("");
    setEditError(null);
    setEditLoading(false);
    setCreateWizardEnabled(true);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
  };

  const closeHooksDialog = () => {
    setHooksOpen(false);
    setHooksTargetAction(null);
    setHookInstruction("");
    setHookName("");
    setHookTrigger("post_action");
    setHookUrl("");
    setHookError(null);
  };

  const openHooksDialog = (actionName?: string) => {
    const target = actionName?.trim() || null;
    const baseName = target ? "hook" : "custom-hook";
    setHooksTargetAction(target);
    setHookInstruction(target ? `notify me when ${target} fails` : "");
    setHookTrigger(target ? "on_error" : "post_action");
    setHookName(baseName);
    setHookUrl("");
    setHookError(null);
    setHooksOpen(true);
  };

  const applyHookInstruction = () => {
    const trigger = inferHookTriggerFromInstruction(
      hookInstruction,
      hooksTargetAction ? "on_error" : "post_action",
    );
    const extractedUrl = extractFirstUrl(hookInstruction);
    const actionPart = hooksTargetAction ? "" : "custom-";
    const triggerPart = trigger.replace(/_/g, "-");
    const suggestedName =
      sanitizeHookName(`${actionPart}${triggerPart}`) || "custom-hook";
    setHookTrigger(trigger);
    if (!hookName.trim()) setHookName(suggestedName);
    if (extractedUrl && !hookUrl.trim()) setHookUrl(extractedUrl);
  };

  const applyEditHookInstruction = () => {
    const trigger = inferHookTriggerFromInstruction(
      editHookInstruction,
      "on_error",
    );
    const extractedUrl = extractFirstUrl(editHookInstruction);
    setEditHookTrigger(trigger);
    if (extractedUrl && !editHookUrl.trim()) setEditHookUrl(extractedUrl);
  };

  const applyEditTaskInstruction = () => {
    const inferredCron = inferTaskCronFromInstruction(editTaskInstruction);
    if (inferredCron && !editTaskCron.trim()) setEditTaskCron(inferredCron);
  };

  const saveHookFromDialog = async () => {
    setHookError(null);
    try {
      const effectiveTrigger = inferHookTriggerFromInstruction(
        hookInstruction,
        hookTrigger,
      );
      const effectiveUrl = hookUrl.trim() || extractFirstUrl(hookInstruction);
      if (!effectiveUrl) {
        setHookError("Send update URL is required.");
        return;
      }
      const rawName = sanitizeHookName(hookName) || "hook";
      const finalName = hooksTargetAction
        ? isHookAttachedToAction(rawName, hooksTargetAction)
          ? rawName
          : sanitizeHookName(`action-${hooksTargetAction}-${rawName}`)
        : rawName;
      await addHookMutation.mutateAsync({
        name: finalName,
        trigger: effectiveTrigger,
        hook_type: "webhook",
        url: effectiveUrl,
        action_name: hooksTargetAction || undefined,
      });
      closeHooksDialog();
    } catch (err) {
      setHookError(errMessage(err));
    }
  };

  const openEditor = async (name: string) => {
    setEditError(null);
    setEditLoading(true);
    setEditTargetName(name);
    setEditForm(defaultSkillEditorForm(name));
    setEditContent("");
    setEditOpen(true);
    setCreateWizardEnabled(false);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
    try {
      const loadDetails = async () => {
        const out = (await api.rawGet(
          `/skills/${encodeURIComponent(name)}`,
        )) as JsonRecord;
        const content = str(out.content, "");
        const parsed = parseSkillEditorForm(content, name);
        setEditContent(content);
        setEditForm({ ...parsed, name });
      };

      try {
        await loadDetails();
      } catch (err) {
        const message = errMessage(err);
        if (message.toLowerCase().includes("rate limit")) {
          await new Promise((resolve) => setTimeout(resolve, 1200));
          await loadDetails();
        } else {
          throw err;
        }
      }
    } catch (err) {
      const message = errMessage(err);
      if (message.toLowerCase().includes("rate limit")) {
        setEditError(
          "Skill details are temporarily rate-limited. Wait a few seconds and reopen the editor.",
        );
      } else {
        setEditError(message);
      }
    } finally {
      setEditLoading(false);
    }
  };

  const openNewEditor = (initial?: { name?: string; content?: string }) => {
    const initialName =
      normalizeActionName(initial?.name || "new-action") || "new-action";
    const initialContent = (initial?.content || "").trim();
    const parsed = initialContent
      ? parseSkillEditorForm(initialContent, initialName)
      : defaultSkillEditorForm(initialName);
    const normalizedName =
      normalizeActionName(parsed.name || initialName) || "new-action";
    const form = { ...parsed, name: normalizedName };
    const content = initialContent || buildSkillMdFromForm("", form);
    setEditTargetName(null);
    setEditForm(form);
    setEditContent(content);
    setEditError(null);
    setCreateWizardEnabled(true);
    setCreateWizardStep(0);
    setEditAttachHook(false);
    setEditHookInstruction("");
    setEditHookTrigger("on_error");
    setEditHookUrl("");
    setEditAttachTask(false);
    setEditTaskInstruction("");
    setEditTaskCron("");
    setEditOpen(true);
  };

  const aiGenerateMutation = useMutation({
    mutationFn: async ({
      prompt,
      nameHint,
    }: {
      prompt: string;
      nameHint: string;
    }) => {
      const fallbackName =
        normalizeActionName(nameHint || "new-action") || "new-action";
      const toolsText =
        availableToolNames.length > 0
          ? availableToolNames.join(", ")
          : "web_search";
      const existingText =
        allSkillNames.length > 0 ? allSkillNames.join(", ") : "(none)";
      const generationPrompt = [
        "Create a complete SKILL.md for AgentArk.",
        "",
        "Return ONLY the SKILL.md content. No explanation, no markdown fences.",
        "The file must use YAML frontmatter exactly with keys: name, description, version, required_inputs, metadata.emoji, requires.tools.",
        'Use version "1.0.0".',
        "Skill name must be lowercase letters, numbers, and hyphens only.",
        `Name hint: ${fallbackName}`,
        `Available tool skills to reference in workflow guidance: ${toolsText}`,
        `Existing skill names (avoid collisions): ${existingText}`,
        "",
        "Task request:",
        prompt.trim(),
      ].join("\n");

      const out = (await api.chat({
        message: generationPrompt,
        channel: "web",
      })) as JsonRecord;
      const raw = str(out.response, "");
      const actionMd = extractActionMdFromModelOutput(raw);
      if (!actionMd.trim()) throw new Error("AI did not return skill content.");
      return { actionMd, fallbackName };
    },
    onSuccess: ({ actionMd, fallbackName }) => {
      const parsed = parseSkillEditorForm(actionMd, fallbackName);
      const normalizedName =
        normalizeActionName(parsed.name || fallbackName) || "new-action";
      const normalizedForm = { ...parsed, name: normalizedName };
      const normalizedContent = buildSkillMdFromForm(actionMd, normalizedForm);
      setAiError(null);
      setAiCreateOpen(false);
      setAiPrompt("");
      setAiNameHint("");
      openNewEditor({ name: normalizedName, content: normalizedContent });
    },
    onError: (err) => {
      setAiError(errMessage(err));
    },
  });

  const saveEditor = async () => {
    setEditError(null);
    try {
      const createMode = !editTargetName;
      let targetName = editTargetName || normalizeActionName(editForm.name);
      if (createMode && editRawMode) {
        const parsed = parseSkillEditorForm(
          editContent,
          targetName || "new-action",
        );
        const parsedName = normalizeActionName(parsed.name);
        if (parsedName) targetName = parsedName;
      }
      if (!targetName) targetName = "new-action";

      if (createMode && !isValidActionName(targetName)) {
        setEditError(
          "Skill name must use lowercase letters, numbers, and hyphens only.",
        );
        return;
      }

      const formForSave: SkillEditorForm = {
        ...editForm,
        name: targetName,
        version: (editForm.version || "").trim() || "1.0.0",
      };
      const finalContent = editRawMode
        ? editContent
        : buildSkillMdFromForm(editContent, formForSave);

      if (createMode) {
        const out = (await api.rawPost("/skills", {
          name: targetName,
          content: finalContent,
          force: false,
        })) as JsonRecord;
        const status = str(out.status, "ok").toLowerCase();
        if (status === "blocked") {
          setEditError(
            str(
              out.error,
              str(out.message, "Skill was blocked by security verification."),
            ),
          );
          return;
        }
      } else {
        await api.rawPost(`/skills/${encodeURIComponent(targetName)}`, {
          content: finalContent,
        });
      }

      const editEffectiveUrl =
        editHookUrl.trim() || extractFirstUrl(editHookInstruction);
      if (editAttachHook && editEffectiveUrl) {
        const hookBase =
          sanitizeHookName(
            inferHookTriggerFromInstruction(
              editHookInstruction,
              editHookTrigger,
            ).replace(/_/g, "-"),
          ) || "hook";
        const hookName =
          sanitizeHookName(`action-${targetName}-${hookBase}`) ||
          `action-${targetName}-hook`;
        await addHookMutation.mutateAsync({
          name: hookName,
          trigger: inferHookTriggerFromInstruction(
            editHookInstruction,
            editHookTrigger,
          ),
          hook_type: "webhook",
          url: editEffectiveUrl,
          action_name: targetName,
        });
      }

      if (editAttachTask) {
        const inferredCron = inferTaskCronFromInstruction(editTaskInstruction);
        const effectiveCron = editTaskCron.trim() || inferredCron;
        const runOnce = isRunOnceInstruction(editTaskInstruction);
        if (!effectiveCron && !runOnce) {
          setEditError(
            "Could not understand schedule. Try: every day at 9am, hourly, weekdays, or paste a cron.",
          );
          return;
        }
        await api.rawPost("/tasks", {
          description: `Run skill '${targetName}' automatically`,
          action: targetName,
          arguments: {},
          cron: runOnce ? null : effectiveCron,
          approval: "auto",
        });
      }

      closeEditor();
      await queryClient.invalidateQueries({ queryKey: ["skills-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    } catch (err) {
      setEditError(errMessage(err));
    }
  };

  const toggleEnabled = async (name: string, nextEnabled: boolean) => {
    if (nextEnabled) {
      try {
        const secrets = await api.getSkillSecrets(name);
        if ((secrets.missing_env || []).length > 0) {
          setLastImport({
            result: {
              status: "needs_secrets",
              name,
              message: "Missing secrets",
              secrets: {
                missing_env: secrets.missing_env,
                required_env: secrets.required_env,
                bindings: secrets.bindings,
              },
            },
            message: `Cannot enable '${name}' until secrets are configured: ${secrets.missing_env.join(", ")}`,
          });
          setSecretsName(name);
          return;
        }
      } catch (err) {
        setLastImport({
          result: { status: "error", name, message: "Secrets check failed" },
          message: `Cannot enable '${name}': ${errMessage(err)}`,
        });
        return;
      }
    }
    await setEnabledMutation.mutateAsync({ name, enabled: nextEnabled });
  };

  const startSkillTest = async (skill: JsonRecord) => {
    const name = str(skill.name, "").trim();
    if (!name || activeTestName) return;

    const runSeq = ++skillTestRunSeqRef.current;
    skillTestAbortRef.current?.abort();
    const abortController = new AbortController();
    skillTestAbortRef.current = abortController;

    setTestRunDialog({
      name,
      skill,
      phase: "checking",
      message: "Checking test requirements...",
      output: "",
      inputFields: [],
      inputValues: {},
      inputError: null,
    });
    setSkillTestStatus(name, "Checking test requirements...");

    try {
      const secrets = await api.getSkillSecrets(name, {
        signal: abortController.signal,
      });
      if (
        runSeq !== skillTestRunSeqRef.current ||
        abortController.signal.aborted
      ) {
        return;
      }
      skillTestAbortRef.current = null;

      if ((secrets.missing_env || []).length > 0) {
        const message = `Configure missing secrets first: ${secrets.missing_env.join(", ")}`;
        setTestRunDialog({
          name,
          skill,
          phase: "error",
          message,
          output: "",
          inputFields: [],
          inputValues: {},
          inputError: null,
        });
        setSkillTestStatus(name, message, "error");
        setSecretsName(name);
        return;
      }
    } catch (err) {
      if (runSeq !== skillTestRunSeqRef.current) return;
      skillTestAbortRef.current = null;
      if (isAbortError(err)) {
        setSkillTestStatus(name, "Skill test cancelled.");
        updateSkillTestDialog(name, (current) => ({
          ...current,
          phase: "cancelled",
          message: "Skill test cancelled.",
        }));
        return;
      }
      const message = errMessage(err);
      setTestRunDialog({
        name,
        skill,
        phase: "error",
        message,
        output: "",
        inputFields: [],
        inputValues: {},
        inputError: null,
      });
      setSkillTestStatus(name, message, "error");
      return;
    }

    const fields = skillRequiredInputNames(skill);
    if (fields.length > 0) {
      setTestRunDialog({
        name,
        skill,
        phase: "waiting_input",
        message: "Provide the required inputs for this skill test run.",
        output: "",
        inputFields: fields,
        inputValues: initialSkillTestInputValues(fields),
        inputError: null,
      });
      setSkillTestStatus(name, "Waiting for test input.");
      return;
    }

    await runSkillTestRequest(skill);
  };

  const runSkillTestFromInputDialog = () => {
    if (!testRunDialog || testRunDialog.phase !== "waiting_input") return;
    const { name, skill, inputFields, inputValues } = testRunDialog;
    const missing = inputFields.filter(
      (field) => !(inputValues[field] || "").trim(),
    );
    if (!name) return;
    if (missing.length > 0) {
      setTestRunDialog((current) =>
        current && current.name === name
          ? {
              ...current,
              inputError: `Required input missing: ${missing.join(", ")}`,
            }
          : current,
      );
      return;
    }

    const argumentsPayload: Record<string, unknown> = {};
    for (const field of inputFields) {
      argumentsPayload[field] = (inputValues[field] || "").trim();
    }
    void runSkillTestRequest(skill, argumentsPayload);
  };

  const closeSkillTestDialog = () => {
    const current = testRunDialog;
    if (!current) return;
    if (isSkillTestRunActive(current.phase)) {
      skillTestRunSeqRef.current += 1;
      const runId = activeSkillTestRunIdRef.current;
      if (runId) {
        void api.cancelSkillTest(runId).catch(() => undefined);
      }
      skillTestAbortRef.current?.abort();
      skillTestAbortRef.current = null;
      activeSkillTestRunIdRef.current = null;
      setSkillTestStatus(current.name, "Skill test cancelled.");
    }
    setTestRunDialog(null);
  };

  const renderActionRow = (action: JsonRecord, type: "system" | "custom") => {
    const name = str(action.name, "Untitled");
    const description = str(action.description, "No description");
    const version = str(action.version, "?");
    const enabled = toBool(action.enabled);
    const testMessage = testResults[name];
    const isTesting = activeTestName === name;
    const testMenuLabel =
      testRunDialog?.name === name && testRunDialog.phase === "waiting_input"
        ? "Test input open"
        : isTesting
          ? "Testing..."
          : "Run test";
    const isSystem = type === "system";

    const menuOpen = skillMenuAnchor?.name === name;

    return (
      <Box
        key={`${type}-${name}`}
        className="action-row"
        sx={{
          width: "100%",
          opacity: isSystem ? 0.7 : 1,
          filter: isSystem ? "saturate(0.85)" : "none",
        }}
      >
        <Stack
          direction="row"
          spacing={2}
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <Stack spacing={0.5} sx={{ flex: 1, minWidth: 0 }}>
            <Stack
              direction="row"
              spacing={1}
              sx={{
                alignItems: "center",
              }}
            >
              <Typography
                variant="subtitle1"
                noWrap
                sx={{
                  fontWeight: 600,
                }}
              >
                {name}
              </Typography>
              {!enabled && !isSystem ? (
                <Chip
                  label="Disabled"
                  size="small"
                  color="warning"
                  variant="outlined"
                  sx={{ height: 20, fontSize: "0.65rem" }}
                />
              ) : null}
            </Stack>
            <Typography
              variant="caption"
              component="div"
              noWrap
              sx={{
                color: "text.secondary",
              }}
            >
              {description}
            </Typography>
            {testMessage ? (
              <Typography
                variant="caption"
                component="div"
                aria-live="polite"
                sx={{
                  color:
                    testResultTones[name] === "error"
                      ? "error.main"
                      : "text.secondary",
                  display: "block",
                }}
              >
                {testMessage}
              </Typography>
            ) : null}
          </Stack>
          <Stack
            direction="row"
            spacing={0.5}
            sx={{
              alignItems: "center",
            }}
          >
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                whiteSpace: "nowrap",
              }}
            >
              v{version}
            </Typography>
            {!isSystem ? (
              <>
                <IconButton
                  size="small"
                  onClick={(e: MouseEvent<HTMLButtonElement>) =>
                    setSkillMenuAnchor({ el: e.currentTarget, name })
                  }
                >
                  <MoreVertIcon fontSize="small" />
                </IconButton>
                <Menu
                  anchorEl={menuOpen ? skillMenuAnchor.el : null}
                  open={menuOpen}
                  onClose={() => setSkillMenuAnchor(null)}
                  slotProps={{ paper: { sx: { minWidth: 160 } } }}
                >
                  <MenuItem
                    onClick={() => {
                      setSkillMenuAnchor(null);
                      openEditor(name);
                    }}
                  >
                    Edit
                  </MenuItem>
                  <MenuItem
                    onClick={() => {
                      setSkillMenuAnchor(null);
                      setSecretsName(name);
                    }}
                  >
                    Secrets
                  </MenuItem>
                  <MenuItem
                    disabled={isTesting}
                    onClick={() => {
                      setSkillMenuAnchor(null);
                      void startSkillTest(action);
                    }}
                  >
                    {testMenuLabel}
                  </MenuItem>
                  <MenuItem
                    disabled={setEnabledMutation.isPending}
                    onClick={() => {
                      setSkillMenuAnchor(null);
                      toggleEnabled(name, !enabled);
                    }}
                  >
                    {enabled ? "Disable" : "Enable"}
                  </MenuItem>
                  {developerModeEnabled ? (
                    <MenuItem
                      onClick={() => {
                        setSkillMenuAnchor(null);
                        openHooksDialog(name);
                      }}
                    >
                      Automations
                    </MenuItem>
                  ) : null}
                  <Divider />
                  <MenuItem
                    disabled={deleteSkillMutation.isPending}
                    sx={{ color: "error.main" }}
                    onClick={async () => {
                      setSkillMenuAnchor(null);
                      const ok = window.confirm(
                        `Delete skill "${name}"? This cannot be undone.`,
                      );
                      if (ok) deleteSkillMutation.mutate(name);
                    }}
                  >
                    Delete
                  </MenuItem>
                </Menu>
              </>
            ) : null}
          </Stack>
        </Stack>
      </Box>
    );
  };

  const isCreateMode = !editTargetName;
  const useCreateWizard = isCreateMode && !editRawMode && createWizardEnabled;
  const scheduleInference =
    editTaskCron.trim() || inferTaskCronFromInstruction(editTaskInstruction);
  const scheduleBlocked =
    editAttachTask &&
    !scheduleInference &&
    !isRunOnceInstruction(editTaskInstruction);
  const hookBlocked =
    editAttachHook &&
    !(editHookUrl.trim() || extractFirstUrl(editHookInstruction));
  const wizardStepBlocked =
    createWizardStep === 0
      ? !editForm.name.trim() ||
        !isValidActionName(editForm.name) ||
        !editForm.description.trim()
      : createWizardStep === 2
        ? hookBlocked || scheduleBlocked
        : false;

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Agent"
        title="Skills"
        description="Create, import, and manage reusable abilities available to the AgentArk OS."
        actions={
          skillsTab === "manage" ? (
            <Stack
              direction="row"
              spacing={0.75}
              useFlexGap
              sx={[
                {
                  flexWrap: "wrap",
                  alignItems: "center",
                },
                ...(Array.isArray(WORKSPACE_HEADER_ACTION_GROUP_SX)
                  ? WORKSPACE_HEADER_ACTION_GROUP_SX
                  : [WORKSPACE_HEADER_ACTION_GROUP_SX]),
              ]}
            >
              <Button
                size="small"
                variant="contained"
                sx={WORKSPACE_HEADER_PRIMARY_BUTTON_SX}
                onClick={() => setAiCreateOpen(true)}
              >
                Create Skill
              </Button>
              <Button
                size="small"
                variant="contained"
                sx={WORKSPACE_HEADER_PRIMARY_BUTTON_SX}
                onClick={() => setImportOpen(true)}
              >
                Import URL
              </Button>
              <Button
                size="small"
                variant="contained"
                sx={WORKSPACE_HEADER_PRIMARY_BUTTON_SX}
                onClick={() => openMarketplaceDialog()}
              >
                Add Marketplace
              </Button>
              <Button
                size="small"
                variant="contained"
                sx={WORKSPACE_HEADER_PRIMARY_BUTTON_SX}
                onClick={() => {
                  setBulkInitialUrls([]);
                  setBulkSourceLabel(undefined);
                  setBulkOpen(true);
                }}
              >
                Bulk Import
              </Button>
            </Stack>
          ) : null
        }
      />
      <Box className="list-shell">
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
          }}
        >
          Start with AI Quick Create for new skills, then drop into the editor
          only when you need manual SKILL.md control.
        </Typography>
        <Typography
          variant="caption"
          sx={{
            color: "text.secondary",
            display: "block",
            mt: 0.5,
          }}
        >
          System skills: {systemSkills.length}, custom skills:{" "}
          {customSkills.length}, automations: {hooks.length}.
        </Typography>
        {skillsTab === "manage" ? (
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
              display: "block",
              mt: 0.5,
            }}
          >
            Create and manage user skills here.
          </Typography>
        ) : null}
        {lastImport?.message ? (
          <Alert
            sx={{ mt: 1 }}
            severity={
              lastImport.result.status === "blocked" ? "warning" : "info"
            }
          >
            {lastImport.message}
          </Alert>
        ) : null}
        <Tabs
          value={skillsTab}
          onChange={(_, value: "manage" | "system") => setSkillsTab(value)}
          className="workspace-page-subnav-tabs"
          sx={{ mt: 1 }}
        >
          <Tab value="manage" label="My Skills" />
          <Tab value="system" label="System Skills" />
        </Tabs>
        <Stack
          direction="row"
          spacing={1}
          sx={{
            alignItems: "center",
            mt: 1.5,
          }}
        >
          <TextField
            size="small"
            placeholder="Search skills by name or description..."
            value={skillSearch}
            onChange={(e) => setSkillSearch(e.target.value)}
            sx={{ flex: 1 }}
            slotProps={{ input: { sx: { fontSize: "0.85rem" } } }}
          />
          <TextField
            select
            size="small"
            value={skillSort}
            onChange={(e) =>
              setSkillSort(e.target.value as "name" | "imported")
            }
            sx={{ minWidth: 140 }}
            slotProps={{ input: { sx: { fontSize: "0.85rem" } } }}
          >
            <MenuItem value="name">Sort: Name</MenuItem>
            <MenuItem value="imported">Sort: Newest</MenuItem>
          </TextField>
        </Stack>
      </Box>
      {skillsTab === "manage" ? (
        <>
          <Box className="list-shell">
            <Stack spacing={1.25}>
              <Stack
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "stretch", md: "center" },
                }}
              >
                <Stack spacing={0.25}>
                  <Typography variant="h6">Marketplaces</Typography>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    Add marketplace catalogs, refresh installers, then review
                    selected installers through security analysis before import.
                  </Typography>
                </Stack>
                <Stack direction="row" spacing={0.75} sx={{ flexWrap: "wrap" }}>
                  <Button
                    size="small"
                    variant="outlined"
                    onClick={() => openMarketplaceDialog()}
                  >
                    Add
                  </Button>
                  <Button
                    size="small"
                    variant="contained"
                    disabled={selectedMarketplaceInstallerUrls.length === 0}
                    onClick={() =>
                      openBulkImportWithUrls(
                        selectedMarketplaceInstallerUrls,
                        "marketplace selection",
                      )
                    }
                  >
                    Review Selected ({selectedMarketplaceInstallerUrls.length})
                  </Button>
                </Stack>
              </Stack>
              {marketplacesQ.error ? (
                <Alert severity="error">{errMessage(marketplacesQ.error)}</Alert>
              ) : marketplaces.length === 0 ? (
                <Typography
                  variant="body2"
                  sx={{
                    color: "text.secondary",
                  }}
                >
                  No marketplaces configured.
                </Typography>
              ) : (
                <TableContainer className="table-shell">
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        <TableCell>Marketplace</TableCell>
                        <TableCell>Source</TableCell>
                        <TableCell>Installers</TableCell>
                        <TableCell>Status</TableCell>
                        <TableCell align="right">Ops</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {marketplaces.map((marketplace) => {
                        const id = str(marketplace.id, "");
                        const lastError = str(marketplace.last_error, "");
                        return (
                          <TableRow key={id}>
                            <TableCell sx={{ maxWidth: 220 }}>
                              <Typography variant="body2" sx={{ fontWeight: 700 }}>
                                {str(marketplace.name, id)}
                              </Typography>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {marketplace.enabled === false
                                  ? "Disabled"
                                  : "Enabled"}
                              </Typography>
                            </TableCell>
                            <TableCell sx={{ maxWidth: 360 }}>
                              <Typography
                                variant="caption"
                                noWrap
                                title={str(marketplace.url, "")}
                                sx={{
                                  color: "text.secondary",
                                  display: "block",
                                }}
                              >
                                {str(marketplace.url, "-")}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              {asRecords(marketplace.installers).length}
                            </TableCell>
                            <TableCell sx={{ maxWidth: 260 }}>
                              {lastError ? (
                                <Typography
                                  variant="caption"
                                  color="error"
                                  title={lastError}
                                  noWrap
                                  sx={{ display: "block" }}
                                >
                                  {lastError}
                                </Typography>
                              ) : (
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  Synced{" "}
                                  {str(marketplace.last_synced_at, "never")}
                                </Typography>
                              )}
                            </TableCell>
                            <TableCell align="right">
                              <RowOpsMenu
                                actions={[
                                  {
                                    label: "Refresh",
                                    disabled: refreshMarketplaceMutation.isPending,
                                    onClick: async () => {
                                      try {
                                        await refreshMarketplaceMutation.mutateAsync(id);
                                      } catch (err) {
                                        setLastImport({
                                          result: {
                                            status: "error",
                                            name: str(marketplace.name, id),
                                            message: errMessage(err),
                                          },
                                          message: `Failed to refresh marketplace '${str(marketplace.name, id)}': ${errMessage(err)}`,
                                        });
                                      }
                                    },
                                  },
                                  {
                                    label: "Edit",
                                    onClick: () => openMarketplaceDialog(marketplace),
                                  },
                                  {
                                    label: "Delete",
                                    tone: "error",
                                    disabled: deleteMarketplaceMutation.isPending,
                                    onClick: async () => {
                                      const ok = window.confirm(
                                        `Delete marketplace "${str(marketplace.name, id)}"?`,
                                      );
                                      if (!ok) return;
                                      try {
                                        await deleteMarketplaceMutation.mutateAsync(id);
                                      } catch (err) {
                                        setLastImport({
                                          result: {
                                            status: "error",
                                            name: str(marketplace.name, id),
                                            message: errMessage(err),
                                          },
                                          message: `Failed to delete marketplace '${str(marketplace.name, id)}': ${errMessage(err)}`,
                                        });
                                      }
                                    },
                                  },
                                ]}
                                ariaLabel="Marketplace options"
                              />
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
              )}
              {marketplaceInstallers.length > 0 ? (
                <Stack spacing={1}>
                  <TextField
                    size="small"
                    placeholder="Search marketplace installers..."
                    value={marketplaceSearch}
                    onChange={(event) => setMarketplaceSearch(event.target.value)}
                    slotProps={{ input: { sx: { fontSize: "0.85rem" } } }}
                  />
                  <TableContainer className="table-shell">
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell padding="checkbox">Pull</TableCell>
                          <TableCell>Installer</TableCell>
                          <TableCell>Marketplace</TableCell>
                          <TableCell>Category</TableCell>
                          <TableCell>Source</TableCell>
                          <TableCell align="right">Review</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {filteredMarketplaceInstallers.map((installer) => {
                          const key = str(installer._key, "");
                          const installUrl = str(installer.install_url, "").trim();
                          const marketplaceEnabled =
                            installer.marketplace_enabled !== false;
                          const checked = marketplaceInstallerKeySet.has(key);
                          return (
                            <TableRow key={key}>
                              <TableCell padding="checkbox">
                                <Checkbox
                                  size="small"
                                  checked={checked}
                                  disabled={!installUrl || !marketplaceEnabled}
                                  onChange={(event) => {
                                    const nextChecked = event.target.checked;
                                    setSelectedMarketplaceInstallerKeys((prev) => {
                                      const set = new Set(prev);
                                      if (nextChecked) set.add(key);
                                      else set.delete(key);
                                      return Array.from(set);
                                    });
                                  }}
                                />
                              </TableCell>
                              <TableCell sx={{ maxWidth: 280 }}>
                                <Typography
                                  variant="body2"
                                  sx={{ fontWeight: 700 }}
                                >
                                  {str(installer.name, "-")}
                                </Typography>
                                {str(installer.description, "") ? (
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color: "text.secondary",
                                      display: "block",
                                    }}
                                  >
                                    {str(installer.description, "")}
                                  </Typography>
                                ) : null}
                              </TableCell>
                              <TableCell>
                                <Typography variant="caption">
                                  {str(installer.marketplace_name, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell>
                                <Typography variant="caption">
                                  {str(installer.category, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell sx={{ maxWidth: 360 }}>
                                <Typography
                                  variant="caption"
                                  noWrap
                                  title={installUrl || str(installer.source_url, "")}
                                  sx={{
                                    color: installUrl
                                      ? "text.secondary"
                                      : "warning.main",
                                    display: "block",
                                  }}
                                >
                                  {installUrl || "No supported install URL"}
                                </Typography>
                              </TableCell>
                              <TableCell align="right">
                                <Button
                                  size="small"
                                  variant="outlined"
                                  disabled={!installUrl || !marketplaceEnabled}
                                  onClick={() =>
                                    openBulkImportWithUrls(
                                      [installUrl],
                                      str(installer.name, "marketplace installer"),
                                    )
                                  }
                                >
                                  Review
                                </Button>
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                </Stack>
              ) : null}
            </Stack>
          </Box>

          {customSkills.length > 0 ? (
            <Box className="list-shell">
              <Stack spacing={1}>
                <Typography variant="h6">Custom Skills</Typography>
                <Stack spacing={1}>
                  {customSkills.map((act) => renderActionRow(act, "custom"))}
                </Stack>
              </Stack>
            </Box>
          ) : null}

          {developerModeEnabled ? (
            <Box className="list-shell">
              <Stack spacing={1}>
                <Stack
                  direction="row"
                  sx={{
                    justifyContent: "space-between",
                    alignItems: "center",
                  }}
                >
                  <Stack spacing={0.25}>
                    <Typography variant="h6">Automations</Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Advanced automation manager (Developer mode). Create from
                      an action row.
                    </Typography>
                  </Stack>
                </Stack>
                {hooksQ.error ? (
                  <Alert severity="error">{errMessage(hooksQ.error)}</Alert>
                ) : hookRunsQ.error ? (
                  <Alert severity="warning">
                    Automations loaded, but run reports failed:{" "}
                    {errMessage(hookRunsQ.error)}
                  </Alert>
                ) : hooks.length === 0 ? (
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    No automations yet.
                  </Typography>
                ) : (
                  <TableContainer className="table-shell">
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          <TableCell>Name</TableCell>
                          <TableCell>Trigger</TableCell>
                          <TableCell>Type</TableCell>
                          <TableCell>URL</TableCell>
                          <TableCell>Enabled</TableCell>
                          <TableCell>Last run</TableCell>
                          <TableCell align="right">Ops</TableCell>
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {hooks.map((hook, idx) => {
                          const id = str(hook.id, `hook-${idx}`);
                          const lastRun = hookLastRunById[id];
                          const runStatus = str(lastRun?.status, "-");
                          const runAttempts = num(lastRun?.attempts, 0);
                          const runError = str(lastRun?.error, "");
                          return (
                            <TableRow key={id}>
                              <TableCell>{str(hook.name, "-")}</TableCell>
                              <TableCell>{humanizeStatusLabel(str(hook.trigger, ""), "-")}</TableCell>
                              <TableCell>{humanizeMachineLabel(str(hook.hook_type, ""), "-")}</TableCell>
                              <TableCell sx={{ maxWidth: 280 }}>
                                <Typography
                                  variant="caption"
                                  noWrap
                                  title={str(hook.url, "-")}
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  {str(hook.url, "-")}
                                </Typography>
                              </TableCell>
                              <TableCell>{boolText(hook.enabled)}</TableCell>
                              <TableCell sx={{ maxWidth: 240 }}>
                                {lastRun ? (
                                  <Typography
                                    variant="caption"
                                    color={
                                      runStatus === "failed"
                                        ? "error.main"
                                        : "text.secondary"
                                    }
                                    noWrap
                                    title={
                                      runError || str(lastRun?.timestamp, "")
                                    }
                                  >
                                    {runStatus}
                                    {runAttempts > 0 ? ` (${runAttempts})` : ""}
                                  </Typography>
                                ) : (
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      color: "text.secondary",
                                    }}
                                  >
                                    never
                                  </Typography>
                                )}
                              </TableCell>
                              <TableCell align="right">
                                <RowOpsMenu
                                  actions={[
                                    {
                                      label: "Remove",
                                      tone: "error",
                                      disabled: removeHookMutation.isPending,
                                      onClick: async () => {
                                        const ok = window.confirm(
                                          "Remove this automation?",
                                        );
                                        if (!ok) return;
                                        try {
                                          await removeHookMutation.mutateAsync(
                                            id,
                                          );
                                        } catch (err) {
                                          setLastImport({
                                            result: {
                                              status: "error",
                                              name: str(
                                                hook.name,
                                                "automation",
                                              ),
                                              message: errMessage(err),
                                            },
                                            message: `Failed to remove automation '${str(hook.name, "automation")}': ${errMessage(err)}`,
                                          });
                                        }
                                      },
                                    },
                                  ]}
                                  ariaLabel="Automation options"
                                />
                              </TableCell>
                            </TableRow>
                          );
                        })}
                      </TableBody>
                    </Table>
                  </TableContainer>
                )}
              </Stack>
            </Box>
          ) : null}
        </>
      ) : (
        <Box className="list-shell">
          <Stack spacing={1}>
            <Typography variant="h6">System Skills</Typography>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Built-in and locked. Always available.
            </Typography>
            {systemSkills.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No system skills detected.
              </Typography>
            ) : (
              <Stack spacing={1}>
                {systemSkills.map((act) => renderActionRow(act, "system"))}
              </Stack>
            )}
          </Stack>
        </Box>
      )}
      <Dialog
        open={marketplaceDialogOpen}
        onClose={closeMarketplaceDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {marketplaceEditingId ? "Edit Marketplace" : "Add Marketplace"}
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {marketplaceError ? (
              <Alert severity="error">{marketplaceError}</Alert>
            ) : null}
            <Alert severity="info" variant="outlined">
              Marketplaces only provide installer metadata. Selected installers
              still run through the normal skill security review before import.
            </Alert>
            {!marketplaceEditingId ? (
              <TextField
                fullWidth
                size="small"
                label="Marketplace ID (optional)"
                value={marketplaceForm.id}
                onChange={(event) =>
                  setMarketplaceForm((current) => ({
                    ...current,
                    id: event.target.value,
                  }))
                }
                helperText="Leave blank to derive it from the name."
              />
            ) : null}
            <TextField
              fullWidth
              size="small"
              label="Name"
              value={marketplaceForm.name}
              onChange={(event) =>
                setMarketplaceForm((current) => ({
                  ...current,
                  name: event.target.value,
                }))
              }
              placeholder="Example Skills Marketplace"
            />
            <TextField
              fullWidth
              size="small"
              label="Marketplace JSON URL"
              value={marketplaceForm.url}
              onChange={(event) =>
                setMarketplaceForm((current) => ({
                  ...current,
                  url: event.target.value,
                }))
              }
              placeholder="https://raw.githubusercontent.com/org/repo/main/marketplace.json"
            />
            <FormControlLabel
              control={
                <Switch
                  checked={marketplaceForm.enabled}
                  onChange={(event) =>
                    setMarketplaceForm((current) => ({
                      ...current,
                      enabled: event.target.checked,
                    }))
                  }
                />
              }
              label="Enabled"
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeMarketplaceDialog}>Cancel</Button>
          <Button
            variant="contained"
            disabled={
              createMarketplaceMutation.isPending ||
              updateMarketplaceMutation.isPending ||
              !marketplaceForm.url.trim()
            }
            onClick={saveMarketplace}
          >
            {createMarketplaceMutation.isPending ||
            updateMarketplaceMutation.isPending
              ? "Saving..."
              : "Save"}
          </Button>
        </DialogActions>
      </Dialog>
      <ImportUrlDialog
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onImported={handleImported}
        onAfterImport={afterImport}
      />
      <BulkImportDialog
        open={bulkOpen}
        onClose={() => {
          setBulkOpen(false);
          setBulkInitialUrls([]);
          setBulkSourceLabel(undefined);
        }}
        onImported={handleImported}
        onAfterImport={afterImport}
        initialUrls={bulkInitialUrls}
        sourceLabel={bulkSourceLabel}
      />
      <Dialog
        open={aiCreateOpen}
        onClose={() => setAiCreateOpen(false)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Create Skill</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Alert severity="info">
              AI Quick Create is recommended for beginners. Describe your goal
              in plain language.
            </Alert>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
                whiteSpace: "pre-line",
              }}
            >
              {`Prompt examples:
1. Track top 10 AI startups weekly, compare funding/news changes, and output a ranked briefing with sources.
2. Review competitor pricing pages every day and generate a change log with impact notes.
3. Generate a pre-meeting research brief for a company from latest news, filings, and leadership updates.
4. If this analysis fails, send update to URL (for example: your Twilio/Telegram notifier endpoint).
5. Run this every weekday at 9am and send a summary after each run.`}
            </Typography>
            {aiError ? <Alert severity="error">{aiError}</Alert> : null}
            <TextField
              fullWidth
              size="small"
              label="Skill name (optional)"
              placeholder="example: market-analyzer"
              value={aiNameHint}
              onChange={(e) =>
                setAiNameHint(normalizeActionName(e.target.value))
              }
              helperText="If blank, AI will suggest a name."
            />
            <TextField
              fullWidth
              multiline
              minRows={6}
              label="What should this skill do?"
              placeholder="Example: Find small-cap momentum stocks, validate with latest filings, then output top 5 picks with risks."
              value={aiPrompt}
              onChange={(e) => setAiPrompt(e.target.value)}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setAiCreateOpen(false);
              openNewEditor();
            }}
          >
            Advanced Editor
          </Button>
          <Button onClick={() => setAiCreateOpen(false)}>Close</Button>
          <Button
            variant="contained"
            disabled={aiGenerateMutation.isPending || !aiPrompt.trim()}
            onClick={() => {
              setAiError(null);
              aiGenerateMutation.mutate({
                prompt: aiPrompt.trim(),
                nameHint: aiNameHint.trim(),
              });
            }}
          >
            {aiGenerateMutation.isPending ? "Creating..." : "Create with AI"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog open={editOpen} onClose={closeEditor} maxWidth="md" fullWidth>
        <DialogTitle>
          {editTargetName ? `Edit skill: ${editTargetName}` : "Create skill"}
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {editError ? <Alert severity="error">{editError}</Alert> : null}
            {editLoading ? (
              <Alert severity="info">Loading skill details...</Alert>
            ) : null}
            {editRawMode ? (
              <Alert severity="warning">
                Developer mode is enabled. You are editing raw SKILL.md
                directly.
              </Alert>
            ) : (
              <Alert severity="info">
                Beginner mode is on. Fill simple fields and AgentArk will
                generate the SKILL file for you. Need raw SKILL.md editing?
                Enable Developer mode in Settings -&gt; Advanced.
              </Alert>
            )}

            {isCreateMode && !editRawMode ? (
              <FormControlLabel
                control={
                  <Switch
                    checked={createWizardEnabled}
                    onChange={(e) => setCreateWizardEnabled(e.target.checked)}
                  />
                }
                label="Use 3-step wizard (recommended). Turn off to use the classic editor."
              />
            ) : null}

            {editRawMode ? (
              <TextField
                fullWidth
                multiline
                minRows={16}
                value={editContent}
                onChange={(e) => setEditContent(e.target.value)}
                label="SKILL.md"
              />
            ) : useCreateWizard ? (
              <Stack spacing={1.25}>
                <Tabs
                  value={createWizardStep}
                  onChange={(_, v) => setCreateWizardStep(Number(v) || 0)}
                  variant="fullWidth"
                >
                  <Tab label="1. What it does" value={0} />
                  <Tab label="2. Inputs" value={1} />
                  <Tab label="3. Run automatically" value={2} />
                </Tabs>

                {createWizardStep === 0 ? (
                  <Grid2 container spacing={1.25}>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Skill name"
                        value={editForm.name}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            name: normalizeActionName(e.target.value),
                          }))
                        }
                        helperText="Use lowercase letters, numbers, and hyphens. Example: market-analysis"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 6 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Version"
                        value={editForm.version}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            version: e.target.value,
                          }))
                        }
                        helperText="Default: 1.0.0"
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Description"
                        value={editForm.description}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            description: e.target.value,
                          }))
                        }
                        helperText="One line: what this skill does."
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        multiline
                        minRows={10}
                        label="Workflow instructions"
                        value={editForm.workflow}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            workflow: e.target.value,
                          }))
                        }
                        helperText="Write clear instructions for how this skill should execute."
                      />
                    </Grid2>
                  </Grid2>
                ) : null}

                {createWizardStep === 1 ? (
                  <Grid2 container spacing={1.25}>
                    <Grid2 size={{ xs: 12 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Required inputs (optional)"
                        placeholder="from, to, budget"
                        value={editForm.requiredInputsCsv}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            requiredInputsCsv: e.target.value,
                          }))
                        }
                        helperText="Comma separated field names. If missing at runtime, user will be asked (or fallback used in scheduled runs)."
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 4 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Emoji (optional)"
                        value={editForm.emoji}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            emoji: e.target.value,
                          }))
                        }
                      />
                    </Grid2>
                    <Grid2 size={{ xs: 12, md: 8 }}>
                      <TextField
                        fullWidth
                        size="small"
                        label="Tools (comma separated)"
                        placeholder="web_search, file_read"
                        value={editForm.toolsCsv}
                        onChange={(e) =>
                          setEditForm((prev) => ({
                            ...prev,
                            toolsCsv: e.target.value,
                          }))
                        }
                        helperText="These are skills/tools your workflow may rely on."
                      />
                    </Grid2>
                  </Grid2>
                ) : null}
              </Stack>
            ) : (
              <Grid2 container spacing={1.25}>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label={editTargetName ? "Invocation name" : "Skill name"}
                    value={editForm.name}
                    disabled={!!editTargetName}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        name: normalizeActionName(e.target.value),
                      }))
                    }
                    helperText={
                      editTargetName
                        ? "This is the fixed invocation name users and agents call."
                        : "Use lowercase letters, numbers, and hyphens. Example: market-analysis"
                    }
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 6 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Version"
                    value={editForm.version}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        version: e.target.value,
                      }))
                    }
                    helperText="Default: 1.0.0"
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Description"
                    value={editForm.description}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        description: e.target.value,
                      }))
                    }
                    helperText="One line: what this skill does."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Required inputs (optional)"
                    placeholder="from, to, budget"
                    value={editForm.requiredInputsCsv}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        requiredInputsCsv: e.target.value,
                      }))
                    }
                    helperText="Comma separated field names. If missing at runtime, user will be asked (or fallback used in scheduled runs)."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 4 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Emoji (optional)"
                    value={editForm.emoji}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        emoji: e.target.value,
                      }))
                    }
                  />
                </Grid2>
                <Grid2 size={{ xs: 12, md: 8 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Tools (comma separated)"
                    placeholder="web_search, file_read"
                    value={editForm.toolsCsv}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        toolsCsv: e.target.value,
                      }))
                    }
                    helperText="These are skills/tools your workflow may rely on."
                  />
                </Grid2>
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    multiline
                    minRows={14}
                    label="Workflow instructions"
                    value={editForm.workflow}
                    onChange={(e) =>
                      setEditForm((prev) => ({
                        ...prev,
                        workflow: e.target.value,
                      }))
                    }
                    helperText="Write clear instructions for how this skill should execute."
                  />
                </Grid2>
              </Grid2>
            )}

            {!useCreateWizard || createWizardStep === 2 ? (
              <Box className="metadata-box">
                <Stack spacing={1}>
                  <FormControlLabel
                    control={
                      <Switch
                        checked={editAttachHook}
                        onChange={(e) => setEditAttachHook(e.target.checked)}
                      />
                    }
                    label="Run automatically (optional)"
                  />
                  {editAttachHook ? (
                    <Stack spacing={1}>
                      <TextField
                        fullWidth
                        size="small"
                        multiline
                        minRows={2}
                        label="When should this run? (plain language)"
                        value={editHookInstruction}
                        onChange={(e) => setEditHookInstruction(e.target.value)}
                        placeholder="Examples: when this skill fails | after each run | before this skill starts"
                      />
                      {developerModeEnabled ? (
                        <>
                          <Stack direction="row" spacing={1}>
                            <Button
                              size="small"
                              variant="outlined"
                              onClick={applyEditHookInstruction}
                            >
                              Interpret Text
                            </Button>
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                                alignSelf: "center",
                              }}
                            >
                              Infers trigger and URL.
                            </Typography>
                          </Stack>
                          <Grid2 container spacing={1}>
                            <Grid2 size={{ xs: 12, md: 4 }}>
                              <TextField
                                fullWidth
                                size="small"
                                select
                                label="When to run"
                                value={editHookTrigger}
                                onChange={(e) =>
                                  setEditHookTrigger(
                                    (e.target.value as HookTriggerValue) ||
                                      "on_error",
                                  )
                                }
                              >
                                <MenuItem value="pre_message">
                                  pre_message
                                </MenuItem>
                                <MenuItem value="post_message">
                                  post_message
                                </MenuItem>
                                <MenuItem value="pre_action">
                                  pre_action
                                </MenuItem>
                                <MenuItem value="post_action">
                                  post_action
                                </MenuItem>
                                <MenuItem value="on_consolidate">
                                  on_consolidate
                                </MenuItem>
                                <MenuItem value="on_error">on_error</MenuItem>
                              </TextField>
                            </Grid2>
                            <Grid2 size={{ xs: 12, md: 8 }}>
                              <TextField
                                fullWidth
                                size="small"
                                label="Send update to URL"
                                value={editHookUrl}
                                onChange={(e) => setEditHookUrl(e.target.value)}
                                placeholder="https://example.com/hook"
                              />
                            </Grid2>
                          </Grid2>
                        </>
                      ) : (
                        <TextField
                          fullWidth
                          size="small"
                          label="Send update to URL"
                          value={editHookUrl}
                          onChange={(e) => setEditHookUrl(e.target.value)}
                          placeholder="https://example.com/hook"
                          helperText="Required to enable this automation."
                        />
                      )}
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          whiteSpace: "pre-line",
                        }}
                      >
                        {`Automation examples:
1. when this skill fails
2. after each successful run
3. before this skill starts
4. when this skill fails, send update to URL https://example.com/hook
5. when this skill fails, send update to URL https://your-notifier.example/twilio`}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        For phone/SMS/WhatsApp/Telegram alerts, use your
                        notification URL endpoint to forward via Twilio or your
                        preferred channel integration.
                      </Typography>
                    </Stack>
                  ) : null}
                  <Divider />
                  <FormControlLabel
                    control={
                      <Switch
                        checked={editAttachTask}
                        onChange={(e) => setEditAttachTask(e.target.checked)}
                      />
                    }
                    label="Schedule this skill (optional)"
                  />
                  {editAttachTask ? (
                    <Stack spacing={1}>
                      <TextField
                        fullWidth
                        size="small"
                        multiline
                        minRows={2}
                        label="When should this run? (plain language)"
                        value={editTaskInstruction}
                        onChange={(e) => setEditTaskInstruction(e.target.value)}
                        placeholder="Examples: every day at 9am | hourly | weekdays at 9am | once now"
                      />
                      <Stack direction="row" spacing={1}>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={applyEditTaskInstruction}
                        >
                          Interpret Text
                        </Button>
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            alignSelf: "center",
                          }}
                        >
                          Infers cron schedule.
                        </Typography>
                      </Stack>
                      <TextField
                        fullWidth
                        size="small"
                        label="Cron (optional, auto-filled)"
                        value={editTaskCron}
                        onChange={(e) => setEditTaskCron(e.target.value)}
                        placeholder="0 9 * * *"
                        helperText="Use 5-field cron. Leave blank if you prefer plain language."
                      />
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                          whiteSpace: "pre-line",
                        }}
                      >
                        {`Schedule examples:
1. every day at 9am
2. every 15 minutes
3. weekdays at 9am
4. once now`}
                      </Typography>
                    </Stack>
                  ) : null}
                </Stack>
              </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeEditor}>Close</Button>
          {useCreateWizard && createWizardStep > 0 ? (
            <Button
              onClick={() => setCreateWizardStep((s) => Math.max(0, s - 1))}
            >
              Back
            </Button>
          ) : null}
          {useCreateWizard ? (
            <Button
              variant="contained"
              onClick={() => {
                if (createWizardStep < 2) {
                  setCreateWizardStep((s) => Math.min(2, s + 1));
                } else {
                  saveEditor();
                }
              }}
              disabled={wizardStepBlocked || editLoading}
            >
              {createWizardStep < 2 ? "Next" : "Save"}
            </Button>
          ) : (
            <Button
              variant="contained"
              onClick={saveEditor}
              disabled={
                editLoading ||
                (editRawMode
                  ? !editContent.trim()
                  : !editForm.description.trim()) ||
                hookBlocked ||
                scheduleBlocked
              }
            >
              Save
            </Button>
          )}
        </DialogActions>
      </Dialog>
      <Dialog
        open={hooksOpen}
        onClose={closeHooksDialog}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {hooksTargetAction
            ? `Automations for ${hooksTargetAction}`
            : "Create Automation"}
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {hookError ? <Alert severity="error">{hookError}</Alert> : null}
            <Alert severity="info">
              Describe in plain language and AgentArk will infer trigger
              defaults.
            </Alert>
            <Typography
              variant="caption"
              sx={{
                color: "text.secondary",
              }}
            >
              Advanced automation editor (Developer mode).
            </Typography>
            <TextField
              fullWidth
              multiline
              minRows={2}
              label="When should this run? (plain language)"
              value={hookInstruction}
              onChange={(e) => setHookInstruction(e.target.value)}
              placeholder={
                hooksTargetAction
                  ? `when ${hooksTargetAction} fails`
                  : "after each run"
              }
            />
            <Stack direction="row" spacing={1}>
              <Button
                size="small"
                variant="outlined"
                onClick={applyHookInstruction}
              >
                Interpret Text
              </Button>
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                  alignSelf: "center",
                }}
              >
                Fills trigger and URL when detectable.
              </Typography>
            </Stack>
            <TextField
              fullWidth
              size="small"
              label="Automation name"
              value={hookName}
              onChange={(e) => setHookName(sanitizeHookName(e.target.value))}
            />
            <TextField
              fullWidth
              size="small"
              select
              label="When to run"
              value={hookTrigger}
              onChange={(e) =>
                setHookTrigger(
                  (e.target.value as HookTriggerValue) || "post_action",
                )
              }
            >
              <MenuItem value="pre_message">pre_message</MenuItem>
              <MenuItem value="post_message">post_message</MenuItem>
              <MenuItem value="pre_action">pre_action</MenuItem>
              <MenuItem value="post_action">post_action</MenuItem>
              <MenuItem value="on_consolidate">on_consolidate</MenuItem>
              <MenuItem value="on_error">on_error</MenuItem>
            </TextField>
            <TextField
              fullWidth
              size="small"
              label="Send update to URL"
              value={hookUrl}
              onChange={(e) => setHookUrl(e.target.value)}
              placeholder="https://example.com/hook"
            />
            {hooksTargetAction ? (
              <>
                <Divider />
                <Typography variant="subtitle2">
                  Existing automations for this skill
                </Typography>
                {hooksForSelectedAction.length === 0 ? (
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    No automations attached yet.
                  </Typography>
                ) : (
                  <Stack spacing={0.6}>
                    {hooksForSelectedAction.map((h, idx) => (
                      <Box
                        key={str(h.id, `dialog-hook-${idx}`)}
                        className="console-line"
                      >
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          {humanizeStatusLabel(str(h.trigger, ""), "-")} | {boolText(h.enabled)}
                        </Typography>
                        <Typography
                          variant="body2"
                          noWrap
                          title={str(h.name, "-")}
                        >
                          {str(h.name, "-")}
                        </Typography>
                      </Box>
                    ))}
                  </Stack>
                )}
              </>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={closeHooksDialog}>Close</Button>
          <Button
            variant="contained"
            disabled={
              addHookMutation.isPending ||
              !(hookUrl.trim() || extractFirstUrl(hookInstruction))
            }
            onClick={saveHookFromDialog}
          >
            {addHookMutation.isPending ? "Saving..." : "Save Automation"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={testRunDialog != null}
        onClose={closeSkillTestDialog}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>
          Skill test
          {testRunDialog ? `: ${testRunDialog.name}` : ""}
        </DialogTitle>
        <DialogContent dividers>
          {testRunDialog ? (
            <Stack spacing={1.5}>
              <Alert
                severity={
                  testRunDialog.phase === "error"
                    ? "error"
                    : testRunDialog.phase === "completed"
                      ? "success"
                      : "info"
                }
              >
                {testRunDialog.message}
              </Alert>
              {testRunDialog.phase === "waiting_input" ? (
                <>
                  {testRunDialog.inputError ? (
                    <Alert severity="error" className="skill-test-error">
                      {testRunDialog.inputError}
                    </Alert>
                  ) : null}
                  <Stack spacing={1.5} className="skill-test-fields">
                    {testRunDialog.inputFields.map((field) => (
                      <TextField
                        key={field}
                        fullWidth
                        multiline
                        minRows={2}
                        label={skillInputLabel(field)}
                        helperText={skillInputDescription(
                          testRunDialog.skill,
                          field,
                        )}
                        value={testRunDialog.inputValues[field] || ""}
                        onChange={(event) => {
                          const value = event.target.value;
                          setTestRunDialog((current) =>
                            current && current.name === testRunDialog.name
                              ? {
                                  ...current,
                                  inputValues: {
                                    ...current.inputValues,
                                    [field]: value,
                                  },
                                  inputError: null,
                                }
                              : current,
                          );
                        }}
                      />
                    ))}
                  </Stack>
                </>
              ) : null}
              {testRunDialog.output ? (
                <Typography component="pre" className="skill-test-output">
                  {testRunDialog.output}
                </Typography>
              ) : null}
            </Stack>
          ) : null}
        </DialogContent>
        <DialogActions>
          <Button onClick={closeSkillTestDialog}>
            {testRunDialog && isSkillTestRunActive(testRunDialog.phase)
              ? "Stop and Close"
              : "Close"}
          </Button>
          {testRunDialog?.phase === "waiting_input" ? (
            <Button variant="contained" onClick={runSkillTestFromInputDialog}>
              Run Test
            </Button>
          ) : null}
        </DialogActions>
      </Dialog>
      <SkillSecretsDialog
        open={secretsName != null}
        skillName={secretsName}
        onClose={() => setSecretsName(null)}
      />
    </WorkspacePageShell>
  );
}


