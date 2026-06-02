import { humanizeMachineLabel } from "../lib/displayLabels";

const CHANNEL_SOURCE_LABELS: Record<string, string> = {
  arkorbit: "Orbit",
  orbit: "Orbit",
  http: "Web",
  web: "Web",
  cli: "CLI",
  gui: "GUI",
  telegram: "Telegram",
  whatsapp: "WhatsApp",
  google_chat: "Google Chat",
  imessage: "iMessage",
  slack: "Slack",
  discord: "Discord",
  matrix: "Matrix",
  teams: "Teams",
  signal: "Signal",
  line: "LINE",
  qq: "QQ",
  wechat: "WeChat",
  voice: "Voice",
};

export function formatChannelSource(
  value?: string | null,
  fallback = "Unknown",
): string {
  const raw = (value || "").trim();
  if (!raw) return fallback;
  const key = raw.toLowerCase().replace(/[\s-]+/g, "_");
  const mapped = CHANNEL_SOURCE_LABELS[key];
  if (mapped) return mapped;
  return humanizeMachineLabel(raw, fallback);
}
