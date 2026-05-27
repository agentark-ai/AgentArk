import Foundation

public enum CompanionState: String, Codable {
    case pairing
    case paired
    case online
    case idle
    case busy
    case offline
    case revoked
    case error
}

public enum CompanionMessageType: String, Codable {
    case hello
    case pairingClaim = "pairing_claim"
    case pairingClaimResult = "pairing_claim_result"
    case auth
    case authOk = "auth_ok"
    case authError = "auth_error"
    case pulse
    case pulseOk = "pulse_ok"
    case capabilityReport = "capability_report"
    case commandDispatch = "command_dispatch"
    case commandResult = "command_result"
    case commandResultOk = "command_result_ok"
    case error
}

public enum JSONValue: Codable, Equatable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: JSONValue])
    case array([JSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

public struct CompanionCommand: Codable, Identifiable, Equatable {
    public let id: String
    public let deviceId: String
    public let capability: String
    public let action: String
    public let arguments: JSONValue?
    public let requestedScopes: [String]
    public let risk: String
    public let status: String

    enum CodingKeys: String, CodingKey {
        case id
        case deviceId = "device_id"
        case capability
        case action
        case arguments
        case requestedScopes = "requested_scopes"
        case risk
        case status
    }
}

public struct CompanionCommandDescriptor: Codable, Equatable {
    public let id: String
    public let label: String
    public let capability: String
    public let action: String
    public let description: String
    public let risk: String
}

public struct CompanionEnvelope: Codable {
    public var type: CompanionMessageType
    public var protocolVersion: String?
    public var sessionId: String?
    public var code: String?
    public var deviceId: String?
    public var token: String?
    public var devicePublicKey: String?
    public var state: CompanionState?
    public var capabilities: [String]?
    public var commands: [CompanionCommandDescriptor]?
    public var metadata: [String: String]?
    public var command: CompanionCommand?
    public var commandId: String?
    public var success: Bool?
    public var resultPreview: String?
    public var error: String?

    public init(type: CompanionMessageType) {
        self.type = type
    }

    enum CodingKeys: String, CodingKey {
        case type
        case protocolVersion = "protocol_version"
        case sessionId = "session_id"
        case code
        case deviceId = "device_id"
        case token
        case devicePublicKey = "device_public_key"
        case state
        case capabilities
        case commands
        case metadata
        case command
        case commandId = "command_id"
        case success
        case resultPreview = "result_preview"
        case error
    }
}

public struct CompanionIdentity: Codable, Equatable {
    public var deviceId: String
    public var token: String

    public init(deviceId: String, token: String) {
        self.deviceId = deviceId
        self.token = token
    }
}
