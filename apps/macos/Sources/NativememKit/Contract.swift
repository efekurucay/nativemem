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
    public static let memoryKind = "memory"
    public static let instructionKind = "instruction"

    public var agent: String
    public var nativeId: String
    public var kind: String
    public var path: String
    public var scope: Scope
    public var name: String
    public var description: String?
    public var category: String?
    public var body: String
    public var replicaOf: ItemRef?
    public var hash: String

    public var isInstruction: Bool { kind == Item.instructionKind }

    public init(
        agent: String, nativeId: String, kind: String = Item.memoryKind, path: String, scope: Scope,
        name: String, description: String? = nil, category: String? = nil, body: String = "",
        replicaOf: ItemRef? = nil, hash: String = ""
    ) {
        self.agent = agent
        self.nativeId = nativeId
        self.kind = kind
        self.path = path
        self.scope = scope
        self.name = name
        self.description = description
        self.category = category
        self.body = body
        self.replicaOf = replicaOf
        self.hash = hash
    }

    /// `kind` defaults to memory when the CLI output does not carry it.
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        agent = try container.decode(String.self, forKey: .agent)
        nativeId = try container.decode(String.self, forKey: .nativeId)
        kind = try container.decodeIfPresent(String.self, forKey: .kind) ?? Item.memoryKind
        path = try container.decode(String.self, forKey: .path)
        scope = try container.decode(Scope.self, forKey: .scope)
        name = try container.decode(String.self, forKey: .name)
        description = try container.decodeIfPresent(String.self, forKey: .description)
        category = try container.decodeIfPresent(String.self, forKey: .category)
        body = try container.decode(String.self, forKey: .body)
        replicaOf = try container.decodeIfPresent(ItemRef.self, forKey: .replicaOf)
        hash = try container.decode(String.self, forKey: .hash)
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

    /// Agents that hold every item they can store: nothing left to write for them. Agents without
    /// project memory only need the instruction items.
    public func agentsInSync(instructionItems: Int) -> Int {
        let memoryItems = logicalItems - instructionItems
        return agents.filter {
            $0.originals + $0.replicas >= instructionItems + (agentHoldsMemory($0.id) ? memoryItems : 0)
        }.count
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
