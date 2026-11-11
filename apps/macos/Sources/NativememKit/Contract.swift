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

/// What `items --kind session --json` adds to a session: its times and how many turns it has.
public struct SessionStats: Codable, Hashable, Sendable {
    public var createdMs: Int64
    public var updatedMs: Int64
    public var turns: Int

    public init(createdMs: Int64, updatedMs: Int64, turns: Int) {
        self.createdMs = createdMs
        self.updatedMs = updatedMs
        self.turns = turns
    }

    public var updated: Date { Date(timeIntervalSince1970: Double(updatedMs) / 1000) }
}

public struct Item: Codable, Hashable, Sendable {
    public static let memoryKind = "memory"
    public static let instructionKind = "instruction"
    public static let sessionKind = "session"
    public static let skillKind = "skill"

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
    /// Present on chat sessions only.
    public var session: SessionStats?

    public var isInstruction: Bool { kind == Item.instructionKind }
    public var isSkill: Bool { kind == Item.skillKind }

    public init(
        agent: String, nativeId: String, kind: String = Item.memoryKind, path: String, scope: Scope,
        name: String, description: String? = nil, category: String? = nil, body: String = "",
        replicaOf: ItemRef? = nil, hash: String = "", session: SessionStats? = nil
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
        self.session = session
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
        session = try container.decodeIfPresent(SessionStats.self, forKey: .session)
    }
}

public struct ItemsDocument: Codable, Sendable {
    public var items: [Item]
    /// Session-only diagnostics, delivered with the on-demand listing.
    public var sessionStatus: [AgentStatus]?
    public var sessionNotice: String?
}

/// The desktop's initial read; transcript loading is a separate, on-demand operation.
public struct SnapshotDocument: Codable, Sendable {
    public var items: ItemsDocument
    public var status: StatusDocument
    public var doctor: DoctorDocument
    public var sessionsIncluded: Bool
}

public struct SkippedEntry: Codable, Hashable, Sendable {
    public var label: String
    public var reason: String
}

public struct AgentStatus: Codable, Hashable, Sendable {
    public var id: String
    public var originals: Int
    public var replicas: Int
    /// Chat sessions, counted apart from memory and instruction entries.
    public var sessions: Int
    /// Skills, counted apart from memory and instruction entries.
    public var skills: Int
    public var skipped: [SkippedEntry]

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        originals = try container.decode(Int.self, forKey: .originals)
        replicas = try container.decode(Int.self, forKey: .replicas)
        sessions = try container.decodeIfPresent(Int.self, forKey: .sessions) ?? 0
        skills = try container.decodeIfPresent(Int.self, forKey: .skills) ?? 0
        skipped = try container.decode([SkippedEntry].self, forKey: .skipped)
    }
}

public struct StatusDocument: Codable, Sendable {
    public var agents: [AgentStatus]
    public var ledgerItems: Int

    /// Logical items: every original counted once, so it always matches the per-agent numbers.
    public var logicalItems: Int {
        agents.reduce(0) { $0 + $1.originals }
    }

    /// The agent that holds the most originals, the first one on a tie; nil without agents.
    public var agentWithMostOriginals: String? {
        agents.reduce(nil as AgentStatus?) { best, agent in
            guard let best else { return agent }
            return agent.originals > best.originals ? agent : best
        }?.id
    }

    /// Agents that hold every logical item: nothing left to write for them.
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

/// The SKILL.md of one skill, as `skill show --json` prints it.
public struct SkillText: Codable, Hashable, Sendable {
    public var agent: String
    public var name: String
    public var path: String
    public var text: String
}

public struct SessionTurn: Codable, Hashable, Sendable {
    public var role: String
    public var text: String
    public var tsMs: Int64

    /// "You" or "Assistant".
    public var speaker: String { role == "user" ? "You" : "Assistant" }
}

/// One session with its turns, as `session show --json` prints it.
public struct SessionTranscript: Codable, Hashable, Sendable {
    public var agent: String
    public var nativeId: String
    public var title: String
    public var cwd: String
    public var createdMs: Int64
    public var updatedMs: Int64
    public var turns: [SessionTurn]
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

public struct AddResult: Codable, Sendable {
    public var agent: String
    public var nativeId: String
    public var path: String
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

public enum CompatStatus: String, Codable, Sendable {
    case ok
    case updated
    case actionRequired = "action_required"
    case notInstalled = "not_installed"
}

public struct DoctorCheck: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var featureKinds: [String]
    public var summary: String
    public var expected: String
    public var observed: String
    public var ok: Bool
    public var accepted: Bool
    public var sources: [String]
}

public struct DoctorAgent: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var installed: Bool
    public var version: String?
    public var previousVersion: String?
    public var status: CompatStatus
    /// Kinds of entries (`memory`, `instruction`, `session`, `skill`) that are left alone for this agent.
    public var paused: [String]
    public var checks: [DoctorCheck]
    /// Present for agents that need action: a prompt for a coding agent working in the repo.
    public var prompt: String?
    /// Informational lines; they never change the status.
    public var notices: [String]?

    public var noticeLines: [String] { notices ?? [] }

    public var displayName: String { agentDisplayName(id) }

    public var failingChecks: [DoctorCheck] { checks.filter { !$0.ok } }

    /// "memory, chat sessions" for the kinds that are paused.
    public var pausedText: String {
        paused.map { kind in
            switch kind {
            case Item.memoryKind: "memory"
            case Item.instructionKind: "instructions"
            case Item.sessionKind: "chat sessions"
            case Item.skillKind: "skills"
            default: kind
            }
        }.joined(separator: ", ")
    }

    /// "Codex updated 0.167.0 → 0.170.0, all checks pass"
    public var updatedLine: String {
        switch (previousVersion, version) {
        case let (old?, new?) where old != new:
            "\(displayName) updated \(old) → \(new), all checks pass"
        default:
            "\(displayName) updated, all checks pass"
        }
    }
}

public struct DoctorDocument: Codable, Sendable {
    public var agents: [DoctorAgent]

    public var needingAction: [DoctorAgent] { agents.filter { $0.status == .actionRequired } }

    public var updated: [DoctorAgent] { agents.filter { $0.status == .updated } }

    public var hasNotices: Bool { !needingAction.isEmpty || !updated.isEmpty }
}
