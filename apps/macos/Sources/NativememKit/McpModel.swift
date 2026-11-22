import Foundation
import Observation

/// The MCP & Integrations screen: the hub's servers, their state and the hub's registration in the agents.
/// Everything goes through `nativemem mcp ... --json`; nothing here knows how the hub works.
@MainActor @Observable
public final class McpModel {
    public var cli: NativememCLI?
    public private(set) var servers: [McpServer] = []
    /// The last `mcp test` of each enabled server.
    public private(set) var tests: [String: McpTestResult] = [:]
    public private(set) var registrations: [McpAgentRegistration] = []
    public private(set) var loadError: String?
    public private(set) var registrationError: String?
    public private(set) var isLoaded = false
    public private(set) var isLoading = false
    /// Servers with a test or a sign-in running.
    public private(set) var testing: Set<String> = []
    public private(set) var signingIn: Set<String> = []
    public private(set) var busyAgents: Set<String> = []
    public var selectedName: String?
    public var errorMessage: String?

    private var loadGeneration = 0

    public init() {}

    public var attentionCount: Int {
        McpServerState.attentionCount(servers: servers, tests: tests)
    }

    public var selectedServer: McpServer? {
        servers.first { $0.name == selectedName }
    }

    /// Agents on this machine; one that is not installed has nothing to register in.
    public var visibleRegistrations: [McpAgentRegistration] {
        registrations.filter { $0.state != .agentMissing }
    }

    public func state(of server: McpServer) -> McpServerState {
        McpServerState.derive(server, test: tests[server.name], isTesting: testing.contains(server.name))
    }

    public func toolCount(of server: McpServer) -> Int? {
        tests[server.name].flatMap { $0.ok ? $0.tools.count : nil }
    }

    // MARK: - Loading

    public func invalidate() {
        loadGeneration += 1
        isLoaded = false
        isLoading = false
        tests = [:]
    }

    public func loadIfNeeded() async {
        if !isLoaded && !isLoading { await reload() }
    }

    /// Reads server configuration and agent registrations without connecting to a server.
    public func reload() async {
        guard let cli else { return }
        loadGeneration += 1
        let generation = loadGeneration
        isLoading = true
        defer { if generation == loadGeneration { isLoading = false } }
        async let listed = Result { try await cli.mcpList() }
        async let registered = Result { try await cli.mcpStatus() }
        let (list, status) = await (listed, registered)
        guard generation == loadGeneration, !Task.isCancelled else { return }
        switch list {
        case .success(let value):
            servers = value
            loadError = nil
        case .failure(let error):
            servers = []
            loadError = error.localizedDescription
        }
        switch status {
        case .success(let value):
            registrations = value
            registrationError = nil
        case .failure(let error):
            registrations = []
            registrationError = error.localizedDescription
        }
        isLoaded = true
        keepSelectionAndTests()
    }

