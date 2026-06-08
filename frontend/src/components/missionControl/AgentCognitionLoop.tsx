export type CognitionStageId = "observe" | "understand" | "plan" | "act" | "reflect" | "learn";

export type AgentCognitionLoopProps = {
  latencyMs?: number | null;
  memoryCount: number;
  skillCount: number;
  appCount: number;
  integrationCount: number;
  traceCount: number;
  selfEvolveEnabled: boolean;
  learningQueueCount: number;
};

type StageNode = {
  id: CognitionStageId;
  number: string;
  title: string;
  detail: string;
  x: number;
  y: number;
  lx: number;
  ly: number;
};

// Organic hexagon ring centred at (280, 195) inside a 560x404 viewBox.
const STAGES: StageNode[] = [
  { id: "observe", number: "01", title: "Observe", detail: "Reading runtime, memory, tasks, and signals", x: 200, y: 95, lx: 200, ly: 60 },
  { id: "understand", number: "02", title: "Understand", detail: "Extracting patterns from traces and context", x: 360, y: 95, lx: 360, ly: 60 },
  { id: "plan", number: "03", title: "Plan", detail: "Choosing actions, approvals, and next moves", x: 445, y: 195, lx: 445, ly: 159 },
  { id: "act", number: "04", title: "Act", detail: "Running tools, skills, apps, and automations", x: 360, y: 295, lx: 360, ly: 338 },
  { id: "reflect", number: "05", title: "Reflect", detail: "Reviewing outcomes, risks, and regressions", x: 200, y: 295, lx: 200, ly: 338 },
  { id: "learn", number: "06", title: "Learn", detail: "Updating memory and reusable routines", x: 115, y: 195, lx: 115, ly: 159 },
];

// Cycle order (drawn with flow arrows) + a few chords for the web mesh.
const CYCLE: Array<[CognitionStageId, CognitionStageId]> = [
  ["observe", "understand"],
  ["understand", "plan"],
  ["plan", "act"],
  ["act", "reflect"],
  ["reflect", "learn"],
  ["learn", "observe"],
];
const CHORDS: Array<[CognitionStageId, CognitionStageId]> = [
  ["observe", "act"],
  ["understand", "reflect"],
  ["plan", "learn"],
];

function nodeById(id: CognitionStageId): StageNode {
  return STAGES.find((s) => s.id === id) ?? STAGES[0];
}

// Bow applied to the drawn cycle edges AND the packet's motion path — shared so
// the packet rides exactly on the visible curves.
const CYCLE_BOW = 24;

// Seconds for one full orbit of the packet. Must stay equal to the durations of
// `nw-cog-orbit` and `nw-cog-node-glow` in neural-web.css — the rim flash
// delays below are fractions of this period.
const ORBIT_DURATION_S = 7;

function cycleControlPoint(a: StageNode, b: StageNode, bow: number): { cx: number; cy: number } {
  const mx = (a.x + b.x) / 2;
  const my = (a.y + b.y) / 2;
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy) || 1;
  return { cx: mx + (-dy / len) * bow, cy: my + (dx / len) * bow };
}

// One continuous path tracing the full cycle (same bow as the drawn cycle edges,
// so motion-driven packets ride exactly on the visible curves). Static — STAGES
// and CYCLE are module constants — so compute it once.
const CYCLE_RING_PATH: string = (() => {
  let d = "";
  CYCLE.forEach(([aId, bId], i) => {
    const a = nodeById(aId);
    const b = nodeById(bId);
    const { cx, cy } = cycleControlPoint(a, b, CYCLE_BOW);
    if (i === 0) d += `M${a.x},${a.y} `;
    d += `Q${cx.toFixed(1)},${cy.toFixed(1)} ${b.x},${b.y} `;
  });
  return d.trim();
})();

