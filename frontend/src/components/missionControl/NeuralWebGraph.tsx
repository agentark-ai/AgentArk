// Pure-SVG neural-web mission-control graph: AgentArk core + symmetric satellite ring.
import { useState } from 'react';

export type NeuralNodeKey =
  | 'core'
  | 'arkmemory'
  | 'arksentinel'
  | 'arkevolve'
  | 'arkreflect'
  | 'arkpulse'
  | 'memories'
  | 'surfaces'
  | 'runtime'
  | 'tasks'
  | 'watchers'
  | 'model'
  | 'skills'
  | 'integrations'
  | 'autonomy'
  | 'brief'
  | 'trace'
  | 'apps';

type Props = {
  skillCount: number;
  memoryCount: number;
  surfaceCount: number;
  integrationCount: number;
  taskCount: number;
  watcherCount: number;
  liveRunCount: number;
  modelConfigured: boolean;
  autonomyActive: boolean;
  rttMs?: number | null;
  serverError?: boolean;
  onNavigate?: (view: string) => void;
};

type TooltipData = {
  title: string;
  value: string;
  detail: string;
  tone: Tone;
  meta?: string;
};

const tooltipFor = (key: NeuralNodeKey, p: Props): TooltipData => {
  switch (key) {
    case 'memories':
    case 'arkmemory':
      return {
        title: 'Memory',
        value: p.memoryCount === 1 ? '1 entry' : `${p.memoryCount} entries`,
        detail: 'Persistent learnings the agent recalls across sessions and conversations.',
        tone: 'green',
        meta: 'Open Memory',
      };
    case 'arksentinel':
      return {
        title: 'Sentinel',
        value: p.taskCount === 0 ? 'Queue clear' : 'Review queue active',
        detail: 'Decision inbox for approvals, observations, and background learning items that need operator attention.',
        tone: p.taskCount === 0 ? 'green' : 'cyan',
        meta: 'Open Sentinel',
      };
    case 'arkevolve':
      return {
        title: 'Evolve',
        value: p.surfaceCount === 0 ? 'Watching' : `${p.surfaceCount} surfaces`,
        detail: 'Improvement lifecycle for learning, review-only suggestions, live tests, stable changes, and rollback.',
        tone: 'green',
        meta: 'Open Evolve',
      };
    case 'arkreflect':
      return {
        title: 'Reflect',
        value: 'Recap layer',
        detail: 'Retrospective view over time windows, work patterns, source coverage, and background activity.',
        tone: 'cyan',
        meta: 'Open Reflect',
      };
    case 'arkpulse':
      return {
        title: 'Pulse',
        value: p.serverError ? 'Needs attention' : p.rttMs != null ? `${Math.round(p.rttMs)}ms pulse` : 'Ready',
        detail: 'Operational health, runtime diagnostics, findings, remediation guidance, and safe fix execution.',
        tone: p.serverError ? 'crit' : 'green',
        meta: 'Open Pulse',
      };
    case 'skills':
      return {
        title: 'Skills',
        value: `${p.skillCount} loaded`,
        detail: 'Capabilities the agent can invoke right now — actions, tools, integrations.',
        tone: 'green',
        meta: 'Open Skills',
      };
    case 'surfaces':
      return {
        title: 'Surfaces',
        value: p.surfaceCount === 1 ? '1 active' : `${p.surfaceCount} active`,
        detail: 'Runtime surfaces — every task, watcher, app, or integration registered with the OS.',
        tone: 'green',
      };
    case 'integrations':
      return {
        title: 'Integrations',
        value: `${p.integrationCount} connected`,
        detail: 'External services and data sources wired into the OS.',
        tone: 'cyan',
      };
    case 'runtime':
      return {
        title: 'Runtime',
        value:
          p.liveRunCount > 0
            ? p.liveRunCount === 1
              ? '1 live run'
              : `${p.liveRunCount} live runs`
            : 'Idle',
        detail: 'Active background sessions and supervised work currently in progress.',
        tone: p.liveRunCount > 0 ? 'cyan' : 'green',
      };
    case 'tasks':
      return {
        title: 'Tasks',
        value: p.taskCount === 0 ? 'Queue clear' : `${p.taskCount} active`,
        detail: 'Pending and in-flight task surfaces in the operator queue.',
        tone: p.taskCount === 0 ? 'green' : 'cyan',
      };
    case 'watchers':
      return {
        title: 'Watchers',
        value: p.watcherCount === 0 ? 'None standing' : `${p.watcherCount} standing`,
        detail: 'Background observers monitoring inboxes, feeds, and triggers.',
        tone: 'cyan',
      };
    case 'model':
      return {
        title: 'Model',
        value: p.modelConfigured ? 'Ready' : 'Needs setup',
        detail: p.modelConfigured
          ? 'LLM provider configured and reachable.'
          : 'No LLM model is configured yet. Add one in Settings → Models.',
        tone: p.modelConfigured ? 'green' : 'amber',
      };
    case 'autonomy':
      return {
        title: 'Autonomy',
        value: p.autonomyActive ? 'Active' : 'Paused',
        detail: p.autonomyActive
          ? 'Background execution enabled — the agent acts on schedules and triggers.'
          : 'Background execution paused. Scheduled reminders still fire while proactive work stays paused.',
        tone: p.autonomyActive ? 'green' : 'amber',
      };
    case 'brief':
      return {
        title: 'Daily Brief',
        value: 'On demand',
        detail: 'Generates a snapshot of recent activity, recommendations, and risks for the day.',
        tone: 'green',
      };
    case 'trace':
      return {
        title: 'Trace',
        value: 'Operator log',
        detail: 'Recent supervised runs and operator-visible outcomes — every reasoning step recorded.',
        tone: 'green',
      };
    case 'apps':
      return {
        title: 'Apps',
        value: 'Connected',
        detail: 'Connected apps — the inventory of external automations and integrations.',
        tone: 'green',
      };
    case 'core':
      return {
        title: 'Ark Core',
        value: p.serverError
          ? 'Offline'
          : p.rttMs != null
          ? `Ready · ${Math.round(p.rttMs)}ms`
          : 'Ready',
        detail: 'Reasoning core — routing, memory, and tool orchestration. The heart of the OS.',
        tone: p.serverError ? 'crit' : 'green',
      };
  }
};

