import Foundation

/// MCP documents carry environment variable and header names as dictionary keys, which the snake-case
/// key strategy of `decodeContract` would rewrite (`GITHUB_TOKEN` becomes `githubToken`). They are decoded
/// with the keys as printed, so every snake-case field names its key explicitly.
public func decodeMcp<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
    try JSONDecoder().decode(type, from: data)
}

public enum McpTransport: String, Codable, Sendable {
    case stdio, http

    /// "Local command" or "Remote URL".
    public var title: String { self == .stdio ? "Local command" : "Remote URL" }
}

public enum McpAuth: String, Codable, Sendable {
    case none, oauth
}

public enum McpAuthState: String, Codable, Sendable {
    case none
    case signedIn = "signed_in"
    case needsLogin = "needs_login"
}

/// One server of the hub, as `mcp list --json` prints it. A secret value is never part of it:
/// `env` and `headers` hold `secret:<id>` references or the word `<literal>`.
public struct McpServer: Codable, Hashable, Sendable, Identifiable {
    public static let secretPrefix = "secret:"

    public var name: String
    public var transport: McpTransport
    public var enabled: Bool
    public var auth: McpAuth
    public var authState: McpAuthState
    public var command: String?
    public var args: [String]
    public var cwd: String?
    public var url: String?
    public var env: [String: String]
    public var headers: [String: String]
    public var secrets: [String]

    public var id: String { name }

    private enum CodingKeys: String, CodingKey {
        case name, transport, enabled, auth, command, args, cwd, url, env, headers, secrets
        case authState = "auth_state"
    }

    public init(
        name: String, transport: McpTransport, enabled: Bool = true, auth: McpAuth = .none,
        authState: McpAuthState = .none, command: String? = nil, args: [String] = [], cwd: String? = nil,
        url: String? = nil, env: [String: String] = [:], headers: [String: String] = [:], secrets: [String] = []
    ) {
        self.name = name
        self.transport = transport
        self.enabled = enabled
        self.auth = auth
        self.authState = authState
        self.command = command
        self.args = args
        self.cwd = cwd
        self.url = url
        self.env = env
        self.headers = headers
        self.secrets = secrets
    }

    public static func isSecretReference(_ value: String) -> Bool {
        value.hasPrefix(secretPrefix)
    }
}

public struct McpListDocument: Codable, Sendable {
    public var servers: [McpServer]
}

/// `mcp test --json`. `error` is a message without secret values or tokens.
public struct McpTestResult: Codable, Hashable, Sendable {
    public var ok: Bool
    public var tools: [String]
    public var error: String?

    public init(ok: Bool, tools: [String] = [], error: String? = nil) {
        self.ok = ok
        self.tools = tools
        self.error = error
    }
}

public struct McpAdded: Codable, Sendable { public var added: String }
public struct McpEdited: Codable, Sendable { public var edited: String }
public struct McpRemoved: Codable, Sendable { public var removed: String }
public struct McpEnabled: Codable, Sendable {
    public var name: String
    public var enabled: Bool
}
public struct McpSecretStored: Codable, Sendable { public var secret: String }
public struct McpSignedIn: Codable, Sendable {
    public var signedIn: String

    private enum CodingKeys: String, CodingKey { case signedIn = "signed_in" }
}
public struct McpSignedOut: Codable, Sendable {
    public var signedOut: String
    public var wasSignedIn: Bool

    private enum CodingKeys: String, CodingKey {
        case signedOut = "signed_out"
        case wasSignedIn = "was_signed_in"
    }
}

/// Where the hub stands in one agent.
public enum McpRegistrationState: Sendable {
    /// The agent has the hub entry.
    case installed
    /// The agent has an entry that differs from what Install would write now.
    case outdated
    case notInstalled
    /// The agent itself is not on this machine.
    case agentMissing
    case failed
}

/// One agent of `mcp install`, `mcp uninstall` and `mcp status`.
public struct McpAgentRegistration: Codable, Hashable, Sendable, Identifiable {
    public var agent: String
    public var status: String
    public var path: String?
    public var warnings: [String]
    public var code: String?
    public var message: String?

