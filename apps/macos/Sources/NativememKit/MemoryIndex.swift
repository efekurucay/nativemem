import Foundation

public func agentDisplayName(_ id: String) -> String {
    switch id {
    case "claude": "Claude Code"
    case "codex": "Codex"
    case "opencode": "OpenCode"
    case "pi": "Pi"
    default: id
    }
}

/// Agents that only keep global instruction sections, never project memory.
public func agentHoldsMemory(_ id: String) -> Bool {
    id != "opencode" && id != "pi"
}

/// A project path, or nil for global items.
public struct ProjectKey: Hashable, Sendable {
    public var path: String?

    public init(path: String?) {
        self.path = path
    }

    public init(scope: Scope) {
        self.path = scope.type == "project" ? scope.path : nil
    }

    public var title: String {
        guard let path else { return "Global" }
        let last = (path as NSString).lastPathComponent
        return last.isEmpty ? path : last
    }
}

public struct Project: Hashable, Sendable, Identifiable {
    public var key: ProjectKey
    public var count: Int
    public var id: ProjectKey { key }
}

public struct LogicalItem: Hashable, Sendable, Identifiable {
    /// The original entry, or the only entry when its original is missing.
    public var primary: Item
    public var copies: [Item]
    public var originalMissing: Bool

    public var id: String { "\(primary.agent)|\(primary.nativeId)" }
    public var project: ProjectKey { ProjectKey(scope: primary.scope) }

    public struct Residence: Hashable, Sendable {
        public var agent: String
        public var label: String
    }

    public var livesIn: [Residence] {
        let first = Residence(agent: primary.agent, label: originalMissing ? "copy (original missing)" : "original")
        return [first] + copies.map { Residence(agent: $0.agent, label: "copy") }
    }

    public func matches(_ query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.isEmpty { return true }
        return [primary.name, primary.description ?? "", primary.body].contains {
            $0.localizedCaseInsensitiveContains(query)
        }
    }
}

public struct MemoryIndex: Sendable {
    public let items: [LogicalItem]

    public static let empty = MemoryIndex(entries: [])

    /// Chat sessions are not memory; they never reach the UI.
    public init(entries all: [Item]) {
        let entries = all.filter { $0.kind != Item.sessionKind }
        var originals: [String: Int] = [:]
        var logical: [LogicalItem] = []
        for entry in entries where entry.replicaOf == nil {
            originals["\(entry.agent)|\(entry.nativeId)"] = logical.count
            logical.append(LogicalItem(primary: entry, copies: [], originalMissing: false))
        }
        for entry in entries {
            guard let origin = entry.replicaOf else { continue }
            if let position = originals["\(origin.origin)|\(origin.nativeId)"] {
                logical[position].copies.append(entry)
            } else {
                logical.append(LogicalItem(primary: entry, copies: [], originalMissing: true))
            }
        }
        items = logical
    }

    /// Largest first; ties by path so the order is stable.
    public var projects: [Project] {
        Dictionary(grouping: items, by: \.project)
            .map { Project(key: $0.key, count: $0.value.count) }
            .sorted { ($0.count, $1.key.path ?? "") > ($1.count, $0.key.path ?? "") }
    }

    /// Items of a sidebar selection; a folder includes everything below it.
    public func items(in selection: SidebarSelection, matching query: String = "") -> [LogicalItem] {
        items
            .filter { Self.contains($0.project.path, in: selection) && $0.matches(query) }
            .sorted { $0.primary.name.localizedStandardCompare($1.primary.name) == .orderedAscending }
    }

    /// One section per project when the selection spans several, headed by the path relative to the selection.
    public func sections(for selection: SidebarSelection, matching query: String = "") -> [ItemSection] {
        let matched = items(in: selection, matching: query)
        let grouped = Dictionary(grouping: matched, by: { $0.project.path ?? "" })
        guard grouped.count > 1, case .folder(let root) = selection else {
            return matched.isEmpty ? [] : [ItemSection(id: "", header: nil, items: matched)]
        }
        return grouped
            .map { ItemSection(id: $0.key, header: Self.relativeHeader($0.key, to: root), items: $0.value) }
            .sorted {
                if $0.id == root { return true }
                if $1.id == root { return false }
                return $0.id.localizedStandardCompare($1.id) == .orderedAscending
            }
    }

    public func item(id: String) -> LogicalItem? {
        items.first { $0.id == id }
    }

    private static func contains(_ path: String?, in selection: SidebarSelection) -> Bool {
        switch selection {
        case .overview: false
        case .global: path == nil
        case .folderOnly(let root): path == root
        case .folder(let root):
            if let path { path == root || path.hasPrefix(root == "/" ? "/" : root + "/") } else { false }
        }
    }

    private static func relativeHeader(_ path: String, to root: String) -> String {
        if path == root { return "This folder" }
        return String(path.dropFirst(root == "/" ? 1 : root.count + 1))
    }
}

public struct ItemSection: Identifiable, Hashable, Sendable {
    public var id: String
    public var header: String?
    public var items: [LogicalItem]
}
