import { useMemo, useState, type CSSProperties } from "react";

export type CognitionStageId = "observe" | "understand" | "plan" | "act" | "reflect" | "learn";

export type AgentCognitionLoopProps = {
  latencyMs?: number | null;
  memoryPressureHistory?: number[];
  latencyHistory?: number[];
  memoryCount: number;
  skillCount: number;
  appCount: number;
  integrationCount: number;
  traceCount: number;
  selfEvolveEnabled: boolean;
  learningQueueCount: number;
  /** True while AgentArk has work in progress — lights the ACT stage. */
  running?: boolean;
};

// "Synaptic field" cortex: the AgentArk logo is the brain at dead centre,
// six cognition stages orbit it on a ring, eight capability dendrites orbit
// further out, and travelling pulses ride the synapses. All geometry is
// hand-laid in a 1060x610 viewBox centred at (530, 305) — the centre matches
// the box's exact midpoint so the HTML logo overlay at 50%/50% lines up.
const CX = 530;
const CY = 305;

type StageNode = {
  id: CognitionStageId;
  number: string;
  title: string;
  x: number;
  y: number;
  // label anchor + position
  lx: number;
  ly: number;
  anchor: "start" | "middle" | "end";
};

// Stage ring r=148: top, then clockwise.
const STAGES: StageNode[] = [
  { id: "observe", number: "01", title: "OBSERVE", x: 530, y: 157, lx: 530, ly: 125, anchor: "middle" },
  { id: "understand", number: "02", title: "UNDERSTAND", x: 658, y: 231, lx: 681, ly: 241, anchor: "start" },
  { id: "plan", number: "03", title: "PLAN", x: 658, y: 379, lx: 681, ly: 371, anchor: "start" },
  { id: "act", number: "04", title: "ACT", x: 530, y: 453, lx: 530, ly: 495, anchor: "middle" },
  { id: "reflect", number: "05", title: "REFLECT", x: 402, y: 379, lx: 379, ly: 371, anchor: "end" },
  { id: "learn", number: "06", title: "LEARN", x: 402, y: 231, lx: 379, ly: 241, anchor: "end" },
];

// Spokes: stage -> cortex core. They stop well short of the logo face so the
// signal appears to dive underneath the brain.
const SPOKES: Record<CognitionStageId, string> = {
  observe: "M 530,178 C 528,205 529,227 530,249",
  understand: "M 640,242 C 619,251 597,263 579,277",
  plan: "M 640,368 C 619,359 597,347 579,333",
  act: "M 530,432 C 532,405 531,383 530,361",
  reflect: "M 420,368 C 441,359 463,347 481,333",
  learn: "M 420,242 C 441,251 463,263 481,277",
};

// Faint inter-stage web (hexagram chords bowed outward) + junction specks.
const CHORDS = [
  "M 530,157 Q 633,251 658,379",
  "M 658,231 Q 637,363 530,453",
  "M 658,379 Q 530,433 402,379",
  "M 530,453 Q 423,363 402,231",
  "M 402,379 Q 427,251 530,157",
  "M 402,231 Q 530,201 658,231",
];
const JUNCTIONS: Array<[number, number]> = [
  [530, 216],
  [614, 260],
  [447, 260],
  [616, 353],
  [445, 353],
  [530, 406],
];

type CapabilityNode = {
  key: CapabilityKey;
  code: string;
  sub: string;
  x: number;
  y: number;
  anchor: "start" | "end";
  dendrite: string;
};

type CapabilityKey =
  | "memory"
  | "skills"
  | "apps"
  | "integrations"
  | "trace"
  | "evolve"
  | "learning"
  | "pulse";

type HoverNodeId = `stage:${CognitionStageId}` | `cap:${CapabilityKey}`;
type TooltipTone = "green" | "cyan" | "amber" | "crit";

type TooltipSpark = {
  label: string;
  values: number[];
  formatValue?: (value: number) => string;
};

