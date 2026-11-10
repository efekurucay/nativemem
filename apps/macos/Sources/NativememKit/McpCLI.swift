import Foundation

/// The `nativemem mcp` commands. Names and secret ids can begin with "-", so they follow `--`.
extension NativememCLI {
    func mcpRun<T: Decodable & Sendable>(
        _ arguments: [String], stdin: Data? = nil, as type: T.Type = T.self
    ) async throws -> T {
        try await run(["mcp"] + arguments, stdin: stdin, as: type, decode: { try decodeMcp(T.self, from: $0) })
    }

    func mcpList() async throws -> [McpServer] {
        try await mcpRun(["list", "--json"], as: McpListDocument.self).servers
    }

    func mcpTest(_ name: String) async throws -> McpTestResult {
        try await mcpRun(["test", "--json", "--", name])
    }

    /// Opens the browser itself and returns when the sign-in finished, was refused or timed out.
    func mcpLogin(_ name: String) async throws -> McpSignedIn {
        try await mcpRun(["login", "--json", "--", name])
    }

    func mcpLogout(_ name: String) async throws -> McpSignedOut {
        try await mcpRun(["logout", "--json", "--", name])
    }

    func mcpSetEnabled(_ name: String, enabled: Bool) async throws -> McpEnabled {
        try await mcpRun([enabled ? "enable" : "disable", "--json", "--", name])
    }

    func mcpRemove(_ name: String) async throws -> McpRemoved {
        try await mcpRun(["remove", "--json", "--", name])
    }

    func mcpAdd(_ document: McpServerDocument) async throws -> McpAdded {
        try await mcpRun(["add", "--json"], stdin: try document.encoded())
    }

    func mcpEdit(_ name: String, document: McpServerDocument) async throws -> McpEdited {
        try await mcpRun(["edit", "--json", "--", name], stdin: try document.encoded())
    }

    /// The value travels on stdin, never in the arguments.
    func mcpSetSecret(_ id: String, value: String) async throws -> McpSecretStored {
        try await mcpRun(["set-secret", "--json", "--", id], stdin: Data(value.utf8))
    }

    func mcpStatus() async throws -> [McpAgentRegistration] {
        try await mcpRun(["status", "--json"], as: McpRegistrationDocument.self).agents
    }

    func mcpInstall(agent: String) async throws -> [McpAgentRegistration] {
        try await mcpRun(["install", "--agent", agent, "--json"], as: McpRegistrationDocument.self).agents
    }

    func mcpUninstall(agent: String) async throws -> [McpAgentRegistration] {
        try await mcpRun(["uninstall", "--agent", agent, "--json"], as: McpRegistrationDocument.self).agents
    }

    func mcpImport(apply: Bool) async throws -> McpImportPlan {
        try await mcpRun(apply ? ["import", "--apply", "--json"] : ["import", "--json"])
    }
}
