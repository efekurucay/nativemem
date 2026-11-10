import Foundation
import Testing
@testable import NativememKit

@Suite struct SubagentDraftTests {
    private func json(_ draft: SubagentDraft) throws -> [String: Any] {
        let data = try draft.document.encoded()
        return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func filled() -> SubagentDraft {
        var draft = SubagentDraft(harness: "claude")
        draft.name = " code-review "
        draft.model = " sonnet "
        draft.description = " Reviews a diff "
        return draft
    }

    @Test func anEmptyDraftAsksForEachFieldInTurn() {
        var draft = SubagentDraft()
        #expect(draft.problem == "Enter a name.")
        draft.name = "scout"
        #expect(draft.problem == "Choose an agent.")
        draft.harness = "pi"
        #expect(draft.problem == "Enter the model.")
        draft.model = "m"
        #expect(draft.problem == "Enter a description.")
        draft.description = "Finds code"
        #expect(draft.isValid)
    }

    @Test func theTimeoutIsMinutesAndBecomesSeconds() {
        var draft = filled()
        #expect(draft.timeoutSecs == 900)
        draft.timeoutText = " 2 "
        #expect(draft.timeoutSecs == 120)
        draft.timeoutText = "1.5"
        #expect(draft.timeoutSecs == 90)
        draft.timeoutText = "0,5"
        #expect(draft.timeoutSecs == 30)
        for bad in ["", "abc", "0", "-3", "inf", "0.0001"] {
            draft.timeoutText = bad
            #expect(draft.timeoutSecs == nil, "\(bad)")
            #expect(draft.problem == "Enter the timeout in minutes.")
        }
    }

    @Test func theDocumentIsTrimmedAndOmitsEmptyInstructions() throws {
        var draft = filled()
        draft.argumentLines = "--allowed-tools\n\n  \nRead"
        let document = try json(draft)
        #expect(document["name"] as? String == "code-review")
        #expect(document["harness"] as? String == "claude")
        #expect(document["model"] as? String == "sonnet")
        #expect(document["description"] as? String == "Reviews a diff")
        #expect(document["timeout_secs"] as? Int == 900)
        #expect(document["args"] as? [String] == ["--allowed-tools", "Read"])
        #expect(document["instructions"] == nil)
        draft.instructions = "\nBe terse.\n"
        #expect(try json(draft)["instructions"] as? String == "Be terse.")
    }

    @Test func anEditStartsFromTheDefinition() throws {
        let subagent = Subagent(
            name: "scout", harness: "pi", model: "m", description: "Finds code", instructions: "Be terse.",
            timeoutSecs: 90, args: ["--a", "b"])
        let draft = SubagentDraft(editing: subagent)
        #expect(draft.isEditing && draft.isValid)
        #expect(draft.timeoutText == "1.5" && draft.timeoutSecs == 90)
        #expect(draft.argumentLines == "--a\nb" && draft.instructions == "Be terse.")
        let document = try json(draft)
        #expect(document["name"] as? String == "scout")
        #expect(document["timeout_secs"] as? Int == 90)
        #expect(SubagentDraft(editing: Subagent(name: "a", harness: "pi", model: "m", description: "d")).timeoutText == "15")
    }

    @Test func onlyInstalledAgentsAreOffered() {
        let installed = ["claude", "pi"]
        #expect(SubagentDraft(harness: "claude").harnessChoices(installed: installed) == installed)
        let codex = SubagentDraft(editing: Subagent(name: "a", harness: "codex", model: "m", description: "d"))
        #expect(codex.harnessChoices(installed: installed) == ["claude", "pi", "codex"])
        let pi = SubagentDraft(editing: Subagent(name: "a", harness: "pi", model: "m", description: "d"))
        #expect(pi.harnessChoices(installed: installed) == installed)
    }
}