type TooltipData = {
  title: string;
  value: string;
  detail: string;
  meta?: string;
  tone: TooltipTone;
  spark?: TooltipSpark;
};

// Capability orbit r=262 (nodes), dendrites curve to the stage ring.
const CAPABILITIES: CapabilityNode[] = [
  { key: "memory", code: "MEM", sub: "memories", x: 430, y: 63, anchor: "end", dendrite: "M 436,69 C 465,97 491,121 514,141" },
  { key: "skills", code: "SKL", sub: "skills", x: 630, y: 63, anchor: "start", dendrite: "M 624,69 C 595,97 569,121 546,141" },
  { key: "apps", code: "APP", sub: "apps", x: 772, y: 205, anchor: "start", dendrite: "M 764,206 C 733,200 695,199 669,213" },
  { key: "integrations", code: "INT", sub: "integrations", x: 772, y: 405, anchor: "start", dendrite: "M 764,404 C 733,410 695,411 669,397" },
  { key: "trace", code: "TRC", sub: "traces", x: 630, y: 547, anchor: "start", dendrite: "M 624,541 C 597,517 567,489 546,470" },
  { key: "evolve", code: "EVO", sub: "self-evolve", x: 430, y: 547, anchor: "end", dendrite: "M 436,541 C 463,517 493,489 514,470" },
  { key: "learning", code: "LRN", sub: "queued learnings", x: 288, y: 405, anchor: "end", dendrite: "M 296,403 C 327,395 355,387 380,382" },
  { key: "pulse", code: "PLS", sub: "pulse", x: 288, y: 205, anchor: "end", dendrite: "M 296,207 C 327,215 355,223 380,230" },
];

// Reach synapses: the field stretches toward the surrounding telemetry
// columns and fades out — decorative, alignment-agnostic.
const REACH = [
  "M 383,279 C 305,265 215,251 60,243",
  "M 383,331 C 305,345 215,359 60,367",
  "M 677,279 C 755,265 845,251 1000,243",
  "M 677,331 C 755,345 845,359 1000,367",
  "M 426,553 C 393,577 357,595 316,608",
  "M 634,553 C 667,577 703,595 744,608",
];

// Travelling pulses ride CSS offset-path (not SMIL: Chrome pauses SMIL
// timelines in hidden tabs, and CSS animations are also what our
// prefers-reduced-motion rules govern).
type Pulse = { path: string; cls: string; dur: number; delay: number };
const PULSES: Pulse[] = [
  { path: SPOKES.observe, cls: "nw-syn-pulse--mint", dur: 2.6, delay: -0.4 },
  { path: SPOKES.plan, cls: "nw-syn-pulse--mint", dur: 2.8, delay: -1.2 },
  { path: SPOKES.reflect, cls: "nw-syn-pulse--mint", dur: 3.0, delay: -1.9 },
  { path: CAPABILITIES[0].dendrite, cls: "nw-syn-pulse--violet", dur: 4.2, delay: -0.6 },
  { path: CAPABILITIES[2].dendrite, cls: "nw-syn-pulse--violet", dur: 4.8, delay: -2.4 },
  { path: CAPABILITIES[4].dendrite, cls: "nw-syn-pulse--violet", dur: 4.4, delay: -1.5 },
  { path: CAPABILITIES[6].dendrite, cls: "nw-syn-pulse--violet", dur: 5.0, delay: -3.1 },
  { path: REACH[0], cls: "nw-syn-pulse--ice", dur: 5.2, delay: -1.1 },
  { path: REACH[3], cls: "nw-syn-pulse--ice", dur: 5.4, delay: -3.4 },
  { path: REACH[5], cls: "nw-syn-pulse--ice", dur: 4.6, delay: -2.2 },
  // Orbiters on the stage ring and capability orbit.
  { path: `M ${CX},157 A 148 148 0 1 1 ${CX - 0.01},157 Z`, cls: "nw-syn-pulse--mint", dur: 12, delay: -3 },
  { path: `M ${CX},43 A 262 262 0 1 1 ${CX - 0.01},43 Z`, cls: "nw-syn-pulse--violet", dur: 24, delay: -8 },
];

