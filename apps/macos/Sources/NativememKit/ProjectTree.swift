import Foundation

/// Folder tree of the projects. Pure: the home directory is an input, never read here.
public enum ProjectTree {
    public struct Node: Identifiable, Hashable, Sendable {
        /// Absolute path of the deepest folder the node stands for.
        public var id: String
        public var label: String
        /// Items in this folder and below.
        public var totalCount: Int
        public var children: [Node]
        /// True for the shared parent folder shown as a leaf: its row lists only its own items.
        public var listsOnlyOwnItems: Bool
    }

    private final class Folder {
        let name: String
        let path: String
        var ownCount = 0
        var children: [String: Folder] = [:]

        init(name: String, path: String) {
            self.name = name
            self.path = path
        }

        func child(named name: String) -> Folder {
            if let existing = children[name] { return existing }
            let created = Folder(name: name, path: path + "/" + name)
            children[name] = created
            return created
        }
    }

    public static func build(projects: [(path: String, count: Int)], home: String) -> [Node] {
        let home = home.count > 1 && home.hasSuffix("/") ? String(home.dropLast()) : home
        let root = Folder(name: "", path: "")
        for project in projects {
            var folder = root
            for component in project.path.split(separator: "/") {
                folder = folder.child(named: String(component))
            }
            folder.ownCount += project.count
        }

        var common = root
        while common.ownCount == 0, common.children.count == 1, let only = common.children.values.first {
            common = only
        }

        var rows: [Node] = []
        if common.ownCount > 0 {
            let path = common.path.isEmpty ? "/" : common.path
            rows.append(Node(
                id: path, label: displayPath(path, home: home), totalCount: common.ownCount,
                children: [], listsOnlyOwnItems: true
            ))
        }
        rows += common.children.values.map { node($0, home: home) }.sorted(by: precedes)
        return rows
    }

    /// Keeps nodes whose path contains the text, and their ancestors.
    public static func filter(_ nodes: [Node], matching query: String) -> [Node] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return nodes }
        return nodes.compactMap { node in
            let kept = filter(node.children, matching: query)
            guard node.id.localizedCaseInsensitiveContains(query) || !kept.isEmpty else { return nil }
            var result = node
            result.children = kept
            return result
        }
    }

    public static func displayPath(_ path: String, home: String) -> String {
        if path == home { return "~" }
        if home != "/", path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }

    private static func node(_ folder: Folder, home: String) -> Node {
        var current = folder
        var label = folder.path == home ? "~" : folder.name
        while current.ownCount == 0, current.children.count == 1, let only = current.children.values.first {
            label = only.path == home ? "~" : label + "/" + only.name
            current = only
        }
        let children = current.children.values.map { node($0, home: home) }.sorted(by: precedes)
        return Node(
            id: current.path, label: label,
            totalCount: current.ownCount + children.reduce(0) { $0 + $1.totalCount },
            children: children, listsOnlyOwnItems: false
        )
    }

    private static func precedes(_ lhs: Node, _ rhs: Node) -> Bool {
        if lhs.totalCount != rhs.totalCount { return lhs.totalCount > rhs.totalCount }
        return lhs.label.localizedStandardCompare(rhs.label) == .orderedAscending
    }
}
