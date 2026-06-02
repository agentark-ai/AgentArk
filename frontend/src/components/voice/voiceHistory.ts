export type VoiceTurnRecord = {
  id: string;
  role: "user" | "assistant";
  content: string;
  timestamp: string;
};

export const VOICE_CONVERSATION_STORAGE_KEY = "agentark.voice.conversation_id";

type StorageLike = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function messageRecords(payload: unknown): Record<string, unknown>[] {
  const root = asRecord(payload);
  const messages = Array.isArray(payload) ? payload : root.messages;
  if (!Array.isArray(messages)) return [];
  return messages.map(asRecord).filter((message) => Object.keys(message).length > 0);
}

export function voiceTurnsFromConversationMessages(payload: unknown): VoiceTurnRecord[] {
  return messageRecords(payload).flatMap((message, index) => {
    const role = text(message.role).toLowerCase();
    if (role !== "user" && role !== "assistant") return [];
    const content = text(message.content);
    if (!content) return [];
    const id =
      text(message.id) ||
      text(message.trace_id) ||
      `${role}:${text(message.timestamp) || "message"}:${index}`;
    return [
      {
        id,
        role,
        content,
        timestamp: text(message.timestamp),
      },
    ];
  });
}

export function loadPersistedVoiceConversationId(
  storage: Pick<Storage, "getItem"> | null | undefined,
): string | null {
  try {
    const id = storage?.getItem(VOICE_CONVERSATION_STORAGE_KEY)?.trim() || "";
    return id || null;
  } catch {
    return null;
  }
}

export function persistVoiceConversationId(
  storage: StorageLike | null | undefined,
  conversationId: string | null | undefined,
): void {
  try {
    const id = conversationId?.trim() || "";
    if (id) {
      storage?.setItem(VOICE_CONVERSATION_STORAGE_KEY, id);
    } else {
      storage?.removeItem(VOICE_CONVERSATION_STORAGE_KEY);
    }
  } catch {
    // Browser privacy modes can block localStorage; saved server history still loads when a session supplies the id.
  }
}
