import type { VoiceConversationPhase } from "./voiceConversation";

type VoiceStreamScope = {
  navigator?: {
    mediaDevices?: {
      getUserMedia?: unknown;
    };
  };
  MediaRecorder?: unknown;
};

type VoiceStreamLocation = {
  protocol: string;
  host: string;
  origin: string;
};

export type VoiceStreamEvent = {
  type?: unknown;
  [key: string]: unknown;
};

export type VoiceTurnCaptureRequest = "start" | "finish";
export type VoiceTurnCaptureAction = "start_turn_capture" | "finish_turn_capture";

export function voiceTurnCaptureAction({
  sessionActive,
  recording,
  busy,
  requested,
}: {
  sessionActive: boolean;
  recording: boolean;
  busy: boolean;
  requested: VoiceTurnCaptureRequest;
}): VoiceTurnCaptureAction | null {
  if (!sessionActive || busy) return null;
  if (requested === "start" && !recording) return "start_turn_capture";
  if (requested === "finish" && recording) return "finish_turn_capture";
  return null;
}

export function voiceStreamApiPath(sessionId: string, streamToken: string): string {
  const encodedSessionId = encodeURIComponent(sessionId);
  const params = new URLSearchParams({ stream_token: streamToken });
  return `/voice/sessions/${encodedSessionId}/stream?${params.toString()}`;
}

export function browserVoiceStreamSupport(
  scope: VoiceStreamScope | null | undefined,
): { available: boolean; reason?: string } {
  const getUserMedia = scope?.navigator?.mediaDevices?.getUserMedia;
  if (typeof getUserMedia !== "function") {
    return { available: false, reason: "media_devices_unavailable" };
  }
  if (typeof scope?.MediaRecorder !== "function") {
    return { available: false, reason: "media_recorder_unavailable" };
  }
  return { available: true };
}

export function voiceStreamSocketUrl({
  path,
  location,
}: {
  path: string;
  location: VoiceStreamLocation;
}): string {
  const url = /^https?:\/\//i.test(path)
    ? new URL(path)
    : new URL(path, location.origin);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export function selectVoiceStreamMimeType(
  recorderCtor: typeof MediaRecorder | undefined,
): string {
  const candidates = [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/ogg;codecs=opus",
    "audio/mp4",
  ];
  const isSupported = recorderCtor?.isTypeSupported;
  if (typeof isSupported !== "function") return "";
  return candidates.find((candidate) => isSupported.call(recorderCtor, candidate)) || "";
}

export function voiceStreamPhaseFromEvent(
  event: VoiceStreamEvent,
): VoiceConversationPhase | null {
  switch (event.type) {
    case "session.ready":
    case "session.listening":
      return "listening";
    case "agent.thinking":
      return "thinking";
    case "tts.audio":
      return "speaking";
    case "error":
      return "error";
    default:
      return null;
  }
}
