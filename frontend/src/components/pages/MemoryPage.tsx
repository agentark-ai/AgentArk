import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Link,
  Stack,
  Tab,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TablePagination,
  TableRow,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
  type JsonRecord,
} from "./pageHelpers";
import {
  humanTs,
  KeyValuePanel,
  RowOpsMenu,
} from "./workspaceUiBits";

const REFRESH_MS = 8000;
const MEMORY_PAGE_SIZE = 20;

type MemoryCategoryKey =
  | "facts"
  | "assistantPreferences"
  | "workPreferences"
  | "domainMemory"
  | "otherMemory"
  | "preferences"
  | "userData"
  | "knowledge";

type RuntimeActionCatalogEntry = {
  actionId: string;
  displayName: string;
  capabilities: string[];
  summary: string;
  details: string;
};

type DeleteMemoryTarget =
  | {
      kind: "learnedMemory";
      id: string;
      label: string;
    }
  | {
      kind: "preference";
      id: string;
      label: string;
      endpoint: string;
    }
  | {
      kind: "userData";
      id: string;
      label: string;
    }
  | {
      kind: "knowledge";
      id: string;
      label: string;
    };

type EditMemoryTarget = {
  id: string;
  label: string;
  value: string;
};

function isInternalAgentArkHelpUrl(value: unknown): boolean {
  return str(value, "").trim().toLowerCase().startsWith("agentark://help/");
}

function isRuntimeActionCatalogKnowledgeItem(
  item: JsonRecord | null | undefined,
): boolean {
  if (!item) return false;
  const source = str(item.source, "").trim().toLowerCase();
  const url = str(item.url, "").trim().toLowerCase();
  const title = str(item.title, "").trim().toLowerCase();
  return (
    (source === "agentark_runtime_help" || url.startsWith("agentark://help/")) &&
    (url.includes("/runtime/actions-") || title.startsWith("live action catalog"))
  );
}

function runtimeCatalogSectionLabel(
  item: JsonRecord | null | undefined,
): string | null {
  const urlMatch = str(item?.url, "").match(/actions-(\d+)$/i);
  if (urlMatch) return `Section ${urlMatch[1]}`;
  const titleMatch = str(item?.title, "").match(/(\d+)\s*$/);
  return titleMatch ? `Section ${titleMatch[1]}` : null;
}

function humanizeCatalogToken(value: string): string {
  return value
    .split(/[_-]+/)
    .filter((part) => part.trim().length > 0)
    .map((part) => {
      const normalized = part.trim().toLowerCase();
      if (normalized === "ssh") return "SSH";
      if (normalized === "api") return "API";
      if (normalized === "mcp") return "MCP";
      return normalized.charAt(0).toUpperCase() + normalized.slice(1);
    })
    .join(" ");
}

function splitRuntimeActionDescription(description: string): {
  summary: string;
  details: string;
} {
  const trimmed = description.trim();
  if (!trimmed) return { summary: "", details: "" };
  const boundary = trimmed.search(/[.!?](?:\s|$)/);
  if (boundary === -1) {
    return { summary: trimmed, details: "" };
  }
  const summary = trimmed.slice(0, boundary + 1).trim();
  const details = trimmed.slice(boundary + 1).trim();
  return { summary, details };
}

