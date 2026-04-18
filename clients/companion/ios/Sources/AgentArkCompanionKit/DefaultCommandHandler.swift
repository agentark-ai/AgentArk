import Foundation

public struct DefaultCommandHandler: CompanionCommandHandler {
    private let supportedCapabilities: Set<String>

    public init(supportedCapabilities: [String]) {
        self.supportedCapabilities = Set(supportedCapabilities)
    }

    public func handle(command: CompanionCommand) async -> CompanionCommandResult {
        guard supportedCapabilities.contains(command.capability) else {
            return CompanionCommandResult(
                success: false,
                error: "Unsupported capability."
            )
        }

        switch command.capability {
        case "approval_prompt", "notifications":
            return CompanionCommandResult(
                success: true,
                preview: "Received \(command.action)."
            )
        default:
            return CompanionCommandResult(
                success: false,
                error: "No local adapter is installed for \(command.capability)."
            )
        }
    }
}
