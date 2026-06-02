export type JsonRecord = Record<string, unknown>;

export function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : {};
}

export function pickRecords(value: unknown, ...keys: string[]): JsonRecord[] {
  if (Array.isArray(value)) return value.filter(isRecord);
  const record = asRecord(value);
  for (const key of keys) {
    const rows = record[key];
    if (Array.isArray(rows)) return rows.filter(isRecord);
  }
  return [];
}

export function str(value: unknown, fallback = "-"): string {
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return fallback;
}

export function num(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

export function memoryRefreshInterval(
  autoRefresh: boolean,
  pendingCount: number,
  intervalMs: number,
): number | false {
  return autoRefresh || pendingCount > 0 ? intervalMs : false;
}

export function canSaveUserData(
  kind: string,
  title: string,
  pending: boolean,
): boolean {
  return !pending && kind.trim().length > 0 && title.trim().length > 0;
}

export function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    return normalized === "true" || normalized === "1" || normalized === "yes";
  }
  return false;
}

export function errMessage(error: unknown): string {
  const normalize = (raw: string): string => {
    const message = (raw || "").trim();
    if (!message) return "Request failed";
    if (message.startsWith("{") && message.endsWith("}")) {
      try {
        const parsed = JSON.parse(message) as Record<string, unknown>;
        const nested =
          str(parsed.error, "").trim() || str(parsed.message, "").trim();
        if (nested) return nested;
      } catch {
        // Fall through to the raw message.
      }
    }
    return message;
  };

  if (error instanceof Error) return normalize(error.message);
  if (typeof error === "string") return normalize(error);
  return "Request failed";
}