function parseRuntimeActionCatalogEntries(
  content: string,
): RuntimeActionCatalogEntry[] {
  return content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith("- `"))
    .map((line) => {
      const match = line.match(
        /^-\s*`([^`]+)`\s*\|\s*capabilities:\s*([^|]+?)\s*\|\s*(.+)$/i,
      );
      if (!match) return null;
      const [, actionId, capabilitiesRaw, description] = match;
      const capabilities =
        capabilitiesRaw.trim().toLowerCase() === "none"
          ? []
          : capabilitiesRaw
              .split(",")
              .map((entry) => entry.trim())
              .filter((entry) => entry.length > 0);
      const { summary, details } = splitRuntimeActionDescription(description);
      return {
        actionId,
        displayName: humanizeCatalogToken(actionId),
        capabilities,
        summary,
        details,
      };
    })
    .filter((entry): entry is RuntimeActionCatalogEntry => Boolean(entry));
}

function knowledgeSourceLabel(item: JsonRecord | null | undefined): string | null {
  const source = str(item?.source, "").trim();
  if (!source) return null;
  if (source.toLowerCase() === "agentark_runtime_help") {
    return "Built-in guide";
  }
  return source;
}

function knowledgeDisplayTitle(item: JsonRecord | null | undefined): string {
  if (isRuntimeActionCatalogKnowledgeItem(item)) {
    return "Available actions on this instance";
  }
  return str(item?.title, "Knowledge Item");
}

function memoryFactEditableValue(item: JsonRecord | null | undefined): string {
  const value = str(item?.value, "").trim();
  if (value) return value;
  const factText = str(item?.fact, "").trim();
  const key = memoryFactKey(item);
  if (key) {
    const prefix = `${key}:`;
    if (factText.startsWith(prefix)) {
      return factText.slice(prefix.length).trim();
    }
    return factText;
  }
  const colonIdx = factText.indexOf(":");
  if (colonIdx > 0 && colonIdx < 80 && !/\s/.test(factText.slice(0, colonIdx))) {
    return factText.slice(colonIdx + 1).trim();
  }
  return factText;
}

function memoryFactKey(item: JsonRecord | null | undefined): string {
  return str(item?.key, "").trim();
}

function memoryFactDisplayText(item: JsonRecord | null | undefined): string {
  const key = memoryFactKey(item);
  const value = memoryFactEditableValue(item);
  if (key && value) return `${key}: ${value}`;
  return value || str(item?.fact, "-");
}

type MemoryPageProps = {
  autoRefresh: boolean;
  showHeader?: boolean;
  showScopeControls?: boolean;
  onNavigateToView?: (view: string, replace?: boolean) => void;
  onViewMemoryEvidence?: (memoryId: string) => void;
};

export default function MemoryPage({
  autoRefresh,
  showHeader = true,
  showScopeControls: _showScopeControls = true,
  onNavigateToView,
  onViewMemoryEvidence,
}: MemoryPageProps) {
  void onNavigateToView;
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [selectedFact, setSelectedFact] = useState<JsonRecord | null>(null);
  const [selectedKnowledge, setSelectedKnowledge] = useState<JsonRecord | null>(
    null,
  );
  const [deleteTarget, setDeleteTarget] = useState<DeleteMemoryTarget | null>(
    null,
  );
  const [editTarget, setEditTarget] = useState<EditMemoryTarget | null>(null);
  const [editValue, setEditValue] = useState("");
  const [memoryTab, setMemoryTab] = useState(0);
  const [memoryPages, setMemoryPages] = useState<Record<MemoryCategoryKey, number>>({
    facts: 0,
    assistantPreferences: 0,
    workPreferences: 0,
    domainMemory: 0,
    otherMemory: 0,
    preferences: 0,
    userData: 0,
    knowledge: 0,
  });
  const [prefKey, setPrefKey] = useState("");
  const [prefValue, setPrefValue] = useState("");
  const [prefConfidence, setPrefConfidence] = useState("0.85");
  const [prefSource, setPrefSource] = useState("");
  const [dataKind, setDataKind] = useState("note");
  const [dataTitle, setDataTitle] = useState("");
  const [dataContent, setDataContent] = useState("");
  const [dataUrl, setDataUrl] = useState("");
  const [knowledgeTitle, setKnowledgeTitle] = useState("");
  const [knowledgeContent, setKnowledgeContent] = useState("");
  const [knowledgeSource, setKnowledgeSource] = useState("");
  const [knowledgeUrl, setKnowledgeUrl] = useState("");
  const [knowledgeTags, setKnowledgeTags] = useState("");
  const invalidateMemoryQueries = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["arkmemory-summary"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-queue"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-ledger"] }),
      queryClient.invalidateQueries({ queryKey: ["arkmemory-health"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-stats"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-facts"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-assistant-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-work-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-domain-memory"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-other-memory"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-preferences"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-user-data"] }),
      queryClient.invalidateQueries({ queryKey: ["memory-knowledge"] }),
    ]);
  };

  const statsQ = useQuery({
    queryKey: ["memory-stats"],
    queryFn: () => api.rawGet("/memory/stats"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const factsQ = useQuery({
    queryKey: ["memory-facts", memoryPages.facts, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/facts?category=profile_fact&limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.facts * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const assistantPreferencesQ = useQuery({
    queryKey: [
      "memory-assistant-preferences",
      memoryPages.assistantPreferences,
      MEMORY_PAGE_SIZE,
    ],
    queryFn: () =>
      api.rawGet(
        `/memory/facts?category=assistant_preference&limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.assistantPreferences * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const workPreferencesQ = useQuery({
    queryKey: ["memory-work-preferences", memoryPages.workPreferences, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/facts?category=work_preference&limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.workPreferences * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const domainMemoryQ = useQuery({
    queryKey: ["memory-domain-memory", memoryPages.domainMemory, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/facts?category=project_domain_memory&limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.domainMemory * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const otherMemoryQ = useQuery({
    queryKey: ["memory-other-memory", memoryPages.otherMemory, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/facts?category=other&limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.otherMemory * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const preferencesQ = useQuery({
    queryKey: ["memory-preferences", memoryPages.preferences, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/preferences?limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.preferences * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const userDataQ = useQuery({
    queryKey: ["memory-user-data", memoryPages.userData, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/user-data?limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.userData * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const knowledgeQ = useQuery({
    queryKey: ["memory-knowledge", memoryPages.knowledge, MEMORY_PAGE_SIZE],
    queryFn: () =>
      api.rawGet(
        `/memory/knowledge?limit=${MEMORY_PAGE_SIZE}&offset=${
          memoryPages.knowledge * MEMORY_PAGE_SIZE
        }`,
      ),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const createPreferenceMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/memory/preferences", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    },
  });
  const deleteLearnedMemoryMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/memory/facts/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      setSelectedFact(null);
      await invalidateMemoryQueries();
    },
  });
  const updateLearnedMemoryMutation = useMutation({
    mutationFn: ({ id, value }: { id: string; value: string }) =>
      api.rawPost(`/memory/facts/${encodeURIComponent(id)}`, { value }),
    onSuccess: async (payload, variables) => {
      const updated = asRecord(asRecord(payload).memory);
      if (selectedFact && str(selectedFact.id, "") === variables.id) {
        setSelectedFact(Object.keys(updated).length > 0 ? updated : null);
      }
      setEditTarget(null);
      setEditValue("");
      await invalidateMemoryQueries();
    },
  });
  const deletePreferenceMutation = useMutation({
    mutationFn: (endpoint: string) => api.rawDelete(endpoint),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    },
  });
  const createUserDataMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/memory/user-data", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    },
  });
  const deleteUserDataMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/memory/user-data/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    },
  });
  const createKnowledgeMutation = useMutation({
    mutationFn: (payload: JsonRecord) =>
      api.rawPost("/memory/knowledge", payload),
    onSuccess: async () => {
      await invalidateMemoryQueries();
    },
  });
  const deleteKnowledgeMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/memory/knowledge/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      setSelectedKnowledge(null);
      await invalidateMemoryQueries();
    },
  });

  const stats = asRecord(statsQ.data);
  const facts = pickRecords(factsQ.data, "facts");
  const assistantPreferences = pickRecords(assistantPreferencesQ.data, "facts");
  const workPreferences = pickRecords(workPreferencesQ.data, "facts");
  const domainMemory = pickRecords(domainMemoryQ.data, "facts");
  const otherMemory = pickRecords(otherMemoryQ.data, "facts");
  const preferences = pickRecords(preferencesQ.data, "preferences");
  const userDataItems = pickRecords(userDataQ.data, "items");
  const knowledgeItems = pickRecords(knowledgeQ.data, "items");
  const factsTotal = num(asRecord(factsQ.data).total, num(stats.facts, facts.length));
  const assistantPreferencesTotal = num(
    asRecord(assistantPreferencesQ.data).total,
    num(stats.assistant_preferences, assistantPreferences.length),
  );
  const workPreferencesTotal = num(
    asRecord(workPreferencesQ.data).total,
    num(stats.work_preferences, workPreferences.length),
  );
  const domainMemoryTotal = num(
    asRecord(domainMemoryQ.data).total,
    num(stats.project_domain_memory, domainMemory.length),
  );
  const otherMemoryTotal = num(
    asRecord(otherMemoryQ.data).total,
    num(stats.other_memory, otherMemory.length),
  );
  const preferencesTotal = num(
    asRecord(preferencesQ.data).total,
    num(stats.preferences, preferences.length),
  );
  const userDataTotal = num(
    asRecord(userDataQ.data).total,
    num(stats.user_data, userDataItems.length),
  );
  const knowledgeTotal = num(
    asRecord(knowledgeQ.data).total,
    num(stats.knowledge, knowledgeItems.length),
  );
  const setMemoryPage = (key: MemoryCategoryKey, page: number) => {
    setMemoryPages((prev) => {
      const nextPage = Math.max(0, page);
      return prev[key] === nextPage ? prev : { ...prev, [key]: nextPage };
    });
  };

  useEffect(() => {
    setMemoryPages((prev) => {
      const next = { ...prev };
      let changed = false;
      ([
        ["facts", factsTotal],
        ["assistantPreferences", assistantPreferencesTotal],
        ["workPreferences", workPreferencesTotal],
        ["domainMemory", domainMemoryTotal],
        ["otherMemory", otherMemoryTotal],
        ["preferences", preferencesTotal],
        ["userData", userDataTotal],
        ["knowledge", knowledgeTotal],
      ] as const).forEach(([key, total]) => {
        const maxPage = Math.max(0, Math.ceil(total / MEMORY_PAGE_SIZE) - 1);
        if (next[key] > maxPage) {
          next[key] = maxPage;
          changed = true;
        }
      });
      return changed ? next : prev;
    });
  }, [
    assistantPreferencesTotal,
    domainMemoryTotal,
    factsTotal,
    knowledgeTotal,
    otherMemoryTotal,
    preferencesTotal,
    userDataTotal,
    workPreferencesTotal,
  ]);

  const parseSources = (value: unknown): string[] => {
    if (Array.isArray(value)) return value.map((v) => String(v));
    if (typeof value !== "string" || !value.trim()) return [];
    try {
      const parsed = JSON.parse(value);
      if (Array.isArray(parsed)) return parsed.map((v) => String(v));
    } catch {
      // Keep fallback below.
    }
    return [value];
  };

  const parseKnowledgeTags = (value: unknown): string[] => {
    if (Array.isArray(value)) {
      return value
        .map((entry) => String(entry).trim())
        .filter((entry) => entry.length > 0);
    }
    return str(value, "")
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
  };

  const selectedKnowledgeContent = str(selectedKnowledge?.content, "-");
  const selectedKnowledgeIsRuntimeCatalog =
    isRuntimeActionCatalogKnowledgeItem(selectedKnowledge);
  const selectedKnowledgeSource = knowledgeSourceLabel(selectedKnowledge);
  const selectedKnowledgeSection = runtimeCatalogSectionLabel(selectedKnowledge);
  const selectedKnowledgeActions = selectedKnowledgeIsRuntimeCatalog
    ? parseRuntimeActionCatalogEntries(selectedKnowledgeContent)
    : [];
  const selectedKnowledgeUrl = str(selectedKnowledge?.url, "").trim();
  const selectedKnowledgeShowsExternalUrl =
    selectedKnowledgeUrl && !isInternalAgentArkHelpUrl(selectedKnowledgeUrl);
  const selectedKnowledgeTags = parseKnowledgeTags(selectedKnowledge?.tags);
  const deleteBusy =
    deleteLearnedMemoryMutation.isPending ||
    deletePreferenceMutation.isPending ||
    deleteUserDataMutation.isPending ||
    deleteKnowledgeMutation.isPending;
  const editBusy = updateLearnedMemoryMutation.isPending;
  const openMemoryEdit = (item: JsonRecord) => {
    const id = str(item.id, "").trim();
    if (!id) return;
    const label = memoryFactDisplayText(item) || id;
    setError(null);
    setSelectedFact(null);
    setEditTarget({
      id,
      label,
      value: memoryFactEditableValue(item),
    });
    setEditValue(memoryFactEditableValue(item));
  };
  const confirmDeleteTarget = (target: DeleteMemoryTarget) => {
    setError(null);
    setDeleteTarget(target);
  };
  const saveMemoryEdit = async () => {
    if (!editTarget) return;
    const value = editValue.trim();
    if (!value) {
      setError("Memory value is required.");
      return;
    }
    setError(null);
    try {
      await updateLearnedMemoryMutation.mutateAsync({
        id: editTarget.id,
        value,
      });
    } catch (e) {
      setError(errMessage(e));
    }
  };
  const runConfirmedDelete = async () => {
    if (!deleteTarget) return;
    setError(null);
    try {
      if (deleteTarget.kind === "learnedMemory") {
        await deleteLearnedMemoryMutation.mutateAsync(deleteTarget.id);
      } else if (deleteTarget.kind === "preference") {
        await deletePreferenceMutation.mutateAsync(deleteTarget.endpoint);
      } else if (deleteTarget.kind === "userData") {
        await deleteUserDataMutation.mutateAsync(deleteTarget.id);
      } else {
        await deleteKnowledgeMutation.mutateAsync(deleteTarget.id);
      }
      setDeleteTarget(null);
    } catch (e) {
      setError(errMessage(e));
    }
  };
  const renderLearnedMemoryTable = (
    title: string,
    items: JsonRecord[],
    total: number,
    pageKey: MemoryCategoryKey,
    queryError: unknown,
    emptyCopy: string,
  ) => (
    <Box className="list-shell">
      <Typography
        variant="h6"
        sx={{
          mb: 1,
        }}
      >
        {title}
      </Typography>
      {queryError ? <Alert severity="error">{errMessage(queryError)}</Alert> : null}
      {items.length === 0 ? (
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
          }}
        >
          {emptyCopy}
        </Typography>
      ) : (
        <>
          <TableContainer className="table-shell">
            <Table size="small">
              <TableHead>
                <TableRow>
                  <TableCell>Memory</TableCell>
                  <TableCell>Topics</TableCell>
                  <TableCell>Confidence</TableCell>
                  <TableCell>Updated</TableCell>
                  <TableCell>Evidence</TableCell>
                  <TableCell align="right">Ops</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {items.map((f, idx) => {
                  const fact = asRecord(f);
                  const id = str(f.id, String(idx));
                  const sources = parseSources(f.sources);
                  const evidenceCount = num(f.evidence_count, sources.length);
                  const factKey = memoryFactKey(fact);
                  const factValue = memoryFactEditableValue(fact) || str(f.fact, "-");
                  const factText = memoryFactDisplayText(fact);
                  const topics = Array.isArray(f.topics)
                    ? f.topics.map((topic) => String(topic)).filter(Boolean)
                    : [];
                  const updatedAt = humanTs(str(f.updated_at, str(f.created_at, "-")));
                  return (
                    <TableRow
                      key={id}
                      hover
                      tabIndex={0}
                      aria-label={`Open memory: ${factText}`}
                      onClick={() => setSelectedFact(fact)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                          e.preventDefault();
                          setSelectedFact(fact);
                        }
                      }}
                      sx={{
                        cursor: "pointer",
                      }}
                    >
                      <TableCell sx={{ maxWidth: 560 }}>
                        {factKey ? (
                          <Typography
                            variant="caption"
                            noWrap
                            title={factKey}
                            sx={{
                              display: "block",
                              color: "text.secondary",
                              fontFamily: "var(--font-mono)",
                            }}
                          >
                            {factKey}
                          </Typography>
                        ) : null}
                        <Typography variant="body2" noWrap title={factText}>
                          {factValue}
                        </Typography>
                      </TableCell>
                      <TableCell sx={{ maxWidth: 240 }}>
                        <Typography
                          variant="body2"
                          noWrap
                          title={topics.join(", ")}
                          sx={{ color: "text.secondary" }}
                        >
                          {topics.length ? topics.join(", ") : "-"}
                        </Typography>
                      </TableCell>
                      <TableCell>{num(f.confidence, 0).toFixed(2)}</TableCell>
                      <TableCell sx={{ whiteSpace: "nowrap" }} title={updatedAt.tip}>
                        {updatedAt.label}
                      </TableCell>
                      <TableCell>{evidenceCount}</TableCell>
                      <TableCell
                        align="right"
                        onClick={(e) => e.stopPropagation()}
                        onKeyDown={(e) => e.stopPropagation()}
                      >
                        <RowOpsMenu
                          actions={[
                            {
                              label: "Edit value",
                              onClick: () => openMemoryEdit(fact),
                            },
                            {
                              label: "Delete",
                              tone: "error",
                              divider: true,
                              onClick: () =>
                                confirmDeleteTarget({
                                  kind: "learnedMemory",
                                  id,
                                  label: factText,
                                }),
                            },
                          ]}
                          ariaLabel="Memory options"
                        />
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </TableContainer>
          <TablePagination
            component="div"
            count={total}
            page={memoryPages[pageKey]}
            onPageChange={(_event, nextPage) => setMemoryPage(pageKey, nextPage)}
            rowsPerPage={MEMORY_PAGE_SIZE}
            rowsPerPageOptions={[MEMORY_PAGE_SIZE]}
          />
        </>
      )}
    </Box>
  );

  return (
    <WorkspacePageShell spacing={1.5}>
      {showHeader ? (
        <WorkspacePageHeader
          eyebrow="Data"
          title="Memory"
          description="Review remembered facts, preferences, user data, and knowledge."
        />
      ) : null}
      {/* Removed the colored counter-pills grid — it duplicated the
          per-category counts that are now shown inline on each filter
          chip below. The category dimension only needs to be conveyed
          once, in the place where you act on it (the filter strip). */}
      <Box className="list-shell workspace-page-subnav-shell">
        <Stack
          direction="row"
          sx={{
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <Tabs
            value={memoryTab}
            onChange={(_event, next) => {
              if (typeof next === "number") setMemoryTab(next);
            }}
            variant="scrollable"
            allowScrollButtonsMobile
            className="workspace-page-subnav-tabs"
            sx={{ flex: 1 }}
          >
            <Tab value={0} label={`Profile Facts (${factsTotal})`} />
            <Tab value={1} label={`Assistant (${assistantPreferencesTotal})`} />
            <Tab value={2} label={`Work Prefs (${workPreferencesTotal})`} />
            <Tab value={3} label={`Domain (${domainMemoryTotal})`} />
            <Tab value={4} label={`Other (${otherMemoryTotal})`} />
            <Tab value={5} label={`Preferences (${preferencesTotal})`} />
            <Tab value={6} label={`User Data (${userDataTotal})`} />
            <Tab value={7} label={`Knowledge (${knowledgeTotal})`} />
          </Tabs>
        </Stack>
      </Box>
      {memoryTab === 0 ? (
        <Box className="list-shell">
          {/* "Profile Facts" h6 header removed — the active filter chip
              above already says "Profile Facts (N)". Same applies to the
              other category panels (Preferences, User Data, Knowledge). */}
          {factsQ.error ? (
            <Alert severity="error">{errMessage(factsQ.error)}</Alert>
          ) : null}
          {facts.length === 0 ? (
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
              }}
            >
              No facts yet.
            </Typography>
          ) : (
            <>
              <TableContainer className="table-shell">
                <Table size="small">
                  <TableHead>
                    <TableRow>
                      <TableCell>Fact</TableCell>
                      <TableCell>Confidence</TableCell>
                      <TableCell>Created</TableCell>
                      <TableCell>Evidence</TableCell>
                      <TableCell align="right">Ops</TableCell>
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {facts.map((f, idx) => {
                      const fact = asRecord(f);
                      const id = str(f.id, String(idx));
                      const sources = parseSources(f.sources);
                      const evidenceCount = num(f.evidence_count, sources.length);
                      const factKey = memoryFactKey(fact);
                      const factValue = memoryFactEditableValue(fact) || str(f.fact, "-");
                      const factText = memoryFactDisplayText(fact);
                      return (
                        <TableRow
                          key={id}
                          hover
                          tabIndex={0}
                          aria-label={`Open memory fact: ${factText}`}
                          onClick={() => setSelectedFact(fact)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              setSelectedFact(fact);
                            }
                          }}
                          sx={{
                            cursor: "pointer",
                          }}
                        >
                          <TableCell sx={{ maxWidth: 640 }}>
                            {factKey ? (
                              <Typography
                                variant="caption"
                                noWrap
                                title={factKey}
                                sx={{
                                  display: "block",
                                  color: "text.secondary",
                                  fontFamily: "var(--font-mono)",
                                }}
                              >
                                {factKey}
                              </Typography>
                            ) : null}
                            <Typography
                              variant="body2"
                              noWrap
                              title={factText}
                            >
                              {factValue}
                            </Typography>
                          </TableCell>
                          <TableCell>{num(f.confidence, 0).toFixed(2)}</TableCell>
                          <TableCell
                            sx={{ whiteSpace: "nowrap" }}
                            title={humanTs(str(f.created_at, "-")).tip}
                          >
                            {humanTs(str(f.created_at, "-")).label}
                          </TableCell>
                          <TableCell>{evidenceCount}</TableCell>
                          <TableCell
                            align="right"
                            onClick={(e) => e.stopPropagation()}
                            onKeyDown={(e) => e.stopPropagation()}
                          >
                            <RowOpsMenu
                              actions={[
                                {
                                  label: "Edit value",
                                  onClick: () => openMemoryEdit(fact),
                                },
                                {
                                  label: "Delete",
                                  tone: "error",
                                  divider: true,
                                  onClick: () =>
                                    confirmDeleteTarget({
                                      kind: "learnedMemory",
                                      id,
                                      label: factText,
                                    }),
                                },
                              ]}
                              ariaLabel="Memory fact options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableContainer>
              <TablePagination
                component="div"
                count={factsTotal}
                page={memoryPages.facts}
                onPageChange={(_event, nextPage) => setMemoryPage("facts", nextPage)}
                rowsPerPage={MEMORY_PAGE_SIZE}
                rowsPerPageOptions={[MEMORY_PAGE_SIZE]}
              />
            </>
          )}
        </Box>
      ) : null}
      {memoryTab === 1
        ? renderLearnedMemoryTable(
            "Assistant Preferences",
            assistantPreferences,
            assistantPreferencesTotal,
            "assistantPreferences",
            assistantPreferencesQ.error,
            "No assistant preferences yet.",
          )
        : null}
      {memoryTab === 2
        ? renderLearnedMemoryTable(
            "Work Preferences",
            workPreferences,
            workPreferencesTotal,
            "workPreferences",
            workPreferencesQ.error,
            "No work preferences yet.",
          )
        : null}
      {memoryTab === 3
        ? renderLearnedMemoryTable(
            "Project / Domain Memory",
            domainMemory,
            domainMemoryTotal,
            "domainMemory",
            domainMemoryQ.error,
            "No project or domain memory yet.",
          )
        : null}
      {memoryTab === 4
        ? renderLearnedMemoryTable(
            "Other Memory",
            otherMemory,
            otherMemoryTotal,
            "otherMemory",
            otherMemoryQ.error,
            "No uncategorized memory yet.",
          )
        : null}
      {memoryTab === 5 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Add Preference
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Key"
                  placeholder="timezone"
                  value={prefKey}
                  onChange={(e) => setPrefKey(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Value"
                  placeholder="Asia/Kolkata"
                  value={prefValue}
                  onChange={(e) => setPrefValue(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 2 }}>
                <TextField
                  fullWidth
                  size="small"
                  type="number"
                  label="Confidence"
                  value={prefConfidence}
                  onChange={(e) => setPrefConfidence(e.target.value)}
                  slotProps={{
                    htmlInput: { min: 0, max: 1, step: 0.05 },
                  }}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Source (optional)"
                  placeholder="user_message"
                  value={prefSource}
                  onChange={(e) => setPrefSource(e.target.value)}
                />
              </Grid2>
              <Grid2
                size={{ xs: 12 }}
                sx={{ display: "flex", justifyContent: "flex-end" }}
              >
                <Button
                  variant="contained"
                  disabled={
                    createPreferenceMutation.isPending ||
                    !prefKey.trim() ||
                    !prefValue.trim()
                  }
                  onClick={async () => {
                    setError(null);
                    try {
                      const parsedConfidence = Number(prefConfidence);
                      await createPreferenceMutation.mutateAsync({
                        key: prefKey.trim(),
                        value: prefValue.trim(),
                        confidence: Number.isFinite(parsedConfidence)
                          ? parsedConfidence
                          : 0.85,
                        source: prefSource.trim() || undefined,
                      });
                      setPrefKey("");
                      setPrefValue("");
                      setPrefSource("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save Preference
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            {/* "Preferences" h6 header removed — chip says it. */}
            {preferencesQ.error ? (
              <Alert severity="error">{errMessage(preferencesQ.error)}</Alert>
            ) : null}
            {preferences.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No preferences yet.
              </Typography>
            ) : (
              <>
                <TableContainer className="table-shell">
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        <TableCell>Key</TableCell>
                        <TableCell>Value</TableCell>
                        <TableCell>Confidence</TableCell>
                        <TableCell>Source</TableCell>
                        <TableCell>Updated</TableCell>
                        <TableCell align="right">Ops</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {preferences.map((pref, idx) => {
                        const key = str(pref.key, String(idx));
                        const endpoint = `/memory/preferences/${encodeURIComponent(key)}`;
                        return (
                          <TableRow key={`${key}-${idx}`}>
                            <TableCell sx={{ whiteSpace: "nowrap" }}>
                              {key}
                            </TableCell>
                            <TableCell sx={{ maxWidth: 480 }}>
                              <Typography
                                variant="body2"
                                noWrap
                                title={str(pref.value, "-")}
                              >
                                {str(pref.value, "-")}
                              </Typography>
                            </TableCell>
                            <TableCell>
                              {num(pref.confidence, 0).toFixed(2)}
                            </TableCell>
                            <TableCell>{str(pref.source, "-")}</TableCell>
                            <TableCell
                              sx={{ whiteSpace: "nowrap" }}
                              title={humanTs(str(pref.updated_at, "-")).tip}
                            >
                              {humanTs(str(pref.updated_at, "-")).label}
                            </TableCell>
                            <TableCell align="right">
                              <RowOpsMenu
                                actions={[
                                  {
                                    label: "Delete",
                                    tone: "error",
                                    divider: true,
                                    onClick: () =>
                                      confirmDeleteTarget({
                                        kind: "preference",
                                        id: key,
                                        label: key,
                                        endpoint,
                                      }),
                                  },
                                ]}
                                ariaLabel="Preference options"
                              />
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
                <TablePagination
                  component="div"
                  count={preferencesTotal}
                  page={memoryPages.preferences}
                  onPageChange={(_event, nextPage) =>
                    setMemoryPage("preferences", nextPage)
                  }
                  rowsPerPage={MEMORY_PAGE_SIZE}
                  rowsPerPageOptions={[MEMORY_PAGE_SIZE]}
                />
              </>
            )}
          </Box>
        </Stack>
      ) : null}
      {memoryTab === 6 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Add User Data
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Kind"
                  placeholder="note | link | file"
                  value={dataKind}
                  onChange={(e) => setDataKind(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 5 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Title"
                  placeholder="Quarterly roadmap doc"
                  value={dataTitle}
                  onChange={(e) => setDataTitle(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="URL (optional)"
                  placeholder="https://..."
                  value={dataUrl}
                  onChange={(e) => setDataUrl(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={3}
                  label="Content"
                  placeholder="Summary or notes"
                  value={dataContent}
                  onChange={(e) => setDataContent(e.target.value)}
                />
              </Grid2>
              <Grid2
                size={{ xs: 12 }}
                sx={{ display: "flex", justifyContent: "flex-end" }}
              >
                <Button
                  variant="contained"
                  disabled={
                    createUserDataMutation.isPending ||
                    !dataKind.trim() ||
                    !dataTitle.trim()
                  }
                  onClick={async () => {
                    setError(null);
                    try {
                      await createUserDataMutation.mutateAsync({
                        kind: dataKind.trim(),
                        title: dataTitle.trim(),
                        content: dataContent.trim(),
                        url: dataUrl.trim() || undefined,
                      });
                      setDataKind("note");
                      setDataTitle("");
                      setDataContent("");
                      setDataUrl("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save User Data
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            {/* "User Data" h6 header removed — chip says it. */}
            {userDataQ.error ? (
              <Alert severity="error">{errMessage(userDataQ.error)}</Alert>
            ) : null}
            {userDataItems.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No user data items yet.
              </Typography>
            ) : (
              <>
                <TableContainer className="table-shell">
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        <TableCell>Kind</TableCell>
                        <TableCell>Title</TableCell>
                        <TableCell>Content</TableCell>
                        <TableCell>URL</TableCell>
                        <TableCell>Updated</TableCell>
                        <TableCell align="right">Ops</TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {userDataItems.map((item, idx) => {
                        const id = str(item.id, String(idx));
                        const url = str(item.url, "");
                        return (
                          <TableRow key={id}>
                            <TableCell>{str(item.kind, "-")}</TableCell>
                            <TableCell sx={{ maxWidth: 220 }}>
                              <Typography
                                variant="body2"
                                noWrap
                                title={str(item.title, "-")}
                              >
                                {str(item.title, "-")}
                              </Typography>
                            </TableCell>
                            <TableCell sx={{ maxWidth: 380 }}>
                              <Typography
                                variant="body2"
                                noWrap
                                title={str(item.content, "-")}
                              >
                                {str(item.content, "-")}
                              </Typography>
                            </TableCell>
                            <TableCell sx={{ maxWidth: 260 }}>
                              {url ? (
                                <Typography
                                  component="a"
                                  href={url}
                                  target="_blank"
                                  rel="noopener noreferrer"
                                  variant="body2"
                                  sx={{
                                    color: "var(--mui-palette-info-main)",
                                    textDecoration: "none",
                                  }}
                                >
                                  Open
                                </Typography>
                              ) : (
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "text.secondary",
                                  }}
                                >
                                  -
                                </Typography>
                              )}
                            </TableCell>
                            <TableCell
                              sx={{ whiteSpace: "nowrap" }}
                              title={humanTs(str(item.updated_at, "-")).tip}
                            >
                              {humanTs(str(item.updated_at, "-")).label}
                            </TableCell>
                            <TableCell align="right">
                              <RowOpsMenu
                                actions={[
                                  {
                                    label: "Delete",
                                    tone: "error",
                                    divider: true,
                                    onClick: () =>
                                      confirmDeleteTarget({
                                        kind: "userData",
                                        id,
                                        label: str(item.title, id),
                                      }),
                                  },
                                ]}
                                ariaLabel="User data options"
                              />
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
                <TablePagination
                  component="div"
                  count={userDataTotal}
                  page={memoryPages.userData}
                  onPageChange={(_event, nextPage) =>
                    setMemoryPage("userData", nextPage)
                  }
                  rowsPerPage={MEMORY_PAGE_SIZE}
                  rowsPerPageOptions={[MEMORY_PAGE_SIZE]}
                />
              </>
            )}
          </Box>
        </Stack>
      ) : null}
      {memoryTab === 7 ? (
        <Stack spacing={2}>
          <Box className="list-shell">
            <Typography
              variant="h6"
              sx={{
                mb: 1,
              }}
            >
              Add Knowledge Base Item
            </Typography>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 5 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Title"
                  placeholder="How we deploy production apps"
                  value={knowledgeTitle}
                  onChange={(e) => setKnowledgeTitle(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 3 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Source (optional)"
                  placeholder="runbook"
                  value={knowledgeSource}
                  onChange={(e) => setKnowledgeSource(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="URL (optional)"
                  placeholder="https://..."
                  value={knowledgeUrl}
                  onChange={(e) => setKnowledgeUrl(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={3}
                  label="Content"
                  placeholder="Durable, reusable knowledge"
                  value={knowledgeContent}
                  onChange={(e) => setKnowledgeContent(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 9 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Tags (optional)"
                  placeholder="ops, deployment, production"
                  value={knowledgeTags}
                  onChange={(e) => setKnowledgeTags(e.target.value)}
                />
              </Grid2>
              <Grid2
                size={{ xs: 12, md: 3 }}
                sx={{
                  display: "flex",
                  justifyContent: { xs: "flex-end", md: "stretch" },
                  alignItems: "stretch",
                }}
              >
                <Button
                  fullWidth
                  variant="contained"
                  disabled={
                    createKnowledgeMutation.isPending ||
                    !knowledgeTitle.trim() ||
                    !knowledgeContent.trim()
                  }
                  onClick={async () => {
                    setError(null);
                    try {
                      await createKnowledgeMutation.mutateAsync({
                        title: knowledgeTitle.trim(),
                        content: knowledgeContent.trim(),
                        source: knowledgeSource.trim() || undefined,
                        url: knowledgeUrl.trim() || undefined,
                        tags: knowledgeTags.trim() || undefined,
                      });
                      setKnowledgeTitle("");
                      setKnowledgeContent("");
                      setKnowledgeSource("");
                      setKnowledgeUrl("");
                      setKnowledgeTags("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  Save Knowledge
                </Button>
              </Grid2>
            </Grid2>
          </Box>

          <Box className="list-shell">
            {/* "Knowledge Base" h6 header removed — chip says it. */}
            {knowledgeQ.error ? (
              <Alert severity="error">{errMessage(knowledgeQ.error)}</Alert>
            ) : null}
            {knowledgeItems.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No knowledge items yet.
              </Typography>
            ) : (
              <>
                <TableContainer className="table-shell">
                  <Table size="small" sx={{ tableLayout: "fixed" }}>
                    <TableHead>
                      <TableRow>
                        <TableCell sx={{ width: "68%" }}>Item</TableCell>
                        <TableCell sx={{ width: 140 }}>Updated</TableCell>
                        <TableCell align="right" sx={{ width: 64 }}>
                          Ops
                        </TableCell>
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {knowledgeItems.map((item, idx) => {
                        const id = str(item.id, String(idx));
                        const title = knowledgeDisplayTitle(item);
                        const content = str(item.content, "-");
                        const source = knowledgeSourceLabel(item);
                        const isRuntimeCatalog =
                          isRuntimeActionCatalogKnowledgeItem(item);
                        const runtimeEntries = isRuntimeCatalog
                          ? parseRuntimeActionCatalogEntries(content)
                          : [];
                        const runtimeSection = isRuntimeCatalog
                          ? runtimeCatalogSectionLabel(item)
                          : null;
                        const tags = parseKnowledgeTags(item.tags);
                        const preview = isRuntimeCatalog
                          ? runtimeEntries.length
                            ? `${runtimeEntries.length} available action${runtimeEntries.length === 1 ? "" : "s"} in this section. Open to see what each one does and when AgentArk uses it.`
                            : "Open to review the actions this AgentArk instance can run directly."
                          : content.replace(/\s+/g, " ").trim() || "-";
                        const meta = [
                          source || null,
                          runtimeSection,
                          isRuntimeCatalog
                            ? null
                            : tags.length
                            ? `${tags.length} tag${tags.length === 1 ? "" : "s"}`
                            : null,
                        ]
                          .filter(Boolean)
                          .join(" | ");
                        const updatedAt = humanTs(str(item.updated_at, "-"));
                        return (
                          <TableRow
                            key={id}
                            hover
                            tabIndex={0}
                            onClick={() => setSelectedKnowledge(item)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                setSelectedKnowledge(item);
                              }
                            }}
                            sx={{
                              cursor: "pointer",
                              "& > td": {
                                verticalAlign: "top",
                              },
                            }}
                          >
                            <TableCell sx={{ pr: 2 }}>
                              <Stack spacing={0.45}>
                                <Typography
                                  variant="body2"
                                  sx={{ fontWeight: 600 }}
                                  noWrap
                                  title={title}
                                >
                                  {title}
                                </Typography>
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                    display: "-webkit-box",
                                    WebkitBoxOrient: "vertical",
                                    WebkitLineClamp: 2,
                                    overflow: "hidden",
                                    whiteSpace: "normal",
                                    lineHeight: 1.45,
                                  }}
                                >
                                  {preview}
                                </Typography>
                                {meta ? (
                                  <Typography
                                    variant="caption"
                                    sx={{ color: "text.secondary" }}
                                    noWrap
                                    title={meta}
                                  >
                                    {meta}
                                  </Typography>
                                ) : null}
                              </Stack>
                            </TableCell>
                            <TableCell
                              sx={{ whiteSpace: "nowrap" }}
                              title={updatedAt.tip}
                            >
                              {updatedAt.label}
                            </TableCell>
                            <TableCell
                              align="right"
                              onClick={(e) => e.stopPropagation()}
                              onKeyDown={(e) => e.stopPropagation()}
                            >
                              <RowOpsMenu
                                actions={[
                                  {
                                    label: "Delete",
                                    tone: "error",
                                    divider: true,
                                    onClick: () =>
                                      confirmDeleteTarget({
                                        kind: "knowledge",
                                        id,
                                        label: title,
                                      }),
                                  },
                                ]}
                                ariaLabel="Knowledge options"
                              />
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </TableContainer>
                <TablePagination
                  component="div"
                  count={knowledgeTotal}
                  page={memoryPages.knowledge}
                  onPageChange={(_event, nextPage) =>
                    setMemoryPage("knowledge", nextPage)
                  }
                  rowsPerPage={MEMORY_PAGE_SIZE}
                  rowsPerPageOptions={[MEMORY_PAGE_SIZE]}
                />
              </>
            )}
          </Box>
        </Stack>
      ) : null}
      {statsQ.error ||
      factsQ.error ||
      assistantPreferencesQ.error ||
      workPreferencesQ.error ||
      domainMemoryQ.error ||
      otherMemoryQ.error ||
      preferencesQ.error ||
      userDataQ.error ||
      knowledgeQ.error ||
      deleteLearnedMemoryMutation.error ||
      updateLearnedMemoryMutation.error ||
      error ? (
        <Alert severity="error">
          {error ||
            errMessage(
              statsQ.error ||
                factsQ.error ||
                assistantPreferencesQ.error ||
                workPreferencesQ.error ||
                domainMemoryQ.error ||
                otherMemoryQ.error ||
                preferencesQ.error ||
                userDataQ.error ||
                knowledgeQ.error ||
                deleteLearnedMemoryMutation.error ||
                updateLearnedMemoryMutation.error,
            )}
        </Alert>
      ) : null}
      <Dialog
        open={selectedFact != null}
        onClose={() => setSelectedFact(null)}
        maxWidth="sm"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              background: "rgba(14, 14, 16, 0.96)",
              border: "1px solid rgba(255, 255, 255, 0.08)",
              borderRadius: 2,
              backdropFilter: "blur(8px)",
              backgroundImage: "none",
            },
          },
        }}
      >
        {(() => {
          const fact = selectedFact;
          if (!fact) return null;
          const factText = memoryFactDisplayText(fact);
          const factKey = memoryFactKey(fact);
          const factValue = memoryFactEditableValue(fact);
          const confidence = num(fact.confidence, 0);
          const confidenceColor =
            confidence >= 0.85
              ? {
                  color: "#7be3a1",
                  borderColor: "rgba(123, 227, 161, 0.34)",
                  background: "rgba(123, 227, 161, 0.08)",
                }
              : confidence >= 0.6
                ? {
                    color: "#e3c47b",
                    borderColor: "rgba(227, 196, 123, 0.34)",
                    background: "rgba(227, 196, 123, 0.08)",
                  }
                : {
                    color: "#e37b8a",
                    borderColor: "rgba(227, 123, 138, 0.34)",
                    background: "rgba(227, 123, 138, 0.08)",
                  };
          const created = humanTs(str(fact.created_at, "-"));
          const updated = humanTs(str(fact.updated_at, ""));
          const factId = str(fact.id, "").trim();
          const factKind = str(fact.kind, "").trim() || "profile_fact";
          const factScope = str(fact.scope, "").trim();
          const sources = parseSources(fact.sources);
          const metaRows: Array<{ label: string; value: string; mono?: boolean }> = [];
          if (factId) metaRows.push({ label: "ID", value: factId, mono: true });
          metaRows.push({ label: "Kind", value: factKind, mono: true });
          if (factScope) metaRows.push({ label: "Scope", value: factScope });
          metaRows.push({
            label: "Confidence",
            value: confidence.toFixed(2),
            mono: true,
          });
          metaRows.push({ label: "Created", value: created.tip });
          if (updated.label && updated.label !== "-") {
            metaRows.push({ label: "Updated", value: updated.tip });
          }
          metaRows.push({
            label: "Evidence",
            value: sources.length > 0 ? `${sources.length} reference${sources.length === 1 ? "" : "s"}` : "—",
          });
          return (
            <>
              <DialogTitle
                sx={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 1,
                  pb: 1.5,
                  pt: 2,
                  px: 2.5,
                  borderBottom: "1px solid rgba(255, 255, 255, 0.06)",
                }}
              >
                <Stack
                  direction="row"
                  spacing={0.75}
                  sx={{ alignItems: "center", flexWrap: "wrap" }}
                >
                  <Chip
                    size="small"
                    variant="outlined"
                    label="Profile Fact"
                    sx={{
                      height: 22,
                      fontSize: "0.66rem",
                      fontWeight: 600,
                      letterSpacing: "0.06em",
                      textTransform: "uppercase",
                      color: "rgba(216, 173, 120, 0.85)",
                      borderColor: "rgba(216, 173, 120, 0.3)",
                      background: "rgba(216, 173, 120, 0.06)",
                      "& .MuiChip-label": { px: 1 },
                    }}
                  />
                  <Chip
                    size="small"
                    variant="outlined"
                    label={confidence.toFixed(2)}
                    sx={{
                      height: 22,
                      fontSize: "0.66rem",
                      fontWeight: 600,
                      letterSpacing: "0.06em",
                      fontFamily: "var(--font-mono)",
                      ...confidenceColor,
                      "& .MuiChip-label": { px: 1 },
                    }}
                  />
                  <Box sx={{ flex: 1 }} />
                  <Typography
                    variant="caption"
                    sx={{
                      color: "rgba(220, 220, 220, 0.55)",
                      fontFamily: "var(--font-mono)",
                      fontSize: "0.68rem",
                      letterSpacing: "0.02em",
                    }}
                    title={created.tip}
                  >
                    {created.label}
                  </Typography>
                </Stack>
                {factKey ? (
                  <Typography
                    sx={{
                      fontFamily: "var(--font-mono)",
                      fontSize: "0.7rem",
                      letterSpacing: "0.04em",
                      color: "rgba(220, 220, 220, 0.55)",
                    }}
                  >
                    {factKey}
                  </Typography>
                ) : null}
                <Typography
                  sx={{
                    fontSize: "1rem",
                    fontWeight: 600,
                    color: "var(--text-primary)",
                    lineHeight: 1.4,
                    whiteSpace: "pre-wrap",
                  }}
                >
                  {factValue || "—"}
                </Typography>
              </DialogTitle>
              <DialogContent sx={{ px: 2.5, py: 2 }}>
                <Stack spacing={2}>
                  <Box
                    sx={{
                      borderRadius: 1.5,
                      border: "1px solid rgba(255, 255, 255, 0.06)",
                      background: "rgba(255, 255, 255, 0.012)",
                    }}
                  >
                    <Stack>
                      {metaRows.map((row, mIdx) => (
                        <Stack
                          key={row.label}
                          direction="row"
                          sx={{
                            px: 1.5,
                            py: 0.85,
                            gap: 1.5,
                            alignItems: "baseline",
                            borderTop:
                              mIdx === 0
                                ? "none"
                                : "1px solid rgba(255, 255, 255, 0.04)",
                          }}
                        >
                          <Typography
                            sx={{
                              fontFamily: "var(--font-mono)",
                              fontSize: "0.68rem",
                              letterSpacing: "0.04em",
                              color: "rgba(220, 220, 220, 0.52)",
                              textTransform: "uppercase",
                              flex: "0 0 110px",
                              lineHeight: 1.4,
                            }}
                          >
                            {row.label}
                          </Typography>
                          <Typography
                            sx={{
                              flex: 1,
                              minWidth: 0,
                              fontFamily: row.mono
                                ? "var(--font-mono)"
                                : undefined,
                              fontSize: row.mono ? "0.78rem" : "0.84rem",
                              color: "var(--text-primary)",
                              wordBreak: "break-all",
                              lineHeight: 1.4,
                            }}
                          >
                            {row.value}
                          </Typography>
                        </Stack>
                      ))}
                    </Stack>
                  </Box>
                  {sources.length > 0 ? (
                    <Box>
                      <Typography
                        sx={{
                          fontFamily: "var(--font-mono)",
                          fontSize: "0.68rem",
                          letterSpacing: "0.04em",
                          color: "rgba(220, 220, 220, 0.52)",
                          textTransform: "uppercase",
                          mb: 0.75,
                        }}
                      >
                        Evidence references
                      </Typography>
                      <Box
                        sx={{
                          borderRadius: 1.5,
                          border: "1px solid rgba(255, 255, 255, 0.06)",
                          background: "rgba(255, 255, 255, 0.012)",
                        }}
                      >
                        <Stack>
                          {sources.slice(0, 50).map((s, i) => (
                            <Box
                              key={`src-${i}`}
                              sx={{
                                px: 1.5,
                                py: 0.7,
                                borderTop:
                                  i === 0
                                    ? "none"
                                    : "1px solid rgba(255, 255, 255, 0.04)",
                                fontFamily: "var(--font-mono)",
                                fontSize: "0.76rem",
                                color: "var(--text-primary)",
                                wordBreak: "break-all",
                                lineHeight: 1.45,
                              }}
                            >
                              {String(s)}
                            </Box>
                          ))}
                        </Stack>
                      </Box>
                    </Box>
                  ) : null}
                </Stack>
              </DialogContent>
              <DialogActions
                sx={{
                  px: 2.5,
                  py: 1.5,
                  borderTop: "1px solid rgba(255, 255, 255, 0.06)",
                  gap: 1,
                }}
              >
                <Button
                  size="small"
                  variant="outlined"
                  onClick={() => openMemoryEdit(fact)}
                  disabled={!factId}
                  sx={{
                    textTransform: "none",
                    fontSize: "0.78rem",
                    fontWeight: 500,
                  }}
                >
                  Edit value
                </Button>
                <Button
                  size="small"
                  variant="outlined"
                  color="error"
                  onClick={() => {
                    const id = factId;
                    if (!id) return;
                    confirmDeleteTarget({
                      kind: "learnedMemory",
                      id,
                      label: factText || id,
                    });
                  }}
                  disabled={!factId}
                  sx={{
                    textTransform: "none",
                    fontSize: "0.78rem",
                    fontWeight: 500,
                  }}
                >
                  Delete
                </Button>
                <Box sx={{ flex: 1 }} />
                {onViewMemoryEvidence ? (
                  <Button
                    size="small"
                    onClick={() => {
                      if (!factId) return;
                      setSelectedFact(null);
                      onViewMemoryEvidence(factId);
                    }}
                    disabled={!factId}
                    sx={{
                      textTransform: "none",
                      fontSize: "0.78rem",
                      color: "rgba(220, 220, 220, 0.75)",
                    }}
                  >
                    View evidence
                  </Button>
                ) : null}
                <Button
                  size="small"
                  onClick={() => setSelectedFact(null)}
                  sx={{
                    textTransform: "none",
                    fontSize: "0.78rem",
                    color: "rgba(220, 220, 220, 0.75)",
                  }}
                >
                  Close
                </Button>
              </DialogActions>
            </>
          );
        })()}
      </Dialog>
      <Dialog
        open={selectedKnowledge != null}
        onClose={() => setSelectedKnowledge(null)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>{knowledgeDisplayTitle(selectedKnowledge)}</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.5}>
            <Stack
              direction="row"
              spacing={0.75}
              useFlexGap
              sx={{ flexWrap: "wrap" }}
            >
              {selectedKnowledgeSource ? (
                <Chip size="small" variant="outlined" label={selectedKnowledgeSource} />
              ) : null}
              {selectedKnowledgeSection ? (
                <Chip
                  size="small"
                  variant="outlined"
                  label={selectedKnowledgeSection}
                />
              ) : null}
            </Stack>
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              Updated{" "}
              <span title={humanTs(str(selectedKnowledge?.updated_at, "-")).tip}>
                {humanTs(str(selectedKnowledge?.updated_at, "-")).label}
              </span>
            </Typography>
            {selectedKnowledgeIsRuntimeCatalog ? (
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                These are the built-in actions this AgentArk instance can run
                directly when the request needs them and the right connections
                or credentials are already available.
              </Typography>
            ) : null}
            {selectedKnowledgeShowsExternalUrl ? (
              <Link
                href={selectedKnowledgeUrl}
                target="_blank"
                rel="noreferrer"
                underline="hover"
              >
                {selectedKnowledgeUrl}
              </Link>
            ) : isInternalAgentArkHelpUrl(selectedKnowledgeUrl) ? (
              <Typography variant="caption" sx={{ color: "text.secondary" }}>
                Built into AgentArk. No external link is needed for this guide.
              </Typography>
            ) : null}
            {!selectedKnowledgeIsRuntimeCatalog && selectedKnowledgeTags.length ? (
              <Stack
                direction="row"
                spacing={0.75}
                useFlexGap
                sx={{ flexWrap: "wrap" }}
              >
                {selectedKnowledgeTags.map((tag) => (
                  <Chip key={tag} size="small" variant="outlined" label={tag} />
                ))}
              </Stack>
            ) : null}
            <Divider />
            {selectedKnowledgeIsRuntimeCatalog ? (
              selectedKnowledgeActions.length ? (
                <Box
                  sx={{
                    border: "1px solid var(--surface-border)",
                    borderRadius: "var(--surface-radius)",
                    background: "var(--ui-rgba-255-255-255-020)",
                    overflow: "hidden",
                  }}
                >
                  <Stack divider={<Divider flexItem />}>
                    {selectedKnowledgeActions.map((action) => (
                      <Stack
                        key={action.actionId}
                        spacing={0.8}
                        sx={{ px: 1.5, py: 1.35 }}
                      >
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          spacing={1}
                          useFlexGap
                          sx={{
                            justifyContent: "space-between",
                            alignItems: { xs: "flex-start", sm: "flex-start" },
                          }}
                        >
                          <Stack spacing={0.3} sx={{ minWidth: 0 }}>
                            <Typography
                              variant="subtitle1"
                              sx={{ fontWeight: 700 }}
                            >
                              {action.displayName}
                            </Typography>
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                                fontFamily: "JetBrains Mono, monospace",
                              }}
                            >
                              {action.actionId}
                            </Typography>
                          </Stack>
                          <Stack
                            direction="row"
                            spacing={0.6}
                            useFlexGap
                            sx={{ flexWrap: "wrap" }}
                          >
                            {action.capabilities.length ? (
                              action.capabilities.map((capability) => (
                                <Chip
                                  key={`${action.actionId}-${capability}`}
                                  size="small"
                                  variant="outlined"
                                  label={humanizeCatalogToken(capability)}
                                />
                              ))
                            ) : (
                              <Chip
                                size="small"
                                variant="outlined"
                                label="Built-in"
                              />
                            )}
                          </Stack>
                        </Stack>
                        <Typography variant="body2">{action.summary}</Typography>
                        {action.details ? (
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                              lineHeight: 1.5,
                              whiteSpace: "pre-wrap",
                            }}
                          >
                            {action.details}
                          </Typography>
                        ) : null}
                      </Stack>
                    ))}
                  </Stack>
                </Box>
              ) : (
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  No actions were available in this guide snapshot.
                </Typography>
              )
            ) : (
              <Typography variant="body2" sx={{ whiteSpace: "pre-wrap" }}>
                {selectedKnowledgeContent}
              </Typography>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            color="error"
            onClick={() => {
              const id = str(selectedKnowledge?.id, "").trim();
              if (!id) return;
              confirmDeleteTarget({
                kind: "knowledge",
                id,
                label: knowledgeDisplayTitle(selectedKnowledge),
              });
            }}
            disabled={!str(selectedKnowledge?.id, "").trim()}
          >
            Delete
          </Button>
          <Button onClick={() => setSelectedKnowledge(null)}>Close</Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={editTarget != null}
        onClose={() => {
          if (!editBusy) {
            setEditTarget(null);
            setEditValue("");
          }
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Edit Memory Value</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Box className="metadata-box">
              <Typography
                variant="body2"
                sx={{ overflowWrap: "anywhere", whiteSpace: "pre-wrap" }}
              >
                {editTarget?.label || editTarget?.id || "Selected memory"}
              </Typography>
            </Box>
            <TextField
              autoFocus
              fullWidth
              multiline
              minRows={5}
              label="Value"
              value={editValue}
              onChange={(event) => setEditValue(event.target.value)}
              disabled={editBusy}
            />
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            disabled={editBusy}
            onClick={() => {
              setEditTarget(null);
              setEditValue("");
            }}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            disabled={editBusy || !editTarget || !editValue.trim()}
            onClick={() => void saveMemoryEdit()}
          >
            Save
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={deleteTarget != null}
        onClose={() => {
          if (!deleteBusy) setDeleteTarget(null);
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Delete Memory Forever?</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1}>
            <Typography variant="body2">
              This will permanently delete this memory from the backend.
            </Typography>
            <Box className="metadata-box">
              <Typography
                variant="body2"
                sx={{ overflowWrap: "anywhere", whiteSpace: "pre-wrap" }}
              >
                {deleteTarget?.label || deleteTarget?.id || "Selected memory"}
              </Typography>
            </Box>
            <Typography variant="caption" sx={{ color: "text.secondary" }}>
              This action does not create a rollback entry and cannot be undone.
            </Typography>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button disabled={deleteBusy} onClick={() => setDeleteTarget(null)}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={deleteBusy || !deleteTarget}
            onClick={runConfirmedDelete}
          >
            Delete forever
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
