import type { JsonRecord } from "./pageHelpers";

export const ADVANCED_SENTINEL_SIGNAL_OPTIONS = [
  {
    key: "enabled",
    label: "Keep ArkSentinel available",
    description: "Lets it stay ready in the background.",
    enabledMessage: "ArkSentinel stays available in the background.",
    disabledMessage: "ArkSentinel follow-up scanning is turned off.",
  },
  {
    key: "watch_in_app",
    label: "Watch chats and runs",
    description:
      "Uses AgentArk chats and execution runs to spot failed, blocked, stalled, or needs-input follow-ups.",
    enabledMessage: "ArkSentinel will watch AgentArk chats and runs for follow-ups.",
    disabledMessage: "ArkSentinel will ignore AgentArk chats and runs.",
  },
  {
    key: "watch_connected_services",
    label: "Pay attention to connected apps",
    description:
      "Uses signals from Gmail, Calendar, Slack, and other services once you connect them.",
    enabledMessage: "ArkSentinel will watch connected apps for follow-ups.",
    disabledMessage: "ArkSentinel will ignore connected-app signals.",
  },
  {
    key: "infer_new_automations",
    label: "Look for routines worth automating",
    description:
      "Uses recent activity to surface one daily automation review plus concrete reminder, watcher, or workflow opportunities.",
    enabledMessage: "ArkSentinel will keep reviewing recent activity for automation opportunities.",
    disabledMessage:
      "ArkSentinel will stop sending daily automation reviews or proposing new routines from recent work.",
  },
] as const;

export const MODEL_PROVIDER_OPTIONS = [
  { value: "ollama", label: "Ollama" },
  { value: "anthropic", label: "Anthropic" },
  { value: "openai", label: "OpenAI" },
  { value: "openrouter", label: "OpenRouter" },
  { value: "huggingface", label: "Hugging Face Inference" },
  { value: "openai-compatible", label: "OpenAI Compatible" },
];

export type TrustApprovalPreset = {
  id: string;
  label: string;
  actionKind: string;
  detailLabel: string;
  detailPlaceholder: string;
  buildPayload: (detail: string) => JsonRecord;
};

export const TRUST_APPROVAL_PRESETS: TrustApprovalPreset[] = [
  {
    id: "run_terminal_command",
    label: "Run a terminal command",
    actionKind: "shell",
    detailLabel: "Command",
    detailPlaceholder: "ls -la",
    buildPayload: (detail) => ({ command: detail }),
  },
  {
    id: "read_file",
    label: "Read a file",
    actionKind: "file_read",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/report.txt",
    buildPayload: (detail) => ({ path: detail }),
  },
  {
    id: "write_file",
    label: "Create or edit a file",
    actionKind: "file_write",
    detailLabel: "File path",
    detailPlaceholder: "/app/data/notes.txt",
    buildPayload: (detail) => ({ path: detail, operation: "write" }),
  },
  {
    id: "open_url",
    label: "Open a URL or call an API",
    actionKind: "http_get",
    detailLabel: "URL",
    detailPlaceholder: "https://api.example.com/status",
    buildPayload: (detail) => ({ url: detail }),
  },
  {
    id: "run_code",
    label: "Run generated code",
    actionKind: "code_execute",
    detailLabel: "What should the code do?",
    detailPlaceholder: "Summarize CSV rows and return totals",
    buildPayload: (detail) => ({ instruction: detail }),
  },
  {
    id: "email_action",
    label: "Read or send an email",
    actionKind: "gmail_reply",
    detailLabel: "Email task",
    detailPlaceholder: "Reply with a short status update",
    buildPayload: (detail) => ({ message: detail }),
  },
];
