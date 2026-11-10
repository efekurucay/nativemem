import Foundation

/// Chat sessions, folded like memory: a copy sits under its original.
public struct SessionIndex: Sendable {
    public let sessions: [LogicalItem]

    public static let empty = SessionIndex(entries: [])

    public init(entries all: [Item]) {
        sessions = LogicalItem.fold(all.filter { $0.kind == Item.sessionKind })
    }

    /// Sessions of a folder and below, newest first; the query matches the title.
    public func sessions(in selection: SidebarSelection, matching query: String = "") -> [LogicalItem] {
        sessions
            .filter { MemoryIndex.contains($0.project.path, in: selection) && $0.matchesTitle(query) }
            .sorted {
                let (lhs, rhs) = ($0.primary.session?.updatedMs ?? 0, $1.primary.session?.updatedMs ?? 0)
                if lhs != rhs { return lhs > rhs }
                return ($0.primary.description ?? "").localizedStandardCompare($1.primary.description ?? "") == .orderedAscending
            }
    }

    /// One section per project when the selection spans several, like memory; newest first inside each.
    public func sections(for selection: SidebarSelection, matching query: String = "") -> [ItemSection] {
        ItemSection.group(sessions(in: selection, matching: query), for: selection)
    }

    public func item(id: String) -> LogicalItem? {
        sessions.first { $0.id == id }
    }
}

extension LogicalItem {
    /// A session's title is its item description.
    public var title: String { primary.description ?? "" }

    func matchesTitle(_ query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return query.isEmpty || title.localizedCaseInsensitiveContains(query)
    }

    /// OpenCode keeps sessions in a database; the other agents keep one file per session.
    public var storedAsFile: Bool { primary.agent != "opencode" }
}
