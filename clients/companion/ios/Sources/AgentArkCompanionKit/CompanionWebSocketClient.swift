import Foundation

public protocol CompanionCommandHandler: Sendable {
    func handle(command: CompanionCommand) async -> CompanionCommandResult
}

public struct CompanionCommandResult: Sendable {
    public var success: Bool
    public var preview: String?
    public var error: String?

    public init(success: Bool, preview: String? = nil, error: String? = nil) {
        self.success = success
        self.preview = preview
        self.error = error
    }
}

public actor CompanionWebSocketClient {
    private let webSocketURL: URL
    private let tokenStore: SecureTokenStore
    private let capabilities: [String]
    private let metadata: [String: String]
    private let handler: CompanionCommandHandler
    private var task: URLSessionWebSocketTask?
    private var pendingPairing: PendingPairing?
    private var pairingRetryTask: Task<Void, Never>?
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    public init(
        webSocketURL: URL,
        tokenStore: SecureTokenStore,
        capabilities: [String],
        metadata: [String: String],
        handler: CompanionCommandHandler
    ) {
        self.webSocketURL = webSocketURL
        self.tokenStore = tokenStore
        self.capabilities = capabilities.sorted()
        self.metadata = metadata
        self.handler = handler
    }

    public func connect() async throws {
        await requestLocalNotificationPermissionIfNeeded()
        let identity = try tokenStore.load()
        var request = URLRequest(url: webSocketURL)
        if let identity {
            request.setValue("Bearer \(identity.token)", forHTTPHeaderField: "Authorization")
            request.setValue(identity.deviceId, forHTTPHeaderField: "X-AgentArk-Companion-Device")
        }
        let task = URLSession.shared.webSocketTask(with: request)
        self.task = task
        task.resume()
        Task {
            try? await self.receiveLoop()
        }
    }

    public func disconnect() {
        pairingRetryTask?.cancel()
        pairingRetryTask = nil
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
    }

    public func claimPairing(sessionId: String, code: String, publicKey: String? = nil) async throws {
        await requestLocalNotificationPermissionIfNeeded()
        let pending = PendingPairing(sessionId: sessionId, code: code, publicKey: publicKey ?? (try tokenStore.devicePublicKey()))
        pendingPairing = pending
        try await sendPairingClaim(pending)
    }

    private func sendPairingClaim(_ pending: PendingPairing) async throws {
        var envelope = CompanionEnvelope(type: .pairingClaim)
        envelope.sessionId = pending.sessionId
        envelope.code = pending.code
        envelope.devicePublicKey = pending.publicKey
        envelope.metadata = metadata
        try await send(envelope)
    }

    public func sendPulse(state: CompanionState = .online) async throws {
        var envelope = CompanionEnvelope(type: .pulse)
        envelope.state = state
        envelope.capabilities = capabilities
        envelope.commands = commandDescriptors()
        envelope.metadata = metadata
        try await send(envelope)
    }

    private func commandDescriptors() -> [CompanionCommandDescriptor] {
        var descriptors: [CompanionCommandDescriptor] = []
        if capabilities.contains("approval_prompt") {
            descriptors.append(
                CompanionCommandDescriptor(
                    id: "approval.prompt",
                    label: "Approval prompt",
                    capability: "approval_prompt",
                    action: "approval.prompt",
                    description: "Ask this iOS companion for an approval decision.",
                    risk: "low"
                )
            )
        }
        if capabilities.contains("notifications") {
            descriptors.append(
                CompanionCommandDescriptor(
                    id: "notifications.show",
                    label: "Show notification",
                    capability: "notifications",
                    action: "notifications.show",
                    description: "Show a local notification on this iOS companion.",
                    risk: "low"
                )
            )
        }
        return descriptors
    }

    private func receiveLoop() async throws {
        guard let task else { return }
        while true {
            let message = try await task.receive()
            guard case .string(let raw) = message else { continue }
            let envelope = try decoder.decode(CompanionEnvelope.self, from: Data(raw.utf8))
            try await handle(envelope, raw: raw)
        }
    }

    private func handle(_ envelope: CompanionEnvelope, raw: String) async throws {
        switch envelope.type {
        case .authOk:
            try await sendPulse()
        case .pulseOk, .hello:
            return
        case .commandResultOk:
            try await sendPulse()
        case .pairingClaimResult:
            try await handlePairingResult(raw)
        case .commandDispatch:
            guard let command = envelope.command else { return }
            guard capabilities.contains(command.capability) else {
                try await sendResult(
                    commandId: command.id,
                    result: CompanionCommandResult(
                        success: false,
                        error: "Capability is not available on this iOS companion."
                    )
                )
                return
            }
            let result = await handler.handle(command: command)
            try await sendResult(commandId: command.id, result: result)
        case .authError, .error:
            throw CompanionClientError.server(envelope.error ?? "Companion server returned an error.")
        default:
            return
        }
    }

    private func handlePairingResult(_ raw: String) async throws {
        guard
            let object = try? JSONSerialization.jsonObject(
                with: Data(raw.utf8),
                options: []
            ) as? [String: Any],
            let result = object["result"] as? [String: Any]
        else {
            return
        }
        if
            let token = result["device_token"] as? String,
            let device = result["device"] as? [String: Any],
            let deviceId = device["id"] as? String
        {
            pairingRetryTask?.cancel()
            pairingRetryTask = nil
            pendingPairing = nil
            let identity = CompanionIdentity(deviceId: deviceId, token: token)
            try tokenStore.save(identity)
            try await sendPulse()
            return
        }
        let status = (result["status"] as? String) ?? ""
        if status == "claimed" || status == "approved" {
            schedulePairingRetry()
        }
    }

    private func schedulePairingRetry() {
        pairingRetryTask?.cancel()
        pairingRetryTask = Task {
            do {
                try await Task.sleep(nanoseconds: 3_000_000_000)
                try await self.resendPendingPairing()
            } catch {
                return
            }
        }
    }

    private func resendPendingPairing() async throws {
        guard let pendingPairing else { return }
        try await sendPairingClaim(pendingPairing)
    }

    private func requestLocalNotificationPermissionIfNeeded() async {
        guard capabilities.contains("notifications") || capabilities.contains("approval_prompt") else {
            return
        }
        _ = await CompanionLocalNotifications.requestAuthorizationIfAvailable()
    }

    private struct PendingPairing: Sendable {
        var sessionId: String
        var code: String
        var publicKey: String?
    }

    private func sendResult(commandId: String, result: CompanionCommandResult) async throws {
        var envelope = CompanionEnvelope(type: .commandResult)
        envelope.commandId = commandId
        envelope.success = result.success
        envelope.resultPreview = result.preview
        envelope.error = result.error
        try await send(envelope)
    }

    private func send(_ envelope: CompanionEnvelope) async throws {
        guard let task else { throw CompanionClientError.notConnected }
        let data = try encoder.encode(envelope)
        guard let text = String(data: data, encoding: .utf8) else {
            throw CompanionClientError.encoding
        }
        try await task.send(.string(text))
    }
}

public enum CompanionClientError: Error, Equatable {
    case notConnected
    case encoding
    case server(String)
}
