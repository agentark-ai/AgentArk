export type JsonRecord = Record<string, unknown>;

export type ChatRunMetrics = {
  inputTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  cachedPromptTokens?: number | null;
  cacheCreationPromptTokens?: number | null;
  durationMs?: number | null;
  timeToFirstStreamActivityMs?: number | null;
  timeToFirstTokenMs?: number | null;
  /** Provider/LLM response latency for the run (sum of per-turn model calls), ms. */
  modelLatencyMs?: number | null;
};

export type ChatRunMetricItem = {
  label: string;
  value: string;
};

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function positiveRunMetric(value: unknown): number | null {
  const amount = num(value, Number.NaN);
  if (!Number.isFinite(amount) || amount <= 0) return null;
  return amount;
}

const RUN_METRIC_INPUT_KEYS = [
  "input_tokens",
  "inputTokens",
  "prompt_tokens",
  "promptTokens",
];
const RUN_METRIC_OUTPUT_KEYS = [
  "output_tokens",
  "outputTokens",
  "completion_tokens",
  "completionTokens",
];
const RUN_METRIC_TOTAL_KEYS = ["total_tokens", "totalTokens"];
const RUN_METRIC_CACHED_PROMPT_KEYS = [
  "cached_prompt_tokens",
  "cachedPromptTokens",
  "cache_read_tokens",
  "cacheReadTokens",
];
const RUN_METRIC_CACHE_CREATION_KEYS = [
  "cache_creation_prompt_tokens",
  "cacheCreationPromptTokens",
  "cache_creation_tokens",
  "cacheCreationTokens",
];
const RUN_METRIC_DURATION_KEYS = ["duration_ms", "durationMs"];
const RUN_METRIC_FIRST_STREAM_ACTIVITY_KEYS = [
  "time_to_first_stream_activity_ms",
  "timeToFirstStreamActivityMs",
];
const RUN_METRIC_FIRST_TOKEN_KEYS = [
  "time_to_first_token_ms",
  "timeToFirstTokenMs",
  "first_token_ms",
  "firstTokenMs",
];
const RUN_METRIC_MODEL_LATENCY_KEYS = [
  "model_latency_ms",
  "modelLatencyMs",
];

function runMetricSourceRecords(payload: unknown): JsonRecord[] {
  const obj = asRecord(payload);
  const nested = asRecord(obj.payload);
  return [
    obj,
    nested,
    asRecord(obj.usage),
    asRecord(nested.usage),
    asRecord(obj.metrics),
    asRecord(nested.metrics),
  ].filter((record) => Object.keys(record).length > 0);
}

function positiveRunMetricFromPayload(
  payload: unknown,
  keys: readonly string[],
): number | null {
  for (const record of runMetricSourceRecords(payload)) {
    for (const key of keys) {
      const value = positiveRunMetric(record[key]);
      if (value != null) return value;
    }
  }
  return null;
}

function validTimeToFirstTokenMs(
  value: number | null,
  durationMs: number | null,
): number | null {
  if (value == null) return null;
  if (durationMs != null && value >= durationMs) return null;
  return value;
}

