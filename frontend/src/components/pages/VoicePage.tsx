import {
  Alert,
  Box,
  Button,
  Chip,
  IconButton,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import CallEndRoundedIcon from "@mui/icons-material/CallEndRounded";
import ChatRoundedIcon from "@mui/icons-material/ChatRounded";
import GraphicEqRoundedIcon from "@mui/icons-material/GraphicEqRounded";
import MicRoundedIcon from "@mui/icons-material/MicRounded";
import PlayArrowRoundedIcon from "@mui/icons-material/PlayArrowRounded";
import SendRoundedIcon from "@mui/icons-material/SendRounded";
import StopCircleRoundedIcon from "@mui/icons-material/StopCircleRounded";
import VolumeUpRoundedIcon from "@mui/icons-material/VolumeUpRounded";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  apiUrl,
  type VoiceSessionSnapshot,
  type VoiceStatusResponse,
} from "../../api/client";
import AgentLogo from "../../assets/logo.svg";
import {
  shouldSubmitVoiceTranscript,
  voiceMascotMood,
  type VoiceConversationPhase,
} from "../voice/voiceConversation";
import {
  loadPersistedVoiceConversationId,
  persistVoiceConversationId,
  voiceTurnsFromConversationMessages,
  type VoiceTurnRecord,
} from "../voice/voiceHistory";
import {
  browserVoiceStreamSupport,
  selectVoiceStreamMimeType,
  voiceStreamApiPath,
  voiceStreamPhaseFromEvent,
  voiceStreamSocketUrl,
  voiceTurnCaptureAction,
  type VoiceStreamEvent,
} from "../voice/voiceStream";

type VoicePageProps = {
  autoRefresh: boolean;
  onNavigateToView?: (view: string, replace?: boolean) => void;
};

function errorText(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  if (typeof error === "string" && error.trim()) return error.trim();
  return "Voice chat hit an unexpected problem.";
}

function phaseLabel(phase: VoiceConversationPhase): string {
  switch (phase) {
    case "requesting_permission":
      return "Requesting mic";
    case "listening":
      return "Ready";
    case "user_speaking":
      return "Recording";
    case "thinking":
      return "Thinking";
    case "speaking":
      return "Speaking";
    case "muted":
      return "Muted";
    case "error":
      return "Needs attention";
    case "idle":
    default:
      return "Ready";
  }
}

