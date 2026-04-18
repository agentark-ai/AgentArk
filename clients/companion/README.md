# AgentArk Companion Clients

This directory contains first-party companion-device clients and the shared protocol contract for devices that connect to AgentArk.

## Clients

- `ios/`: Swift iOS companion client source. Uses `URLSessionWebSocketTask` and Keychain-backed token storage.
- `android/`: Kotlin Android companion client source. Uses OkHttp WebSocket and encrypted token storage.
- `desktop-agent/`: Rust desktop/headless companion agent for macOS, Windows, Linux, home servers, and Raspberry Pi.
- `custom-device/`: documentation and examples for user-built devices that are not bundled with AgentArk.
- `protocol/`: shared `agentark-companion-v1` message schema.

## Runtime Contract

All clients use the same backend surface:

- UI path: `Settings > Integrations > Companion Devices`
- WebSocket path: `/companion/ws`
- Protocol: `agentark-companion-v1`
- Pairing: short-lived pairing session, stable `device_public_key`, explicit UI approval, retry approved claim, then one-time scoped device token
- Auth: paired devices reconnect with `Authorization: Bearer <token>` and `X-AgentArk-Companion-Device: <device_id>` WebSocket headers
- Commands: typed JSON actions only
- Sensitive actions: fresh approval required before dispatch

Companion tokens are not UI sessions, admin sessions, or API keys.
Production deployments should expose the companion WebSocket over TLS (`wss://`). Plain `ws://127.0.0.1` is for local development only.

## Custom Devices

Users can add devices that are not bundled with AgentArk by choosing **Custom Device** in the UI and implementing the WebSocket protocol in `protocol/agentark-companion-v1.schema.json`.

Custom capability ids should be structured and stable, for example:

- `custom.greenhouse_sensor`
- `custom.garage_controller`
- `custom.local_lab_power`

Do not dispatch raw natural-language strings to a device. Convert user intent into typed actions, validate scopes, and then dispatch structured JSON.
