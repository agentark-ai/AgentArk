export type JsonRecord = Record<string, unknown>;

export type PromptHoldoutFootprintRow = {
  label: string;
  count: number;
  representative: JsonRecord;
  targetChars: number;
  restChars: number;
  totalChars: number;
};

function finiteNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function positiveSampleCount(row: JsonRecord): number {
  const count = finiteNumber(row.matching_samples) ?? finiteNumber(row.sample_count);
  if (count === null || count < 1) return 1;
  return Math.max(1, Math.floor(count));
}

function str(value: unknown, fallback = "-"): string {
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

function humanizeStatusLabel(value: string): string {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (char) => char.toUpperCase());
}

export function promptHoldoutCaseLabel(row: JsonRecord): string {
  const traceId = str(row.trace_id, "").trim();
  const runId = str(row.run_id, "").trim();
  return traceId || runId || "Representative sample";
}

function caseOutcomeLabel(row: JsonRecord): string {
  const outcome = humanizeStatusLabel(str(row.outcome, "Sample"));
  return outcome === "-" ? "Sample" : outcome;
}

export function promptHoldoutFootprintRows(
  cases: JsonRecord[],
  maxRows = 5,
): PromptHoldoutFootprintRow[] {
  const groups = new Map<
    string,
    {
      outcome: string;
      count: number;
      representative: JsonRecord;
      targetChars: number;
      restChars: number;
      totalChars: number;
    }
  >();

  for (const caseRow of cases) {
    const targetChars = Math.max(0, finiteNumber(caseRow.section_chars) ?? 0);
    const totalChars = Math.max(0, finiteNumber(caseRow.final_prompt_chars) ?? 0);
    const restChars = Math.max(0, totalChars - targetChars);
    const outcome = caseOutcomeLabel(caseRow);
    const count = positiveSampleCount(caseRow);
    const key = [outcome, targetChars, restChars, totalChars].join("|");
    const existing = groups.get(key);
    if (existing) {
      existing.count += count;
    } else {
      groups.set(key, {
        outcome,
        count,
        representative: caseRow,
        targetChars,
        restChars,
        totalChars,
      });
    }
  }

  return Array.from(groups.values())
    .slice(0, maxRows)
    .map((group, idx) => ({
      label:
        group.count > 1
          ? `${group.outcome} cases (${group.count})`
          : `${group.outcome} case ${idx + 1}`,
      count: group.count,
      representative: group.representative,
      targetChars: group.targetChars,
      restChars: group.restChars,
      totalChars: group.totalChars,
    }));
}