// Arc-length fraction of the ring at which the packet ARRIVES at each stage.
// The packet moves at constant speed (linear offset-distance), and the ring's
// segments have different lengths, so arrivals are NOT at uniform i/6 marks —
// the rim flash delays must come from real geometry or the "packet lands,
// node lights up" beat drifts by up to ~0.25s per node. Quadratic beziers
// have no closed-form length; sample them once at module scope.
const STAGE_ARRIVAL_FRACTIONS: Record<CognitionStageId, number> = (() => {
  const segmentLengths = CYCLE.map(([aId, bId]) => {
    const a = nodeById(aId);
    const b = nodeById(bId);
    const { cx, cy } = cycleControlPoint(a, b, CYCLE_BOW);
    const STEPS = 64;
    let length = 0;
    let prevX = a.x;
    let prevY = a.y;
    for (let s = 1; s <= STEPS; s += 1) {
      const t = s / STEPS;
      const u = 1 - t;
      const x = u * u * a.x + 2 * u * t * cx + t * t * b.x;
      const y = u * u * a.y + 2 * u * t * cy + t * t * b.y;
      length += Math.hypot(x - prevX, y - prevY);
      prevX = x;
      prevY = y;
    }
    return length;
  });
  const total = segmentLengths.reduce((sum, len) => sum + len, 0) || 1;
  const fractions = {} as Record<CognitionStageId, number>;
  let cumulative = 0;
  CYCLE.forEach(([aId], i) => {
    fractions[aId] = cumulative / total;
    cumulative += segmentLengths[i];
  });
  return fractions;
})();

// Quadratic curve between two nodes, bowed perpendicular to the chord for an organic feel.
function edgePath(aId: CognitionStageId, bId: CognitionStageId, bow: number): string {
  const a = nodeById(aId);
  const b = nodeById(bId);
  const { cx, cy } = cycleControlPoint(a, b, bow);
  return `M${a.x},${a.y} Q${cx.toFixed(1)},${cy.toFixed(1)} ${b.x},${b.y}`;
}