const NAV_KEY_TO_POSITION: Record<NeuralNodeKey, string> = {
  core: 'CORE',
  arkmemory: 'N',
  arksentinel: 'NE',
  arkevolve: 'E',
  arkreflect: 'SE',
  arkpulse: 'S',
  memories: 'N',
  surfaces: 'NE',
  runtime: 'SE',
  integrations: 'E',
  tasks: 'IN_E',
  watchers: 'SW',
  model: 'W',
  skills: 'NW',
  autonomy: 'IN_N',
  trace: 'IN_S',
  apps: 'IN_W',
  brief: 'IN_E',
};

const NAV_KEY_TO_VIEW: Partial<Record<NeuralNodeKey, string>> = {
  core: 'overview',
  arkmemory: 'arkmemory',
  arksentinel: 'sentinel',
  arkevolve: 'evolution',
  arkreflect: 'arkreflect',
  arkpulse: 'arkpulse',
  memories: 'arkmemory',
  surfaces: 'apps',
  runtime: 'status',
  tasks: 'tasks',
  watchers: 'status',
  model: 'settings',
  skills: 'skills',
  integrations: 'settings',
  autonomy: 'autonomy',
  brief: 'chat',
  trace: 'trace',
  apps: 'apps',
};

const tooltipTransform = (px: number, py: number): { tx: string; ty: string } => {
  // Place tooltip away from the node based on quadrant.
  const dx = px - CX;
  const dy = py - CY;
  const isCore = dx === 0 && dy === 0;
  if (isCore) {
    return { tx: '-50%', ty: 'calc(38px)' };
  }
  // Side nodes (E/W): tooltip placed horizontally toward center
  if (Math.abs(dy) < 50) {
    return dx < 0
      ? { tx: 'calc(28px)', ty: '-50%' }
      : { tx: 'calc(-100% - 28px)', ty: '-50%' };
  }
  // Top half: tooltip below; bottom half: tooltip above
  return dy < 0
    ? { tx: '-50%', ty: 'calc(36px)' }
    : { tx: '-50%', ty: 'calc(-100% - 36px)' };
};

const VIEW_W = 1120;
const VIEW_H = 560;
const CX = 560;
const CY = 280;
const OUTER_RX = 430;
const OUTER_RY = 178;
const INNER_RX = 150;
const INNER_RY = 86;

const polar = (rx: number, ry: number, deg: number) => ({
  x: CX + rx * Math.cos((deg * Math.PI) / 180),
  y: CY + ry * Math.sin((deg * Math.PI) / 180),
});

