export type JsonRecord = Record<string, unknown>;

export type IntegrationHubPanel =
  | "custom_apis"
  | "webhooks"
  | "plugins"
  | "extension_packs"
  | "custom_messaging_channels";

export type IntegrationHubCardSource =
  | "builtin_integration"
  | "custom_api"
  | "webhook"
  | "plugin"
  | "extension_pack"
  | "custom_messaging_channel"
  | "messaging_channel"
  | "email_channel";

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

export function integrationHubPanelForCardSource(
  source: IntegrationHubCardSource
): IntegrationHubPanel | null {
  switch (source) {
    case "custom_api":
      return "custom_apis";
    case "webhook":
      return "webhooks";
    case "plugin":
      return "plugins";
    case "extension_pack":
      return "extension_packs";
    case "custom_messaging_channel":
      return "custom_messaging_channels";
    case "builtin_integration":
    case "messaging_channel":
    case "email_channel":
      return null;
  }
}
