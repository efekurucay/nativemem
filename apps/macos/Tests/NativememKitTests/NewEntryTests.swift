import Foundation
import Testing
@testable import NativememKit

@Suite struct NewEntryTests {
    @Test func decodesAddResult() throws {
        let result = try decodeContract(AddResult.self, from: fixture("add"))
        #expect(result.agent == "pi")
        #expect(result.nativeId == "app-1a2b3c4d/feedback-tests.md")
        #expect(result.path.hasSuffix("app-1a2b3c4d/feedback-tests.md"))
        #expect(result.hash == "ef56")
        #expect(result.backupDir.hasSuffix("20261110T062319Z"))
    }

    @Test func projectDraftIsMemoryWithItsPath() throws {
        var draft = NewEntryDraft(agent: "claude", isGlobal: false, projectPath: "/Users/x/app")
        #expect(!draft.isValid)
        draft.name = " tests "
        draft.description = "Run the fast suite first"
        draft.category = "feedback"
        draft.body = "body"
        #expect(draft.isValid)
        #expect(draft.kind == "memory")
        #expect(draft.project == "/Users/x/app")
        #expect(draft.document == EditDocument(
            name: "tests", description: "Run the fast suite first", category: "feedback", body: "body"))
    }

    @Test func projectDraftNeedsAPath() {
        var draft = NewEntryDraft(agent: "claude", isGlobal: false)
        draft.name = "n"
        draft.description = "d"
        #expect(!draft.isValid)
        draft.projectPath = "/Users/x/app"
        #expect(draft.isValid)
    }

    @Test func globalDraftIsAnInstructionSectionWithoutAPath() throws {
        var draft = NewEntryDraft(agent: "codex", isGlobal: true, projectPath: "/Users/x/app")
        draft.name = "ignored"
        draft.category = "user"
        #expect(!draft.isValid)
        draft.description = "  Be terse "
        draft.body = "Short answers."
        #expect(draft.isValid)
        #expect(draft.kind == "instruction")
        #expect(draft.project == nil)
        #expect(draft.document == EditDocument(
            name: "Be terse", description: "Be terse", category: nil, body: "Short answers."))
        let encoded = try #require(String(data: JSONEncoder().encode(draft.document), encoding: .utf8))
        #expect(encoded.contains("\"category\":null"))
    }

    @Test func defaultAgentHoldsTheMostOriginalsAndTheFirstWinsATie() throws {
        var status = try decodeContract(StatusDocument.self, from: fixture("status_four_agents"))
        let counts = status.agents.map(\.originals)
        let first = try #require(status.agents.first { $0.originals == counts.max() }?.id)
        #expect(status.agentWithMostOriginals == first)
        for index in status.agents.indices { status.agents[index].originals = 2 }
        #expect(status.agentWithMostOriginals == status.agents[0].id)
        status.agents[2].originals = 5
        #expect(status.agentWithMostOriginals == status.agents[2].id)
        status.agents = []
        #expect(status.agentWithMostOriginals == nil)
    }
}
