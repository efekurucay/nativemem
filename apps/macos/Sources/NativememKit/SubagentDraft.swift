import Foundation

/// One subagent as `subagent add` and `subagent edit` read it from stdin.
public struct SubagentDocument: Encodable, Equatable, Sendable {
    public var name: String
    public var harness: String
    public var model: String
    public var description: String
    public var instructions: String?
    public var timeoutSecs: Int
    public var args: [String]

    private enum CodingKeys: String, CodingKey {
        case name, harness, model, description, instructions, args
        case timeoutSecs = "timeout_secs"
    }

    func encoded() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(self)
    }
}

/// What the New subagent and Edit sheet collects: validation and the document for the CLI.
public struct SubagentDraft: Equatable, Sendable {
    public static let defaultTimeoutText = "15"

    public let isEditing: Bool
    public var name = ""
    public var harness: String
    public var model = ""
    public var description = ""
    public var instructions = ""
    /// Minutes, as typed; a fraction is allowed.
    public var timeoutText = SubagentDraft.defaultTimeoutText
    /// One argument per line.
    public var argumentLines = ""

    public init(harness: String = "") {
        isEditing = false
        self.harness = harness
    }

    public init(editing subagent: Subagent) {
        isEditing = true
        name = subagent.name
        harness = subagent.harness
        model = subagent.model
        description = subagent.description
        instructions = subagent.instructions ?? ""
        timeoutText = Self.minutesText(seconds: subagent.timeoutSecs)
        argumentLines = subagent.args.joined(separator: "\n")
    }

    private static func minutesText(seconds: Int) -> String {
        if seconds % 60 == 0 { return String(seconds / 60) }
        return String(format: "%g", Double(seconds) / 60)
    }

    public var trimmedName: String { name.trimmed }

    public var arguments: [String] {
        argumentLines.split(separator: "\n", omittingEmptySubsequences: true)
            .map(String.init)
            .filter { !$0.trimmed.isEmpty }
    }

    /// The timeout in seconds; nil when the text is not a positive number of minutes.
    public var timeoutSecs: Int? {
        let text = timeoutText.trimmed.replacingOccurrences(of: ",", with: ".")
        guard let minutes = Double(text), minutes.isFinite, minutes > 0 else { return nil }
        let seconds = (minutes * 60).rounded()
        guard seconds >= 1, seconds <= Double(Int32.max) else { return nil }
        return Int(seconds)
    }

    /// The agents the harness can be chosen from: the installed ones, and the current one when editing.
    public func harnessChoices(installed: [String]) -> [String] {
        isEditing && !harness.isEmpty && !installed.contains(harness) ? installed + [harness] : installed
    }

    /// The first thing wrong with the draft, in plain words; nil when it can be saved.
    public var problem: String? {
        if trimmedName.isEmpty { return "Enter a name." }
        if harness.isEmpty { return "Choose an agent." }
        if model.trimmed.isEmpty { return "Enter the model." }
        if description.trimmed.isEmpty { return "Enter a description." }
        if timeoutSecs == nil { return "Enter the timeout in minutes." }
        return nil
    }

    public var isValid: Bool { problem == nil }

    public var document: SubagentDocument {
        SubagentDocument(
            name: trimmedName, harness: harness, model: model.trimmed, description: description.trimmed,
            instructions: instructions.trimmed.isEmpty ? nil : instructions.trimmed,
            timeoutSecs: timeoutSecs ?? 0, args: arguments)
    }
}
