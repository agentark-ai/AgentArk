import { useMemo, type CSSProperties } from "react";
import { NeuralPanel } from "./NeuralPanel";
import type { BackgroundSessionSummary, BriefingResponse, RuntimeHealth, Task, TraceSummary } from "../../types";

type SecurityLog = {
  event_type: string;
  severity: string;
  message: string;
};

type AutomationCounts = {
  tasks: number;
  watchers: number;
  apps: number;
  integrations: number;
};

function cleanText(value?: string | null, fallback = "-"): string {
  const text = String(value || "").replace(/\s+/g, " ").trim();
  return text || fallback;
}

function pct(value?: number | null): string {
  return typeof value === "number" && Number.isFinite(value) ? `${Math.round(value)}%` : "-";
}

function formatRuntimeBytes(value?: number | null): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "-";
  const mib = value / 1024 / 1024;
  if (mib < 1024) return `${Math.round(mib)} MB`;
  return `${(mib / 1024).toFixed(1)} GB`;
}

function memorySourceLabel(value?: string | null): string {
  switch (value) {
    case "docker_stack":
      return "Docker stack";
    case "cgroup":
      return "Runtime cgroup";
    case "proc_meminfo":
      return "Host runtime";
    default:
      return "Runtime";
  }
}

function taskStatus(task: Task): string {
  return String(task?.status || "").toLowerCase();
}

export function ReflectionNotesCard({
  briefing,
  traces,
}: {
  briefing?: BriefingResponse;
  traces: TraceSummary[];
}) {
  const topOpportunity = briefing?.top_opportunities?.[0];
  const topRisk = briefing?.top_risks?.[0];
  const note =
    cleanText(topOpportunity?.summary || topOpportunity?.detail || topOpportunity?.title, "") ||
    cleanText(topRisk?.summary || topRisk?.detail || topRisk?.title, "") ||
    cleanText(traces[0]?.message_preview, "No reflection note has landed yet.");
  const score = typeof topOpportunity?.score === "number" ? topOpportunity.score : null;

  return (
    <NeuralPanel title="Reflection Notes" tag={briefing ? "BRIEFING" : "TRACE"} tagTone="cyan" className="nw-card--reflection">
      <div className="nw-quote-card">
        <div className="nw-quote-mark">"</div>
        <p>{note}</p>
        <div className="nw-reflection-footer">
          <span>Confidence</span>
          <strong>{score == null ? "-" : score.toFixed(2)}</strong>
        </div>
      </div>
    </NeuralPanel>
  );
}

export function RecentLearningsCard({ briefing }: { briefing?: BriefingResponse }) {
  const rows = [
    ...(briefing?.top_opportunities || []).slice(0, 1).map((item) => ({
      key: `opportunity-${item.title || item.summary || item.detail}`,
      label: "OPPORTUNITY",
      title: cleanText(item.title || item.summary || item.detail, "Opportunity detected"),
      meta: typeof item.score === "number" ? `SCORE ${item.score.toFixed(2)}` : "BRIEFING",
    })),
    ...(briefing?.top_risks || []).slice(0, 1).map((item) => ({
      key: `risk-${item.title || item.summary || item.detail}`,
      label: "RISK",
      title: cleanText(item.title || item.summary || item.detail, "Risk detected"),
      meta: cleanText(item.severity, "REVIEW").toUpperCase(),
    })),
    ...([...(briefing?.recommended_actions || []), ...(briefing?.recommended_skills || [])]).slice(0, 2).map((item) => ({
      key: `action-${item.id || item.title}`,
      label: "ACTION",
      title: cleanText(item.title || item.summary || item.readiness?.plain_summary, "Recommended action"),
      meta: cleanText(item.readiness?.stage || item.readiness?.label, "READY").toUpperCase(),
    })),
  ].slice(0, 3);
  return (
    <NeuralPanel title="Briefing Signals" tag={`${rows.length} ITEMS`} tagTone="cyan" className="nw-card--learnings">
      <div className="nw-learning-list">
        {rows.length === 0 ? (
          <div className="nw-panel-muted">No briefing signals yet.</div>
        ) : (
          rows.map((row) => (
            <div className="nw-learning-row" key={row.key}>
              <div className="nw-learning-icon">{row.label.slice(0, 2)}</div>
              <div className="nw-learning-copy">
                <div className="nw-learning-title">{row.title}</div>
                <div className="nw-learning-meta">{row.meta}</div>
              </div>
            </div>
          ))
        )}
      </div>
    </NeuralPanel>
  );
}

