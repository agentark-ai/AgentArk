# AgentArk Desktop and Headless Companion Agent

Rust companion agent for macOS, Windows, Linux, home servers, mini PCs, and Raspberry Pi devices.

## Build

```bash
cargo build --release
```

## Pair

Create a pairing session in `Settings > Integrations > Companion Devices`, then run:

```bash
agentark-companion-desktop-agent \
  --ws-url ws://127.0.0.1:8990/companion/ws \
  --profile desktop \
  --session-id pairing-... \
  --code ...
```

The agent sends a stable `device_public_key` with the claim. After you approve the claimed identity in the UI, it stores the scoped device token in `agentark-companion-identity.json`.

## Run

```bash
agentark-companion-desktop-agent \
  --ws-url ws://127.0.0.1:8990/companion/ws \
  --identity-file agentark-companion-identity.json \
  --profile desktop
```

The stored token is sent in WebSocket headers. Use `wss://<host>/companion/ws` outside local development.

For scoped file access:

```bash
agentark-companion-desktop-agent --root C:\Users\User\AgentArkCompanionFiles
```

For local command execution:

```bash
agentark-companion-desktop-agent --allow-system-run
```

`system_run` uses typed JSON arguments and executes a program directly without a shell:

```json
{
  "program": "python",
  "args": ["--version"],
  "timeout_secs": 10
}
```

## Profiles

- `desktop`: notifications, screen capture, browser control, file read/write, and local commands.
- `home-server`: notifications, local commands, LAN access, and file read/write.
- `raspberry-pi`: sensor read, smart-home control, LAN access, and local commands.
- `custom`: starts with `custom.example` unless `--capability` is supplied.

You can override the capability set:

```bash
agentark-companion-desktop-agent --profile custom --capability custom.greenhouse_sensor
```

## Security

- Device tokens are scoped companion tokens, not UI sessions or admin sessions.
- Device tokens are sent through WebSocket headers, not JSON messages.
- Pairing approval is bound to the claimed device identity.
- The agent rejects commands outside the configured local capability set.
- File access requires `--root` and rejects absolute paths or `..` traversal.
- Local process execution requires `--allow-system-run` and never invokes a shell.
- High-risk dispatch still requires fresh AgentArk approval before the command is sent.

For packaged desktop apps, store the identity in the platform keychain instead of a JSON file.
