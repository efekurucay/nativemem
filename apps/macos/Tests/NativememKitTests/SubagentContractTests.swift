import Foundation
import Testing
@testable import NativememKit

@Suite struct SubagentContractTests {
    private func subagents() throws -> [Subagent] {
        try decodeContract(SubagentListDocument.self, from: fixture("subagent_list")).subagents
    }

    @Test func decodesTheList() throws {
        let list = try subagents()
        #expect(list.map(\.name) == ["code-review", "scout"])
        let review = list[0]
        #expect(review.harness == "claude" && review.harnessName == "Claude Code")
        #expect(review.model == "sonnet" && review.description == "Reviews a diff for bugs")
        #expect(review.instructions == "Be terse." && review.timeoutSecs == 900)
        #expect(review.args == ["--allowed-tools", "Read"])
        let scout = list[1]
        #expect(scout.harnessName == "Pi" && scout.model == "anthropic/claude-haiku-4-5")
        #expect(scout.instructions == nil && scout.args.isEmpty)
    }

    @Test func harnessLabelsFollowTheAgentNames() {
        let names = ["claude", "codex", "opencode", "pi"].map {
            Subagent(name: "a", harness: $0, model: "m", description: "d").harnessName
        }
        #expect(names == ["Claude Code", "Codex", "OpenCode", "Pi"])
    }

    @Test func timeoutReadsInMinutesWhenItCan() {
        func text(_ seconds: Int) -> String {
            Subagent(name: "a", harness: "pi", model: "m", description: "d", timeoutSecs: seconds).timeoutText
        }
        #expect(text(900) == "15 minutes")
        #expect(text(60) == "1 minute")
        #expect(text(90) == "90 seconds")
    }

    @Test func decodesTheManagementAnswers() throws {
        #expect(try decodeContract(SubagentAdded.self, from: fixture("subagent_add")).added == "scout")
        #expect(try decodeContract(SubagentEdited.self, from: fixture("subagent_edit")).edited == "scout")
        #expect(try decodeContract(SubagentRemoved.self, from: fixture("subagent_remove")).removed == "scout")
    }

    @Test func decodesTheCliErrors() throws {
        for (name, code) in [("exists", "exists"), ("not_installed", "not_installed"), ("invalid", "invalid")] {
            let error = try decodeContract(ErrorDocument.self, from: fixture("subagent_error_\(name)"))
            #expect(error.code == code)
            #expect(!error.error.isEmpty)
        }
    }

    @Test func decodesARunThatAnswered() throws {
        let result = try decodeContract(SubagentRunResult.self, from: fixture("subagent_run_ok"))
        #expect(result.ok && result.text == "pi saw: Summarise the repo")
        #expect(result.exitCode == 0 && result.error == nil)
        #expect(result.durationMs == 66)
    }

    @Test func decodesARunThatFailed() throws {
        let result = try decodeContract(SubagentRunResult.self, from: fixture("subagent_run_failed"))
        #expect(!result.ok && result.text.isEmpty && result.exitCode == 4)
        #expect(result.error?.hasPrefix("claude exited with code 4") == true)
    }

    @Test func aKilledRunHasNoExitCode() throws {
        let json = #"{"ok":false,"text":"","exit_code":null,"duration_ms":120000,"error":"timed out"}"#
        let result = try decodeContract(SubagentRunResult.self, from: Data(json.utf8))
        #expect(result.exitCode == nil && result.error == "timed out")
    }

    @Test func durationsReadInTheUnitThatFits() {
        func text(_ milliseconds: Int) -> String {
            SubagentRunResult(ok: true, durationMs: milliseconds).durationText
        }
        #expect(text(66) == "66 ms")
        #expect(text(8421) == "8.4 s")
        #expect(text(125_000) == "2 min 5 s")
    }
}
