export type UiDateMeta = {
  label: string;
  tip: string;
};

type DateInput = string | number | Date | null | undefined;

type FormatUiDateOptions = {
  fallback?: string;
  includeTime?: boolean;
  includeSeconds?: boolean;
  includeYear?: boolean | "auto";
  timeZoneName?: boolean;
};

function ordinalDay(day: number): string {
  const remainder = day % 10;
  const teen = day % 100;
  if (teen >= 11 && teen <= 13) return `${day}th`;
  if (remainder === 1) return `${day}st`;
  if (remainder === 2) return `${day}nd`;
  if (remainder === 3) return `${day}rd`;
  return `${day}th`;
}

function uppercaseMeridiem(value: string): string {
  return value.replace(/\b(am|pm)\b/gi, (match) => match.toUpperCase());
}

function parseDateInput(value: DateInput): { raw: string; date: Date | null } {
  if (value == null) return { raw: "", date: null };
  if (value instanceof Date) {
    return {
      raw: Number.isNaN(value.getTime()) ? "" : value.toISOString(),
      date: Number.isNaN(value.getTime()) ? null : value
    };
  }
  if (typeof value === "number") {
    const date = new Date(value);
    return { raw: String(value), date: Number.isNaN(date.getTime()) ? null : date };
  }
  const raw = value.trim();
  if (!raw) return { raw, date: null };
  const localDateOnlyMatch = raw.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (localDateOnlyMatch) {
    const [, year, month, day] = localDateOnlyMatch;
    const date = new Date(Number(year), Number(month) - 1, Number(day));
    return { raw, date: Number.isNaN(date.getTime()) ? null : date };
  }
  const naiveUtcDateTimeMatch = raw.match(
    /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?$/
  );
  if (naiveUtcDateTimeMatch) {
    const [, year, month, day, hour, minute, second = "0"] = naiveUtcDateTimeMatch;
    const date = new Date(
      Date.UTC(
        Number(year),
        Number(month) - 1,
        Number(day),
        Number(hour),
        Number(minute),
        Number(second)
      )
    );
    return { raw, date: Number.isNaN(date.getTime()) ? null : date };
  }
  const date = new Date(raw);
  return { raw, date: Number.isNaN(date.getTime()) ? null : date };
}

export function formatUiDateTime(value: DateInput, options: FormatUiDateOptions = {}): string {
  const {
    fallback = "-",
    includeTime = true,
    includeSeconds = false,
    includeYear = "auto",
    timeZoneName = false
  } = options;
  const parsed = parseDateInput(value);
  if (!parsed.raw && !parsed.date) return fallback;
  if (!parsed.date) return parsed.raw || fallback;

  const date = parsed.date;
  const includeYearResolved =
    includeYear === "auto" ? date.getFullYear() !== new Date().getFullYear() : includeYear;
  const month = date.toLocaleDateString([], { month: "short" });
  const day = ordinalDay(date.getDate());
  const yearPart = includeYearResolved ? ` ${date.getFullYear()}` : "";
  if (!includeTime) return `${day} ${month}${yearPart}`;

  const time = uppercaseMeridiem(
    date.toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
      ...(includeSeconds ? { second: "2-digit" as const } : {}),
      ...(timeZoneName ? { timeZoneName: "short" as const } : {})
    })
  );
  return `${day} ${month}${yearPart} ${time}`;
}

export function formatUiDateTimeMeta(
  value: DateInput,
  options: Omit<FormatUiDateOptions, "timeZoneName" | "includeSeconds" | "includeYear"> & {
    fallback?: string;
    includeYear?: boolean | "auto";
  } = {}
): UiDateMeta {
  const { fallback = "-", includeTime = true, includeYear = "auto" } = options;
  const parsed = parseDateInput(value);
  if (!parsed.raw && !parsed.date) return { label: fallback, tip: "" };
  if (!parsed.date) {
    const raw = parsed.raw || fallback;
    return { label: raw, tip: raw };
  }
  return {
    label: formatUiDateTime(parsed.date, { fallback, includeTime, includeYear }),
    tip: formatUiDateTime(parsed.date, {
      fallback,
      includeTime,
      includeYear: true,
      includeSeconds: true,
      timeZoneName: true
    })
  };
}

function formatRelativeFromNow(date: Date): string {
  const diffMs = Date.now() - date.getTime();
  const isPast = diffMs >= 0;
  const absMs = Math.abs(diffMs);
  const absSec = Math.round(absMs / 1000);
  const unit = (count: number, singular: string, plural: string) =>
    `${count} ${count === 1 ? singular : plural}`;

  if (absSec < 30) return "just now";
  const absMin = Math.round(absSec / 60);
  if (absMin < 60) {
    const display = unit(absMin, "minute", "minutes");
    return isPast ? `${display} ago` : `in ${display}`;
  }
  const absHours = Math.round(absMin / 60);
  if (absHours < 24) {
    const display = unit(absHours, "hour", "hours");
    return isPast ? `${display} ago` : `in ${display}`;
  }
  const absDays = Math.round(absHours / 24);
  if (absDays < 7) {
    const display = unit(absDays, "day", "days");
    return isPast ? `${display} ago` : `in ${display}`;
  }
  const absWeeks = Math.round(absDays / 7);
  if (absWeeks < 5) {
    const display = unit(absWeeks, "week", "weeks");
    return isPast ? `${display} ago` : `in ${display}`;
  }
  const absMonths = Math.round(absDays / 30);
  if (absMonths < 12) {
    const display = unit(absMonths, "month", "months");
    return isPast ? `${display} ago` : `in ${display}`;
  }
  const absYears = Math.round(absDays / 365);
  const display = unit(absYears, "year", "years");
  return isPast ? `${display} ago` : `in ${display}`;
}

export function formatUiRelativeDateTimeMeta(
  value: DateInput,
  options: { fallback?: string } = {}
): UiDateMeta {
  const { fallback = "-" } = options;
  const parsed = parseDateInput(value);
  if (!parsed.raw && !parsed.date) return { label: fallback, tip: "" };
  if (!parsed.date) {
    const raw = parsed.raw || fallback;
    return { label: raw, tip: raw };
  }
  return {
    label: formatRelativeFromNow(parsed.date),
    tip: formatUiDateTime(parsed.date, {
      fallback,
      includeYear: true,
      includeSeconds: true,
      timeZoneName: true
    })
  };
}

export function formatUiDateOnly(
  value: DateInput,
  options: { fallback?: string; includeYear?: boolean | "auto" } = {}
): string {
  return formatUiDateTime(value, {
    fallback: options.fallback,
    includeTime: false,
    includeYear: options.includeYear
  });
}

export function formatUiDateRange(start: DateInput, end: DateInput, fallback = "-"): string {
  return `${formatUiDateTime(start, { fallback })} to ${formatUiDateTime(end, { fallback })}`;
}