export function chatRunMetricsFromPayload(payload: unknown): ChatRunMetrics {
  const inputTokens = positiveRunMetricFromPayload(payload, RUN_METRIC_INPUT_KEYS);
  const outputTokens = positiveRunMetricFromPayload(payload, RUN_METRIC_OUTPUT_KEYS);
  const explicitTotalTokens = positiveRunMetricFromPayload(
    payload,
    RUN_METRIC_TOTAL_KEYS,
  );
  const durationMs = positiveRunMetricFromPayload(payload, RUN_METRIC_DURATION_KEYS);
  const cachedPromptTokens = positiveRunMetricFromPayload(
    payload,
    RUN_METRIC_CACHED_PROMPT_KEYS,
  );
  const cacheCreationPromptTokens = positiveRunMetricFromPayload(
    payload,
    RUN_METRIC_CACHE_CREATION_KEYS,
  );
  const timeToFirstTokenMs = validTimeToFirstTokenMs(
    positiveRunMetricFromPayload(payload, RUN_METRIC_FIRST_TOKEN_KEYS),
    durationMs,
  );
  const timeToFirstStreamActivityMs = positiveRunMetricFromPayload(
    payload,
    RUN_METRIC_FIRST_STREAM_ACTIVITY_KEYS,
  );
  const modelLatencyMs = positiveRunMetricFromPayload(
    payload,
    RUN_METRIC_MODEL_LATENCY_KEYS,
  );
  const totalTokens =
    explicitTotalTokens ??
    (inputTokens != null || outputTokens != null
      ? (inputTokens ?? 0) + (outputTokens ?? 0)
      : null);
  return {
    ...(inputTokens != null ? { inputTokens } : {}),
    ...(outputTokens != null ? { outputTokens } : {}),
    ...(totalTokens != null ? { totalTokens } : {}),
    ...(cachedPromptTokens != null ? { cachedPromptTokens } : {}),
    ...(cacheCreationPromptTokens != null
      ? { cacheCreationPromptTokens }
      : {}),
    ...(durationMs != null ? { durationMs } : {}),
    ...(timeToFirstStreamActivityMs != null
      ? { timeToFirstStreamActivityMs }
      : {}),
    ...(timeToFirstTokenMs != null ? { timeToFirstTokenMs } : {}),
    ...(modelLatencyMs != null ? { modelLatencyMs } : {}),
  };
}

export function buildChatRunMetricItems(
  metrics: ChatRunMetrics,
): ChatRunMetricItem[] {
  const nonNegativeMetric = (value: unknown): number => {
    const amount = num(value, 0);
    if (!Number.isFinite(amount) || amount < 0) return 0;
    return amount;
  };
  const inputTokens = nonNegativeMetric(metrics.inputTokens);
  const outputTokens = nonNegativeMetric(metrics.outputTokens);
  const cachedPromptTokens = nonNegativeMetric(metrics.cachedPromptTokens);
  const cacheCreationPromptTokens = nonNegativeMetric(
    metrics.cacheCreationPromptTokens,
  );
  const explicitTotalTokens = positiveRunMetric(metrics.totalTokens);
  const totalTokens = explicitTotalTokens ?? inputTokens + outputTokens;
  if (totalTokens <= 0 && inputTokens <= 0 && outputTokens <= 0) return [];

  const items: ChatRunMetricItem[] = [
    { label: "Total tokens", value: Math.round(totalTokens).toLocaleString() },
    { label: "Input tokens", value: Math.round(inputTokens).toLocaleString() },
    { label: "Output tokens", value: Math.round(outputTokens).toLocaleString() },
  ];
  if (cachedPromptTokens > 0) {
    items.push({
      label: "Cached prompt",
      value: Math.round(cachedPromptTokens).toLocaleString(),
    });
  }
  if (cacheCreationPromptTokens > 0) {
    items.push({
      label: "Cache write",
      value: Math.round(cacheCreationPromptTokens).toLocaleString(),
    });
  }
  return items;
}

export function chatRunMetricMessageFieldsFromPayload(
  payload: unknown,
): JsonRecord {
  const metrics = chatRunMetricsFromPayload(payload);
  const fields: JsonRecord = {};
  if (metrics.inputTokens != null) fields.input_tokens = metrics.inputTokens;
  if (metrics.outputTokens != null) fields.output_tokens = metrics.outputTokens;
  if (metrics.totalTokens != null) fields.total_tokens = metrics.totalTokens;
  if (metrics.cachedPromptTokens != null) {
    fields.cached_prompt_tokens = metrics.cachedPromptTokens;
  }
  if (metrics.cacheCreationPromptTokens != null) {
    fields.cache_creation_prompt_tokens = metrics.cacheCreationPromptTokens;
  }
  if (metrics.durationMs != null) fields.duration_ms = metrics.durationMs;
  if (metrics.timeToFirstStreamActivityMs != null) {
    fields.time_to_first_stream_activity_ms =
      metrics.timeToFirstStreamActivityMs;
  }
  if (metrics.timeToFirstTokenMs != null) {
    fields.time_to_first_token_ms = metrics.timeToFirstTokenMs;
  }
  if (metrics.modelLatencyMs != null) {
    fields.model_latency_ms = metrics.modelLatencyMs;
  }
  return fields;
}
