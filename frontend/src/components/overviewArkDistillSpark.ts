export type ArkDistillSavedTokenSparkPoint = {
  bucket_start?: string | null;
  estimated_saved_tokens?: number | null;
};

export type ArkDistillSavedTokenSparkWindow = {
  start?: string | null;
  end?: string | null;
  bucket?: string | null;
};

const DAY_MS = 24 * 60 * 60 * 1000;

function nonNegativeFinite(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, value)
    : 0;
}

function parseTimestamp(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function utcDayStart(timestamp: number): number {
  const date = new Date(timestamp);
  return Date.UTC(
    date.getUTCFullYear(),
    date.getUTCMonth(),
    date.getUTCDate(),
  );
}

function cumulativeInInputOrder(points: ArkDistillSavedTokenSparkPoint[]): number[] {
  let running = 0;
  return points.map((point) => {
    running += nonNegativeFinite(point.estimated_saved_tokens);
    return running;
  });
}

export function buildCumulativeSavedTokenSparkValues(
  points: ArkDistillSavedTokenSparkPoint[],
  window: ArkDistillSavedTokenSparkWindow = {},
): number[] {
  if (points.length === 0) return [];

  const bucket = String(window.bucket || "").trim().toLowerCase();
  const start = parseTimestamp(window.start);
  const end = parseTimestamp(window.end);
  const savedTokensByDay = new Map<number, number>();

  for (const point of points) {
    const timestamp = parseTimestamp(point.bucket_start);
    if (timestamp == null) continue;
    const day = utcDayStart(timestamp);
    savedTokensByDay.set(
      day,
      (savedTokensByDay.get(day) || 0) +
        nonNegativeFinite(point.estimated_saved_tokens),
    );
  }

  if (bucket === "day" && start != null && end != null && savedTokensByDay.size > 0) {
    const startDay = utcDayStart(Math.min(start, end));
    const endDay = utcDayStart(Math.max(start, end));
    const values: number[] = [];
    let running = 0;
    for (let day = startDay; day <= endDay; day += DAY_MS) {
      running += savedTokensByDay.get(day) || 0;
      values.push(running);
    }
    return values;
  }

  if (savedTokensByDay.size === 0) {
    return cumulativeInInputOrder(points);
  }

  let running = 0;
  return Array.from(savedTokensByDay.entries())
    .sort(([left], [right]) => left - right)
    .map(([, savedTokens]) => {
      running += savedTokens;
      return running;
    });
}
