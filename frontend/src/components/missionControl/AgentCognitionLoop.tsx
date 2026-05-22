export type CognitionStageId = "observe" | "understand" | "plan" | "act" | "reflect" | "learn";

export type AgentCognitionLoopProps = {
  activeStage: CognitionStageId;
  iteration: number;
  modelConfigured: boolean;
  latencyMs?: number | null;
  memoryCount: number;
  skillCount: number;
  appCount: number;
  integrationCount: number;
  traceCount: number;
  selfEvolveEnabled: boolean;
  learningQueueCount: number;
};

const STAGES: Array<{
  id: CognitionStageId;
  number: string;
  title: string;
  detail: string;
  tone: "cyan" | "violet" | "teal";
}> = [
  {
    id: "observe",
    number: "01",
    title: "Observe",
    detail: "Reading runtime, memory, tasks, and signals",
    tone: "teal",
  },
  {
    id: "understand",
    number: "02",
    title: "Understand",
    detail: "Extracting patterns from traces and context",
    tone: "teal",
  },
  {
    id: "plan",
    number: "03",
    title: "Plan",
    detail: "Choosing actions, approvals, and next moves",
    tone: "violet",
  },
  {
    id: "act",
    number: "04",
    title: "Act",
    detail: "Running tools, skills, apps, and automations",
    tone: "teal",
  },
  {
    id: "reflect",
    number: "05",
    title: "Reflect",
    detail: "Reviewing outcomes, risks, and regressions",
    tone: "violet",
  },
  {
    id: "learn",
    number: "06",
    title: "Learn",
    detail: "Updating memory and reusable routines",
    tone: "teal",
  },
];

export function AgentCognitionLoop({
  activeStage,
  latencyMs,
  memoryCount,
  skillCount,
  appCount,
  integrationCount,
  traceCount,
  selfEvolveEnabled,
  learningQueueCount,
}: AgentCognitionLoopProps) {
  const surfaces = [
    { label: "Memory", value: `${memoryCount}` },
    { label: "Skills", value: `${skillCount}` },
    { label: "Apps", value: `${appCount}` },
    { label: "Integrations", value: `${integrationCount}` },
    { label: "Evolve", value: selfEvolveEnabled ? "ON" : "OFF" },
    { label: "Learning", value: `${learningQueueCount}` },
    { label: "Trace", value: `${traceCount}` },
    { label: "Pulse", value: latencyMs == null ? "-" : `${Math.round(latencyMs)}ms` },
  ];

  return (
    <div className="nw-loop">
      <svg className="nw-loop-paths" viewBox="0 0 520 600" aria-hidden="true">
        <path className="nw-loop-path nw-loop-path--cyan" d="M130 74 C64 74 64 160 130 160" />
        <path className="nw-loop-path nw-loop-path--violet" d="M390 160 C456 160 456 246 390 246" />
        <path className="nw-loop-path nw-loop-path--cyan" d="M130 246 C64 246 64 332 130 332" />
        <path className="nw-loop-path nw-loop-path--violet" d="M390 332 C456 332 456 418 390 418" />
        <path className="nw-loop-path nw-loop-path--cyan" d="M130 418 C64 418 64 504 130 504" />
        <path className="nw-loop-path nw-loop-path--outer" d="M420 74 C492 90 492 492 420 516" />
        <path className="nw-loop-path nw-loop-path--outer" d="M100 516 C28 492 28 90 100 74" />
      </svg>

      <div className="nw-loop-surfaces" aria-hidden="true">
        {surfaces.map((surface) => (
          <span className="nw-loop-surface" key={surface.label} title={`${surface.label}: ${surface.value}`}>
            <b>{surface.label}</b>
            <i>{surface.value}</i>
          </span>
        ))}
      </div>

      <div className="nw-loop-column">
        {STAGES.map((stage) => {
          const active = stage.id === activeStage;
          return (
            <div
              className={`nw-loop-stage nw-loop-stage--${stage.tone}${active ? " nw-loop-stage--active" : ""}`}
              key={stage.id}
            >
              <div className="nw-loop-icon">{stage.number}</div>
              <div className="nw-loop-copy">
                <div className="nw-loop-title">
                  <span>{stage.number}</span>
                  {stage.title}
                </div>
                <div className="nw-loop-detail">{stage.detail}</div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
