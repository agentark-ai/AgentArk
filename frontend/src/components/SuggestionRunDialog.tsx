import CloseIcon from "@mui/icons-material/Close";
import { Alert, Box, Button, Chip, Dialog, DialogActions, DialogContent, DialogTitle, IconButton, Stack, Typography } from "@mui/material";

type JsonRecord = Record<string, unknown>;

export type SuggestionRunState = {
  title: string;
  status: "running" | "completed" | "error";
  summary: string;
  traceId?: string;
  startedAt?: string;
  completedAt?: string;
  suggestionId?: string;
};

type ConsoleView = {
  detail: string;
  dataText: string;
};

type ChipColor = "default" | "success" | "warning" | "error" | "info";

type Props = {
  run: SuggestionRunState | null;
  open: boolean;
  minimized: boolean;
  trace: JsonRecord;
  traceSteps: JsonRecord[];
  traceLoading: boolean;
  traceError: unknown;
  detailError: unknown;
  acceptedOutcomes: JsonRecord[];
  onClose: () => void;
  onMinimize: () => void;
  onRestore: () => void;
  onOpenWorkspacePanel: (view: string) => void;
  getConsoleView: (step: JsonRecord) => ConsoleView;
  getTraceStepColor: (stepType: string) => ChipColor;
  humanTs: (raw: string) => { label: string; tip: string };
  errMessage: (error: unknown) => string;
};

function asRecord(value: unknown): JsonRecord {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as JsonRecord) : {};
}

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function toBool(value: unknown): boolean {
  return value === true || value === "true" || value === 1 || value === "1";
}

function suggestionKindColor(kind: string): ChipColor {
  const normalized = kind.toLowerCase();
  if (normalized === "watcher") return "info";
  if (normalized === "app") return "success";
  if (normalized === "workflow") return "warning";
  return "default";
}

function suggestionOutcomeStatusColor(status: string): ChipColor {
  const normalized = status.toLowerCase();
  if (normalized === "running" || normalized === "active" || normalized === "completed" || normalized === "triggered") {
    return "success";
  }
  if (normalized === "pending" || normalized === "paused" || normalized === "awaiting_approval") {
    return "warning";
  }
  if (normalized === "failed" || normalized === "cancelled" || normalized === "timed_out" || normalized === "stopped") {
    return "error";
  }
  return "default";
}

function workspaceLabel(view: string): string {
  const normalized = view.toLowerCase();
  if (normalized === "watcher" || normalized === "watchers" || normalized === "status") return "Open Watchers";
  if (normalized === "task" || normalized === "tasks") return "Open Tasks";
  if (normalized === "app" || normalized === "apps") return "Open Apps";
  if (normalized === "session" || normalized === "sessions") return "Open Sessions";
  if (normalized === "project" || normalized === "projects") return "Open Projects";
  if (normalized === "document" || normalized === "documents" || normalized === "file" || normalized === "files") {
    return "Open Documents";
  }
  if (normalized === "skill" || normalized === "skills") return "Open Skills";
  if (normalized === "goal" || normalized === "goals") return "Open Goals";
  return `Open ${view}`;
}

