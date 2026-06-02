export type ChatLiveRunArchiveSnapshot = {
  conversationId: string;
  message?: string;
  startedAt: number;
  initialMessageCount?: number;
  runId?: string;
  mode?: string;
  phase?: string;
  taskId?: string;
  streamingResponse?: string;
  streamingSteps?: unknown[];
  failedUserMessage?: string;
  lastRunSeq?: number;
  attachments?: unknown[];
};

export type ChatLiveRunWorkspaceSnapshot = {
  conversationId: string;
  updatedAt: number;
  deployedFiles: unknown[];
  liveFileWrites: Record<string, unknown>;
  streamedWorkspaceApp: Record<string, unknown> | null;
  codeViewerFileIdx: number;
};

export type ChatLiveRunArchiveInput = {
  pendingSnapshot: ChatLiveRunArchiveSnapshot | null;
  conversationId?: string | null;
  taskId?: string | null;
  streamingResponse?: string | null;
  streamingSteps?: unknown[] | null;
  deployedFiles?: unknown[] | null;
  liveFileWrites?: Record<string, unknown> | null;
  streamedWorkspaceApp?: Record<string, unknown> | null;
  codeViewerFileIdx?: number | null;
  nowMs: number;
  maxResponseChars: number;
};

export type ChatLiveRunArchiveResult = {
  pendingSnapshot: ChatLiveRunArchiveSnapshot | null;
  workspaceSnapshot: ChatLiveRunWorkspaceSnapshot | null;
};

export function buildChatLiveRunArchive(
  input: ChatLiveRunArchiveInput,
): ChatLiveRunArchiveResult {
  const conversationId = (
    input.pendingSnapshot?.conversationId ||
    input.conversationId ||
    ""
  ).trim();
  if (!conversationId) {
    return { pendingSnapshot: null, workspaceSnapshot: null };
  }

  const response = (
    input.streamingResponse ||
    input.pendingSnapshot?.streamingResponse ||
    ""
  ).slice(0, Math.max(0, input.maxResponseChars));
  const steps =
    input.streamingSteps && input.streamingSteps.length > 0
      ? input.streamingSteps
      : input.pendingSnapshot?.streamingSteps || [];

  const pendingSnapshot: ChatLiveRunArchiveSnapshot = {
    ...(input.pendingSnapshot || {
      conversationId,
      startedAt: input.nowMs,
    }),
    conversationId,
    taskId:
      (input.taskId || "").trim() ||
      input.pendingSnapshot?.taskId ||
      "",
    streamingResponse: response,
    streamingSteps: steps,
  };

  const deployedFiles = input.deployedFiles || [];
  const liveFileWrites = input.liveFileWrites || {};
  const streamedWorkspaceApp = input.streamedWorkspaceApp || null;
  const hasWorkspaceState =
    deployedFiles.length > 0 ||
    Object.keys(liveFileWrites).length > 0 ||
    streamedWorkspaceApp !== null;

  return {
    pendingSnapshot,
    workspaceSnapshot: hasWorkspaceState
      ? {
          conversationId,
          updatedAt: input.nowMs,
          deployedFiles,
          liveFileWrites,
          streamedWorkspaceApp,
          codeViewerFileIdx: Math.max(
            0,
            Math.floor(input.codeViewerFileIdx || 0),
          ),
        }
      : null,
  };
}