type NodePos = { x: number; y: number; angle: number };

const OUTER_ANGLES = [-90, -45, 0, 45, 90, 135, 180, 225] as const;
const OUTER_KEYS = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'] as const;
const INNER_ANGLES = [-90, 0, 90, 180] as const;
const INNER_KEYS = ['IN_N', 'IN_E', 'IN_S', 'IN_W'] as const;

const buildPositions = (): Record<string, NodePos> => {
  const out: Record<string, NodePos> = { CORE: { x: CX, y: CY, angle: 0 } };
  OUTER_ANGLES.forEach((a, i) => {
    const p = polar(OUTER_RX, OUTER_RY, a);
    out[OUTER_KEYS[i]] = { x: p.x, y: p.y, angle: a };
  });
  INNER_ANGLES.forEach((a, i) => {
    const p = polar(INNER_RX, INNER_RY, a);
    out[INNER_KEYS[i]] = { x: p.x, y: p.y, angle: a };
  });
  return out;
};

export const NEURAL_NODE_POSITIONS: Record<string, NodePos> = buildPositions();

type Tone = 'green' | 'cyan' | 'amber' | 'crit';

const nodeFill = (tone: Tone): string => {
  switch (tone) {
    case 'cyan':
      return 'url(#grad-cyan)';
    case 'amber':
      return 'url(#grad-amber)';
    case 'crit':
      return 'url(#grad-crit)';
    default:
      return 'url(#grad-node)';
  }
};

const edgeStroke = (tone: Tone): string => {
  switch (tone) {
    case 'cyan':
      return 'url(#edge-grad-cyan)';
    case 'amber':
      return 'url(#edge-grad-amber)';
    case 'crit':
      return 'url(#edge-grad-crit)';
    default:
      return 'url(#edge-grad)';
  }
};

const nodeStrokeColor = (tone: Tone): string => {
  switch (tone) {
    case 'cyan':
      return 'rgba(120,220,242,0.85)';
    case 'amber':
      return 'rgba(242,196,120,0.9)';
    case 'crit':
      return 'rgba(255,155,155,0.95)';
    default:
      return 'rgba(120,242,176,0.88)';
  }
};

const labelFill = (tone: Tone): string => {
  switch (tone) {
    case 'cyan':
      return '#bfeaf5';
    case 'amber':
      return '#f5d9b0';
    case 'crit':
      return '#ffc8c8';
    default:
      return '#dff7e7';
  }
};

type OuterDef = { key: typeof OUTER_KEYS[number]; navKey: NeuralNodeKey; label: string; tone: Tone };
type InnerDef = { key: typeof INNER_KEYS[number]; navKey: NeuralNodeKey; label: string; tone: Tone };

const buildOuter = (p: Props): OuterDef[] => {
  const runtimeLabel = p.liveRunCount >= 1 ? 'RUNTIME · LIVE' : 'RUNTIME · IDLE';
  const tasksTone: Tone = p.taskCount === 0 ? 'green' : 'cyan';
  const modelLabel = p.modelConfigured ? 'MODEL · READY' : 'MODEL · SETUP';
  const modelTone: Tone = p.modelConfigured ? 'green' : 'amber';
  return [
    { key: 'N', navKey: 'memories', label: `MEMORIES · ${p.memoryCount}`, tone: 'green' },
    { key: 'NE', navKey: 'surfaces', label: `SURFACES · ${p.surfaceCount}`, tone: 'green' },
    { key: 'E', navKey: 'integrations', label: `INTEGRATIONS · ${p.integrationCount}`, tone: 'cyan' },
    { key: 'SE', navKey: 'runtime', label: runtimeLabel, tone: 'green' },
    { key: 'S', navKey: 'tasks', label: `TASKS · ${p.taskCount}`, tone: tasksTone },
    { key: 'SW', navKey: 'watchers', label: `WATCHERS · ${p.watcherCount}`, tone: 'cyan' },
    { key: 'W', navKey: 'model', label: modelLabel, tone: modelTone },
    { key: 'NW', navKey: 'skills', label: `SKILLS · ${p.skillCount}`, tone: 'green' },
  ];
};

