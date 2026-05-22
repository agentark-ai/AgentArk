// Pure-function adapter that turns the technical ReflectResponse shape into
// plain-English UI primitives the hero renders. No React, no MUI — just
// strings and small typed objects. Keeping this file free of UI imports
// means it can be unit-tested and reused by future surfaces (briefing
// emails, Slack digests, etc.) without dragging in the page's React tree.

// We import only the data types from the page module so this stays a leaf.
// The page module re-exports these types implicitly through its public
// types; we re-declare a minimal structural subset here to avoid a circular
// import. Keep these aligned with the source page's contract.

export type NarrativePeriod = "daily" | "weekly" | "monthly";

export type NarrativeSourceCounts = {
  main_chat: number;
  orbit_chat: number;
  memory: number;
  procedures: number;
  apps: number;
  goals: number;
  watchers: number;
  sentinel: number;
  arkpulse: number;
  arkevolve: number;
  usage: number;
};

export type NarrativeCluster = {
  id: string;
  label: string;
  plain_summary: string;
  unit_count: number;
  message_count: number;
  color: string;
  source_mix: Record<string, number>;
};

export type NarrativeFollowup = {
  id: string;
  title: string;
  detail: string;
  prompt: string;
  source_label: string;
  rank_score: number;
};

export type NarrativeInput = {
  period: NarrativePeriod;
  source_counts: NarrativeSourceCounts;
  clusters: NarrativeCluster[];
  suggested_followups: NarrativeFollowup[];
  // True when the backend reports any meaningful activity for the window.
  has_activity: boolean;
  // True when the embeddings have caught up; affects confidence wording.
  embeddings_ready: boolean;
};

export type HeroSentence = {
  // Single plain-English sentence describing the period.
  text: string;
  // Optional supporting line shown beneath, smaller.
  detail: string;
};

export type HeadlineNumber = {
  value: string;       // formatted number for display ("23", "1.2k", "—")
  unitLabel: string;   // "conversations", "items reflected", "moments"
  caption: string;     // "this week", "today", "this month"
  positive: boolean;   // colour intent: green glow when positive activity
};

export type Moment = {
  id: string;
  // Friendly tone label ("Mostly Drive", "A burst of email"). Never raw
  // cluster ids, never `unit_count` strings.
  title: string;
  // One-sentence plain-English summary of what happened in this moment.
  sentence: string;
  // The dominant source family — used to pick an icon, never shown raw.
  sourceFamily: SourceFamily;
};

export type NextStep = {
  id: string;
  title: string;
  reason: string;       // short, plain-English "why we suggest this"
  prompt: string;
} | null;

// SourceFamily collapses the 11 technical source kinds into 5 user-facing
// buckets. The page's existing SOURCE_DISPLAY map uses 11 categories; the
// novice hero only ever needs to know which icon family to draw, so we
// reduce here. Anti-jargon: never show these enum names in the UI;
// they're only used to select an icon and a colour.
export type SourceFamily =
  | "conversations"
  | "memory"
  | "apps"
  | "background"
  | "system"
  | "mixed";

function periodCaption(period: NarrativePeriod): string {
  switch (period) {
    case "daily":
      return "today";
    case "weekly":
      return "this week";
    case "monthly":
      return "this month";
  }
}

function formatCount(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  if (value >= 1_000) {
    const thousands = value / 1_000;
    if (thousands >= 10) return `${Math.round(thousands)}k`;
    return `${thousands.toFixed(1)}k`;
  }
  return String(Math.round(value));
}

const SOURCE_FAMILY_FOR_KIND: Record<string, SourceFamily> = {
  conversation: "conversations",
  orbit_chat: "conversations",
  experience_item: "memory",
  procedural_pattern: "memory",
  app: "apps",
  goal: "apps",
  watcher: "background",
  sentinel: "system",
  arkpulse: "system",
  arkevolve: "system",
  llm_usage: "system",
};

