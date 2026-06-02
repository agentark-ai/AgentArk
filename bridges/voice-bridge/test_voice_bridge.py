import os
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from voice_bridge.app import (
    AudioTurnBuffer,
    create_status_payload,
    is_websocket_disconnect_message,
    select_voice_provider,
)


class VoiceBridgeConfigTest(unittest.TestCase):
    def test_status_payload_reports_missing_local_assets(self):
        with mock.patch.dict(os.environ, {"PATH": ""}):
            payload = create_status_payload(
                provider=select_voice_provider({}),
                session_count=0,
            )

        self.assertEqual(payload["status"], "setup_needed")
        self.assertEqual(payload["engine"], "pipecat")
        self.assertEqual(payload["stt"]["provider"], "local")
        self.assertEqual(payload["tts"]["provider"], "piper")
        self.assertFalse(payload["stt"]["ready"])
        self.assertFalse(payload["tts"]["ready"])
        self.assertIn("browser_websocket", payload["transport"])
        self.assertEqual(payload["stream_path"], "/sessions/{session_id}/stream")
        self.assertGreaterEqual(len(payload["setup_errors"]), 1)

    def test_provider_selection_discovers_packaged_tools_without_user_env(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "bin"
            model_dir = root / "models"
            bin_dir.mkdir()
            model_dir.mkdir()
            (bin_dir / "whisper-cli").write_text("#!/bin/sh\n", encoding="utf-8")
            (bin_dir / "piper").write_text("#!/bin/sh\n", encoding="utf-8")
            (bin_dir / "ffmpeg").write_text("#!/bin/sh\n", encoding="utf-8")
            (model_dir / "ggml-base.en.bin").write_bytes(b"model")
            (model_dir / "en_US-lessac-medium.onnx").write_bytes(b"voice")

            provider = select_voice_provider({}, roots=[root])

        self.assertTrue(provider.enabled)
        self.assertEqual(provider.disabled_reason, None)
        self.assertIsNotNone(provider.stt_command)
        self.assertIsNotNone(provider.tts_command)
        self.assertIsNotNone(provider.audio_converter)
        self.assertIn("whisper-cli", provider.stt_command or "")
        self.assertIn("piper", provider.tts_command or "")

    def test_provider_selection_reports_actionable_missing_model_and_tts_errors(self):
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.dict(os.environ, {"PATH": ""}):
                provider = select_voice_provider({}, roots=[Path(tmp)])

        self.assertFalse(provider.enabled)
        self.assertEqual(provider.disabled_reason, "voice_assets_missing")
        codes = {error["code"] for error in provider.setup_errors}
        self.assertIn("local_stt_backend_missing", codes)
        self.assertIn("local_stt_model_missing", codes)
        self.assertIn("local_tts_backend_missing", codes)
        self.assertIn("local_tts_voice_missing", codes)

    def test_provider_selection_uses_structured_defaults(self):
        with mock.patch.dict(os.environ, {"PATH": ""}):
            provider = select_voice_provider({})

        self.assertEqual(provider.stt, "whisper_cpp")
        self.assertEqual(provider.tts, "piper")
        self.assertEqual(provider.model, "base.en")

    def test_audio_turn_buffer_uses_structured_turn_boundaries(self):
        buffer = AudioTurnBuffer()

        self.assertFalse(buffer.accept_audio(b"ignored"))
        self.assertIsNone(buffer.complete_turn())
        buffer.start_turn("audio/webm;codecs=opus")
        self.assertTrue(buffer.accept_audio(b"chunk-a"))
        self.assertTrue(buffer.accept_audio(b"chunk-b"))
        completed = buffer.complete_turn()

        self.assertIsNotNone(completed)
        self.assertEqual(completed.audio, b"chunk-achunk-b")
        self.assertEqual(completed.mime_type, "audio/webm;codecs=opus")

    def test_websocket_disconnect_messages_stop_receive_loop(self):
        self.assertTrue(is_websocket_disconnect_message({"type": "websocket.disconnect"}))
        self.assertFalse(is_websocket_disconnect_message({"type": "websocket.receive"}))

    def test_dockerfile_packages_whisper_runtime_libraries(self):
        dockerfile = Path(__file__).with_name("Dockerfile").read_text(encoding="utf-8")

        self.assertIn("/opt/agentark-voice/lib", dockerfile)
        self.assertIn("libwhisper*.so*", dockerfile)
        self.assertIn("libggml*.so*", dockerfile)
        self.assertIn("LD_LIBRARY_PATH", dockerfile)
        self.assertNotIn("${LD_LIBRARY_PATH}", dockerfile)


if __name__ == "__main__":
    unittest.main()