export function MemoryStateCard({
  memoryCount,
  health,
}: {
  memoryCount: number;
  health?: RuntimeHealth | null;
}) {
  const source = memorySourceLabel(health?.memory_source);
  const tag = health?.memory_source === "docker_stack" ? "DOCKER STACK" : source.toUpperCase();
  return (
    <NeuralPanel title="Runtime Memory" tag={tag} tagTone="cyan" className="nw-card--memory">
      <div className="nw-mini-metric-grid">
        <div>
          <strong>{formatRuntimeBytes(health?.memory_used_bytes)}</strong>
          <span>Used</span>
        </div>
        <div>
          <strong>{pct(health?.memory_pressure_percent ?? health?.ram_percent)}</strong>
          <span>Pressure</span>
        </div>
      </div>
      <div className="nw-memory-line">
        <span>Source</span>
        <strong>{source}</strong>
      </div>
      <div className="nw-memory-line">
        <span>RAM total</span>
        <strong>{formatRuntimeBytes(health?.memory_total_bytes)}</strong>
      </div>
      {memoryCount > 0 && (
        <div className="nw-memory-line">
          <span>Saved memory</span>
          <strong>{memoryCount}</strong>
        </div>
      )}
    </NeuralPanel>
  );
}

export function ActiveMissionsCard({
  tasks,
  sessions,
}: {
  tasks: Task[];
  sessions: BackgroundSessionSummary[];
}) {
  const counts = useMemo(() => {
    const running = tasks.filter((task) => {
      const status = taskStatus(task);
      return status.includes("progress") || status.includes("running");
    }).length;
    const pending = tasks.filter((task) => taskStatus(task).includes("pending")).length;
    return {
      running: running + sessions.length,
      pending,
      total: running + sessions.length + pending,
    };
  }, [sessions.length, tasks]);
  const total = Math.max(1, counts.total);

  return (
    <NeuralPanel title="Active Missions" tag={`${counts.total} TOTAL`} tagTone="cyan" className="nw-card--missions">
      <div className="nw-mission-body">
        <div className="nw-mission-total">
          {counts.total > 0 ? (
            <div className="nw-mission-donut" style={{ "--nw-progress": `${(counts.running / total) * 360}deg` } as CSSProperties}>
              <strong>{counts.total}</strong>
              <span>Total</span>
            </div>
          ) : (
            <div className="nw-mission-zero">
              <strong>0</strong>
              <span>Total</span>
            </div>
          )}
        </div>
        <div className="nw-mission-legend">
          <div>
            <span><i /> In progress</span>
            <strong>{counts.running}</strong>
          </div>
          <div>
            <span><i /> Pending</span>
            <strong>{counts.pending}</strong>
          </div>
          <div>
            <span><i /> Queue</span>
            <strong>{counts.total === 0 ? "Clear" : "Active"}</strong>
          </div>
        </div>
      </div>
    </NeuralPanel>
  );
}

export function SafetyGuardrailsCard({
  securityLogs,
  hasLlmConfigured,
}: {
  securityLogs: SecurityLog[];
  hasLlmConfigured: boolean;
}) {
  const hasAlerts = securityLogs.length > 0;
  const latestSeverity = cleanText(securityLogs[0]?.severity, "clear").toUpperCase();
  return (
    <NeuralPanel title="Safety / Guardrails" tag={hasAlerts ? "REVIEW" : "CLEAR"} tagTone={hasAlerts ? "warn" : "good"} className="nw-card--safety">
      <div className="nw-safety-list">
        <div><span>Security events</span><strong>{securityLogs.length}</strong></div>
        <div><span>Latest severity</span><strong>{latestSeverity}</strong></div>
        <div><span>Model access</span><strong>{hasLlmConfigured ? "CONFIGURED" : "SETUP"}</strong></div>
      </div>
    </NeuralPanel>
  );
}

export function SurfaceSummaryCard({
  automationCounts,
}: {
  automationCounts: AutomationCounts;
}) {
  return (
    <NeuralPanel title="Automation Surfaces" tag={`${automationCounts.tasks + automationCounts.watchers + automationCounts.apps + automationCounts.integrations} TOTAL`} tagTone="cyan" className="nw-card--surfaces">
      <div className="nw-mini-metric-grid nw-mini-metric-grid--4">
        <div><strong>{automationCounts.tasks}</strong><span>Tasks</span></div>
        <div><strong>{automationCounts.watchers}</strong><span>Watchers</span></div>
        <div><strong>{automationCounts.apps}</strong><span>Apps</span></div>
        <div><strong>{automationCounts.integrations}</strong><span>Integrations</span></div>
      </div>
    </NeuralPanel>
  );
}