export function AgentCognitionLoop({
  latencyMs,
  memoryCount,
  skillCount,
  appCount,
  integrationCount,
  traceCount,
  selfEvolveEnabled,
  learningQueueCount,
}: AgentCognitionLoopProps) {
  // Surface metrics as satellite nodes wired into the nearest stage.
  const surfaces: Array<{
    key: string;
    label: string;
    value: string;
    x: number;
    y: number;
    near: CognitionStageId;
  }> = [
    { key: "memory", label: "MEM", value: `${memoryCount}`, x: 48, y: 80, near: "observe" },
    { key: "evolve", label: "EVO", value: selfEvolveEnabled ? "ON" : "OFF", x: 40, y: 195, near: "learn" },
    { key: "learning", label: "LRN", value: `${learningQueueCount}`, x: 48, y: 312, near: "learn" },
    { key: "skills", label: "SKL", value: `${skillCount}`, x: 512, y: 80, near: "understand" },
    { key: "pulse", label: "PLS", value: latencyMs == null ? "-" : `${Math.round(latencyMs)}ms`, x: 520, y: 195, near: "plan" },
    { key: "trace", label: "TRC", value: `${traceCount}`, x: 512, y: 312, near: "plan" },
    { key: "apps", label: "APP", value: `${appCount}`, x: 280, y: 34, near: "observe" },
    { key: "integrations", label: "INT", value: `${integrationCount}`, x: 280, y: 368, near: "act" },
  ];

  return (
    <div className="nw-cog">
      <svg
        className="nw-cog-svg"
        viewBox="0 0 560 404"
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label="Agent cognition loop — neural graph"
      >
        <defs>
          <filter id="cogGlow" x="-40%" y="-40%" width="180%" height="180%">
            <feGaussianBlur stdDeviation="3" result="b" />
            <feMerge>
              <feMergeNode in="b" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
          <marker
            id="cogArrow"
            viewBox="0 0 10 10"
            refX="8"
            refY="5"
            markerWidth="5.5"
            markerHeight="5.5"
            orient="auto-start-reverse"
          >
            <path d="M0 0 L10 5 L0 10 z" fill="#7ce7ff" />
          </marker>
        </defs>

        {/* satellite connectors */}
        <g className="nw-cog-connectors" stroke="rgba(124,231,255,0.16)" strokeWidth={1} strokeDasharray="2 3" fill="none">
          {surfaces.map((s) => {
            const n = nodeById(s.near);
            return <line key={`con-${s.key}`} x1={s.x} y1={s.y} x2={n.x} y2={n.y} />;
          })}
        </g>

        {/* chords (web mesh) */}
        <g className="nw-cog-chords" fill="none" stroke="rgba(124,231,255,0.16)" strokeWidth={1.2} strokeDasharray="5 6">
          {CHORDS.map(([a, b], i) => (
            <path key={`chord-${i}`} d={edgePath(a, b, 0)} />
          ))}
        </g>

        {/* cycle edges with flow direction */}
        <g className="nw-cog-cycle" fill="none" stroke="rgba(124,231,255,0.5)" strokeWidth={1.6} filter="url(#cogGlow)">
          {CYCLE.map(([a, b], i) => (
            <path key={`cyc-${i}`} d={edgePath(a, b, 24)} markerEnd="url(#cogArrow)" />
          ))}
        </g>

        {/* energy streaming around the ring — dashed overlay on the exact cycle
            curves. No blur filter (would re-rasterize every frame); just an
            animated stroke-dashoffset, which the compositor handles cheaply. */}
        <path
          className="nw-cog-flow"
          d={CYCLE_RING_PATH}
          fill="none"
          stroke="#9af0ff"
          strokeWidth={1.8}
          strokeLinecap="round"
        />

        {/* satellite nodes */}
        {surfaces.map((s, i) => (
          <g key={`sat-${s.key}`}>
            <circle
              className="nw-cog-sat"
              style={{ animationDelay: `${(i % 5) * 0.45}s` }}
              cx={s.x}
              cy={s.y}
              r={5}
              fill="rgba(124,231,255,0.14)"
              stroke="rgba(124,231,255,0.42)"
              strokeWidth={1}
            />
            <text
              x={s.x}
              y={s.y + 17}
              textAnchor="middle"
              fontFamily="'JetBrains Mono', monospace"
              fontSize={8}
              fill="rgba(213,216,223,0.7)"
            >
              {s.label}{" "}
              <tspan fill="#7ce7ff" fontWeight={700}>
                {s.value}
              </tspan>
            </text>
          </g>
        ))}

        {/* stage nodes — each rim flashes exactly when the packet arrives:
            delay = the stage's arc-length fraction of the orbit period. */}
        {STAGES.map((s) => {
          const glowDelay = `${(STAGE_ARRIVAL_FRACTIONS[s.id] * ORBIT_DURATION_S).toFixed(3)}s`;
          return (
            <g key={s.id}>
              <circle
                className="nw-cog-node-rim"
                style={{ animationDelay: glowDelay }}
                cx={s.x}
                cy={s.y}
                r={25}
                fill="none"
                stroke="#7ce7ff"
                strokeWidth={1.6}
              />
              <circle
                cx={s.x}
                cy={s.y}
                r={25}
                fill="rgba(10,14,18,0.92)"
                stroke="rgba(124,231,255,0.55)"
                strokeWidth={1.4}
              />
              <text
                x={s.x}
                y={s.y}
                textAnchor="middle"
                dominantBaseline="central"
                fontFamily="'JetBrains Mono', monospace"
                fontSize={12}
                fontWeight={700}
                fill="rgba(124,231,255,0.82)"
              >
                {s.number}
              </text>
              <text
                x={s.lx}
                y={s.ly}
                textAnchor="middle"
                fontFamily="'JetBrains Mono', monospace"
                fontSize={9.5}
                fontWeight={600}
                letterSpacing="0.08em"
                fill="#d8f5ff"
              >
                {s.title.toUpperCase()}
              </text>
            </g>
          );
        })}

        {/* light packet orbiting the loop. CSS offset-path (not SMIL): SMIL
            runs on the SVG's own timeline, which Chrome pauses while the tab
            is hidden — CSS animations keep wall-clock phase — so the packet
            drifted out of sync with the rim flashes after every tab switch.
            One clock for both keeps "packet lands -> node lights" locked.
            Drawn after the stage nodes so the packet visibly travels ONTO
            each disc as the rim fires. Halo + core pair, no SVG filter. */}
        <g className="nw-cog-packet" aria-hidden="true">
          <circle
            r={7}
            fill="rgba(124,231,255,0.18)"
            style={{ offsetPath: `path("${CYCLE_RING_PATH}")` }}
          />
          <circle
            r={2.6}
            fill="#cdf6ff"
            style={{ offsetPath: `path("${CYCLE_RING_PATH}")` }}
          />
        </g>
      </svg>

      <div className="nw-cog-caption">
        <span className="nw-cog-caption-num">01-06</span>
        <span className="nw-cog-caption-title">Live loop</span>
        <span className="nw-cog-caption-detail">Observe, understand, plan, act, reflect, learn</span>
      </div>
    </div>
  );
}
