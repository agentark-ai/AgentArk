# AgentArk iOS Companion

Swift iOS companion client source for `agentark-companion-v1`.

## Contents

- `Package.swift`: Swift package for `AgentArkCompanionKit`.
- `Sources/AgentArkCompanionKit`: protocol models, Keychain storage, WebSocket client, and safe command handling.
- `AppSources`: SwiftUI app shell that can be copied into an Xcode iOS app target.

## Build

Open this directory in Xcode or add `AgentArkCompanionKit` as a local package to an iOS app target.

Minimum target: iOS 16.

## Security

- Advertises only `approval_prompt` and `notifications`; it does not read SMS, iMessage, photos, location, camera, or Shortcuts.
- Stores the scoped device token in Keychain.
- Sends stored tokens in WebSocket headers, not JSON messages.
- Sends a stable device identity with each pairing claim.
- Uses typed command ids and JSON arguments.
- Refuses unsupported capabilities.
- Leaves high-risk platform permissions to iOS permission prompts and AgentArk fresh approval.

## Pairing Behavior

The Swift client sends `pairing_claim`, waits while AgentArk is pending approval, retries the claim after approval, stores the scoped token in Keychain, and uses header-based WebSocket authentication on later connections.
