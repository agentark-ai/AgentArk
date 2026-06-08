export type ReflectOpportunitySearchResult = {
  title?: string | null;
  url?: string | null;
  snippet?: string | null;
  source?: string | null;
  published_date?: string | null;
};

export type ReflectOpportunityLike = {
  kind: string;
  title?: string | null;
  detail?: string | null;
  status?: string | null;
  search_results?: ReflectOpportunitySearchResult[];
  search_error?: string | null;
  latest_summary?: string | null;
  latest_summary_error?: string | null;
  latest_summary_evidence_supported?: boolean | null;
};

export type ReflectOpportunitySourceCounts = Record<string, unknown> | null | undefined;

function compactText(value: string, maxChars: number): string {
  const cleaned = value.split(/\s+/).join(" ").trim();
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function stripInlineMarkup(value: string): string {
  return value
    .replace(/\*\*/g, "")
    .replace(/#+\s*/g, "")
    .replace(/`/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function compactMultilineText(value: string, maxChars: number): string {
  const cleaned = value
    .replace(/\*\*/g, "")
    .replace(/#+\s*/g, "")
    .replace(/`/g, "")
    .split(/\r?\n/)
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .join("\n");
  if (cleaned.length <= maxChars) return cleaned;
  return `${cleaned.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}

function meaningTokens(value: string): string[] {
  return stripInlineMarkup(value)
    .toLowerCase()
    .replace(/https?:\/\/\S+/g, " ")
    .replace(/[^a-z0-9]+/g, " ")
    .split(/\s+/)
    .filter((token) => token.length >= 3);
}

function meaningSimilarity(left: string, right: string): number {
  const leftTokens = new Set(meaningTokens(left));
  const rightTokens = new Set(meaningTokens(right));
  if (!leftTokens.size || !rightTokens.size) return 0;
  let overlap = 0;
  for (const token of leftTokens) {
    if (rightTokens.has(token)) overlap += 1;
  }
  const union = new Set([...leftTokens, ...rightTokens]).size;
  return union > 0 ? overlap / union : 0;
}

function isNearDuplicateText(left: string, right: string): boolean {
  const leftClean = stripInlineMarkup(left).toLowerCase();
  const rightClean = stripInlineMarkup(right).toLowerCase();
  if (!leftClean || !rightClean) return false;
  return leftClean === rightClean || meaningSimilarity(leftClean, rightClean) >= 0.72;
}

function readableSourceTitle(result: ReflectOpportunitySearchResult): string {
  const raw = stripInlineMarkup(result.title || result.url || "");
  if (!raw) return "";
  const parts = raw
    .split(/\s*>\s*/g)
    .map((part) => part.trim())
    .filter(Boolean);
  const last = parts.length > 0 ? parts[parts.length - 1] : raw;
  const withoutUrlFragments = last
    .replace(/\b[a-z0-9.-]+\.[a-z]{2,}\b/gi, " ")
    .replace(/_/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const words = withoutUrlFragments.split(/\s+/);
  if (words.length > 1 && words[0].toLowerCase() === words[1].toLowerCase()) {
    words.shift();
  }
  return compactText(words.join(" "), 110);
}

export function followupHasSourceEvidence(item: ReflectOpportunityLike): boolean {
  return item.latest_summary_evidence_supported === true && Boolean(item.latest_summary?.trim());
}

export function isDisplayableOpportunity(item: ReflectOpportunityLike): boolean {
  if (item.kind !== "latest_developments") return false;
  return true;
}

export function hasReflectActivity(sourceCounts: ReflectOpportunitySourceCounts): boolean {
  if (!sourceCounts) return false;
  return Object.values(sourceCounts).some((value) => typeof value === "number" && Number.isFinite(value) && value > 0);
}

export function shouldPollForOpportunitySettlement(input: {
  sourceCounts: ReflectOpportunitySourceCounts;
  opportunityCount: number;
  queuedSourceCheckCount: number;
  refreshRunning: boolean;
}): boolean {
  if (input.refreshRunning) return false;
  if (input.queuedSourceCheckCount > 0) return true;
  if (input.opportunityCount > 0) return false;
  return hasReflectActivity(input.sourceCounts);
}

export function shouldStartOpportunitySettlementPoll(input: {
  shouldPoll: boolean;
  currentUntil?: number;
  now: number;
}): boolean {
  if (!input.shouldPoll) return false;
  return typeof input.currentUntil !== "number";
}

export function isOpportunitySettlementActive(input: {
  shouldPoll: boolean;
  currentUntil?: number;
  now: number;
}): boolean {
  return input.shouldPoll && typeof input.currentUntil === "number" && input.currentUntil > input.now;
}

export function latestUpdateTitle(item: ReflectOpportunityLike): string {
  const rawTitle = stripInlineMarkup(item.title || "Reflected topic");
  const sourceTitle = (item.search_results || [])
    .map(readableSourceTitle)
    .find((title) => title && !isNearDuplicateText(title, rawTitle));
  if (sourceTitle) return sourceTitle;
  return compactText(rawTitle, 110);
}

export function latestDevelopmentSummary(item: ReflectOpportunityLike): string {
  const generated = compactMultilineText(item.latest_summary || "", 640);
  if (generated) return generated;
  if (item.latest_summary_error && (item.search_results || []).length > 0) {
    return "Source check found cached sources, but the synthesis worker did not finish. Review the source cards below.";
  }
  if (item.search_error) return compactText(item.search_error, 180);
  return (item.search_results || []).length > 0
    ? "Source check found cached sources. Review the source cards below."
    : compactText(item.detail || "Next step queued for source checking.", 180);
}
