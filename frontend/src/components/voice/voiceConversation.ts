export type VoiceConversationPhase =
  | "idle"
  | "requesting_permission"
  | "listening"
  | "user_speaking"
  | "thinking"
  | "speaking"
  | "muted"
  | "error";

export type VoiceMascotMood =
  | "idle"
  | "listening"
  | "thinking"
  | "speaking"
  | "muted"
  | "error";

export function shouldSubmitVoiceTranscript(
  transcript: string,
  turnInFlight: boolean,
): boolean {
  return transcript.trim().length > 0 && !turnInFlight;
}

export function voiceMascotMood({
  phase,
  muted,
}: {
  phase: VoiceConversationPhase;
  muted: boolean;
}): VoiceMascotMood {
  if (muted) return "muted";
  switch (phase) {
    case "requesting_permission":
    case "listening":
    case "user_speaking":
      return "listening";
    case "thinking":
      return "thinking";
    case "speaking":
      return "speaking";
    case "error":
      return "error";
    case "muted":
      return "muted";
    case "idle":
    default:
      return "idle";
  }
}