export function sourceFamilyForKind(kind: string): SourceFamily {
  return SOURCE_FAMILY_FOR_KIND[kind] ?? "mixed";
}

const FAMILY_FRIENDLY_LABELS: Record<SourceFamily, string> = {
  conversations: "chats",
  memory: "memory and patterns",
  apps: "apps and goals",
  background: "background work",
  system: "system signals",
  mixed: "mixed activity",
};

const FAMILY_HEADLINE_LABELS: Record<SourceFamily, string> = {
  conversations: "Mostly chats",
  memory: "Mostly memory work",
  apps: "Mostly apps and goals",
  background: "Mostly background work",
  system: "Mostly system signals",
  mixed: "Mixed activity",
};

// Maps the 11-category source counts to family totals so the narrative can
// describe "mostly chats" / "mostly apps" without enumerating raw kinds.
function familyTotals(counts: NarrativeSourceCounts): Record<SourceFamily, number> {
  return {
    conversations: counts.main_chat + counts.orbit_chat,
    memory: counts.memory + counts.procedures,
    apps: counts.apps + counts.goals,
    background: counts.watchers,
    system: counts.sentinel + counts.arkpulse + counts.arkevolve + counts.usage,
    mixed: 0,
  };
}

function dominantFamily(counts: NarrativeSourceCounts): SourceFamily {
  const totals = familyTotals(counts);
  let top: SourceFamily = "mixed";
  let topValue = 0;
  let total = 0;
  for (const family of Object.keys(totals) as SourceFamily[]) {
    const value = totals[family];
    total += value;
    if (value > topValue) {
      topValue = value;
      top = family;
    }
  }
  if (total === 0) return "mixed";
  // If the top family has less than half of all signals, the period is
  // genuinely mixed — be honest about it instead of forcing a winner.
  if (topValue / total < 0.45) return "mixed";
  return top;
}

function totalActivityCount(counts: NarrativeSourceCounts): number {
  return (
    counts.main_chat +
    counts.orbit_chat +
    counts.memory +
    counts.procedures +
    counts.apps +
    counts.goals +
    counts.watchers +
    counts.sentinel +
    counts.arkpulse +
    counts.arkevolve +
    counts.usage
  );
}

// Headline number: the single biggest readable number for the period.
// Prefers the dominant family's count over total when the period leans
// hard into one area; otherwise falls back to the total. This is the
// "23 conversations this week" line that anchors the hero visually.
export function headlineNumber(input: NarrativeInput): HeadlineNumber {
  const total = totalActivityCount(input.source_counts);
  if (total === 0) {
    return {
      value: "0",
      unitLabel: "moments",
      caption: periodCaption(input.period),
      positive: false,
    };
  }
  const dom = dominantFamily(input.source_counts);
  const totals = familyTotals(input.source_counts);
  if (dom !== "mixed") {
    const family = totals[dom];
    return {
      value: formatCount(family),
      unitLabel: FAMILY_FRIENDLY_LABELS[dom],
      caption: periodCaption(input.period),
      positive: true,
    };
  }
  return {
    value: formatCount(total),
    unitLabel: "moments",
    caption: periodCaption(input.period),
    positive: true,
  };
}

// Hero sentence: one warm plain-English line describing the period.
// Never names a cluster id, never uses words like "units", "clusters",
// "embeddings", or "rank_score". Voice: third-person about AgentArk's
// subsystems, second-person about the user.
export function heroSentence(input: NarrativeInput): HeroSentence {
  const total = totalActivityCount(input.source_counts);
  const caption = periodCaption(input.period);
  if (total === 0) {
    return {
      text: `Quiet ${caption === "today" ? "day" : caption.replace(/^this /, "")}.`,
      detail:
        "Nothing meaningful to reflect on yet. Check back when AgentArk has captured more activity.",
    };
  }
  const dom = dominantFamily(input.source_counts);
  const clusterCount = input.clusters.length;
  const headline =
    dom === "mixed"
      ? `A mixed ${caption.replace(/^this /, "")} across several areas.`
      : `${FAMILY_HEADLINE_LABELS[dom]} ${caption}.`;
  const detailParts: string[] = [];
  if (clusterCount > 0) {
    detailParts.push(
      `${clusterCount} topic${clusterCount === 1 ? "" : "s"} stood out`,
    );
  }
  if (!input.embeddings_ready) {
    detailParts.push("still indexing");
  }
  const detail =
    detailParts.length > 0
      ? `${detailParts.join(" · ")}.`
      : "Here are the moments that stood out.";
  return { text: headline, detail };
}

