import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography
} from "@mui/material";

type JsonRecord = Record<string, unknown>;

type ObservabilityValues = {
  enabled: boolean;
  provider: string;
  endpoint: string;
  serviceName: string;
  headerName: string;
  privacyMode: string;
  authToken: string;
  authTokenConfigured: boolean;
};

type ObservabilityPanelProps = {
  values: ObservabilityValues;
  logs: JsonRecord[];
  issues: string[];
  logsLoading: boolean;
  logsError: string | null;
  testing: boolean;
  onValueChange: (next: Partial<ObservabilityValues>) => void;
  onTest: () => void;
};

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function humanTs(value: string): { label: string; tip: string } {
  const trimmed = (value || "").trim();
  if (!trimmed) return { label: "-", tip: "-" };
  const parsed = new Date(trimmed);
  if (Number.isNaN(parsed.getTime())) return { label: trimmed, tip: trimmed };
  return {
    label: parsed.toLocaleString(),
    tip: parsed.toISOString()
  };
}

export function ObservabilityPanel({
  values,
  logs,
  issues,
  logsLoading,
  logsError,
  testing,
  onValueChange,
  onTest
}: ObservabilityPanelProps) {
  const provider = (values.provider || "langtrace").trim().toLowerCase();
  const endpointLabel = provider === "langtrace" ? "Langtrace Base URL" : provider === "langsmith" ? "LangSmith Endpoint" : "OTLP Endpoint";
  const endpointHelper =
    provider === "langtrace"
      ? "Use the Langtrace host URL. AgentArk will push JSON traces to /api/trace."
      : provider === "langsmith"
        ? "Use the LangSmith API endpoint (e.g. https://eu.api.smith.langchain.com). AgentArk will append /otel/v1/traces."
        : "Use the OTLP/HTTP traces endpoint. AgentArk will append /v1/traces when needed.";
  const statusChip = !values.enabled
    ? { label: "Off", color: "default" as const }
    : values.endpoint.trim() && values.authTokenConfigured
      ? { label: "Ready", color: "success" as const }
      : { label: "Incomplete", color: "warning" as const };

  return (
    <Stack spacing={1.5}>
      <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
        <Typography variant="h6">Observability Export</Typography>
        <Chip size="small" label={statusChip.label} color={statusChip.color} />
        {values.authTokenConfigured ? (
          <Chip size="small" variant="outlined" label="Token saved" color="success" />
        ) : null}
      </Stack>

      <Typography variant="caption" color="text.secondary">
        Optional. When enabled and configured, AgentArk exports completed run traces to Langtrace, LangSmith, or any OTLP-compatible backend.
      </Typography>

      <FormControlLabel
        control={
          <Switch
            checked={values.enabled}
            onChange={(e) => onValueChange({ enabled: e.target.checked })}
          />
        }
        label={values.enabled ? "Observability export is enabled" : "Enable observability export"}
      />

      <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
        <TextField
          select
          label="Provider"
          size="small"
          fullWidth
          value={values.provider}
          onChange={(e) => {
            const nextProvider = e.target.value;
            onValueChange({
              provider: nextProvider,
              headerName: nextProvider === "langtrace" ? "x-api-key" : nextProvider === "langsmith" ? "x-api-key" : values.headerName
            });
          }}
        >
          <MenuItem value="langtrace">Langtrace</MenuItem>
          <MenuItem value="langsmith">LangSmith</MenuItem>
          <MenuItem value="generic_otlp">Generic OTLP</MenuItem>
        </TextField>
        <TextField
          label={provider === "langsmith" ? "Project Name" : "Service Name"}
          size="small"
          fullWidth
          value={values.serviceName}
          onChange={(e) => onValueChange({ serviceName: e.target.value })}
          placeholder="agentark"
          helperText={provider === "langsmith" ? "Maps to LANGSMITH_PROJECT. Traces go to this project in LangSmith." : undefined}
        />
      </Stack>

      <TextField
        label={endpointLabel}
        size="small"
        fullWidth
        value={values.endpoint}
        onChange={(e) => onValueChange({ endpoint: e.target.value })}
        placeholder={provider === "langtrace" ? "https://app.langtrace.ai" : provider === "langsmith" ? "https://eu.api.smith.langchain.com" : "https://collector.example.com"}
        helperText={endpointHelper}
      />

      <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
        <TextField
          label="Auth Header Name"
          size="small"
          fullWidth
          value={values.headerName}
          onChange={(e) => onValueChange({ headerName: e.target.value })}
          placeholder="x-api-key"
        />
        <TextField
          label={values.authTokenConfigured ? "API Key / Token (leave blank to keep current)" : "API Key / Token"}
          size="small"
          fullWidth
          type="password"
          value={values.authToken}
          onChange={(e) => onValueChange({ authToken: e.target.value })}
          helperText="Stored encrypted. Enter a blank value and save only if you want to keep the existing token unchanged."
        />
      </Stack>

      <Stack direction={{ xs: "column", md: "row" }} spacing={1.5} alignItems={{ md: "center" }}>
        <TextField
          select
          label="Privacy Level"
          size="small"
          fullWidth
          value={values.privacyMode}
          onChange={(e) => onValueChange({ privacyMode: e.target.value })}
        >
          <MenuItem value="metadata_only">Metadata only</MenuItem>
          <MenuItem value="redacted_content">Redacted content</MenuItem>
          <MenuItem value="full_content">Full content</MenuItem>
        </TextField>
        <Button
          variant="outlined"
          onClick={onTest}
          disabled={testing || !values.enabled}
          sx={{ whiteSpace: "nowrap", minWidth: 180 }}
        >
          {testing ? "Sending test..." : "Send Test Trace"}
        </Button>
      </Stack>

      <Alert severity={values.enabled ? "info" : "warning"}>
        {values.enabled
          ? "Exports only happen when observability is enabled and fully configured. If the endpoint or token is missing, AgentArk keeps tracing locally only."
          : "Observability export is off. AgentArk continues storing local traces in Trace."}
      </Alert>

      {issues.length > 0 ? (
        <Alert severity="error">
          {issues[0]}
          {issues.length > 1 ? ` (+${issues.length - 1} more recent export issue${issues.length === 2 ? "" : "s"})` : ""}
        </Alert>
      ) : null}

      <Divider />

      <Typography variant="caption" color="text.secondary">
        Export delivery logs are shown on the Trace page.
      </Typography>
    </Stack>
  );
}
