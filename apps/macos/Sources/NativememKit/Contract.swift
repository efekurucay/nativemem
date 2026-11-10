import Foundation

public struct Scope: Codable, Hashable, Sendable {
    public var type: String
    public var path: String?

    public init(type: String, path: String? = nil) {
        self.type = type
        self.path = path
    }
}

public struct ItemRef: Codable, Hashable, Sendable {
    public var origin: String
    public var nativeId: String
}

public struct Item: Codable, Hashable, Sendable {
    public var agent: String
    public var nativeId: String
    public var path: String
    public var scope: Scope
    public var name: String
    public var description: String?
    public var category: String?
    public var body: String
    public var replicaOf: ItemRef?
    public var hash: String

    public init(
        agent: String, nativeId: String, path: String, scope: Scope, name: String,
        description: String? = nil, category: String? = nil, body: String = "",
        replicaOf: ItemRef? = nil, hash: String = ""
    ) {
        self.agent = agent
        self.nativeId = nativeId
        self.path = path
        self.scope = scope
        self.name = name
        self.description = description
        self.category = category
        self.body = body
        self.replicaOf = replicaOf
        self.hash = hash
    }
}

public struct ItemsDocument: Codable, Sendable {
    public var items: [Item]
}

public struct SkippedEntry: Codable, Hashable, Sendable {
    public var label: String
    public var reason: String
}

public struct AgentStatus: Codable, Hashable, Sendable {
    public var id: String
    public var originals: Int
    public var replicas: Int
    public var skipped: [SkippedEntry]
}

public struct StatusDocument: Codable, Sendable {
    public var agents: [AgentStatus]
    public var ledgerItems: Int

    /// Logical items: every original counted once, so it always matches the per-agent numbers.
    public var logicalItems: Int {
        agents.reduce(0) { $0 + $1.originals }
    }

    /// Agents whose copies match their originals: nothing left to write for them.
    public var agentsInSync: Int {
        agents.filter { $0.originals + $0.replicas >= logicalItems }.count
    }
}

public enum SyncOp: String, Codable, Sendable {
    case add, update, remove
}

public struct SyncAction: Codable, Hashable, Sendable {
    public var op: SyncOp
    public var agent: String
    public var name: String
    public var scope: Scope
    public var origin: ItemRef?
}

public enum NoteKind: String, Codable, Sendable {
    case conflict, skipped, orphan
}

public struct SyncNote: Codable, Hashable, Sendable {
    public var kind: NoteKind
    public var message: String
}

public struct SyncSummary: Codable, Hashable, Sendable {
    public var add: Int
    public var update: Int
    public var remove: Int
    public var conflicts: Int
    public var skipped: Int
}

public struct SyncPlan: Codable, Sendable {
    public var applied: Bool
    public var actions: [SyncAction]
    public var notes: [SyncNote]
    public var summary: SyncSummary
}

public struct EntryRef: Codable, Hashable, Sendable {
    public var agent: String
    public var nativeId: String
}

public struct DeleteResult: Codable, Sendable {
    public var deleted: EntryRef
    public var backupDir: String
}

public struct EditDocument: Encodable, Equatable, Sendable {
    public var name: String
    public var description: String
    public var category: String?
    public var body: String

    public init(name: String, description: String, category: String?, body: String) {
        self.name = name
        self.description = description
        self.category = category
        self.body = body
    }

    private enum CodingKeys: String, CodingKey {
        case name, description, category, body
    }

    /// The CLI expects `category` to be present, as `null` when there is none.
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(name, forKey: .name)
        try container.encode(description, forKey: .description)
        try container.encode(category, forKey: .category)
        try container.encode(body, forKey: .body)
    }
}

public struct EditResult: Codable, Sendable {
    public var edited: EntryRef
    public var hash: String
    public var backupDir: String
}

public struct ErrorDocument: Codable, Sendable {
    public var error: String
    public var code: String
}

extension JSONDecoder {
    static var contract: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }
}

public func decodeContract<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
    try JSONDecoder.contract.decode(type, from: data)
}
