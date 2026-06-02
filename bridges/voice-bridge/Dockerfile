FROM python:3.12-slim AS voice-assets

ARG WHISPER_CPP_REPO=https://github.com/ggml-org/whisper.cpp.git
ARG WHISPER_MODEL_URL=https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
ARG PIPER_RELEASE_URL=https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz
ARG PIPER_VOICE_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
ARG PIPER_VOICE_CONFIG_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        curl \
        g++ \
        git \
        make \
        tar && \
    rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    mkdir -p /opt/agentark-voice/bin /opt/agentark-voice/lib /opt/agentark-voice/models && \
    git clone --depth 1 "${WHISPER_CPP_REPO}" /tmp/whisper.cpp && \
    cmake -S /tmp/whisper.cpp -B /tmp/whisper.cpp/build \
        -DCMAKE_BUILD_TYPE=Release \
        -DGGML_NATIVE=OFF \
        -DBUILD_SHARED_LIBS=OFF && \
    (cmake --build /tmp/whisper.cpp/build --target whisper-cli --parallel 1 || \
        cmake --build /tmp/whisper.cpp/build --target main --parallel 1) && \
    whisper_cli="$(find /tmp/whisper.cpp/build -type f \( -name whisper-cli -o -name main \) -perm /111 -print -quit)" && \
    test -n "${whisper_cli}" && \
    cp "${whisper_cli}" /opt/agentark-voice/bin/whisper-cli && \
    find /tmp/whisper.cpp/build \( -name 'libwhisper*.so*' -o -name 'libggml*.so*' \) \
        -exec cp -L "{}" /opt/agentark-voice/lib/ \; && \
    curl -fsSL "${WHISPER_MODEL_URL}" -o /opt/agentark-voice/models/ggml-base.en.bin && \
    curl -fsSL "${PIPER_RELEASE_URL}" -o /tmp/piper.tar.gz && \
    tar -xzf /tmp/piper.tar.gz -C /opt/agentark-voice && \
    piper_bin="$(find /opt/agentark-voice -type f -name piper -perm /111 -print -quit)" && \
    test -n "${piper_bin}" && \
    ln -s "${piper_bin}" /opt/agentark-voice/bin/piper && \
    curl -fsSL "${PIPER_VOICE_URL}" -o /opt/agentark-voice/models/en_US-lessac-medium.onnx && \
    curl -fsSL "${PIPER_VOICE_CONFIG_URL}" -o /opt/agentark-voice/models/en_US-lessac-medium.onnx.json && \
    chmod +x /opt/agentark-voice/bin/whisper-cli /opt/agentark-voice/bin/piper

FROM python:3.12-slim

WORKDIR /app
ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PATH="/opt/agentark-voice/bin:${PATH}" \
    LD_LIBRARY_PATH="/opt/agentark-voice/lib:/opt/agentark-voice/piper"

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        ffmpeg \
        libgomp1 && \
    rm -rf /var/lib/apt/lists/*

COPY requirements.txt .
RUN python -m pip install --no-cache-dir --upgrade pip && \
    python -m pip install --no-cache-dir -r requirements.txt

COPY --from=voice-assets /opt/agentark-voice /opt/agentark-voice
RUN set -eux; \
    if ldd /opt/agentark-voice/bin/whisper-cli > /tmp/whisper-ldd.txt 2>&1; then \
        ! grep -q "not found" /tmp/whisper-ldd.txt; \
    fi; \
    rm -f /tmp/whisper-ldd.txt
COPY voice_bridge ./voice_bridge

EXPOSE 3105
CMD ["python", "-m", "voice_bridge.app"]
