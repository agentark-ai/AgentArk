import AgentArkCompanionKit
import SwiftUI

@main
struct AgentArkCompanionApp: App {
    var body: some Scene {
        WindowGroup {
            CompanionHomeView()
        }
    }
}

struct CompanionHomeView: View {
    @State private var wsURL = "ws://localhost:8990/companion/ws"
    @State private var sessionId = ""
    @State private var code = ""
    @State private var status = "Not connected"

    private let tokenStore = SecureTokenStore()

    var body: some View {
        NavigationStack {
            Form {
                Section("AgentArk") {
                    TextField("WebSocket URL", text: $wsURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Pairing session id", text: $sessionId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Pairing code", text: $code)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section("Status") {
                    Text(status)
                }
                Section {
                    Button("Claim pairing") {
                        Task { await claimPairing() }
                    }
                    Button("Clear stored token", role: .destructive) {
                        try? tokenStore.clear()
                        status = "Stored token cleared"
                    }
                }
            }
            .navigationTitle("AgentArk Companion")
        }
    }

    private func claimPairing() async {
        guard let url = URL(string: wsURL) else {
            status = "Invalid WebSocket URL"
            return
        }
        let capabilities = [
            "approval_prompt",
            "notifications",
            "camera",
            "photos",
            "location",
            "shortcuts_run"
        ]
        let client = CompanionWebSocketClient(
            webSocketURL: url,
            tokenStore: tokenStore,
            capabilities: capabilities,
            metadata: ["platform": "ios", "client": "AgentArk iOS"],
            handler: DefaultCommandHandler(supportedCapabilities: capabilities)
        )
        do {
            try await client.connect()
            try await client.claimPairing(sessionId: sessionId, code: code)
            status = "Pairing claim sent. Approve in AgentArk."
        } catch {
            status = "Failed: \(error.localizedDescription)"
        }
    }
}
