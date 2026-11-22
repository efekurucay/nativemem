import Foundation

struct MaintenanceReport: Sendable {
    enum Finding: String, Hashable, Sendable {
        case duplicate = "Duplicate note"
        case differentCopies = "Different copies"
        case unlisted = "Missing from index"
    }

    struct Duplicate: Identifiable, Sendable {
        let notes: [LogicalItem]
        var id: String { "duplicate:\(notes[0].id)" }
    }

    struct IndexIssue: Identifiable, Sendable {
        let index: URL
        let missingTargets: [String]
        let unlistedNotes: [String]
        let indexMissing: Bool
        var id: URL { index }
    }

    let notes: [LogicalItem]
    let duplicates: [Duplicate]
    let differentCopies: [LogicalItem]
    let indexIssues: [IndexIssue]
    let skills: [LogicalItem]
    let findings: [String: [Finding]]

    init(index: MemoryIndex, scope: ProjectKey) {
        self.init(items: index.items.filter { $0.project == scope })
    }

    init(items: [LogicalItem]) {
        let items = items.sorted { ($0.primary.name, $0.id) < ($1.primary.name, $1.id) }
        notes = items.filter { $0.primary.kind == Item.memoryKind }
        let originals = notes.filter { !$0.originalMissing && !Self.body(of: $0).isEmpty }
        duplicates = Dictionary(grouping: originals, by: \.project).values
            .flatMap { project in Dictionary(grouping: project, by: Self.body).values.filter { $0.count > 1 } }
            .map { Duplicate(notes: $0) }
            .sorted { $0.id < $1.id }
        differentCopies = items.filter { entry in
            let hash = entry.primary.hash
            return !hash.isEmpty && entry.copies.contains { !$0.hash.isEmpty && $0.hash != hash }
        }
        indexIssues = Self.checkClaudeIndexes(notes)
        skills = items.filter { $0.primary.isSkill }
        var findings: [String: [Finding]] = [:]
        for group in duplicates {
            for note in group.notes { findings[note.id, default: []].append(.duplicate) }
        }
        for item in differentCopies { findings[item.id, default: []].append(.differentCopies) }
        let unlisted = Set(indexIssues.flatMap { issue in
            issue.unlistedNotes.map { issue.index.deletingLastPathComponent().appendingPathComponent($0).path }
        })
        for note in notes where ([note.primary] + note.copies).contains(where: { unlisted.contains($0.path) }) {
            findings[note.id, default: []].append(.unlisted)
        }
        self.findings = findings
    }

    private static func body(of item: LogicalItem) -> String {
        item.primary.body.replacingOccurrences(of: "\r\n", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func checkClaudeIndexes(_ notes: [LogicalItem]) -> [IndexIssue] {
        let claudeNotes = notes.flatMap { [$0.primary] + $0.copies }.compactMap { item -> URL? in
            guard item.agent == "claude", item.path.hasPrefix("/") else { return nil }
            let file = URL(fileURLWithPath: item.path)
            guard file.pathExtension == "md",
                  file.deletingLastPathComponent().lastPathComponent == "memory" else { return nil }
            return file
        }
        let byDirectory = Dictionary(grouping: claudeNotes) { $0.deletingLastPathComponent() }
        return byDirectory.compactMap { directory, files in
            let manager = FileManager.default
            guard let children = try? manager.contentsOfDirectory(atPath: directory.path) else { return nil }
            let index = directory.appendingPathComponent("MEMORY.md")
            let noteNames = Set(files.map(\.lastPathComponent))
            guard children.contains(index.lastPathComponent) else {
                return IndexIssue(index: index, missingTargets: [], unlistedNotes: noteNames.sorted(), indexMissing: true)
            }
            guard (try? index.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true,
                  let text = try? String(contentsOf: index, encoding: .utf8) else { return nil }
            let links = Set(text.split(whereSeparator: \.isNewline).compactMap(Self.indexTarget))
            let missing = links.subtracting(children).sorted()
            let unlisted = noteNames.subtracting(links).intersection(children).sorted()
            guard !missing.isEmpty || !unlisted.isEmpty else { return nil }
            return IndexIssue(index: index, missingTargets: missing, unlistedNotes: unlisted, indexMissing: false)
        }.sorted { $0.index.path < $1.index.path }
    }

    /// Claude's note index uses local Markdown list links. Ignore external and nested targets.
    private static func indexTarget(_ line: Substring) -> String? {
        let line = line.trimmingCharacters(in: .whitespaces)
        guard line.hasPrefix("- [") || line.hasPrefix("* [") || line.hasPrefix("+ ["),
              let opening = line.range(of: "]("),
              let closing = line[opening.upperBound...].firstIndex(of: ")") else { return nil }
        let raw = String(line[opening.upperBound..<closing])
        let target: String
        if raw.hasPrefix("<"), raw.hasSuffix(">") {
            target = String(raw.dropFirst().dropLast())
        } else {
            target = raw
        }
        guard !target.contains(":"), !target.contains("#"), !target.contains("?"),
              let decoded = target.removingPercentEncoding else { return nil }
        let name = decoded.hasPrefix("./") ? String(decoded.dropFirst(2)) : decoded
        guard name.lowercased().hasSuffix(".md"), name != "MEMORY.md",
              !name.contains("/"), !name.contains("\\") else { return nil }
        return name
    }
}
