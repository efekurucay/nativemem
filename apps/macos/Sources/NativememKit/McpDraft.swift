import Foundation

/// One server as `mcp add` and `mcp edit` read it from stdin.
public struct McpServerDocument: Encodable, Equatable, Sendable {
    public var name: String
    public var transport: String
    public var command: String?
    public var args: [String]?
    public var env: [String: String]?
    public var cwd: String?
    public var url: String?
    public var headers: [String: String]?
    public var auth: String
    public var enabled: Bool

    func encoded() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(self)
    }
}

/// A row of environment variables (local command) or headers (remote URL).
public struct McpKeyValueRow: Identifiable, Equatable, Sendable {
    public var id = UUID()
    public var key = ""
    public var value = ""
    public var isSecret = false
    /// The `secret:<id>` reference the server already holds for this row.
    public var reference: String?
    /// A plain value of the server. The CLI does not show it, so it has to be typed again.
    public var isHiddenLiteral = false

    public init() {}

    var isBlank: Bool { key.trimmed.isEmpty && value.isEmpty }
}

/// What the Add server and Edit sheet collects: validation, the document for the CLI and the secret
/// values that go to `mcp set-secret`. Secret values never enter the document.
public struct McpServerDraft: Equatable, Sendable {
    public enum Kind: String, CaseIterable, Sendable {
        case local, remote

        public var title: String { self == .local ? "Local command" : "Remote URL" }
    }

    public let isEditing: Bool
    public var name = ""
    public var kind = Kind.local {
        didSet {
            guard kind != oldValue else { return }
            rows = []
            if kind == .local { usesOAuth = false }
        }
    }
    public var command = ""
    /// One argument per line.
    public var argumentLines = ""
    public var url = ""
    public var usesOAuth = false
    public var rows: [McpKeyValueRow] = []
    /// Not shown in the sheet; an edit keeps them.
    private var cwd: String?
    private var enabled = true

    public init() {
        isEditing = false
    }

    public init(editing server: McpServer) {
        isEditing = true
        name = server.name
        kind = server.transport == .stdio ? .local : .remote
        command = server.command ?? ""
        argumentLines = server.args.joined(separator: "\n")
        url = server.url ?? ""
        usesOAuth = server.auth == .oauth
        cwd = server.cwd
        enabled = server.enabled
        let values = server.transport == .stdio ? server.env : server.headers
        rows = values.sorted { $0.key < $1.key }.map { key, value in
            var row = McpKeyValueRow()
            row.key = key
            if McpServer.isSecretReference(value) {
                row.isSecret = true
                row.reference = value
            } else {
                row.isHiddenLiteral = true
            }
            return row
        }
    }

    public var trimmedName: String { name.trimmed }

    /// "Environment" or "Headers".
    public var rowsTitle: String { kind == .local ? "Environment" : "Headers" }

    public var arguments: [String] {
        argumentLines.split(separator: "\n", omittingEmptySubsequences: true)
            .map(String.init)
            .filter { !$0.trimmed.isEmpty }
    }

    /// Rows that hold something; a row left empty is ignored.
    var activeRows: [McpKeyValueRow] { rows.filter { !$0.isBlank } }

    /// The reference the config stores for a secret row: the one it already has, else `secret:<server>-<key>`.
    func reference(for row: McpKeyValueRow) -> String {
        row.reference ?? "\(McpServer.secretPrefix)\(trimmedName)-\(row.key.trimmed)"
    }

    /// Secret values to store before the server is saved, by secret id.
    public var secretValues: [(id: String, value: String)] {
        activeRows.filter { $0.isSecret && !$0.value.isEmpty }.map { row in
            (id: String(reference(for: row).dropFirst(McpServer.secretPrefix.count)), value: row.value)
        }
    }

    /// The first thing wrong with the draft, in plain words; nil when it can be saved.
    public var problem: String? {
        if trimmedName.isEmpty { return "Enter a name." }
        switch kind {
        case .local: if command.trimmed.isEmpty { return "Enter the command." }
        case .remote: if url.trimmed.isEmpty { return "Enter the address." }
        }
        var seen = Set<String>()
        for row in activeRows {
            let key = row.key.trimmed
            if key.isEmpty { return "Every row needs a name." }
            if !seen.insert(key).inserted { return "\(key) appears twice." }
            if row.value.isEmpty && !(row.isSecret && row.reference != nil) {
                return row.isHiddenLiteral && !row.isSecret
                    ? "Type the value of \(key) again, or remove the row."
                    : "Enter a value for \(key)."
            }
        }
        return nil
    }

    public var isValid: Bool { problem == nil }

    /// Some plain values cannot be shown, so saving needs them typed again.
    public var hasHiddenValues: Bool { rows.contains { $0.isHiddenLiteral && !$0.isSecret } }

    public var document: McpServerDocument {
        var values: [String: String] = [:]
        for row in activeRows {
            values[row.key.trimmed] = row.isSecret ? reference(for: row) : row.value
        }
        let hasValues = !values.isEmpty
        switch kind {
        case .local:
            return McpServerDocument(
                name: trimmedName, transport: McpTransport.stdio.rawValue, command: command.trimmed,
                args: arguments, env: hasValues ? values : nil, cwd: cwd, url: nil, headers: nil,
                auth: McpAuth.none.rawValue, enabled: enabled)
        case .remote:
            return McpServerDocument(
                name: trimmedName, transport: McpTransport.http.rawValue, command: nil, args: nil, env: nil,
                cwd: nil, url: url.trimmed, headers: hasValues ? values : nil,
                auth: (usesOAuth ? McpAuth.oauth : McpAuth.none).rawValue, enabled: enabled)
        }
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}