const buildInner = (p: Props): InnerDef[] => [
  { key: 'IN_N', navKey: 'autonomy', label: 'AUTONOMY', tone: p.autonomyActive ? 'green' : 'amber' },
  { key: 'IN_E', navKey: 'brief', label: 'BRIEF', tone: 'green' },
  { key: 'IN_S', navKey: 'trace', label: 'TRACE', tone: 'green' },
  { key: 'IN_W', navKey: 'apps', label: 'APPS', tone: 'green' },
];

const buildArkCoreOuter = (p: Props): OuterDef[] => {
  const modelLabel = p.modelConfigured ? 'MODEL READY' : 'MODEL SETUP';
  const modelTone: Tone = p.modelConfigured ? 'green' : 'amber';
  const sentinelTone: Tone = p.taskCount === 0 ? 'green' : 'cyan';
  const pulseTone: Tone = p.serverError ? 'crit' : 'green';
  const pulseLabel = p.serverError
    ? 'ARKPULSE ALERT'
    : p.rttMs != null
      ? `ARKPULSE ${Math.round(p.rttMs)}MS`
      : 'ARKPULSE READY';
  return [
    { key: 'N', navKey: 'arkmemory', label: `ARKMEMORY ${p.memoryCount}`, tone: 'green' },
    { key: 'NE', navKey: 'arksentinel', label: p.taskCount === 0 ? 'ARKSENTINEL CLEAR' : `ARKSENTINEL ${p.taskCount}`, tone: sentinelTone },
    { key: 'E', navKey: 'arkevolve', label: p.surfaceCount === 0 ? 'ARKEVOLVE WATCH' : `ARKEVOLVE ${p.surfaceCount}`, tone: 'green' },
    { key: 'SE', navKey: 'arkreflect', label: 'ARKREFLECT READY', tone: 'cyan' },
    { key: 'S', navKey: 'arkpulse', label: pulseLabel, tone: pulseTone },
    { key: 'SW', navKey: 'watchers', label: `WATCHERS ${p.watcherCount}`, tone: 'cyan' },
    { key: 'W', navKey: 'model', label: modelLabel, tone: modelTone },
    { key: 'NW', navKey: 'skills', label: `SKILLS ${p.skillCount}`, tone: 'green' },
  ];
};

// Top-half labels above (-22), bottom-half labels below (+30), pure E/W above (-22).
const labelOffsetY = (angle: number): number => {
  // Side nodes: angle === 0 (E) or 180 (W) -> above
  if (angle === 0 || angle === 180) return -22;
  // Top half: sin(angle) < 0 -> above
  if (Math.sin((angle * Math.PI) / 180) < 0) return -22;
  return 30;
};

const hexPath = (cx: number, cy: number, r: number): string => {
  const pts: string[] = [];
  for (let i = 0; i < 6; i += 1) {
    const a = (Math.PI / 3) * i - Math.PI / 2;
    const x = cx + r * Math.cos(a);
    const y = cy + r * Math.sin(a);
    pts.push(`${i === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`);
  }
  pts.push('Z');
  return pts.join(' ');
};

