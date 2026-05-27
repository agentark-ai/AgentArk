# AgentArk Android Companion

Kotlin Android companion client for `agentark-companion-v1`.

## Build

Open `clients/companion/android` in Android Studio.

Minimum SDK: 26.

## Security

- The bundled app advertises only `approval_prompt` and `notifications`; SMS needs a separate SMS-capable Android build or bridge.
- Stores scoped companion identity in encrypted shared preferences.
- Sends stored tokens in WebSocket headers, not JSON messages.
- Sends a stable device identity with each pairing claim.
- Uses typed WebSocket messages over `/companion/ws`.
- Refuses commands outside the local capability set.
- Leaves Android runtime permissions and user prompts to the app layer.

## First Run

1. In AgentArk, open `Settings > Integrations > Companion Devices`.
2. Create an Android pairing session.
3. Enter the WebSocket URL, session id, and code in the app.
4. Tap `Claim pairing`.
5. Approve the claim in AgentArk.
6. The app retries the claim, receives the scoped token, stores it, and uses header-based WebSocket authentication on later connections.
