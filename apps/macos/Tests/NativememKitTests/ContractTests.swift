import Foundation
import Testing
@testable import NativememKit

func fixture(_ name: String) throws -> Data {
    let url = try #require(Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Fixtures"))
    return try Data(contentsOf: url)
}

func fixtureItems() throws -> [Item] {
    try decodeContract(ItemsDocument.self, from: fixture("items")).items
}

@Suite struct ContractTests {
    @Test func decodesStatus() throws {
        let status = try decodeContract(StatusDocument.self, from: fixture("status"))
        #expect(status.ledgerItems == 399)
        #expect(status.agents.count == 2)
        #expect(status.logicalItems == 399)
        #expect(status.agentsInSync == 2)
        #expect(status.agents[0].skipped == [SkippedEntry(label: "-Users-x-old", reason: "no cwd in jsonl ...")])
        #expect(status.agents[1].replicas == 399)
    }

    @Test func decodesItems() throws {
        let items = try fixtureItems()
        #expect(items.count == 3)
        #expect(items[0].nativeId == "-Users-x-app/feedback_tests.md")
        #expect(items[0].scope == Scope(type: "project", path: "/Users/x/app"))
        #expect(items[0].replicaOf == nil)
        #expect(items[1].replicaOf == ItemRef(origin: "claude", nativeId: "-Users-x-app/feedback_tests.md"))
        #expect(items[2].scope == Scope(type: "global"))
        #expect(items[2].category == nil)
    }

    @Test func kindDefaultsToMemoryWhenAbsent() throws {
        let items = try fixtureItems()
        #expect(items[0].kind == "memory")
        #expect(items[2].kind == "memory")
        #expect(!items[2].isInstruction)
    }

    @Test func decodesInstructionItems() throws {
        let items = try decodeContract(ItemsDocument.self, from: fixture("instruction_items")).items
        #expect(items.map(\.kind) == ["instruction", "instruction"])
        #expect(items.allSatisfy { $0.isInstruction })
        #expect(items[0].category == nil)
        #expect(items[1].replicaOf == ItemRef(origin: "claude", nativeId: "CLAUDE.md#style"))
    }

    @Test func decodesSyncPlan() throws {
        let plan = try decodeContract(SyncPlan.self, from: fixture("sync"))
        #expect(!plan.applied)
        #expect(plan.actions.first?.op == .add)
        #expect(plan.actions.first?.origin?.origin == "claude")
        #expect(plan.notes == [SyncNote(kind: .conflict, message: "both changed")])
        #expect(plan.summary.conflicts == 1)
    }

    @Test func decodesDelete() throws {
        let result = try decodeContract(DeleteResult.self, from: fixture("delete"))
        #expect(result.deleted.agent == "claude")
        #expect(result.backupDir.hasSuffix("20261110T062319Z"))
    }

    @Test func decodesEditResult() throws {
        let result = try decodeContract(EditResult.self, from: fixture("edit"))
        #expect(result.edited == EntryRef(agent: "claude", nativeId: "-Users-x-app/feedback_tests.md"))
        #expect(result.hash == "cd34")
        #expect(result.backupDir.hasSuffix("20261110T062319Z"))
    }

    @Test func editDocumentAlwaysWritesCategoryKey() throws {
        let withCategory = EditDocument(name: "n", description: "d", category: "feedback", body: "b")
        let encoded = try #require(String(data: JSONEncoder().encode(withCategory), encoding: .utf8))
        #expect(encoded.contains("\"category\":\"feedback\""))

        let withNil = EditDocument(name: "n", description: "d", category: nil, body: "b")
        let nilEncoded = try #require(String(data: JSONEncoder().encode(withNil), encoding: .utf8))
        #expect(nilEncoded.contains("\"category\":null"))
    }

    @Test func decodesErrorDocument() throws {
        let failure = try decodeContract(ErrorDocument.self, from: fixture("error"))
        #expect(failure.code == "mass_delete")
        #expect(CLIError.failed(message: failure.error, code: failure.code).isMassDelete)
    }

    @Test func locatesBinaryInSpecOrder() {
        let home = URL(fileURLWithPath: "/h")
        let exeDir = URL(fileURLWithPath: "/app/Contents/MacOS")
        func locate(_ present: Set<String>, saved: String? = nil, env: [String: String] = [:]) -> String? {
            NativememCLI.locate(savedPath: saved, environment: env, executableDirectory: exeDir, home: home,
                                isExecutable: { present.contains($0) })?.path
        }
        let all: Set = ["/s", "/e", "/app/Contents/MacOS/nativemem-cli", "/h/.cargo/bin/nativemem",
                        "/usr/local/bin/nativemem", "/opt/homebrew/bin/nativemem"]
        #expect(locate(all, saved: "/s", env: ["NATIVEMEM_BIN": "/e"]) == "/s")
        #expect(locate(all.subtracting(["/s"]), saved: "/s", env: ["NATIVEMEM_BIN": "/e"]) == "/e")
        #expect(locate(all.subtracting(["/s", "/e"])) == "/app/Contents/MacOS/nativemem-cli")
        #expect(locate(["/usr/local/bin/nativemem", "/opt/homebrew/bin/nativemem"]) == "/usr/local/bin/nativemem")
        #expect(locate([]) == nil)
    }
}
