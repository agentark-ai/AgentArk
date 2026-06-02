from __future__ import annotations

import asyncio
import base64
import json
import os
import shutil
import shlex
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


@dataclass(frozen=True)
class VoiceProvider:
    enabled: bool
    stt: str = "whisper_cpp"
    tts: str = "piper"
    model: str = "base.en"
    stt_command: str | None = None
    tts_command: str | None = None
    audio_converter: str | None = None
    setup_errors: tuple[dict[str, str], ...] = ()
    disabled_reason: str | None = None


@dataclass(frozen=True)
class CompletedAudioTurn:
    audio: bytes
    mime_type: str


class AudioTurnBuffer:
    def __init__(self) -> None:
        self._active = False
        self._mime_type = "application/octet-stream"
        self._chunks: list[bytes] = []

    def start_turn(self, mime_type: str | None = None) -> None:
        self._active = True
        self._mime_type = (mime_type or "application/octet-stream").strip() or "application/octet-stream"
        self._chunks = []

    def accept_audio(self, chunk: bytes) -> bool:
        if not self._active or not chunk:
            return False
        self._chunks.append(bytes(chunk))
        return True

    def complete_turn(self) -> CompletedAudioTurn | None:
        if not self._active:
            return None
        self._active = False
        audio = b"".join(self._chunks)
        self._chunks = []
        if not audio:
            return None
        return CompletedAudioTurn(audio=audio, mime_type=self._mime_type)


def _default_roots() -> list[Path]:
    app_root = Path(__file__).resolve().parents[1]
    return [
        app_root,
        Path("/opt/agentark-voice"),
        Path("/app"),
        Path.cwd(),
    ]


def _first_existing(paths: Sequence[Path]) -> Path | None:
    for path in paths:
        if path.exists():
            return path
    return None


def _tool_candidates(roots: Sequence[Path], names: Sequence[str]) -> list[Path]:
    candidates: list[Path] = []
    for root in roots:
        for name in names:
            candidates.append(root / "bin" / name)
            candidates.append(root / name)
    return candidates


def _first_executable(roots: Sequence[Path], names: Sequence[str]) -> Path | None:
    rooted = _first_existing(_tool_candidates(roots, names))
    if rooted is not None:
        return rooted
    for name in names:
        resolved = shutil.which(name)
        if resolved:
            return Path(resolved)
    return None


def _model_candidates(roots: Sequence[Path], names: Sequence[str]) -> list[Path]:
    candidates: list[Path] = []
    for root in roots:
        for name in names:
            candidates.append(root / "models" / name)
            candidates.append(root / name)
    return candidates


def _setup_error(code: str, message: str) -> dict[str, str]:
    return {"code": code, "message": message}


def select_voice_provider(
    env: Mapping[str, str],
    *,
    roots: Sequence[Path] | None = None,
) -> VoiceProvider:
    _ = env
    roots = list(roots or _default_roots())
    model = "base.en"
    stt = "whisper_cpp"
    tts = "piper"
    whisper_binary = _first_executable(roots, ["whisper-cli", "whisper-cli.exe", "main", "main.exe"])
    piper_binary = _first_executable(roots, ["piper", "piper.exe"])
    audio_converter = _first_executable(roots, ["ffmpeg", "ffmpeg.exe"])
    stt_model = _first_existing(
        _model_candidates(roots, [f"ggml-{model}.bin", f"{model}.bin"]),
    )
    tts_model = _first_existing(
        _model_candidates(
            roots,
            [
                "en_US-lessac-medium.onnx",
                "en_US-amy-medium.onnx",
                "piper-voice.onnx",
            ],
        ),
    )
    setup_errors: list[dict[str, str]] = []
    if whisper_binary is None:
        setup_errors.append(
            _setup_error(
                "local_stt_backend_missing",
                "Local speech recognition is not installed in this voice bridge build.",
            ),
        )
    if stt_model is None:
        setup_errors.append(
            _setup_error(
                "local_stt_model_missing",
                "Local speech recognition model is not installed in this voice bridge build.",
            ),
        )
    if piper_binary is None:
        setup_errors.append(
            _setup_error(
                "local_tts_backend_missing",
                "Local text-to-speech is not installed in this voice bridge build.",
            ),
        )
    if tts_model is None:
        setup_errors.append(
            _setup_error(
                "local_tts_voice_missing",
                "Local text-to-speech voice model is not installed in this voice bridge build.",
            ),
        )
    if audio_converter is None:
        setup_errors.append(
            _setup_error(
                "local_audio_converter_missing",
                "Local audio conversion is not installed in this voice bridge build.",
            ),
        )
    enabled = not setup_errors
    stt_command = None
    tts_command = None
    if whisper_binary is not None and stt_model is not None:
        stt_command = f"{_quote(whisper_binary)} -m {_quote(stt_model)} -f {{audio}} -nt"
    if piper_binary is not None and tts_model is not None:
        tts_command = (
            f"{_quote(piper_binary)} --model {_quote(tts_model)} "
            "--output_file {output} < {text_file}"
        )
    return VoiceProvider(
        enabled=enabled,
        stt=stt,
        tts=tts,
        model=model,
        stt_command=stt_command,
        tts_command=tts_command,
        audio_converter=str(audio_converter) if audio_converter is not None else None,
        setup_errors=tuple(setup_errors),
        disabled_reason=None if enabled else "voice_assets_missing",
    )