    public var id: String { agent }
    public var displayName: String { agentDisplayName(agent) }

    public var state: McpRegistrationState {
        switch status {
        case "installed", "unchanged", "registered": .installed
        case "outdated": .outdated
        case "removed", "absent", "missing": .notInstalled
        case "not_installed": .agentMissing
        default: .failed
        }
    }
}

/// The document of `mcp install`, `mcp uninstall` and `mcp status`; `command` and `args` are absent for uninstall.
public struct McpRegistrationDocument: Codable, Sendable {
    public var command: String?
    public var args: [String]?
    public var agents: [McpAgentRegistration]
}

public enum McpImportStatus: String, Codable, Sendable {
    case add, present, conflict
}

public struct McpImportServer: Codable, Hashable, Sendable, Identifiable {
    public var name: String
    public var transport: McpTransport
    public var status: McpImportStatus
    /// The agents that hold it; `hub` is among them for a conflict with the hub's own entry.
    public var agents: [String]
    public var secrets: [String]

    public var id: String { name }

    /// "Claude Code, Codex"
    public var agentsText: String {
        agents.map { $0 == "hub" ? "the hub" : agentDisplayName($0) }.joined(separator: ", ")
    }
}

public struct McpImportSkipped: Codable, Hashable, Sendable {
    public var agent: String
    public var name: String
    public var reason: String
}

public struct McpImportSummary: Codable, Hashable, Sendable {
    public var add: Int
    public var present: Int
    public var conflicts: Int
    public var skipped: Int
}

/// `mcp import --json` (preview) and `mcp import --apply --json`.
public struct McpImportPlan: Codable, Sendable {
    public var applied: Bool
    public var servers: [McpImportServer]
    public var skipped: [McpImportSkipped]
    public var warnings: [String]
    public var summary: McpImportSummary

    public func servers(_ status: McpImportStatus) -> [McpImportServer] {
        servers.filter { $0.status == status }
    }

    /// "2 to add, 1 already in the hub, 1 conflict"
    public var headline: String {
        let parts = [
            (summary.add, "to add"),
            (summary.present, "already in the hub"),
            (summary.conflicts, summary.conflicts == 1 ? "conflict" : "conflicts"),
        ].filter { $0.0 > 0 }.map { "\($0.0) \($0.1)" }
        return parts.isEmpty ? "Your agents have no servers to import." : parts.joined(separator: ", ")
    }

    public var resultSummary: String {
        summary.add == 0 ? "Nothing was imported." : "Imported \(summary.add) \(summary.add == 1 ? "server" : "servers")."
    }
}

/// What a server is doing, from `mcp list` and the last `mcp test`.
public enum McpServerState: Equatable, Sendable {
    /// The test has not answered yet.
    case checking
    case connected
    case needsSignIn
    case error(String)
    case disabled

    /// A message shown when the server needs sign-in; the hub's own wording starts like this.
    static let signInMessagePrefix = "Sign in again"

    public static func derive(_ server: McpServer, test: McpTestResult?) -> McpServerState {
        if !server.enabled { return .disabled }
        if server.auth == .oauth && server.authState == .needsLogin { return .needsSignIn }
        guard let test else { return .checking }
        if test.ok { return .connected }
        let message = test.error ?? "The server did not answer."
        if message.hasPrefix(signInMessagePrefix) { return .needsSignIn }
        return .error(message)
    }

    /// Counted in the sidebar badge.
    public var needsAttention: Bool {
        switch self {
        case .needsSignIn, .error: true
        case .checking, .connected, .disabled: false
        }
    }

    public var title: String {
        switch self {
        case .checking: "Checking…"
        case .connected: "Connected"
        case .needsSignIn: "Needs sign-in"
        case .error: "Error"
        case .disabled: "Disabled"
        }
    }

    /// Servers that need sign-in or are in error.
    public static func attentionCount(servers: [McpServer], tests: [String: McpTestResult]) -> Int {
        servers.filter { derive($0, test: tests[$0.name]).needsAttention }.count
    }
}
