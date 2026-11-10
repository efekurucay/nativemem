import Foundation

/// The `nativemem subagent` commands. Names follow `--`, because a name can begin with "-".
extension NativememCLI {
    func subagentList() async throws -> [Subagent] {
        try await run(["subagent", "list", "--json"], as: SubagentListDocument.self).subagents
    }

    func subagentAdd(_ document: SubagentDocument) async throws -> SubagentAdded {
        try await run(["subagent", "add", "--json"], stdin: try document.encoded())
    }

    func subagentEdit(_ name: String, document: SubagentDocument) async throws -> SubagentEdited {
        try await run(["subagent", "edit", "--json", "--", name], stdin: try document.encoded())
    }

    func subagentRemove(_ name: String) async throws -> SubagentRemoved {
        try await run(["subagent", "remove", "--json", "--", name])
    }

    /// Runs the subagent once; the prompt travels on stdin. Cancelling the task ends the command, and the
    /// command ends the harness it started. A run that fails is a result with `ok` false, not a thrown error.
    func subagentTry(_ name: String, prompt: String, folder: String?) async throws -> SubagentRunResult {
        var arguments = ["subagent", "run", "--prompt-file", "-", "--json"]
        if let folder { arguments.append("--cwd=\(folder)") }
        arguments += ["--", name]
        return try await run(arguments, stdin: Data(prompt.utf8))
    }
}