function parseStreamEvent(raw: string): VoiceStreamEvent | null {
  try {
    const parsed = JSON.parse(raw) as VoiceStreamEvent;
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function textField(event: VoiceStreamEvent, key: string): string {
  const value = event[key];
  return typeof value === "string" ? value.trim() : "";
}

function voiceReadinessMessage(status: VoiceStatusResponse | null): string {
  const messages =
    status?.setup_errors
      ?.map((setupError) => setupError.message?.trim() || "")
      .filter(Boolean) || [];
  if (messages.length > 0) return messages.join(" ");
  if (status?.disabled_reason === "voice_assets_missing") {
    return "Local voice assets are not installed. Voice is planned as a future opt-in capability.";
  }
  if (status?.disabled_reason === "voice_bridge_unavailable") {
    return "Local voice backend is not running. Voice is planned as a future opt-in capability.";
  }
  if (status?.disabled_reason === "voice_not_enabled") {
    return "Voice is not enabled in this build. Two-way local voice is planned as a future opt-in capability.";
  }
  return "Local voice is not enabled on this machine.";
}

function decodeBase64Audio(audio: string, mimeType: string): string {
  const binary = window.atob(audio);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  const blob = new Blob([bytes], { type: mimeType || "audio/wav" });
  return URL.createObjectURL(blob);
}

function browserStorage(): Storage | null {
  return typeof window === "undefined" ? null : window.localStorage;
}

export default function VoicePage({ autoRefresh, onNavigateToView }: VoicePageProps) {
  const [status, setStatus] = useState<VoiceStatusResponse | null>(null);
  const [session, setSession] = useState<VoiceSessionSnapshot | null>(null);
  const [conversationId, setConversationId] = useState<string | null>(() =>
    loadPersistedVoiceConversationId(browserStorage()),
  );
  const [phase, setPhase] = useState<VoiceConversationPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [liveCaption, setLiveCaption] = useState("");
  const [pendingTranscript, setPendingTranscript] = useState("");
  const [recording, setRecording] = useState(false);
  const [streamConnected, setStreamConnected] = useState(false);
  const [turns, setTurns] = useState<VoiceTurnRecord[]>([]);
  const [turnInFlight, setTurnInFlight] = useState(false);

  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const playbackRef = useRef<HTMLAudioElement | null>(null);
  const playbackUrlRef = useRef("");
  const sessionIdRef = useRef("");
  const recordingRef = useRef(false);
  const phaseRef = useRef<VoiceConversationPhase>("idle");
  const endingRef = useRef(false);
  const recorderMimeTypeRef = useRef("");
  const recordedChunksRef = useRef<Blob[]>([]);

  const streamSupport = useMemo(
    () =>
      typeof window === "undefined"
        ? { available: false, reason: "window_unavailable" }
        : browserVoiceStreamSupport({
            navigator,
            MediaRecorder: window.MediaRecorder,
          }),
    [],
  );
  const voiceAvailable = Boolean(status?.voice_available);
  const sessionId = (session?.id || "").trim();
  const mascotMood = voiceMascotMood({ phase, muted: false });
  const transcriptDraft = pendingTranscript.trim();
  const streamBusy = turnInFlight || phase === "thinking" || phase === "speaking";

  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    recordingRef.current = recording;
  }, [recording]);

  const setVoicePhase = useCallback((nextPhase: VoiceConversationPhase) => {
    phaseRef.current = nextPhase;
    setPhase(nextPhase);
  }, []);

  const sendJson = useCallback((payload: Record<string, unknown>) => {
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(payload));
    return true;
  }, []);

  const stopPlayback = useCallback(() => {
    const playback = playbackRef.current;
    playbackRef.current = null;
    if (playback) {
      playback.pause();
      playback.src = "";
    }
    if (playbackUrlRef.current) {
      URL.revokeObjectURL(playbackUrlRef.current);
      playbackUrlRef.current = "";
    }
  }, []);

  const stopRecorder = useCallback(() => {
    const recorder = recorderRef.current;
    recorderRef.current = null;
    if (recorder && recorder.state !== "inactive") {
      try {
        recorder.stop();
      } catch {
        // The recorder may already be stopped by the browser.
      }
    }
    recordingRef.current = false;
    setRecording(false);
  }, []);

  const prepareRecorder = useCallback((): boolean => {
    if (typeof MediaRecorder === "undefined") {
      setVoicePhase("error");
      setError("Microphone streaming is unavailable in this browser.");
      return false;
    }
    recorderMimeTypeRef.current = selectVoiceStreamMimeType(MediaRecorder);
    return true;
  }, [setVoicePhase]);

  const startTurnRecorder = useCallback((stream: MediaStream): boolean => {
    if (!prepareRecorder()) return false;
    stopRecorder();
    recordedChunksRef.current = [];
    const mimeType = recorderMimeTypeRef.current;
    const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
    recorder.ondataavailable = (event) => {
      if (event.data.size > 0) recordedChunksRef.current.push(event.data);
    };
    recorder.onerror = () => {
      recordingRef.current = false;
      setRecording(false);
      setTurnInFlight(false);
      setVoicePhase("error");
      setError("Microphone recorder failed.");
    };
    recorder.onstop = () => {
      recordingRef.current = false;
      setRecording(false);
      if (endingRef.current) {
        recordedChunksRef.current = [];
        return;
      }
      const socket = socketRef.current;
      const chunks = recordedChunksRef.current;
      recordedChunksRef.current = [];
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        setVoicePhase("error");
        setError("Voice stream is not connected. Start voice again.");
        return;
      }
      if (chunks.length === 0) {
        setVoicePhase("listening");
        return;
      }
      const blob = new Blob(chunks, { type: mimeType || chunks[0]?.type || "audio/webm" });
      setTurnInFlight(true);
      setVoicePhase("thinking");
      void blob
        .arrayBuffer()
        .then((buffer) => {
          if (socket.readyState !== WebSocket.OPEN || endingRef.current) return;
          socket.send(
            JSON.stringify({
              type: "turn.start",
              mime_type: blob.type || recorderMimeTypeRef.current,
            }),
          );
          socket.send(buffer);
          socket.send(
            JSON.stringify({
              type: "turn.end",
              mime_type: blob.type || recorderMimeTypeRef.current,
            }),
          );
        })
        .catch((recordingError) => {
          setTurnInFlight(false);
          setVoicePhase("error");
          setError(errorText(recordingError));
        });
    };
    recorder.start();
    recorderRef.current = recorder;
    recordingRef.current = true;
    setRecording(true);
    setPendingTranscript("");
    setLiveCaption("");
    setVoicePhase("user_speaking");
    return true;
  }, [prepareRecorder, setVoicePhase, stopRecorder]);

  const stopMicStream = useCallback(() => {
    const stream = streamRef.current;
    streamRef.current = null;
    stream?.getTracks().forEach((track) => track.stop());
  }, []);

  const closeSocket = useCallback(() => {
    const socket = socketRef.current;
    socketRef.current = null;
    setStreamConnected(false);
    if (socket && socket.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ type: "session.stop" }));
    }
    socket?.close();
  }, []);

  const stopStreamingResources = useCallback(() => {
    endingRef.current = true;
    closeSocket();
    stopRecorder();
    stopPlayback();
    stopMicStream();
    setLiveCaption("");
    recordingRef.current = false;
    setRecording(false);
    setTurnInFlight(false);
  }, [closeSocket, stopMicStream, stopPlayback, stopRecorder]);

  const loadConversationTurns = useCallback(async (id: string) => {
    const trimmed = id.trim();
    if (!trimmed) return;
    const payload = await api.rawGet(
      `/conversations/${encodeURIComponent(trimmed)}/messages?limit=100`,
    );
    setTurns(voiceTurnsFromConversationMessages(payload));
  }, []);

  useEffect(() => {
    const id = conversationId?.trim() || "";
    persistVoiceConversationId(browserStorage(), id || null);
    if (!id) return undefined;
    let active = true;
    void api
      .rawGet(`/conversations/${encodeURIComponent(id)}/messages?limit=100`)
      .then((payload) => {
        if (active) setTurns(voiceTurnsFromConversationMessages(payload));
      })
      .catch((historyError) => {
        if (active) setError(errorText(historyError));
      });
    return () => {
      active = false;
    };
  }, [conversationId]);

  const playAudio = useCallback(
    (audio: string, mimeType: string) => {
      if (!audio) {
        if (!endingRef.current) setVoicePhase("listening");
        return;
      }
      stopPlayback();
      const url = decodeBase64Audio(audio, mimeType);
      playbackUrlRef.current = url;
      const playback = new Audio(url);
      playbackRef.current = playback;
      setVoicePhase("speaking");
      playback.onended = () => {
        stopPlayback();
        if (!endingRef.current) setVoicePhase("listening");
      };
      playback.onerror = () => {
        stopPlayback();
        if (!endingRef.current) setVoicePhase("listening");
      };
      void playback.play().catch((playError) => {
        setError(errorText(playError));
        if (!endingRef.current) setVoicePhase("listening");
      });
    },
    [setVoicePhase, stopPlayback],
  );

  const handleStreamEvent = useCallback(
    (event: VoiceStreamEvent) => {
      const nextPhase = voiceStreamPhaseFromEvent(event);
      if (
        nextPhase &&
        event.type !== "session.listening" &&
        !(nextPhase === "listening" && recordingRef.current)
      ) {
        setVoicePhase(nextPhase);
      }
      switch (event.type) {
        case "session.ready":
          setNotice(null);
          setError(null);
          break;
        case "session.listening":
          setTurnInFlight(false);
          if (!playbackRef.current && !pendingTranscript.trim() && !recordingRef.current) {
            setVoicePhase("listening");
          }
          break;
        case "transcript.final": {
          setTurnInFlight(false);
          const text = textField(event, "text");
          if (text) {
            setPendingTranscript(text);
            setLiveCaption(text);
          }
          setVoicePhase("listening");
          break;
        }
        case "tts.audio":
          playAudio(textField(event, "audio"), textField(event, "mime_type") || "audio/wav");
          break;
        case "error":
          setTurnInFlight(false);
          setError(textField(event, "message") || "Voice stream failed.");
          setVoicePhase("error");
          break;
        default:
          break;
      }
    },
    [pendingTranscript, playAudio, setVoicePhase],
  );

  const loadStatus = useCallback(async () => {
    const payload = await api.getVoiceStatus();
    setStatus(payload);
    const nextSession = payload.session || null;
    setSession(nextSession);
    sessionIdRef.current = nextSession?.id?.trim() || "";
    const statusConversationId = nextSession?.conversation_id?.trim() || "";
    if (statusConversationId) setConversationId(statusConversationId);
  }, []);

  useEffect(() => {
    void loadStatus().catch((loadError) => {
      setError(errorText(loadError));
    });
  }, [loadStatus]);

  useEffect(() => {
    if (!autoRefresh) return undefined;
    const timer = window.setInterval(() => {
      void loadStatus().catch(() => undefined);
    }, 8000);
    return () => window.clearInterval(timer);
  }, [autoRefresh, loadStatus]);

  const requestMic = useCallback(async (): Promise<MediaStream | null> => {
    if (streamRef.current?.active) return streamRef.current;
    if (!navigator.mediaDevices?.getUserMedia) {
      setVoicePhase("error");
      setError("This browser cannot access a microphone from this page.");
      return null;
    }
    setVoicePhase("requesting_permission");
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      stopMicStream();
      streamRef.current = stream;
      return stream;
    } catch (micError) {
      const blocked =
        micError instanceof DOMException &&
        (micError.name === "NotAllowedError" || micError.name === "SecurityError");
      setVoicePhase("error");
      setError(
        blocked
          ? "Microphone permission was denied. Allow microphone access for AgentArk and try again."
          : errorText(micError),
      );
      return null;
    }
  }, [setVoicePhase, stopMicStream]);

  const connectVoiceSocket = useCallback(
    (nextSessionId: string, streamToken: string): Promise<WebSocket> =>
      new Promise((resolve, reject) => {
        const path = apiUrl(voiceStreamApiPath(nextSessionId, streamToken));
        const socket = new WebSocket(
          voiceStreamSocketUrl({
            path,
            location: window.location,
          }),
        );
        socket.binaryType = "arraybuffer";
        socket.onopen = () => resolve(socket);
        socket.onerror = () => reject(new Error("Voice stream could not connect."));
      }),
    [],
  );

  const startVoice = useCallback(async () => {
    setError(null);
    setNotice(null);
    if (!voiceAvailable) {
      setVoicePhase("error");
      setError(voiceReadinessMessage(status));
      return;
    }
    if (!streamSupport.available) {
      setVoicePhase("error");
      setError("Microphone streaming is unavailable in this browser.");
      return;
    }
    try {
      stopStreamingResources();
      endingRef.current = false;
      const stream = await requestMic();
      if (!stream) return;
      if (!prepareRecorder()) {
        stopMicStream();
        return;
      }
      const created = await api.createVoiceSession({
        conversation_id: conversationId || undefined,
        transport: "browser_websocket",
      });
      if (created.status === "setup_needed" || created.voice_available === false) {
        stopMicStream();
        setVoicePhase("error");
        setError(voiceReadinessMessage(created as VoiceStatusResponse));
        return;
      }
      const nextSession = created.session || null;
      const nextSessionId = nextSession?.id?.trim() || "";
      const streamToken = (created.stream_token || "").trim();
      if (!nextSessionId) {
        stopMicStream();
        setVoicePhase("error");
        setError("Voice session could not be created.");
        return;
      }
      if (!streamToken) {
        stopMicStream();
        setVoicePhase("error");
        setError("Voice stream authorization was not issued for this session.");
        return;
      }
      const socket = await connectVoiceSocket(nextSessionId, streamToken);
      socketRef.current = socket;
      setStreamConnected(true);
      socket.onmessage = (event) => {
        if (typeof event.data === "string") {
          const streamEvent = parseStreamEvent(event.data);
          if (streamEvent) handleStreamEvent(streamEvent);
          return;
        }
        if (event.data instanceof Blob) {
          const url = URL.createObjectURL(event.data);
          playbackUrlRef.current = url;
          const playback = new Audio(url);
          playbackRef.current = playback;
          setVoicePhase("speaking");
          playback.onended = () => {
            stopPlayback();
            if (!endingRef.current) setVoicePhase("listening");
          };
          void playback.play().catch(() => {
            stopPlayback();
            if (!endingRef.current) setVoicePhase("listening");
          });
        }
      };
      socket.onclose = () => {
        setStreamConnected(false);
        if (!endingRef.current) {
          setVoicePhase("error");
          setError("Voice stream disconnected.");
        }
      };
      setSession(nextSession);
      sessionIdRef.current = nextSessionId;
      const nextConversationId = nextSession?.conversation_id?.trim() || "";
      if (nextConversationId) setConversationId(nextConversationId);
      sendJson({
        type: "session.start",
        mime_type: recorderMimeTypeRef.current,
        conversation_id: conversationId || null,
      });
      const action = voiceTurnCaptureAction({
        sessionActive: true,
        recording: false,
        busy: false,
        requested: "start",
      });
      if (action === "start_turn_capture") {
        startTurnRecorder(stream);
      } else {
        setVoicePhase("listening");
      }
    } catch (startError) {
      stopStreamingResources();
      setVoicePhase("error");
      setError(errorText(startError));
    }
  }, [
    connectVoiceSocket,
    conversationId,
    handleStreamEvent,
    prepareRecorder,
    requestMic,
    sendJson,
    setVoicePhase,
    startTurnRecorder,
    stopMicStream,
    stopPlayback,
    stopStreamingResources,
    streamSupport.available,
    status,
    voiceAvailable,
  ]);

  const beginTurnCapture = useCallback(async () => {
    const action = voiceTurnCaptureAction({
      sessionActive: streamConnected && Boolean(sessionIdRef.current),
      recording,
      busy: streamBusy || Boolean(transcriptDraft),
      requested: "start",
    });
    if (action !== "start_turn_capture") return;
    setError(null);
    setNotice(null);
    const stream = await requestMic();
    if (!stream) return;
    stopPlayback();
    startTurnRecorder(stream);
  }, [
    recording,
    requestMic,
    startTurnRecorder,
    stopPlayback,
    streamBusy,
    streamConnected,
    transcriptDraft,
  ]);

  const finishTurnCapture = useCallback(() => {
    const action = voiceTurnCaptureAction({
      sessionActive: streamConnected && Boolean(sessionIdRef.current),
      recording,
      busy: false,
      requested: "finish",
    });
    if (action === "finish_turn_capture") stopRecorder();
  }, [recording, stopRecorder, streamConnected]);

  const submitTranscript = useCallback(async () => {
    const transcript = pendingTranscript.trim();
    const activeSessionId = sessionIdRef.current;
    if (!shouldSubmitVoiceTranscript(transcript, turnInFlight) || !activeSessionId) return;
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      setVoicePhase("error");
      setError("Voice stream is not connected. Start voice again.");
      return;
    }
    setPendingTranscript("");
    setLiveCaption(transcript);
    setTurns((prev) => [
      ...prev,
      {
        id: `user:${Date.now()}:${prev.length}`,
        role: "user",
        content: transcript,
        timestamp: new Date().toISOString(),
      },
    ]);
    setTurnInFlight(true);
    setVoicePhase("thinking");
    setError(null);
    try {
      const result = await api.submitVoiceTurn(activeSessionId, transcript);
      const nextSession = result.session || null;
      if (nextSession) setSession(nextSession);
      const nextConversationId =
        result.conversation_id?.trim() || nextSession?.conversation_id?.trim() || conversationId || "";
      if (nextConversationId) setConversationId(nextConversationId);
      const assistantText = (result.assistant_text || "").trim();
      if (assistantText) {
        setTurns((prev) => [
          ...prev,
          {
            id: `assistant:${result.trace_id || Date.now()}:${prev.length}`,
            role: "assistant",
            content: assistantText,
            timestamp: new Date().toISOString(),
          },
        ]);
        sendJson({ type: "tts.synthesize", text: assistantText });
      } else {
        setVoicePhase("listening");
      }
      if (nextConversationId) {
        void loadConversationTurns(nextConversationId).catch(() => undefined);
      }
    } catch (turnError) {
      setVoicePhase("error");
      setError(errorText(turnError));
    } finally {
      setTurnInFlight(false);
    }
  }, [
    conversationId,
    loadConversationTurns,
    pendingTranscript,
    sendJson,
    setVoicePhase,
    turnInFlight,
  ]);

  const endVoice = useCallback(async () => {
    const activeSessionId = sessionIdRef.current;
    stopStreamingResources();
    setVoicePhase("idle");
    setSession(null);
    sessionIdRef.current = "";
    if (!activeSessionId) return;
    try {
      await api.stopVoiceSession(activeSessionId);
      setNotice("Voice session ended.");
    } catch (stopError) {
      setError(errorText(stopError));
    }
  }, [setVoicePhase, stopStreamingResources]);

  useEffect(() => () => stopStreamingResources(), [stopStreamingResources]);

  const transcriptPreview =
    pendingTranscript ||
    liveCaption ||
    (recording
      ? "Listening to you..."
      : streamConnected
        ? "Record a thought, review the transcript, then send it."
        : "Start voice chat when you are ready.");

  const canSubmitTranscript =
    streamConnected && shouldSubmitVoiceTranscript(pendingTranscript, turnInFlight) && !recording;
  const canStartCapture =
    streamConnected && !recording && !streamBusy && !transcriptDraft;

  return (
    <Box className="voice-page-shell">
      <Box className="voice-page-main">
        <Box className="voice-page-header">
          <Box>
            <Typography className="voice-page-kicker">Voice Chat</Typography>
            <Typography component="h1" className="voice-page-title">
              Talk with AgentArk
            </Typography>
          </Box>
          <Stack direction="row" spacing={1} sx={{ alignItems: "center" }}>
            <Chip
              size="small"
              className={`voice-status-chip phase-${phase}`}
              label={streamConnected ? phaseLabel(phase) : "Ready"}
            />
            {conversationId ? (
              <Button
                size="small"
                variant="outlined"
                className="voice-secondary-btn"
                startIcon={<ChatRoundedIcon fontSize="small" />}
                onClick={() => onNavigateToView?.("chat")}
              >
                Open Chat
              </Button>
            ) : null}
          </Stack>
        </Box>

        {error ? <Alert severity="error">{error}</Alert> : null}
        {notice && !error ? <Alert severity="info">{notice}</Alert> : null}
        {!streamSupport.available ? (
          <Alert severity="warning">
            Microphone streaming is unavailable in this browser.
          </Alert>
        ) : null}
        {status && !voiceAvailable ? (
          <Alert severity="warning">{voiceReadinessMessage(status)}</Alert>
        ) : null}

        <Box className="voice-stage">
          <Box className={`voice-mascot mood-${mascotMood}`}>
            <span className="voice-mascot-ring ring-one" />
            <span className="voice-mascot-ring ring-two" />
            <Box className="voice-mascot-core">
              <img src={AgentLogo} alt="" className="voice-mascot-logo" />
              <span className="voice-mascot-mouth" />
            </Box>
          </Box>
          <Box className="voice-stage-copy">
            <Typography className="voice-stage-label">
              {streamConnected ? phaseLabel(phase) : "Ready"}
            </Typography>
            <Typography className="voice-stage-transcript">{transcriptPreview}</Typography>
          </Box>
        </Box>

        {pendingTranscript ? (
          <Box className="voice-transcript-draft">
            <TextField
              fullWidth
              multiline
              minRows={2}
              maxRows={5}
              value={pendingTranscript}
              onChange={(event) => setPendingTranscript(event.target.value)}
              placeholder="Transcript"
              className="voice-transcript-input"
            />
          </Box>
        ) : null}

        <Box className="voice-controls" aria-label="Voice controls">
          {!streamConnected ? (
            <Button
              size="large"
              variant="contained"
              className="voice-primary-btn"
              startIcon={<PlayArrowRoundedIcon />}
              disabled={!voiceAvailable || !streamSupport.available || phase === "requesting_permission"}
              onClick={() => {
                void startVoice();
              }}
            >
              Start Voice
            </Button>
          ) : (
            <>
              {recording ? (
                <Tooltip title="Finish speech">
                  <IconButton className="voice-control-btn" onClick={finishTurnCapture}>
                    <StopCircleRoundedIcon />
                  </IconButton>
                </Tooltip>
              ) : (
                <Tooltip title={transcriptDraft ? "Submit or edit the transcript first" : "Record speech"}>
                  <span>
                    <IconButton
                      className="voice-control-btn"
                      disabled={!canStartCapture}
                      onClick={() => {
                        void beginTurnCapture();
                      }}
                    >
                      <MicRoundedIcon />
                    </IconButton>
                  </span>
                </Tooltip>
              )}
              <Tooltip title="Submit transcript">
                <span>
                  <IconButton
                    className="voice-control-btn"
                    disabled={!canSubmitTranscript}
                    onClick={() => {
                      void submitTranscript();
                    }}
                  >
                    <SendRoundedIcon />
                  </IconButton>
                </span>
              </Tooltip>
              <Tooltip title="End voice chat">
                <IconButton
                  className="voice-control-btn danger"
                  onClick={() => {
                    void endVoice();
                  }}
                >
                  <CallEndRoundedIcon />
                </IconButton>
              </Tooltip>
            </>
          )}
        </Box>
      </Box>

      <Box className="voice-transcript-panel">
        <Box className="voice-transcript-head">
          <GraphicEqRoundedIcon fontSize="small" />
          <Typography>Conversation</Typography>
        </Box>
        <Box className="voice-turn-list">
          {turns.length === 0 ? (
            <Typography className="voice-empty-copy">No turns yet.</Typography>
          ) : (
            turns.map((turn) => (
              <Box key={turn.id} className={`voice-turn role-${turn.role}`}>
                <Typography className="voice-turn-role">
                  {turn.role === "user" ? "You" : "AgentArk"}
                </Typography>
                <Typography className="voice-turn-content">{turn.content}</Typography>
              </Box>
            ))
          )}
          {turnInFlight ? (
            <Box className="voice-turn role-assistant is-pending">
              <VolumeUpRoundedIcon fontSize="small" />
              <Typography className="voice-turn-content">AgentArk is thinking...</Typography>
            </Box>
          ) : null}
        </Box>
      </Box>
    </Box>
  );
}
