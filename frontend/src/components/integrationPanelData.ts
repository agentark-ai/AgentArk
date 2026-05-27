export type JsonRecord = Record<string, unknown>;

const EMPTY_JSON_RECORD: JsonRecord = Object.freeze({}) as JsonRecord;

export function isRecord(value: unknown): value is JsonRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function asRecord(value: unknown): JsonRecord {
  return isRecord(value) ? value : EMPTY_JSON_RECORD;
}

export function asRecords(value: unknown): JsonRecord[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord);
}

export function str(value: unknown, fallback = ""): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

export function toBool(value: unknown): boolean {
  if (typeof value === "boolean") return value;
  if (typeof value === "number") return value !== 0;
  if (typeof value === "string") {
    const v = value.trim().toLowerCase();
    return v === "true" || v === "1" || v === "yes";
  }
  return false;
}
