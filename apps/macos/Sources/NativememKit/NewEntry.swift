import Foundation

/// What the New sheet collects. A project entry is memory; a global entry is an instruction section
/// whose heading is `description`.
public struct NewEntryDraft: Equatable, Sendable {
    public static let categories = ["user", "feedback", "project", "reference"]

    public var projectPath: String?
    public var isGlobal: Bool
    public var agent: String
    public var name = ""
    public var description = ""
    public var category: String?
    public var body = ""

    public init(agent: String, isGlobal: Bool = true, projectPath: String? = nil) {
        self.agent = agent
        self.isGlobal = isGlobal
        self.projectPath = projectPath
    }

    public var kind: String { isGlobal ? Item.instructionKind : Item.memoryKind }

    /// Only a project entry travels with a path.
    public var project: String? { isGlobal ? nil : projectPath }

    public var document: EditDocument {
        isGlobal
            ? EditDocument(name: heading, description: heading, category: nil, body: body)
            : EditDocument(name: trimmed(name), description: trimmed(description), category: category, body: body)
    }

    public var isValid: Bool {
        if isGlobal { return !heading.isEmpty }
        return !trimmed(name).isEmpty && !trimmed(description).isEmpty && !(projectPath ?? "").isEmpty
    }

    private var heading: String { trimmed(description) }

    private func trimmed(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
