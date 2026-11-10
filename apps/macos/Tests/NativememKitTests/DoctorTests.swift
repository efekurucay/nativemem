import Foundation
import Testing
@testable import NativememKit

@Suite struct DoctorTests {
    private func document(_ name: String = "doctor") throws -> DoctorDocument {
        try decodeContract(DoctorDocument.self, from: fixture(name))
    }

    @Test func decodesEveryStatus() throws {
        let doctor = try document()
        #expect(doctor.agents.map(\.status) == [.ok, .actionRequired, .notInstalled, .updated])
        #expect(doctor.agents.map(\.installed) == [true, true, false, true])
        #expect(doctor.agents[1].previousVersion == "0.167.0")
        #expect(doctor.agents[0].previousVersion == nil)
        #expect(doctor.agents[2].version == nil)
    }

    @Test func actionRequiredCarriesChecksPausedKindsAndPrompt() throws {
        let agent = try document().agents[1]
        #expect(agent.displayName == "Codex")
        #expect(agent.paused == ["session"])
        #expect(agent.pausedText == "chat sessions")
        #expect(agent.checks.count == 2)
        #expect(agent.failingChecks.map(\.id) == ["codex.threads_columns"])
        let check = agent.failingChecks[0]
        #expect(check.featureKinds == ["session"])
        #expect(check.expected.hasPrefix("43 columns"))
        #expect(check.observed.contains("added: brand_new"))
        #expect(!check.accepted)
        #expect(agent.prompt?.contains("/Users/x/nativemem") == true)
    }

    @Test func noticesCountOnlyAgentsThatNeedAction() throws {
        let doctor = try document()
        #expect(doctor.needingAction.map(\.id) == ["codex"])
        #expect(doctor.updated.map(\.id) == ["pi"])
        #expect(doctor.hasNotices)
        let quiet = try document("doctor_ok")
        #expect(quiet.needingAction.isEmpty && quiet.updated.isEmpty)
        #expect(!quiet.hasNotices)
        #expect(quiet.agents[0].prompt == nil)
    }

    @Test func noticesAreInformationalAndOptional() throws {
        let doctor = try document()
        #expect(doctor.agents[0].noticeLines.count == 1)
        #expect(doctor.agents[0].noticeLines[0].hasPrefix("transcripts older than 30 days"))
        #expect(doctor.agents[0].status == .ok)
        #expect(!doctor.needingAction.contains { $0.id == "claude" })
        #expect(doctor.agents[1].noticeLines.isEmpty)
    }

    @Test func updatedLineNamesTheVersionChange() throws {
        let doctor = try document()
        #expect(doctor.updated[0].updatedLine == "Pi updated 1.5.0 → 1.6.0, all checks pass")
        var agent = doctor.updated[0]
        agent.previousVersion = nil
        #expect(agent.updatedLine == "Pi updated, all checks pass")
    }

    @Test func pausedKindsReadAsWords() throws {
        var agent = try document().agents[1]
        agent.paused = ["memory", "instruction", "session"]
        #expect(agent.pausedText == "memory, instructions, chat sessions")
    }

    @Test func statusDocumentIgnoresTheAdditiveCompatSummary() throws {
        let json = """
        { "agents": [ { "id": "claude", "originals": 1, "replicas": 0, "sessions": 0, "automated_sessions": 0, "skipped": [] } ],
          "ledger_items": 1,
          "compat": { "action_required": 1, "updated": 0, "agents": [ { "id": "claude", "status": "action_required" } ] } }
        """
        let status = try decodeContract(StatusDocument.self, from: Data(json.utf8))
        #expect(status.agents.count == 1)
    }
}
