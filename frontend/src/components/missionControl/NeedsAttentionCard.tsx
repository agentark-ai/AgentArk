import { useMemo } from "react";
import { NeuralPanel } from "./NeuralPanel";
import { buildAttentionItems } from "../NeedsAttentionInbox";
import type { AttentionItem } from "../NeedsAttentionInbox";
import type { Notification, Task } from "../../types";

type SecurityLog = {
  event_type: string;
  severity: string;
  message: string;
  source?: string;
  created_at?: string;
};

export type NeedsAttentionCardProps = {
  tasks: Task[];
  notifications: Notification[];
  securityLogs: SecurityLog[];
  settingsLoaded: boolean;
  hasLlmConfigured: boolean;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  onRetry: (id: string) => void;
  onNavigate: (view: string) => void;
  approving: boolean;
  rejecting: boolean;
  retrying: boolean;
};

function kindLabel(kind: AttentionItem["kind"]): string {
  switch (kind) {
    case "approval":
      return "APPROVAL";
    case "input":
      return "INPUT";
    case "failed":
      return "FAILURE";
    case "security":
      return "ALERT";
    case "setup":
      return "SETUP";
    default:
      return "ALERT";
  }
}

function iconToneClsForKind(kind: AttentionItem["kind"]): string {
  if (kind === "failed") return "nw-activity-ic nw-activity-ic--crit";
  return "nw-activity-ic nw-activity-ic--warn";
}

export function NeedsAttentionCard({
  tasks,
  notifications,
  securityLogs,
  settingsLoaded,
  hasLlmConfigured,
  onApprove,
  onReject,
  onRetry,
  onNavigate,
  approving,
  rejecting,
  retrying,
}: NeedsAttentionCardProps) {
  const items = useMemo(
    () =>
      buildAttentionItems(
        tasks,
        notifications,
        securityLogs,
        settingsLoaded,
        hasLlmConfigured
      ),
    [tasks, notifications, securityLogs, settingsLoaded, hasLlmConfigured]
  );

  const waitingCount = useMemo(
    () =>
      items.filter((it) => it.kind === "approval" || it.kind === "input").length,
    [items]
  );
  const failedCount = useMemo(
    () => items.filter((it) => it.kind === "failed").length,
    [items]
  );
  const unreadAlerts = useMemo(
    () => items.filter((it) => it.kind === "security").length,
    [items]
  );

  const setupItem = useMemo(
    () => items.find((it) => it.kind === "setup"),
    [items]
  );
  const actionableItems = useMemo(
    () => items.filter((it) => it.kind !== "setup"),
    [items]
  );

  const count = items.length;
  const urgentCount = waitingCount + failedCount + unreadAlerts;
  const setupOnly = Boolean(setupItem && actionableItems.length === 0);
  const tag = urgentCount > 0 ? `! ${count}` : setupItem ? "SETUP" : "CLEAR";
  const tagTone = urgentCount > 0 ? "warn" : setupItem ? "cyan" : "good";

  function renderActionButtonsForKind(item: AttentionItem) {
    if (item.kind === "approval") {
      return (
        <div className="nw-actions" style={{ marginTop: 6 }}>
          <button
            className="nw-btn nw-btn--small nw-btn--primary"
            disabled={approving}
            onClick={() => onApprove(item.id)}
          >
            Approve
          </button>
          <button
            className="nw-btn nw-btn--small nw-btn--ghost"
            disabled={rejecting}
            onClick={() => onReject(item.id)}
          >
            Reject
          </button>
        </div>
      );
    }
    if (item.kind === "failed") {
      return (
        <div className="nw-actions" style={{ marginTop: 6 }}>
          <button
            className="nw-btn nw-btn--small"
            disabled={retrying}
            onClick={() => onRetry(item.id)}
          >
            Retry
          </button>
        </div>
      );
    }
    if (item.kind === "input" || item.kind === "security") {
      return (
        <div className="nw-actions" style={{ marginTop: 6 }}>
          <button
            className="nw-btn nw-btn--small"
            onClick={() => onNavigate(item.targetView ?? "settings")}
          >
            View <span className="nw-arrow">-&gt;</span>
          </button>
        </div>
      );
    }
    return null;
  }

  return (
    <NeuralPanel
      title="Needs Attention"
      tag={tag}
      tagTone={tagTone}
      alert={urgentCount > 0}
      className={`nw-panel--attention${setupOnly ? " nw-panel--attention-setup" : ""}`}
      bodyClassName="nw-attention-body"
      dataTourTarget="overview-attention"
    >
      <div className="nw-attention-counts">
        <div className="nw-alert-row">
          <span className="nw-alert-name">Waiting</span>
          <span
            className={
              waitingCount === 0
                ? "nw-alert-val nw-alert-val--zero"
                : "nw-alert-val nw-alert-val--warn"
            }
          >
            {waitingCount}
          </span>
        </div>
        <div className="nw-alert-row">
          <span className="nw-alert-name">Failed</span>
          <span
            className={
              failedCount === 0
                ? "nw-alert-val nw-alert-val--zero"
                : "nw-alert-val nw-alert-val--crit"
            }
          >
            {failedCount}
          </span>
        </div>
        <div className="nw-alert-row">
          <span className="nw-alert-name">Unread alerts</span>
          <span
            className={
              unreadAlerts === 0
                ? "nw-alert-val nw-alert-val--zero"
                : "nw-alert-val nw-alert-val--warn"
            }
          >
            {unreadAlerts}
          </span>
        </div>
      </div>

      {setupItem ? (
        <div className="nw-setup-card">
          <div className="nw-activity-ic nw-activity-ic--cyan">!</div>
          <div className="nw-setup-body">
            <div className="nw-setup-title">{setupItem.title}</div>
            {setupItem.detail ? (
              <div className="nw-setup-desc">{setupItem.detail}</div>
            ) : null}
            <button
              className="nw-btn nw-btn--small"
              style={{ marginTop: 8 }}
              onClick={() => onNavigate("settings")}
            >
              Set up <span className="nw-arrow">-&gt;</span>
            </button>
          </div>
        </div>
      ) : null}

      {actionableItems.slice(0, 3).map((item) => (
        <div className="nw-activity-row" key={item.id}>
          <div className={iconToneClsForKind(item.kind)}>.</div>
          <div className="nw-activity-meta">
            <div className="nw-activity-ts">{kindLabel(item.kind)}</div>
            <div className="nw-activity-txt">{item.title}</div>
            {item.detail ? (
              <div
                className="nw-panel-muted"
                style={{ fontSize: 11, marginTop: 3 }}
              >
                {item.detail}
              </div>
            ) : null}
            {renderActionButtonsForKind(item)}
          </div>
        </div>
      ))}

      {count === 0 ? (
        <div className="nw-panel-muted" style={{ paddingTop: 8 }}>
          Operator queue is clear.
        </div>
      ) : null}

      {waitingCount > 0 || failedCount > 0 ? (
        <div className="nw-actions" style={{ marginTop: 8 }}>
          <button className="nw-btn" onClick={() => onNavigate("tasks")}>
            Open task queue <span className="nw-arrow">-&gt;</span>
          </button>
        </div>
      ) : null}
    </NeuralPanel>
  );
}