function compactValue(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(abs >= 10_000_000 ? 1 : 2).replace(/\.0+$/, "")}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(abs >= 10_000 ? 1 : 2).replace(/\.0+$/, "")}K`;
  return `${Math.round(value)}`;
}

function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function formatLatency(value: number): string {
  return `${Math.round(value)}ms`;
}

function sparklinePoints(values: number[], width = 184, height = 34): string {
  const finite = values.filter((value) => Number.isFinite(value));
  if (finite.length < 2) return "";
  const step = Math.max(1, Math.ceil(finite.length / 80));
  const compact = finite.filter((_, index) => index % step === 0).slice(-80);
  const min = Math.min(...compact);
  const max = Math.max(...compact);
  const range = Math.max(1, max - min);
  return compact
    .map((value, index) => {
      const x = (index / Math.max(1, compact.length - 1)) * width;
      const y = height - ((value - min) / range) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function TooltipSparkline({ spark }: { spark: TooltipSpark }) {
  const values = spark.values.filter((value) => Number.isFinite(value));
  const points = sparklinePoints(values);
  const latest = values.length > 0 ? values[values.length - 1] : null;
  return (
    <div className={`nw-tooltip-spark${points ? "" : " nw-tooltip-spark--empty"}`}>
      <div className="nw-tooltip-spark-head">
        <span>{spark.label}</span>
        <b>{latest == null ? "pending" : spark.formatValue ? spark.formatValue(latest) : compactValue(latest)}</b>
      </div>
      <svg viewBox="0 0 184 34" preserveAspectRatio="none" aria-hidden="true">
        {points ? <polyline points={points} /> : <line x1="0" y1="17" x2="184" y2="17" />}
      </svg>
    </div>
  );
}

function tooltipTransform(x: number, y: number): { tx: string; ty: string } {
  if (x < CX - 120) return { tx: "18px", ty: "-50%" };
  if (x > CX + 120) return { tx: "calc(-100% - 18px)", ty: "-50%" };
  if (y < CY) return { tx: "-50%", ty: "18px" };
  return { tx: "-50%", ty: "calc(-100% - 18px)" };
}

function tooltipStyle(x: number, y: number): CSSProperties {
  const { tx, ty } = tooltipTransform(x, y);
  return {
    left: `${((x - 150) / 760) * 100}%`,
    top: `${(y / 610) * 100}%`,
    transform: `translate(${tx}, ${ty})`,
    "--tx": tx,
    "--ty": ty,
  } as CSSProperties;
}

export function AgentCognitionLoop({
  latencyMs,
  memoryPressureHistory = [],
  latencyHistory = [],
  memoryCount,
  skillCount,
  appCount,
  integrationCount,
  traceCount,
  selfEvolveEnabled,
  learningQueueCount,
  running = false,
}: AgentCognitionLoopProps) {
  const values: Record<string, { value: string; dim: boolean }> = {
    memory: { value: `${memoryCount}`, dim: memoryCount === 0 },
    skills: { value: `${skillCount}`, dim: skillCount === 0 },
    apps: { value: `${appCount}`, dim: appCount === 0 },
    integrations: { value: `${integrationCount}`, dim: integrationCount === 0 },
    trace: { value: `${traceCount}`, dim: traceCount === 0 },
    evolve: { value: selfEvolveEnabled ? "ON" : "OFF", dim: !selfEvolveEnabled },
    learning: { value: `${learningQueueCount}`, dim: learningQueueCount === 0 },
    pulse: {
      value: latencyMs == null ? "-" : `${Math.round(latencyMs)}ms`,
      dim: latencyMs == null,
    },
  };

  const activeStage: CognitionStageId = running ? "act" : "observe";
  const activeWord = running ? "FIRING" : "WATCHING";
  const active = STAGES.find((s) => s.id === activeStage) ?? STAGES[0];
  const [hovered, setHovered] = useState<HoverNodeId | null>(null);
  const tooltipByNode = useMemo<Record<HoverNodeId, TooltipData>>(
    () => ({
      "stage:observe": {
        title: "Observe",
        value: running ? "Watching run" : "Watching system",
        detail: "Live status, latency, memory pressure, and queue signals are being sampled.",
        meta: "Stage 01",
        tone: "green",
        spark: { label: "Last 1h pulse", values: latencyHistory, formatValue: formatLatency },
      },
      "stage:understand": {
        title: "Understand",
        value: `${skillCount} skills`,
        detail: "Loaded skills and current signals shape request understanding.",
        meta: "Stage 02 · current signal",
        tone: "cyan",
      },
      "stage:plan": {
        title: "Plan",
        value: `${appCount + integrationCount} routes`,
        detail: "Apps and integrations available for planning the next action.",
        meta: "Stage 03 · current signal",
        tone: "cyan",
      },
      "stage:act": {
        title: "Act",
        value: running ? "Active" : "Idle",
        detail: running ? "A mission is currently in progress." : "No active mission is pinned right now.",
        meta: "Stage 04 · current signal",
        tone: running ? "green" : "cyan",
      },
      "stage:reflect": {
        title: "Reflect",
        value: `${traceCount} traces`,
        detail: "Recent traces are available for reflection and outcome review.",
        meta: "Stage 05 · current signal",
        tone: traceCount > 0 ? "green" : "cyan",
      },
      "stage:learn": {
        title: "Learn",
        value: `${learningQueueCount} queued`,
        detail: "Learning queue items waiting for consolidation or review.",
        meta: "Stage 06 · current signal",
        tone: learningQueueCount > 0 ? "green" : "cyan",
      },
      "cap:memory": {
        title: "Memory",
        value: `${memoryCount} entries`,
        detail: "Durable memory entries available to the agent.",
        meta: "Last-hour graph uses memory pressure when available",
        tone: "green",
        spark: { label: "Last 1h pressure", values: memoryPressureHistory, formatValue: formatPercent },
      },
      "cap:skills": {
        title: "Skills",
        value: `${skillCount} loaded`,
        detail: "Callable skills loaded into the current runtime.",
        meta: "Current count",
        tone: "cyan",
      },
      "cap:apps": {
        title: "Apps",
        value: `${appCount} apps`,
        detail: "Registered apps available for automation and action routing.",
        meta: "Current count",
        tone: "cyan",
      },
      "cap:integrations": {
        title: "Integrations",
        value: `${integrationCount} active`,
        detail: "Active external integrations available to AgentArk.",
        meta: "Current count",
        tone: integrationCount > 0 ? "green" : "cyan",
      },
      "cap:trace": {
        title: "Trace",
        value: `${traceCount} traces`,
        detail: "Recent execution traces available for review.",
        meta: "Current count",
        tone: traceCount > 0 ? "green" : "cyan",
      },
      "cap:evolve": {
        title: "Self-Evolve",
        value: selfEvolveEnabled ? "ON" : "OFF",
        detail: selfEvolveEnabled
          ? "Reviewed improvements and background learning are enabled."
          : "Self-evolve is disabled.",
        meta: "Current setting",
        tone: selfEvolveEnabled ? "green" : "amber",
      },
      "cap:learning": {
        title: "Learning Queue",
        value: `${learningQueueCount} queued`,
        detail: "Candidate learnings, provisional runs, and pending reflection items.",
        meta: "Current count",
        tone: learningQueueCount > 0 ? "green" : "cyan",
      },
      "cap:pulse": {
        title: "Pulse",
        value: latencyMs == null ? "No pulse" : formatLatency(latencyMs),
        detail: "Round-trip status pulse from the runtime.",
        meta: "Last-hour graph uses latency samples when available",
        tone: latencyMs == null ? "amber" : "green",
        spark: { label: "Last 1h latency", values: latencyHistory, formatValue: formatLatency },
      },
    }),
    [
      appCount,
      integrationCount,
      latencyHistory,
      latencyMs,
      learningQueueCount,
      memoryCount,
      memoryPressureHistory,
      running,
      selfEvolveEnabled,
      skillCount,
      traceCount,
    ]
  );
  const hoveredPosition = hovered
    ? hovered.startsWith("stage:")
      ? STAGES.find((node) => `stage:${node.id}` === hovered)
      : CAPABILITIES.find((node) => `cap:${node.key}` === hovered)
    : null;
  const hoveredTooltip = hovered ? tooltipByNode[hovered] : null;

  return (
    <div className="nw-syn">
      <div className="nw-syn-stage">
        {/* Cropped tighter than the drawn geometry on purpose: the reach
            synapses run off-canvas toward the telemetry columns, and the
            crop keeps the cortex large inside its grid cell. 530 stays the
            horizontal midpoint (150 + 760/2) so the logo overlay at 50%/50%
            still lands on the nucleus. */}
        <svg
          className="nw-syn-svg"
          viewBox="150 0 760 610"
          preserveAspectRatio="xMidYMid meet"
          role="img"
          aria-label="Agent cognition loop — synaptic cortex"
        >
          {/* concentric dendrite rings */}
          <circle className="nw-syn-ring nw-syn-ring--a" cx={CX} cy={CY} r={78} />
          <circle className="nw-syn-ring nw-syn-ring--b" cx={CX} cy={CY} r={96} />
          <circle className="nw-syn-ring nw-syn-ring--b" cx={CX} cy={CY} r={118} />
          <circle className="nw-syn-ring nw-syn-ring--stage" cx={CX} cy={CY} r={148} />
          <circle className="nw-syn-ring nw-syn-ring--cap" cx={CX} cy={CY} r={262} />

          {/* faint inter-stage web */}
          <g className="nw-syn-chords">
            {CHORDS.map((d, i) => (
              <path key={`chord-${i}`} d={d} />
            ))}
          </g>
          <g className="nw-syn-junctions">
            {JUNCTIONS.map(([x, y], i) => (
              <circle key={`j-${i}`} cx={x} cy={y} r={1.3} />
            ))}
          </g>

          {/* reach synapses toward the surrounding telemetry */}
          <g className="nw-syn-reach">
            {REACH.map((d, i) => (
              <path key={`reach-${i}`} d={d} />
            ))}
          </g>

          {/* stage -> core spokes */}
          <g className="nw-syn-spokes">
            {STAGES.map((s) => (
              <path key={`spoke-${s.id}`} d={SPOKES[s.id]} />
            ))}
          </g>

          {/* capability dendrites */}
          <g className="nw-syn-dendrites">
            {CAPABILITIES.map((c) => (
              <path key={`den-${c.key}`} d={c.dendrite} />
            ))}
          </g>

          {/* travelling pulses */}
          <g aria-hidden="true">
            {PULSES.map((p, i) => (
              <circle
                key={`pulse-${i}`}
                className={`nw-syn-pulse ${p.cls}`}
                r={2.1}
                style={{
                  offsetPath: `path("${p.path}")`,
                  animationDuration: `${p.dur}s`,
                  animationDelay: `${p.delay}s`,
                }}
              />
            ))}
          </g>

          {/* active-stage halos (under the node discs) */}
          <circle className="nw-syn-halo nw-syn-halo--outer" cx={active.x} cy={active.y} r={29} />
          <circle className="nw-syn-halo" cx={active.x} cy={active.y} r={22} />

          {/* stage nodes */}
          {STAGES.map((s) => {
            const isActive = s.id === activeStage;
            const nodeId = `stage:${s.id}` as HoverNodeId;
            return (
              <g
                key={s.id}
                className="nw-node-hover"
                role="button"
                tabIndex={0}
                aria-label={`${s.title.toLowerCase()} stage details`}
                aria-expanded={hovered === nodeId}
                onMouseEnter={() => setHovered(nodeId)}
                onMouseLeave={() => setHovered(null)}
                onFocus={() => setHovered(nodeId)}
                onBlur={() => setHovered(null)}
              >
                <circle cx={s.x} cy={s.y} r={24} fill="transparent" pointerEvents="all" />
                <circle
                  className={`nw-syn-stg${isActive ? " nw-syn-stg--act" : ""}`}
                  cx={s.x}
                  cy={s.y}
                  r={15}
                />
                <text
                  className={`nw-syn-stgnum${isActive ? " nw-syn-stgnum--act" : ""}`}
                  x={s.x}
                  y={s.y + 3.5}
                  textAnchor="middle"
                >
                  {s.number}
                </text>
                <text
                  className={`nw-syn-stgname${isActive ? " nw-syn-stgname--act" : ""}`}
                  x={s.lx}
                  y={s.ly}
                  textAnchor={s.anchor}
                >
                  {s.title}
                </text>
              </g>
            );
          })}
          <text
            className="nw-syn-stgsub"
            x={active.lx}
            y={active.id === "observe" ? active.ly - 14 : active.ly + 15}
            textAnchor={active.anchor}
          >
            {activeWord}
          </text>

          {/* capability dendrite nodes */}
          {CAPABILITIES.map((c) => {
            const v = values[c.key];
            const textX = c.anchor === "end" ? c.x - 16 : c.x + 16;
            const nodeId = `cap:${c.key}` as HoverNodeId;
            return (
              <g
                key={c.key}
                className={`${v.dim ? "nw-syn-cap nw-syn-cap--dim" : "nw-syn-cap"} nw-node-hover`}
                role="button"
                tabIndex={0}
                aria-label={`${c.code} ${c.sub} details`}
                aria-expanded={hovered === nodeId}
                onMouseEnter={() => setHovered(nodeId)}
                onMouseLeave={() => setHovered(null)}
                onFocus={() => setHovered(nodeId)}
                onBlur={() => setHovered(null)}
              >
                <circle cx={c.x} cy={c.y} r={18} fill="transparent" pointerEvents="all" />
                <circle className="nw-syn-capn" cx={c.x} cy={c.y} r={6} />
                <circle className="nw-syn-capcore" cx={c.x} cy={c.y} r={1.8} />
                <text className="nw-syn-capval" x={textX} y={c.y - 2} textAnchor={c.anchor}>
                  {c.code} · {v.value}
                </text>
                <text className="nw-syn-capsub" x={textX} y={c.y + 11} textAnchor={c.anchor}>
                  {c.sub}
                </text>
              </g>
            );
          })}
        </svg>

        {/* cortex aura + fade mask + the brain itself (clean — no frame) */}
        <div className="nw-syn-aura" aria-hidden="true" />
        <div className="nw-syn-coremask" aria-hidden="true" />
        <img className="nw-syn-logo" src="/logo.svg" alt="" aria-hidden="true" />
        {hoveredPosition && hoveredTooltip ? (
          <div
            className={`nw-tooltip nw-tooltip--${hoveredTooltip.tone}`}
            role="tooltip"
            style={tooltipStyle(hoveredPosition.x, hoveredPosition.y)}
          >
            <div className="nw-tooltip-title">{hoveredTooltip.title}</div>
            <div className="nw-tooltip-value">{hoveredTooltip.value}</div>
            <div className="nw-tooltip-detail">{hoveredTooltip.detail}</div>
            {hoveredTooltip.spark ? <TooltipSparkline spark={hoveredTooltip.spark} /> : null}
            {hoveredTooltip.meta ? <div className="nw-tooltip-meta">{hoveredTooltip.meta}</div> : null}
          </div>
        ) : null}
      </div>

    </div>
  );
}