export function SuggestionRunDialog({
  run,
  open,
  minimized,
  trace,
  traceSteps,
  traceLoading,
  traceError,
  detailError,
  acceptedOutcomes,
  onClose,
  onMinimize,
  onRestore,
  onOpenWorkspacePanel,
  getConsoleView,
  getTraceStepColor,
  humanTs,
  errMessage
}: Props) {
  return (
    <>
      <Dialog open={open && !minimized && run != null} onClose={onClose} maxWidth="md" fullWidth>
        <DialogTitle sx={{ display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 2 }}>
          <Box>
            <Typography variant="h6">{run?.title || "Suggestion Run"}</Typography>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              Live execution trace for this run
            </Typography>
          </Box>
            <Stack direction="row" spacing={1}>
              {run?.traceId ? (
                <Button size="small" onClick={() => onOpenWorkspacePanel("trace")}>
                  Open Trace
                </Button>
              ) : null}
              <Button size="small" onClick={onMinimize}>
                Minimize
              </Button>
            <IconButton size="small" onClick={onClose}>
              <CloseIcon fontSize="small" />
            </IconButton>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={2}>
            <Stack direction="row" spacing={1} useFlexGap sx={{
              flexWrap: "wrap"
            }}>
              <Chip
                size="small"
                color={run?.status === "completed" ? "success" : run?.status === "error" ? "error" : "warning"}
                label={run?.status || "running"}
              />
              {run?.traceId ? <Chip size="small" variant="outlined" label={`Trace ${run.traceId}`} /> : null}
              {run?.startedAt ? <Chip size="small" variant="outlined" label={`Started ${humanTs(run.startedAt).label}`} /> : null}
              {run?.completedAt ? <Chip size="small" variant="outlined" label={`Completed ${humanTs(run.completedAt).label}`} /> : null}
            </Stack>
            <Alert severity={run?.status === "completed" ? "success" : run?.status === "error" ? "error" : "info"}>
              {run?.summary || "Running suggestion acceptance..."}
            </Alert>
            {traceError ? <Alert severity="error">{errMessage(traceError)}</Alert> : null}
            {detailError ? <Alert severity="error">{errMessage(detailError)}</Alert> : null}
            {acceptedOutcomes.length > 0 ? (
              <Box className="list-shell">
                <Typography variant="subtitle2" sx={{
                  mb: 1
                }}>Saved Outcome</Typography>
                <Stack spacing={1}>
                  {acceptedOutcomes.map((outcome, idx) => {
                    const kind = str(outcome.kind, "artifact");
                    const view = str(outcome.view, "");
                    const url = str(outcome.url, "").trim();
                    const title = str(outcome.title, `${kind} ${idx + 1}`).trim() || `${kind} ${idx + 1}`;
                    const detail = str(outcome.detail, "").trim();
                    const status = str(outcome.status, "").trim();
                    const createdAt = str(outcome.created_at, "").trim();
                    return (
                      <Box key={`${str(outcome.id, `outcome-${idx}`)}-${kind}`} className="action-row">
                        <Stack spacing={0.9}>
                          <Stack
                            direction={{ xs: "column", sm: "row" }}
                            spacing={1}
                            sx={{
                              justifyContent: "space-between",
                              alignItems: { xs: "flex-start", sm: "center" }
                            }}>
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                                alignItems: "center"
                              }}>
                              <Chip size="small" color={suggestionKindColor(kind)} label={kind} />
                              {status ? <Chip size="small" variant="outlined" color={suggestionOutcomeStatusColor(status)} label={status} /> : null}
                              {toBool(outcome.primary) ? <Chip size="small" variant="outlined" label="primary" /> : null}
                            </Stack>
                            <Stack direction="row" spacing={1}>
                              {view ? (
                                <Button size="small" variant="outlined" onClick={() => onOpenWorkspacePanel(view)}>
                                  {workspaceLabel(view)}
                                </Button>
                              ) : null}
                              {url ? (
                                <Button size="small" component="a" href={url} target="_blank" rel="noreferrer">
                                  Open
                                </Button>
                              ) : null}
                            </Stack>
                          </Stack>
                          <Typography variant="body2" sx={{ fontWeight: 600 }}>
                            {title}
                          </Typography>
                          {detail ? (
                            <Typography variant="caption" sx={{
                              color: "text.secondary"
                            }}>
                              {detail}
                            </Typography>
                          ) : null}
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            ID: {str(outcome.id, "-")}
                            {createdAt ? ` | Created ${humanTs(createdAt).label}` : ""}
                          </Typography>
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            ) : null}
            <Box className="list-shell">
              <Box className="micro-surface-head">
                <Typography className="micro-surface-kicker">Diagnostics</Typography>
                <Typography className="micro-surface-title">Execution log</Typography>
                <Typography className="micro-surface-copy">Step-by-step activity captured while this run was in progress.</Typography>
              </Box>
              <Box className="metadata-box micro-surface" sx={{ maxHeight: 360 }}>
                {traceLoading && !traceSteps.length ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>Waiting for trace...</Typography>
                ) : traceSteps.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>Trace initialized. Waiting for the first execution step...</Typography>
                ) : (
                  <Stack spacing={1}>
                    {traceSteps.map((step, idx) => {
                      const stepRecord = asRecord(step);
                      const consoleView = getConsoleView(stepRecord);
                      return (
                        <Box key={`${str(stepRecord.time, "step")}-${idx}`} className="console-line">
                          <Stack spacing={0.75}>
                            <Stack
                              direction="row"
                              spacing={1}
                              useFlexGap
                              sx={{
                                alignItems: "center",
                                flexWrap: "wrap"
                              }}>
                              <Chip size="small" color={getTraceStepColor(str(stepRecord.type || stepRecord.step_type, "step"))} label={str(stepRecord.type || stepRecord.step_type, "step")} />
                              <Typography variant="caption" sx={{
                                color: "text.secondary"
                              }}>{str(stepRecord.time)}</Typography>
                            </Stack>
                            <Typography variant="body2" sx={{
                              fontWeight: 600
                            }}>{str(stepRecord.title)}</Typography>
                            <Typography
                              variant="caption"
                              sx={{
                                color: "text.secondary",
                                whiteSpace: "pre-wrap"
                              }}>
                              {consoleView.detail}
                            </Typography>
                            {consoleView.dataText ? (
                              <Box
                                component="pre"
                                className="micro-surface-scroll"
                                sx={{
                                  m: 0,
                                  whiteSpace: "pre-wrap",
                                  wordBreak: "break-word",
                                  overflowX: "auto",
                                  fontSize: 12
                                }}
                              >
                                {consoleView.dataText}
                              </Box>
                            ) : null}
                          </Stack>
                        </Box>
                      );
                    })}
                  </Stack>
                )}
              </Box>
            </Box>
            {str(trace.response, "").trim() ? (
              <Box className="list-shell">
                <Box className="micro-surface-head">
                  <Typography className="micro-surface-kicker">Diagnostics</Typography>
                  <Typography className="micro-surface-title">Agent response</Typography>
                  <Typography className="micro-surface-copy">The final response captured for this run.</Typography>
                </Box>
                <Box
                  component="pre"
                  className="micro-surface-scroll"
                  sx={{
                    m: 0,
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    overflowX: "auto",
                    fontSize: 12
                  }}
                >
                  {str(trace.response, "")}
                </Box>
              </Box>
            ) : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={onMinimize}>Minimize</Button>
          <Button onClick={onClose}>Close</Button>
        </DialogActions>
      </Dialog>
      {open && minimized && run ? (
        <Box
          sx={{
            position: "fixed",
            right: 20,
            bottom: 20,
            zIndex: 1600,
            width: { xs: "calc(100vw - 32px)", sm: 360 },
            p: 1.5,
            borderRadius: 2,
            bgcolor: "rgba(8,14,28,0.96)",
            border: "1px solid rgba(100,160,230,0.24)",
            boxShadow: "0 18px 50px rgba(0,0,0,0.35)"
          }}
        >
          <Stack spacing={1}>
            <Stack
              direction="row"
              sx={{
                justifyContent: "space-between",
                alignItems: "center"
              }}>
              <Typography variant="subtitle2" noWrap title={run.title}>
                {run.title}
              </Typography>
              <Stack direction="row" spacing={0.5}>
                <Button size="small" onClick={onRestore}>
                  Open
                </Button>
                <IconButton size="small" onClick={onClose}>
                  <CloseIcon fontSize="small" />
                </IconButton>
              </Stack>
            </Stack>
            <Typography variant="caption" sx={{
              color: "text.secondary"
            }}>
              {run.summary}
            </Typography>
          </Stack>
        </Box>
      ) : null}
    </>
  );
}
