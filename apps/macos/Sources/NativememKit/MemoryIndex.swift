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

    /// Folds each copy into its original; a copy whose original is missing stands alone.
    static func fold(_ entries: [Item]) -> [LogicalItem] {
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
        return logical
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

    /// Chat sessions are not memory; they are listed by `SessionIndex` only.
    public init(entries all: [Item]) {
        items = LogicalItem.fold(all.filter { $0.kind != Item.sessionKind })
    }

    /// Skills are listed under their own sidebar row, not under Global.
    public var skills: [LogicalItem] {
        items.filter { $0.primary.isSkill }
    }

    /// Largest first; ties by path so the order is stable.
    public var projects: [Project] {
        Dictionary(grouping: items.filter { !$0.primary.isSkill }, by: \.project)
            .map { Project(key: $0.key, count: $0.value.count) }
            .sorted { ($0.count, $1.key.path ?? "") > ($1.count, $0.key.path ?? "") }
    }

    /// Items of a sidebar selection; a folder includes everything below it.
    public func items(in selection: SidebarSelection, matching query: String = "") -> [LogicalItem] {
        items
            .filter { Self.holds($0, in: selection) && $0.matches(query) }
            .sorted { $0.primary.name.localizedStandardCompare($1.primary.name) == .orderedAscending }
    }

    /// One section per project when the selection spans several, headed by the path relative to the selection.
    public func sections(for selection: SidebarSelection, matching query: String = "") -> [ItemSection] {
        ItemSection.group(items(in: selection, matching: query), for: selection)
    }

    public func item(id: String) -> LogicalItem? {
        items.first { $0.id == id }
    }

    private static func holds(_ item: LogicalItem, in selection: SidebarSelection) -> Bool {
        if selection == .skills { return item.primary.isSkill }
        return !item.primary.isSkill && contains(item.project.path, in: selection)
    }

    static func contains(_ path: String?, in selection: SidebarSelection) -> Bool {
        switch selection {
        case .overview, .skills, .mcp, .subagents: false
        case .global: path == nil
        case .folderOnly(let root): path == root
        case .folder(let root):
            if let path { path == root || path.hasPrefix(root == "/" ? "/" : root + "/") } else { false }
        }
    }
}

public struct ItemSection: Identifiable, Hashable, Sendable {
    public var id: String
    public var header: String?
    public var items: [LogicalItem]
}

extension ItemSection {
    /// Groups already ordered items by project, keeping their order inside each section. A selection that
    /// spans one project gets one headerless section; the selected folder's own items come first.
    static func group(_ matched: [LogicalItem], for selection: SidebarSelection) -> [ItemSection] {
        let grouped = Dictionary(grouping: matched, by: { $0.project.path ?? "" })
        guard grouped.count > 1, case .folder(let root) = selection else {
            return matched.isEmpty ? [] : [ItemSection(id: "", header: nil, items: matched)]
        }
        return grouped
            .map { ItemSection(id: $0.key, header: relativeHeader($0.key, to: root), items: $0.value) }
            .sorted {
                if $0.id == root { return true }
                if $1.id == root { return false }
                return $0.id.localizedStandardCompare($1.id) == .orderedAscending
            }
    }

    private static func relativeHeader(_ path: String, to root: String) -> String {
        if path == root { return "This folder" }
        return String(path.dropFirst(root == "/" ? 1 : root.count + 1))
    }
}
