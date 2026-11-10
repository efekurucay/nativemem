import Foundation
import Testing
@testable import NativememKit

@Suite struct SessionTests {
    private func entries() throws -> [Item] {
        try decodeContract(ItemsDocument.self, from: fixture("items-sessions")).items
    }

    @Test func decodesSessionItems() throws {
        let items = try entries()
        let session = try #require(items.first { $0.nativeId == "-Users-x-app/0198a1b2.jsonl" })
        #expect(session.kind == "session")
        #expect(session.body.isEmpty)
        #expect(session.description == "Fix login")
        #expect(session.session == SessionStats(createdMs: 1_794_304_800_000, updatedMs: 1_794_308_400_000, turns: 12))
        #expect(session.session?.updated == Date(timeIntervalSince1970: 1_794_308_400))
        #expect(items[0].session == nil)
    }

    @Test func decodesTheTranscript() throws {
        let shown = try decodeContract(SessionTranscript.self, from: fixture("session-show"))
        #expect(shown.agent == "claude")
        #expect(shown.title == "Fix login")
        #expect(shown.cwd == "/Users/x/app")
        #expect(shown.turns.map(\.speaker) == ["You", "Assistant"])
        #expect(shown.turns[1].tsMs == 1_794_304_860_000)
        #expect(MarkdownBlocks.parse(shown.turns[1].text) == [
            .paragraph("Check the **token expiry**:"), .bullet("read the claim"), .bullet("compare it to now"),
        ])
    }

    @Test func memoryViewsStayFreeOfSessions() throws {
        let memory = MemoryIndex(entries: try entries())
        #expect(memory.items.map(\.primary.name) == ["feedback-tests"])
        #expect(memory.projects.map(\.count) == [1])
        let sessions = SessionIndex(entries: try entries())
        #expect(sessions.sessions.allSatisfy { $0.primary.kind == "session" })
    }

    @Test func copiesFoldIntoTheirOriginal() throws {
        let index = SessionIndex(entries: try entries())
        #expect(index.sessions.count == 4)
        let login = try #require(index.sessions.first { $0.title == "Fix login" })
        #expect(login.livesIn.map(\.agent) == ["claude", "pi"])
        #expect(login.livesIn.map(\.label) == ["original", "copy"])
        let orphan = try #require(index.sessions.first { $0.title == "Style the header" })
        #expect(orphan.originalMissing)
        #expect(orphan.livesIn.map(\.label) == ["copy (original missing)"])
    }

    @Test func folderListsItsOwnAndNestedSessionsNewestFirstInSections() throws {
        let index = SessionIndex(entries: try entries())
        let selection = SidebarSelection.folder(path: "/Users/x/app")
        #expect(index.sessions(in: selection).count == 3)
        let sections = index.sections(for: selection)
        #expect(sections.map(\.header) == ["This folder", "api", "web"])
        #expect(sections.map { $0.items.map(\.title) } == [["Fix login"], ["Add pagination"], ["Style the header"]])
        #expect(index.sessions(in: .folder(path: "/Users/x")).map(\.title)
            == ["Add pagination", "Fix login", "Unrelated chat", "Style the header"])
    }

    @Test func aSingleProjectIsOneSectionWithoutHeader() throws {
        let index = SessionIndex(entries: try entries())
        let sections = index.sections(for: .folderOnly(path: "/Users/x/app"))
        #expect(sections.count == 1)
        #expect(sections[0].header == nil)
        #expect(index.sections(for: .global).isEmpty)
        #expect(index.sections(for: .skills).isEmpty)
    }

    @Test func searchMatchesTheTitleOnly() throws {
        let index = SessionIndex(entries: try entries())
        let selection = SidebarSelection.folder(path: "/Users/x")
        #expect(index.sessions(in: selection, matching: "PAGINATION").map(\.title) == ["Add pagination"])
        #expect(index.sessions(in: selection, matching: "0198a1b2").isEmpty, "the id is not searched")
        #expect(index.sessions(in: selection, matching: "nothing").isEmpty)
    }

    @Test func onlyFileBasedStoresCanBeShownInFinder() throws {
        let index = SessionIndex(entries: try entries())
        let byTitle = { (title: String) in index.sessions.first { $0.title == title }?.storedAsFile }
        #expect(byTitle("Fix login") == true)
        #expect(byTitle("Style the header") == true)
        #expect(byTitle("Add pagination") == false)
    }

    @MainActor @Test func sessionsShowOnlyForProjectFolders() {
        let model = AppModel()
        model.requestSidebar(.folder(path: "/Users/x/app"))
        #expect(!model.showsSessions)
        model.requestMode(.sessions)
        #expect(model.showsSessions)
        model.requestSidebar(.global)
        #expect(!model.showsSessions, "every other row lists memory")
        model.requestSidebar(.folderOnly(path: "/Users/x/app"))
        #expect(model.showsSessions)
    }
}