export function NeuralWebGraph(props: Props) {
  const [hovered, setHovered] = useState<NeuralNodeKey | null>(null);
  const forceCritical = Boolean(props.serverError);
  const outer = buildArkCoreOuter(props).map((node) =>
    forceCritical ? { ...node, tone: 'crit' as Tone } : node,
  );
  const inner = buildInner(props).map((node) =>
    forceCritical ? { ...node, tone: 'crit' as Tone } : node,
  );
  const coreTone: Tone = props.serverError ? 'crit' : 'green';
  const coreGradient = coreTone === 'crit' ? 'url(#grad-core-crit)' : 'url(#grad-core)';
  const coreLine2 = props.serverError
    ? 'OFFLINE'
    : props.rttMs != null
    ? `READY · ${Math.round(props.rttMs)}MS`
    : 'READY';

  const handleEnter = (k: NeuralNodeKey) => () => setHovered(k);
  const handleLeave = () => setHovered(null);
  const handleActivate = (k: NeuralNodeKey) => () => {
    const view = NAV_KEY_TO_VIEW[k];
    if (view && props.onNavigate) {
      props.onNavigate(view);
      return;
    }
    setHovered((prev) => (prev === k ? null : k));
  };
  const handleKey = (k: NeuralNodeKey) => (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      const view = NAV_KEY_TO_VIEW[k];
      if (view && props.onNavigate) {
        props.onNavigate(view);
      }
    } else if (e.key === ' ') {
      e.preventDefault();
      setHovered((prev) => (prev === k ? null : k));
    } else if (e.key === 'Escape') {
      setHovered(null);
    }
  };

  return (
    <div className={`nw-stage nw-stage--${coreTone}`}>
      <span className="nw-stage-corner tl" />
      <span className="nw-stage-corner tr" />
      <span className="nw-stage-corner bl" />
      <span className="nw-stage-corner br" />
      <span className="nw-stage-axis t">// NEURAL WEB · CORE TOPOLOGY</span>
      <span className="nw-stage-axis b">// 13 NODES · 18 EDGES · COHERENCE 0.94</span>

      <svg
        className="nw-svg"
        viewBox={`0 0 ${VIEW_W} ${VIEW_H}`}
        preserveAspectRatio="xMidYMid meet"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <radialGradient id="grad-core" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#ffffff" stopOpacity="0.95" />
            <stop offset="35%" stopColor="#78f2b0" stopOpacity="0.82" />
            <stop offset="100%" stopColor="#78f2b0" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="grad-core-crit" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#ffe5e5" stopOpacity="0.95" />
            <stop offset="45%" stopColor="#ff6b6b" stopOpacity="0.78" />
            <stop offset="100%" stopColor="#ff6b6b" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="grad-node" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#e7fff0" stopOpacity="0.95" />
            <stop offset="60%" stopColor="#78f2b0" stopOpacity="0.54" />
            <stop offset="100%" stopColor="#78f2b0" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="grad-amber" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#fff1d6" stopOpacity="0.95" />
            <stop offset="60%" stopColor="#f2c478" stopOpacity="0.55" />
            <stop offset="100%" stopColor="#f2c478" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="grad-cyan" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#e7fff0" stopOpacity="0.95" />
            <stop offset="60%" stopColor="#78f2b0" stopOpacity="0.55" />
            <stop offset="100%" stopColor="#78f2b0" stopOpacity="0" />
          </radialGradient>
          <radialGradient id="grad-crit" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#ffe5e5" stopOpacity="0.95" />
            <stop offset="60%" stopColor="#ff6b6b" stopOpacity="0.62" />
            <stop offset="100%" stopColor="#ff6b6b" stopOpacity="0" />
          </radialGradient>
          <linearGradient id="edge-grad" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#78f2b0" stopOpacity="0" />
            <stop offset="50%" stopColor="#78f2b0" stopOpacity="0.82" />
            <stop offset="100%" stopColor="#78f2b0" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="edge-grad-cyan" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#78f2b0" stopOpacity="0" />
            <stop offset="50%" stopColor="#78f2b0" stopOpacity="0.85" />
            <stop offset="100%" stopColor="#78f2b0" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="edge-grad-amber" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#f2c478" stopOpacity="0" />
            <stop offset="50%" stopColor="#f2c478" stopOpacity="0.85" />
            <stop offset="100%" stopColor="#f2c478" stopOpacity="0" />
          </linearGradient>
          <linearGradient id="edge-grad-crit" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#ff9b9b" stopOpacity="0" />
            <stop offset="50%" stopColor="#ff9b9b" stopOpacity="0.9" />
            <stop offset="100%" stopColor="#ff9b9b" stopOpacity="0" />
          </linearGradient>
          <filter id="glow" x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="2.4" result="blur" />
            <feMerge>
              <feMergeNode in="blur" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
        </defs>

        {/* Background guide ellipses */}
        {[
          { rx: 150, ry: 62 },
          { rx: 270, ry: 112 },
          { rx: 390, ry: 160 },
        ].map((ring) => (
          <ellipse
            key={`guide-${ring.rx}`}
            cx={CX}
            cy={CY}
            rx={ring.rx}
            ry={ring.ry}
            fill="none"
            stroke={coreTone === 'crit' ? 'rgba(255,107,107,.08)' : 'rgba(120,242,176,.065)'}
            strokeWidth={1}
            strokeDasharray="2 6"
          />
        ))}

        {/* Outer cross-links (subtle, along same radius arc) */}
        {OUTER_ANGLES.map((a, i) => {
          const next = OUTER_ANGLES[(i + 1) % OUTER_ANGLES.length];
          const p1 = polar(OUTER_RX, OUTER_RY, a);
          const p2 = polar(OUTER_RX, OUTER_RY, next);
          return (
            <path
              key={`cross-${i}`}
              d={`M ${p1.x.toFixed(2)} ${p1.y.toFixed(2)} A ${OUTER_RX} ${OUTER_RY} 0 0 1 ${p2.x.toFixed(2)} ${p2.y.toFixed(2)}`}
              fill="none"
              stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.24)' : 'rgba(120,242,176,0.18)'}
              strokeWidth={1}
              strokeDasharray="3 5"
            />
          );
        })}

        {/* Outer edges core -> outer node */}
        {outer.map((n) => {
          const pos = NEURAL_NODE_POSITIONS[n.key];
          return (
            <path
              key={`edge-${n.key}`}
              className="nw-edge"
              d={`M ${CX} ${CY} L ${pos.x.toFixed(2)} ${pos.y.toFixed(2)}`}
              fill="none"
              stroke={edgeStroke(n.tone)}
              strokeWidth={1.4}
            />
          );
        })}

        {/* Inner edges core -> inner node (slow, soft healthy tone) */}
        {inner.map((n) => {
          const pos = NEURAL_NODE_POSITIONS[n.key];
          return (
            <path
              key={`iedge-${n.key}`}
              className="nw-edge-slow"
              d={`M ${CX} ${CY} L ${pos.x.toFixed(2)} ${pos.y.toFixed(2)}`}
              fill="none"
              stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.50)' : 'rgba(120,242,176,0.44)'}
              strokeOpacity={0.4}
              strokeWidth={1}
            />
          );
        })}

        {/* Inner nodes */}
        {inner.map((n) => {
          const pos = NEURAL_NODE_POSITIONS[n.key];
          return (
            <g
              key={`inode-${n.key}`}
              className="nw-node-hover"
              role="button"
              tabIndex={0}
              aria-label={`${n.label} — show details`}
              aria-expanded={hovered === n.navKey}
              data-nw-nav={n.navKey}
              onMouseEnter={handleEnter(n.navKey)}
              onMouseLeave={handleLeave}
              onFocus={handleEnter(n.navKey)}
              onBlur={handleLeave}
              onClick={handleActivate(n.navKey)}
              onKeyDown={handleKey(n.navKey)}
            >
              <circle
                cx={pos.x}
                cy={pos.y}
                r={18}
                fill={nodeFill(n.tone)}
                opacity={0.7}
                filter="url(#glow)"
              />
              <circle
                cx={pos.x}
                cy={pos.y}
                r={6}
                fill={nodeStrokeColor(n.tone)}
                opacity={0.95}
              />
              <text
                x={pos.x}
                y={pos.y + labelOffsetY(pos.angle)}
                textAnchor="middle"
                fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace"
                fontSize={10.5}
                fill={labelFill(n.tone)}
                letterSpacing={1.1}
                opacity={0.85}
              >
                {n.label}
              </text>
            </g>
          );
        })}

        {/* Outer nodes */}
        {outer.map((n) => {
          const pos = NEURAL_NODE_POSITIONS[n.key];
          return (
            <g
              key={`onode-${n.key}`}
              className="nw-node-hover"
              role="button"
              tabIndex={0}
              aria-label={`${n.label} — show details`}
              aria-expanded={hovered === n.navKey}
              data-nw-nav={n.navKey}
              onMouseEnter={handleEnter(n.navKey)}
              onMouseLeave={handleLeave}
              onFocus={handleEnter(n.navKey)}
              onBlur={handleLeave}
              onClick={handleActivate(n.navKey)}
              onKeyDown={handleKey(n.navKey)}
            >
              {/* Invisible hit target for easier hovering */}
              <circle
                cx={pos.x}
                cy={pos.y}
                r={38}
                fill="transparent"
                pointerEvents="all"
              />
              <circle
                cx={pos.x}
                cy={pos.y}
                r={28}
                fill={nodeFill(n.tone)}
                opacity={0.85}
                filter="url(#glow)"
              />
              <circle
                cx={pos.x}
                cy={pos.y}
                r={10}
                fill={coreTone === 'crit' ? 'rgba(28,8,12,0.88)' : 'rgba(8,14,18,0.90)'}
                stroke={nodeStrokeColor(n.tone)}
                strokeWidth={1.4}
              />
              <circle
                cx={pos.x}
                cy={pos.y}
                r={3}
                fill={nodeStrokeColor(n.tone)}
              />
              <text
                x={pos.x}
                y={pos.y + labelOffsetY(pos.angle)}
                textAnchor="middle"
                fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace"
                fontSize={11}
                fill={labelFill(n.tone)}
                letterSpacing={1}
              >
                {n.label}
              </text>
            </g>
          );
        })}

        {/* Core */}
        <g
          className="nw-node-hover"
          role="button"
          tabIndex={0}
          aria-label="Ark Core — show details"
          aria-expanded={hovered === 'core'}
          data-nw-nav="core"
          onMouseEnter={handleEnter('core')}
          onMouseLeave={handleLeave}
          onFocus={handleEnter('core')}
          onBlur={handleLeave}
          onClick={handleActivate('core')}
          onKeyDown={handleKey('core')}
        >
          {/* Invisible hit target around core for easier hovering */}
          <circle cx={CX} cy={CY} r={84} fill="transparent" pointerEvents="all" />
          <circle
            className="nw-core-glow"
            cx={CX}
            cy={CY}
            r={74}
            fill={coreGradient}
            opacity={0.55}
          />
          <circle cx={CX} cy={CY} r={50} fill={coreGradient} opacity={0.7} />
          <circle cx={CX} cy={CY} r={32} fill={coreGradient} opacity={0.85} />
          <circle
            cx={CX}
            cy={CY}
            r={18}
            fill={coreTone === 'crit' ? '#ffb9b9' : '#e7fff0'}
            opacity={0.95}
          />
          <circle
            className="nw-core-ring-1"
            cx={CX}
            cy={CY}
            r={60}
            fill="none"
            stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.72)' : 'rgba(120,242,176,0.68)'}
            strokeWidth={1}
          />
          <circle
            className="nw-core-ring-2"
            cx={CX}
            cy={CY}
            r={88}
            fill="none"
            stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.48)' : 'rgba(120,242,176,0.42)'}
            strokeWidth={1}
          />
          <path
            d={hexPath(CX, CY, 28)}
            fill="none"
            stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.92)' : 'rgba(120,242,176,0.88)'}
            strokeWidth={1.2}
          />
          {/* Crosshair ticks */}
          <line x1={CX - 110} y1={CY} x2={CX - 100} y2={CY} stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.58)' : 'rgba(120,242,176,0.55)'} strokeWidth={1} />
          <line x1={CX + 100} y1={CY} x2={CX + 110} y2={CY} stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.58)' : 'rgba(120,242,176,0.55)'} strokeWidth={1} />
          <line x1={CX} y1={CY - 110} x2={CX} y2={CY - 100} stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.58)' : 'rgba(120,242,176,0.55)'} strokeWidth={1} />
          <line x1={CX} y1={CY + 100} x2={CX} y2={CY + 110} stroke={coreTone === 'crit' ? 'rgba(255,107,107,0.58)' : 'rgba(120,242,176,0.55)'} strokeWidth={1} />
          <text
            x={CX}
            y={CY + 4}
            textAnchor="middle"
            fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace"
            fontSize={13}
            fill={coreTone === 'crit' ? '#ffc8c8' : 'var(--nw-green, #78f2b0)'}
            letterSpacing={2}
          >
            ARKCORE
          </text>
          <text
            x={CX}
            y={CY + 22}
            textAnchor="middle"
            fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace"
            fontSize={10}
            fill={coreTone === 'crit' ? '#ffc8c8' : '#dff7e7'}
            letterSpacing={1.2}
          >
            {coreLine2}
          </text>
        </g>
      </svg>

      {hovered ? (() => {
        const posKey = NAV_KEY_TO_POSITION[hovered];
        const pos = NEURAL_NODE_POSITIONS[posKey];
        if (!pos) return null;
        const data = tooltipFor(hovered, props);
        const tooltipTone = forceCritical ? 'crit' : data.tone;
        const t = tooltipTransform(pos.x, pos.y);
        return (
          <div
            className={`nw-tooltip nw-tooltip--${tooltipTone}`}
            role="tooltip"
            style={{
              left: `${(pos.x / VIEW_W) * 100}%`,
              top: `${(pos.y / VIEW_H) * 100}%`,
              transform: `translate(${t.tx}, ${t.ty})`,
            }}
          >
            <div className="nw-tooltip-title">{data.title}</div>
            <div className="nw-tooltip-value">{data.value}</div>
            <div className="nw-tooltip-detail">{data.detail}</div>
            {data.meta ? <div className="nw-tooltip-meta">{data.meta}</div> : null}
          </div>
        );
      })() : null}
    </div>
  );
}

export default NeuralWebGraph;
