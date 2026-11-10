import Foundation
import Testing
@testable import NativememKit

@Suite struct IndexTests {
    @Test func foldsCopiesIntoOriginals() throws {
        let index = MemoryIndex(entries: try fixtureItems())
        #expect(index.items.count == 2)
        let folded = try #require(index.items.first { $0.primary.name == "feedback-tests" })
        #expect(folded.copies.map(\.agent) == ["codex"])
        #expect(!folded.originalMissing)
        #expect(folded.livesIn.map(\.label) == ["original", "copy"])
        #expect(folded.primary.agent == "claude")
    }

    @Test func instructionItemsLiveUnderGlobalAndFoldCopies() throws {
        let entries = try decodeContract(ItemsDocument.self, from: fixture("instruction_items")).items
        let index = MemoryIndex(entries: entries)
        #expect(index.items.count == 1)
        #expect(index.items[0].primary.isInstruction)
        #expect(index.items[0].copies.map(\.agent) == ["codex"])
        #expect(index.items(in: .global).count == 1)
        #expect(index.projects.map(\.key.title) == ["Global"])
        #expect(index.items.contains { $0.project.path == nil })
    }

    @Test func instructionEditDraftKeepsHeadingFields() throws {
        let entries = try decodeContract(ItemsDocument.self, from: fixture("instruction_items")).items
        let item = try #require(MemoryIndex(entries: entries).items.first)
        let document = EditDraft(item: item).document
        #expect(document == EditDocument(name: "style", description: "Style", category: nil, body: "Be terse"))
    }

    @Test func orphanCopyStandsAlone() throws {
        let entries = try fixtureItems().filter { $0.agent != "claude" || $0.scope.type == "global" }
        let index = MemoryIndex(entries: entries)
        let orphan = try #require(index.items.first { $0.primary.agent == "codex" })
        #expect(orphan.originalMissing)
        #expect(orphan.copies.isEmpty)
        #expect(orphan.livesIn == [.init(agent: "codex", label: "copy (original missing)")])
    }

    @Test func groupsByProjectLargestFirst() {
        func item(_ id: String, _ path: String?) -> Item {
            Item(agent: "claude", nativeId: id, path: "/p/\(id)",
                 scope: path.map { Scope(type: "project", path: $0) } ?? Scope(type: "global"), name: id)
        }
        let index = MemoryIndex(entries: [
            item("a", "/w/small"), item("b", "/w/big"), item("c", "/w/big"), item("d", nil),
        ])
        let projects = index.projects
        #expect(projects.first?.key.path == "/w/big")
        #expect(projects.first?.count == 2)
        #expect(projects.map(\.count) == [2, 1, 1])
        #expect(projects.first?.key.title == "big")
        #expect(projects.contains { $0.key.path == nil && $0.key.title == "Global" })
        // A leaf project folder lists only its own items.
        #expect(index.items(in: .folderOnly(path: "/w/big")).count == 2)
    }

    @Test func searchMatchesNameDescriptionAndBody() throws {
        let index = MemoryIndex(entries: try fixtureItems())
        let selection = SidebarSelection.folderOnly(path: "/Users/x/app")
        #expect(index.items(in: selection, matching: "FAST suite").count == 1)
        #expect(index.items(in: selection, matching: "body text").count == 1)
        #expect(index.items(in: selection, matching: "nothing").isEmpty)
    }
}