    private func refreshServers() async {
        guard let cli else { return }
        do {
            servers = try await cli.mcpList()
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
        keepSelectionAndTests()
    }

    private func keepSelectionAndTests() {
        let names = Set(servers.map(\.name))
        tests = tests.filter { names.contains($0.key) }
        if let selectedName, !names.contains(selectedName) { self.selectedName = nil }
    }

    private nonisolated static func runTest(_ cli: NativememCLI, _ name: String) async -> McpTestResult {
        do {
            return try await cli.mcpTest(name)
        } catch {
            return McpTestResult(ok: false, error: error.localizedDescription)
        }
    }

    // MARK: - Servers

    public func test(_ name: String) async {
        guard let cli, !testing.contains(name) else { return }
        testing.insert(name)
        tests[name] = nil
        let result = await Self.runTest(cli, name)
        testing.remove(name)
        if servers.contains(where: { $0.name == name }) {
            tests[name] = result
            if result.ok {
                setAuthState(.signedIn, for: name)
            } else if result.error?.hasPrefix(McpServerState.signInMessagePrefix) == true {
                setAuthState(.needsLogin, for: name)
            }
        }
    }

    private func setAuthState(_ state: McpAuthState, for name: String) {
        if let position = servers.firstIndex(where: { $0.name == name && $0.auth == .oauth }) {
            servers[position].authState = state
        }
    }

    /// Returns when `mcp login` exits; the command opens the browser itself.
    public func signIn(_ name: String) async {
        guard let cli else { return }
        signingIn.insert(name)
        do {
            _ = try await cli.mcpLogin(name)
            signingIn.remove(name)
            await refreshServers()
            tests[name] = nil
            setAuthState(.signedIn, for: name)
        } catch {
            signingIn.remove(name)
            errorMessage = error.localizedDescription
        }
    }

    public func signOut(_ name: String) async {
        guard let cli else { return }
        do {
            _ = try await cli.mcpLogout(name)
            tests[name] = nil
            await refreshServers()
            setAuthState(.needsLogin, for: name)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    public func setEnabled(_ name: String, _ enabled: Bool) async {
        guard let cli else { return }
        do {
            _ = try await cli.mcpSetEnabled(name, enabled: enabled)
        } catch {
            errorMessage = error.localizedDescription
        }
        await refreshServers()
        tests[name] = nil
    }

    public func remove(_ name: String) async {
        guard let cli else { return }
        do {
            _ = try await cli.mcpRemove(name)
        } catch {
            errorMessage = error.localizedDescription
        }
        await refreshServers()
    }

    /// Stores the secret values, then adds or replaces the server; a CLI error is thrown for the sheet to show.
    /// The secrets go first so the saved server never points at a value that is not there.
    public func save(_ draft: McpServerDraft) async throws {
        guard let cli else { throw CLIError.notFound }
        for secret in draft.secretValues {
            _ = try await cli.mcpSetSecret(secret.id, value: secret.value)
        }
        if draft.isEditing {
            _ = try await cli.mcpEdit(draft.trimmedName, document: draft.document)
        } else {
            _ = try await cli.mcpAdd(draft.document)
        }
        await refreshServers()
        selectedName = draft.trimmedName
        tests[draft.trimmedName] = nil
    }

    // MARK: - Agents

    public func install(_ agent: String) async {
        await register(agent) { try await $0.mcpInstall(agent: agent) }
    }

    public func uninstall(_ agent: String) async {
        await register(agent) { try await $0.mcpUninstall(agent: agent) }
    }

    private func register(
        _ agent: String, _ command: (NativememCLI) async throws -> [McpAgentRegistration]
    ) async {
        guard let cli else { return }
        busyAgents.insert(agent)
        defer { busyAgents.remove(agent) }
        do {
            for result in try await command(cli) {
                if let position = registrations.firstIndex(where: { $0.agent == result.agent }) {
                    registrations[position] = result
                }
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    // MARK: - Import

    public func makeImportSession() -> McpImportSession? {
        guard let cli else { return nil }
        return McpImportSession(cli: cli, onApplied: { [weak self] in await self?.reload() })
    }
}

extension Result where Failure == any Error {
    fileprivate init(_ body: () async throws -> Success) async {
        do { self = .success(try await body()) } catch { self = .failure(error) }
    }
}

@MainActor @Observable
public final class McpImportSession: Identifiable {
    public enum Phase {
        case loading
        case preview(McpImportPlan)
        case applying(McpImportPlan)
        case done(McpImportPlan)
        case failed(String)
    }

    public private(set) var phase: Phase = .loading
    private let cli: NativememCLI
    private let onApplied: @MainActor () async -> Void

    public init(cli: NativememCLI, onApplied: @escaping @MainActor () async -> Void) {
        self.cli = cli
        self.onApplied = onApplied
    }

    public func loadPreview() async {
        phase = .loading
        do {
            phase = .preview(try await cli.mcpImport(apply: false))
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    public func apply(plan: McpImportPlan) async {
        phase = .applying(plan)
        do {
            phase = .done(try await cli.mcpImport(apply: true))
            await onApplied()
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }
}
