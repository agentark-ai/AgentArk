export type VoiceSessionPhase =
  | "connecting"
  | "listening"
  | "thinking"
  | "speaking"
  | "muted"
  | "error"
  | "stopped";

export type VoiceSessionSnapshot = {
  id?: string | null;
  phase?: VoiceSessionPhase | string | null;
  last_error?: string | null;
};

export type VoiceStatusSnapshot = {
  voice_available?: boolean;
  status?: string | null;
  session?: VoiceSessionSnapshot | null;
  disabled_reason?: string | null;
};

export type VoiceControlState = {
  kind:
    | "setup_needed"
    | "ready"
    | "active"
    | "connecting"
    | "listening"
    | "thinking"
    | "speaking"
    | "muted"
    | "error";
  canStart: boolean;
  canStop: boolean;
  label: string;
};

const ACTIVE_PHASE_LABELS: Record<string, Pick<VoiceControlState, "kind" | "label">> = {
  connecting: { kind: "connecting", label: "Connecting" },
  listening: { kind: "listening", label: "Listening" },
  thinking: { kind: "thinking", label: "Thinking" },
  speaking: { kind: "speaking", label: "Speaking" },
  muted: { kind: "muted", label: "Muted" },
  error: { kind: "error", label: "Voice issue" },
};

export function voiceControlStateFromSession(
  snapshot: VoiceStatusSnapshot | null | undefined,
): VoiceControlState {
  const session = snapshot?.session;
  const phase = String(session?.phase || "").trim().toLowerCase();
  const activeSession = Boolean(session && phase !== "stopped");
  if (activeSession) {
    const active = ACTIVE_PHASE_LABELS[phase] ?? { kind: "active", label: "Voice active" };
    return {
      kind: active.kind,
      canStart: false,
      canStop: true,
      label: active.label,
    };
  }

  if (!snapshot?.voice_available) {
    return {
      kind: "setup_needed",
      canStart: false,
      canStop: false,
      label: "Voice setup",
    };
  }

  return {
    kind: "ready",
    canStart: true,
    canStop: false,
    label: "Start voice",
  };
}
