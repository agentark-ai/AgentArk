# Custom Companion Devices

Users can add devices that are not bundled with AgentArk.

## Setup

1. Open `Settings > Integrations > Companion Devices`.
2. Choose `Custom Device`.
3. Add stable structured capability ids, such as `custom.greenhouse_sensor`.
4. Create a pairing code.
5. Connect your device to `/companion/ws`; use `wss://` outside local development.
6. Send `pairing_claim`.
7. Approve the claim in the UI.
8. Retry `pairing_claim` until the approved session returns the one-time scoped token.
9. Store the token in the device's secure storage.
10. Reconnect with `Authorization: Bearer <token>` and `X-AgentArk-Companion-Device: <device_id>` WebSocket headers.

## Message Flow

Pairing claim:

```json
{
  "type": "pairing_claim",
  "session_id": "pairing-...",
  "code": "...",
  "device_public_key": "stable-device-identity",
  "metadata": {
    "model": "my custom device"
  }
}
```

Auth:

```text
Authorization: Bearer acd_...
X-AgentArk-Companion-Device: device-...
```

Pulse:

```json
{
  "type": "pulse",
  "state": "online",
  "capabilities": ["custom.greenhouse_sensor"],
  "commands": [
    {
      "id": "custom.greenhouse_sensor.read",
      "label": "Read greenhouse sensor",
      "capability": "custom.greenhouse_sensor",
      "action": "custom.greenhouse_sensor.read",
      "description": "Read the local greenhouse sensor adapter.",
      "risk": "low"
    }
  ],
  "metadata": {
    "version": "1.0.0"
  }
}
```

Command result:

```json
{
  "type": "command_result",
  "command_id": "cmd-...",
  "success": true,
  "result_preview": "22.4 C"
}
```

## Rules

- Device tokens are scoped to one device.
- Device tokens are not admin sessions.
- Device tokens must be sent in WebSocket headers, not JSON messages.
- Pairing approval is bound to the claimed `device_public_key`.
- Commands are typed JSON actions, not raw natural-language strings.
- Devices should declare concrete commands in pulse messages; broad capability labels alone should not imply extra local adapters.
- Capability reports do not expand grants automatically.
- High-risk commands require fresh approval before dispatch.
- Custom devices must reject commands outside their local capability set.
