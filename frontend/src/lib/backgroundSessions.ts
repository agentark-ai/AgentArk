import type { BackgroundSessionSummary, Task } from "../types";

export function taskKind(task: Task | null | undefined): string {
  if (!task) return "task";
  const explicit = String(task.task_kind || "").trim().toLowerCase();
  if (explicit) return explicit;
  const action = String(task.action || "").trim().toLowerCase();
  const cron = String(task.cron || "").trim();
  const scheduledFor = String(task.scheduled_for || "").trim();
  if (action === "notify_user" && (cron.length > 0 || scheduledFor.length > 0)) return "reminder";
  if (action === "chat_request") return "chat_request";
  if (action === "goal") return "goal";
  return "task";
}

export function taskKindLabel(task: Task | null | undefined): string {
  if (!task) return "Task";
  const explicit = String(task.task_kind_label || "").trim();
  if (explicit) return explicit;
  const kind = taskKind(task);
  if (kind === "reminder") return "Reminder";
  if (kind === "chat_request") return "Chat Task";
  if (kind === "goal") return "Goal";
  return "Task";
}

export function taskActionDisplay(task: Task | null | undefined): string {
  if (!task) return "Task";
  return taskKind(task) === "reminder"
    ? taskKindLabel(task)
    : String(task.action || "").trim() || taskKindLabel(task);
}

export function isOneShotReminderTask(task: Task | null | undefined): boolean {
  if (!task) return false;
  const cron = String(task.cron || "").trim();
  const scheduledFor = String(task.scheduled_for || "").trim();
  return taskKind(task) === "reminder" && !cron && scheduledFor.length > 0;
}

export function isSystemManagedTask(task: Task | null | undefined): boolean {
  if (!task) return false;
  const action = String(task.action || "").trim().toLowerCase();
  return action === "daily_brief";
}

export function isForegroundChatTask(task: Task | null | undefined): boolean {
  if (!task) return false;
  return taskKind(task) === "chat_request";
}

export function isTerminalTask(task: Task | null | undefined): boolean {
  if (!task) return false;
  const status = String(task.status || "").trim().toLowerCase();
  if (!status) return false;
  return (
    status === "completed" ||
    status === "cancelled" ||
    status === "canceled" ||
    status.startsWith("failed")
  );
}

export function isStandaloneBackgroundWorkTask(task: Task | null | undefined): boolean {
  if (!task) return false;
  if (isSystemManagedTask(task)) return false;
  if (isForegroundChatTask(task)) return false;
  if (isTerminalTask(task)) return false;
  return true;
}

export function isOneShotReminderSession(session: BackgroundSessionSummary): boolean {
  return String(session.ui_kind || "").trim().toLowerCase() === "one_shot_reminder";
}

export function isChatContextSession(session: BackgroundSessionSummary): boolean {
  return String(session.ui_kind || "").trim().toLowerCase() === "chat_context";
}

export function backgroundSessionLinkedWorkCount(session: BackgroundSessionSummary): number {
  const counts = session.counts;
  const countedTasks = Number(counts?.tasks_total || 0);
  const countedWatchers = Number(counts?.watchers_total || 0);
  const linkedTasks = Array.isArray(session.linked_task_ids) ? session.linked_task_ids.length : 0;
  const linkedWatchers = Array.isArray(session.linked_watcher_ids) ? session.linked_watcher_ids.length : 0;
  return countedTasks + countedWatchers + linkedTasks + linkedWatchers;
}

export function hasBackgroundSessionLinkedWork(session: BackgroundSessionSummary): boolean {
  return backgroundSessionLinkedWorkCount(session) > 0;
}

export function isBackgroundSessionVisibleInUi(session: BackgroundSessionSummary): boolean {
  if (session.default_visible === false) return false;
  if (isOneShotReminderSession(session)) return false;
  if (isChatContextSession(session)) return false;
  return hasBackgroundSessionLinkedWork(session);
}
