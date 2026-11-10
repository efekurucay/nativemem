import Foundation

/// One subagent definition, as `subagent list --json` prints it. `model` is in the harness's own syntax.
public struct Subagent: Codable, Hashable, Sendable, Identifiable {
    public var name: String
    /// `claude`, `codex`, `opencode` or `pi`.
    public var harness: String
    public var model: String
    public var description: String
    public var instructions: String?
    public var timeoutSecs: Int
    public var args: [String]

    public var id: String { name }

    /// "Claude Code", "Codex", "OpenCode" or "Pi".
    public var harnessName: String { agentDisplayName(harness) }

    public init(
        name: String, harness: String, model: String, description: String, instructions: String? = nil,
        timeoutSecs: Int = 900, args: [String] = []
    ) {
        self.name = name
        self.harness = harness
        self.model = model
        self.description = description
        self.instructions = instructions
        self.timeoutSecs = timeoutSecs
        self.args = args
    }

    /// "15 minutes", "1 minute", "90 seconds".
    public var timeoutText: String {
        if timeoutSecs % 60 == 0 {
            let minutes = timeoutSecs / 60
            return "\(minutes) \(minutes == 1 ? "minute" : "minutes")"
        }
        return "\(timeoutSecs) \(timeoutSecs == 1 ? "second" : "seconds")"
    }
}

public struct SubagentListDocument: Codable, Sendable {
    public var subagents: [Subagent]
}

public struct SubagentAdded: Codable, Sendable { public var added: String }
public struct SubagentEdited: Codable, Sendable { public var edited: String }
public struct SubagentRemoved: Codable, Sendable { public var removed: String }

/// `subagent run --json`. On failure `ok` is false, `text` is empty and `error` says why.
public struct SubagentRunResult: Codable, Hashable, Sendable {
    public var ok: Bool
    public var text: String
    /// The harness's exit code; nil when it was killed.
    public var exitCode: Int?
    public var durationMs: Int
    public var error: String?

    public init(ok: Bool, text: String = "", exitCode: Int? = nil, durationMs: Int = 0, error: String? = nil) {
        self.ok = ok
        self.text = text
        self.exitCode = exitCode
        self.durationMs = durationMs
        self.error = error
    }

    /// "66 ms", "8.4 s", "2 min 5 s".
    public var durationText: String {
        if durationMs < 1000 { return "\(durationMs) ms" }
        if durationMs < 60_000 { return String(format: "%.1f s", Double(durationMs) / 1000) }
        let seconds = durationMs / 1000
        return "\(seconds / 60) min \(seconds % 60) s"
    }
}
