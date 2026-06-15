import type { StatusResponse } from "../types";

export type RuntimeMetricSample = {
  t: number;
  memoryPressure?: number;
  latencyMs?: number;
};

export const RUNTIME_METRIC_HISTORY_EVENT = "agentark-runtime-metric-history";

const STORAGE_KEY = "agentark.runtimeMetricHistory.v1";
const WINDOW_MS = 6 * 60 * 60 * 1000;
const MAX_SAMPLES = 3000;
const PERSIST_INTERVAL_MS = 60_000;

let cacheLoaded = false;
let cachedSamples: RuntimeMetricSample[] = [];
let lastPersistAt = 0;

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function prune(samples: RuntimeMetricSample[], now = Date.now()): RuntimeMetricSample[] {
  return samples
    .filter((sample) => Number.isFinite(sample.t) && sample.t >= now - WINDOW_MS)
    .slice(-MAX_SAMPLES);
}

export function readRuntimeMetricHistory(now = Date.now()): RuntimeMetricSample[] {
  if (typeof window === "undefined") return cachedSamples;
  if (cacheLoaded) {
    cachedSamples = prune(cachedSamples, now);
    return cachedSamples;
  }
  cacheLoaded = true;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return cachedSamples;
    const parsed = JSON.parse(raw) as RuntimeMetricSample[];
    if (!Array.isArray(parsed)) return cachedSamples;
    cachedSamples = prune(parsed, now);
    return cachedSamples;
  } catch {
    return cachedSamples;
  }
}

export function recordRuntimeMetricSample({
  at = Date.now(),
  latencyMs,
  status,
}: {
  at?: number;
  latencyMs?: number | null;
  status?: StatusResponse | null;
}): RuntimeMetricSample[] {
  if (typeof window === "undefined") return [];

  const health = status?.runtime_health ?? null;
  const memoryPressure = num(health?.memory_pressure_percent ?? health?.ram_percent);
  const latency = num(latencyMs);
  if (memoryPressure == null && latency == null) return readRuntimeMetricHistory(at);

  const nextSample: RuntimeMetricSample = { t: at };
  if (memoryPressure != null) nextSample.memoryPressure = memoryPressure;
  if (latency != null) nextSample.latencyMs = latency;

  const previous = readRuntimeMetricHistory(at);
  const last = previous[previous.length - 1];
  const next =
    last && Math.abs(at - last.t) < 2000
      ? [...previous.slice(0, -1), { ...last, ...nextSample }]
      : [...previous, nextSample];
  cachedSamples = prune(next, at);

  if (at - lastPersistAt >= PERSIST_INTERVAL_MS || cachedSamples.length <= 1) {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(cachedSamples));
      lastPersistAt = at;
    } catch {
      // Metric history is best-effort; status rendering should not depend on storage.
    }
  }

  try {
    window.dispatchEvent(new CustomEvent(RUNTIME_METRIC_HISTORY_EVENT, { detail: cachedSamples }));
  } catch {
    // Ignore event failures; the next render can read from the in-memory cache.
  }

  return cachedSamples;
}

export function metricValues(
  samples: RuntimeMetricSample[],
  key: "memoryPressure" | "latencyMs"
): number[] {
  return samples
    .map((sample) => sample[key])
    .filter((value): value is number => typeof value === "number" && Number.isFinite(value));
}