def create_status_payload(provider: VoiceProvider, session_count: int) -> dict:
    return {
        "status": "ready" if provider.enabled else "setup_needed",
        "engine": "pipecat",
        "transport": ["browser", "browser_websocket"],
        "stream_path": "/sessions/{session_id}/stream",
        "sessions": max(0, int(session_count)),
        "stt": {
            "provider": "local",
            "engine": provider.stt,
            "model": provider.model,
            "ready": bool(provider.stt_command),
        },
        "tts": {
            "provider": provider.tts,
            "ready": bool(provider.tts_command),
        },
        "audio": {
            "converter": "ffmpeg",
            "ready": bool(provider.audio_converter),
        },
        "disabled_reason": provider.disabled_reason,
        "setup_errors": list(provider.setup_errors),
    }


def _extension_for_mime(mime_type: str) -> str:
    mime = (mime_type or "").split(";", 1)[0].strip().lower()
    if mime == "audio/webm":
        return ".webm"
    if mime == "audio/ogg":
        return ".ogg"
    if mime == "audio/mp4":
        return ".m4a"
    if mime in {"audio/wav", "audio/x-wav"}:
        return ".wav"
    return ".audio"


def _quote(value: str | Path) -> str:
    return shlex.quote(str(value))


def _format_command(template: str, values: Mapping[str, str | Path]) -> str:
    quoted = {key: _quote(value) for key, value in values.items()}
    return template.format(**quoted)


def _run_shell(command: str, *, text: bool) -> subprocess.CompletedProcess:
    return subprocess.run(command, shell=True, check=False, capture_output=True, text=text)


def is_websocket_disconnect_message(message: Mapping[str, object]) -> bool:
    return message.get("type") == "websocket.disconnect"


def normalize_audio_for_stt(provider: VoiceProvider, audio_path: Path, mime_type: str) -> Path:
    if _extension_for_mime(mime_type) == ".wav":
        return audio_path
    if not provider.audio_converter:
        raise RuntimeError(_provider_setup_message(provider, "local_audio_converter_missing"))

    output_path = audio_path.with_suffix(".wav")
    command = (
        f"{_quote(provider.audio_converter)} -nostdin -hide_banner -loglevel error -y "
        f"-i {_quote(audio_path)} -ar 16000 -ac 1 {_quote(output_path)}"
    )
    result = _run_shell(command, text=True)
    if result.returncode != 0:
        raise RuntimeError((result.stderr or result.stdout or "Local audio conversion failed").strip())
    if not output_path.exists():
        raise RuntimeError("Local audio conversion did not produce a WAV file")
    return output_path


def transcribe_completed_turn(provider: VoiceProvider, turn: CompletedAudioTurn) -> str:
    if not provider.stt_command:
        raise RuntimeError(_provider_setup_message(provider, "local_stt_model_missing"))
    with tempfile.TemporaryDirectory(prefix="agentark-voice-stt-") as tmp:
        audio_path = Path(tmp) / f"turn{_extension_for_mime(turn.mime_type)}"
        audio_path.write_bytes(turn.audio)
        stt_audio_path = normalize_audio_for_stt(provider, audio_path, turn.mime_type)
        command = _format_command(
            provider.stt_command,
            {
                "audio": stt_audio_path,
                "model": provider.model,
            },
        )
        result = _run_shell(command, text=True)
        if result.returncode != 0:
            raise RuntimeError((result.stderr or result.stdout or "Local STT failed").strip())
        return (result.stdout or "").strip()


