import AgentArkCompanionKit
import SwiftUI

private enum AgentArkTheme {
    static let background = Color(red: 0.012, green: 0.020, blue: 0.016)
    static let panel = Color(red: 0.040, green: 0.044, blue: 0.044).opacity(0.92)
    static let field = Color(red: 0.024, green: 0.035, blue: 0.039)
    static let line = Color.white.opacity(0.12)
    static let text = Color(red: 0.937, green: 0.969, blue: 0.937)
    static let muted = Color(red: 0.670, green: 0.690, blue: 0.720)
    static let cyan = Color(red: 0.486, green: 0.906, blue: 1.000)
    static let violet = Color(red: 0.545, green: 0.361, blue: 0.965)
    static let amber = Color(red: 0.961, green: 0.620, blue: 0.043)
    static let danger = Color(red: 1.000, green: 0.608, blue: 0.608)
}

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
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    brandHeader

                    CompanionCard("Connection") {
                        BrandedTextField(title: "WebSocket URL", text: $wsURL)
                        BrandedTextField(title: "Pairing session id", text: $sessionId)
                        BrandedTextField(title: "Pairing code", text: $code)
                    }

                    CompanionCard("Actions") {
                        Button("Claim pairing") {
                            Task { await claimPairing() }
                        }
                        .buttonStyle(CompanionButtonStyle(tone: .primary))

                        Button("Clear stored token", role: .destructive) {
                            try? tokenStore.clear()
                            status = "Stored token cleared"
                        }
                        .buttonStyle(CompanionButtonStyle(tone: .danger))
                    }

                    CompanionCard("Status") {
                        Text(status)
                            .foregroundStyle(AgentArkTheme.text)
                            .font(.body.weight(.medium))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(.horizontal, 18)
                .padding(.top, 20)
                .padding(.bottom, 28)
            }
            .background(pageBackground.ignoresSafeArea())
            .navigationTitle("AgentArk Companion")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(AgentArkTheme.background, for: .navigationBar)
            .toolbarColorScheme(.dark, for: .navigationBar)
        }
        .tint(AgentArkTheme.cyan)
    }

    private var brandHeader: some View {
        HStack(spacing: 12) {
            AgentArkLogoMark()
                .frame(width: 54, height: 54)
                .shadow(color: AgentArkTheme.cyan.opacity(0.30), radius: 12)

            VStack(alignment: .leading, spacing: 2) {
                Text("AgentArk")
                    .font(.caption.weight(.bold))
                    .foregroundStyle(AgentArkTheme.cyan)
                    .textCase(.uppercase)
                Text("Companion")
                    .font(.system(size: 30, weight: .bold, design: .rounded))
                    .foregroundStyle(AgentArkTheme.text)
                Text("Personal AI Agent OS")
                    .font(.subheadline)
                    .foregroundStyle(AgentArkTheme.muted)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.bottom, 6)
    }

    private var pageBackground: some View {
        LinearGradient(
            colors: [
                AgentArkTheme.cyan.opacity(0.12),
                AgentArkTheme.violet.opacity(0.08),
                AgentArkTheme.background
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
    }

    private func claimPairing() async {
        guard let url = URL(string: wsURL) else {
            status = "Invalid WebSocket URL"
            return
        }
        let capabilities = [
            "approval_prompt",
            "notifications"
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

private struct CompanionCard<Content: View>: View {
    private let title: String
    private let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title)
                .font(.headline)
                .foregroundStyle(AgentArkTheme.text)
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(AgentArkTheme.panel)
                .overlay(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(AgentArkTheme.line)
                )
        )
    }
}

private struct BrandedTextField: View {
    let title: String
    @Binding var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.caption)
                .foregroundStyle(AgentArkTheme.muted)
            TextField(title, text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .foregroundStyle(AgentArkTheme.text)
                .padding(.horizontal, 12)
                .frame(minHeight: 48)
                .background(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(AgentArkTheme.field)
                        .overlay(
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .stroke(AgentArkTheme.line)
                        )
                )
        }
    }
}

private enum CompanionButtonTone {
    case primary
    case danger
}

private struct CompanionButtonStyle: ButtonStyle {
    let tone: CompanionButtonTone

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.body.weight(.semibold))
            .foregroundStyle(tone == .danger ? AgentArkTheme.danger : AgentArkTheme.text)
            .frame(maxWidth: .infinity, minHeight: 48)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(tone == .primary ? AgentArkTheme.cyan.opacity(0.16) : Color.clear)
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(tone == .primary ? AgentArkTheme.cyan.opacity(0.58) : AgentArkTheme.danger.opacity(0.45))
                    )
            )
            .opacity(configuration.isPressed ? 0.72 : 1.0)
    }
}

private struct AgentArkLogoMark: View {
    var body: some View {
        GeometryReader { proxy in
            let size = min(proxy.size.width, proxy.size.height)
            let r = size * 0.34
            let cx = proxy.size.width * 0.5
            let cy = proxy.size.height * 0.55

            ZStack {
                Path { path in
                    path.move(to: CGPoint(x: cx - r * 0.45, y: cy - r * 0.72))
                    path.addLine(to: CGPoint(x: cx - r * 0.80, y: cy - r * 1.18))
                }
                .stroke(AgentArkTheme.cyan, style: StrokeStyle(lineWidth: size * 0.045, lineCap: .round))

                Path { path in
                    path.move(to: CGPoint(x: cx + r * 0.45, y: cy - r * 0.72))
                    path.addLine(to: CGPoint(x: cx + r * 0.80, y: cy - r * 1.18))
                }
                .stroke(AgentArkTheme.amber, style: StrokeStyle(lineWidth: size * 0.045, lineCap: .round))

                Ellipse()
                    .fill(
                        RadialGradient(
                            colors: [Color(red: 0.655, green: 0.545, blue: 0.980), AgentArkTheme.violet, Color(red: 0.298, green: 0.114, blue: 0.584)],
                            center: .topLeading,
                            startRadius: 2,
                            endRadius: size * 0.62
                        )
                    )
                    .frame(width: r * 2.02, height: r * 2.05)
                    .position(x: cx, y: cy)

                Circle()
                    .fill(Color.white)
                    .frame(width: r * 0.42, height: r * 0.42)
                    .position(x: cx - r * 0.34, y: cy - r * 0.16)
                Circle()
                    .fill(Color.white)
                    .frame(width: r * 0.42, height: r * 0.42)
                    .position(x: cx + r * 0.34, y: cy - r * 0.16)
                Circle()
                    .fill(AgentArkTheme.cyan)
                    .frame(width: r * 0.18, height: r * 0.18)
                    .position(x: cx - r * 0.34, y: cy - r * 0.16)
                Circle()
                    .fill(AgentArkTheme.amber)
                    .frame(width: r * 0.18, height: r * 0.18)
                    .position(x: cx + r * 0.34, y: cy - r * 0.16)
            }
        }
        .aspectRatio(1, contentMode: .fit)
    }
}