// Pick up to N moments from the technical cluster list and reshape each
// into a Moment with novice-friendly copy. We pick by message_count *
// unit_count as a rough "magnitude" proxy because that's what the user
// actually felt. Tie-breaker is the cluster's existing label.
export function topMoments(input: NarrativeInput, max = 5): Moment[] {
  const ranked = [...input.clusters]
    .map((cluster) => ({
      cluster,
      score:
        Math.max(1, cluster.message_count) * Math.max(1, cluster.unit_count),
    }))
    .sort((left, right) => {
      if (right.score !== left.score) return right.score - left.score;
      return left.cluster.label.localeCompare(right.cluster.label);
    })
    .slice(0, max);

  return ranked.map(({ cluster }) => {
    const dominantKind = Object.entries(cluster.source_mix)
      .sort((a, b) => b[1] - a[1])
      .map(([kind]) => kind)[0] ?? "mixed";
    const family = sourceFamilyForKind(dominantKind);
    const cleanTitle = momentTitle(cluster, family);
    const sentence = momentSentence(cluster, family);
    return {
      id: cluster.id,
      title: cleanTitle,
      sentence,
      sourceFamily: family,
    };
  });
}

function momentTitle(cluster: NarrativeCluster, family: SourceFamily): string {
  const raw = (cluster.label ?? "").trim();
  if (raw.length > 0 && raw.length <= 64) return raw;
  if (raw.length > 64) return `${raw.slice(0, 61).trimEnd()}…`;
  // Fallback by family — never expose cluster ids.
  switch (family) {
    case "conversations":
      return "A series of chats";
    case "memory":
      return "Memory and patterns";
    case "apps":
      return "Apps and goals";
    case "background":
      return "Background work";
    case "system":
      return "System signals";
    case "mixed":
      return "Mixed activity";
  }
}

function momentSentence(cluster: NarrativeCluster, family: SourceFamily): string {
  if (cluster.plain_summary && cluster.plain_summary.trim().length > 0) {
    return cluster.plain_summary.trim();
  }
  const familyText = FAMILY_FRIENDLY_LABELS[family];
  const count = cluster.unit_count;
  if (count <= 1) return `One moment from ${familyText}.`;
  return `${count} moments from ${familyText}.`;
}

// The "next step" card is the single most valuable suggested followup the
// page already produces. We pick by rank_score descending and shape it
// into a novice-friendly card. Returns null if there's nothing actionable
// — in which case the hero hides the card instead of showing a stub.
export function nextStep(input: NarrativeInput): NextStep {
  if (input.suggested_followups.length === 0) return null;
  const top = [...input.suggested_followups]
    .sort((left, right) => right.rank_score - left.rank_score)[0];
  if (!top || !top.prompt.trim()) return null;
  return {
    id: top.id,
    title: top.title || "Continue this in Chat",
    reason:
      top.detail.trim().length > 0
        ? top.detail.trim()
        : `Suggested from ${top.source_label || "Reflect"}.`,
    prompt: top.prompt,
  };
}

// hasMeaningfulActivity collapses the embedding/cache/digest checks into a
// single boolean the hero uses to decide between the narrative view and
// the warm empty state. Keeps the empty-state rule in one place instead
// of scattered conditional renders.
export function hasMeaningfulActivity(input: NarrativeInput): boolean {
  return input.has_activity && totalActivityCount(input.source_counts) > 0;
}