def synthesize_speech(provider: VoiceProvider, text: str) -> tuple[bytes, str]:
    if not provider.tts_command:
        raise RuntimeError(_provider_setup_message(provider, "local_tts_voice_missing"))
    with tempfile.TemporaryDirectory(prefix="agentark-voice-tts-") as tmp:
        text_path = Path(tmp) / "input.txt"
        output_path = Path(tmp) / "speech.wav"
        text_path.write_text(text, encoding="utf-8")
        command = _format_command(
            provider.tts_command,
            {
                "text_file": text_path,
                "output": output_path,
                "model": provider.model,
            },
        )
        result = _run_shell(command, text=False)
        if result.returncode != 0:
            stderr = result.stderr.decode("utf-8", errors="replace") if result.stderr else ""
            stdout = result.stdout.decode("utf-8", errors="replace") if result.stdout else ""
            raise RuntimeError((stderr or stdout or "Local TTS failed").strip())
        if output_path.exists():
            return output_path.read_bytes(), "audio/wav"
        if result.stdout:
            return result.stdout, "audio/wav"
        raise RuntimeError("Local TTS did not produce audio")


def _provider_setup_message(provider: VoiceProvider, fallback_code: str) -> str:
    for setup_error in provider.setup_errors:
        if setup_error.get("code") == fallback_code:
            return setup_error.get("message") or "Local voice setup is incomplete."
    if provider.setup_errors:
        return provider.setup_errors[0].get("message") or "Local voice setup is incomplete."
    return "Local voice setup is incomplete."


try:
    from fastapi import FastAPI, WebSocket, WebSocketDisconnect
except Exception:  # pragma: no cover - allows unit tests without runtime deps
    FastAPI = None


if FastAPI is not None:
    app = FastAPI(title="AgentArk Voice Bridge")
    _sessions: set[str] = set()

    @app.get("/health")
    async def health() -> dict:
        return {"status": "ok"}

    @app.get("/status")
    async def status() -> dict:
        return create_status_payload(select_voice_provider(os.environ), len(_sessions))

    @app.websocket("/sessions/{session_id}/stream")
    async def stream_session(websocket: WebSocket, session_id: str) -> None:
        await websocket.accept()
        provider = select_voice_provider(os.environ)
        buffer = AudioTurnBuffer()
        _sessions.add(session_id)
        try:
            await websocket.send_json({
                "type": "session.ready",
                "session_id": session_id,
                "engine": "pipecat",
            })
            while True:
                message = await websocket.receive()
                if is_websocket_disconnect_message(message):
                    break
                audio = message.get("bytes")
                if audio is not None:
                    buffer.accept_audio(audio)
                    continue

                raw_text = message.get("text")
                if raw_text is None:
                    continue
                try:
                    event = json.loads(raw_text)
                except json.JSONDecodeError:
                    await websocket.send_json({
                        "type": "error",
                        "code": "invalid_event",
                        "message": "Voice stream event was not valid JSON",
                    })
                    continue

                event_type = event.get("type")
                if event_type == "turn.start":
                    buffer.start_turn(event.get("mime_type"))
                elif event_type == "turn.end":
                    completed = buffer.complete_turn()
                    if completed is None:
                        await websocket.send_json({"type": "session.listening"})
                        continue
                    try:
                        transcript = await asyncio.to_thread(
                            transcribe_completed_turn,
                            provider,
                            completed,
                        )
                    except Exception as error:
                        await websocket.send_json({
                            "type": "error",
                            "code": "stt_unavailable",
                            "message": str(error),
                        })
                        continue
                    if transcript:
                        await websocket.send_json({
                            "type": "transcript.final",
                            "text": transcript,
                        })
                    else:
                        await websocket.send_json({"type": "session.listening"})
                elif event_type == "tts.synthesize":
                    text = str(event.get("text") or "").strip()
                    if not text:
                        continue
                    try:
                        audio_bytes, mime_type = await asyncio.to_thread(
                            synthesize_speech,
                            provider,
                            text,
                        )
                    except Exception as error:
                        await websocket.send_json({
                            "type": "error",
                            "code": "tts_unavailable",
                            "message": str(error),
                        })
                        await websocket.send_json({"type": "session.listening"})
                        continue
                    await websocket.send_json({
                        "type": "tts.audio",
                        "mime_type": mime_type,
                        "audio": base64.b64encode(audio_bytes).decode("ascii"),
                    })
                    await websocket.send_json({"type": "session.listening"})
                elif event_type == "session.stop":
                    await websocket.close()
                    break
        except WebSocketDisconnect:
            pass
        finally:
            _sessions.discard(session_id)
else:
    app = None


def main() -> None:
    if app is None:
        raise SystemExit("fastapi is not installed; install bridges/voice-bridge/requirements.txt")
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=3105)


if __name__ == "__main__":
    main()
