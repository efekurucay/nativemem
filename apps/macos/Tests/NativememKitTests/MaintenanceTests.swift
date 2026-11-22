import Foundation
import Testing
@testable import NativememKit

@Test func onlyAnEditRecommendationCanSupplyAnEditorDraft() throws {
    let edit = try #require(ReviewAdvice.decode("""
        ```json
        {"decision":"edit","reason":"Preserve the write recovery requirement.","replacement":"Back up before writing."}
        ```
        """))
    #expect(edit.decision == .edit)
    #expect(edit.proposedBody(replacing: "Back up.") == "Back up before writing.")
    #expect(edit.proposedBody(replacing: "Back up before writing.\n") == nil)
    let remove = try #require(ReviewAdvice.decode("""
        {"decision":"remove","reason":"This duplicates the formatter.","replacement":"Unexpected draft."}
        """))
    #expect(remove.proposedBody(replacing: "Use four spaces.") == nil)
    #expect(ReviewAdvice.decode(#"{"decision":"edit","reason":"","replacement":"Text"}"#) == nil)
}

@Test func reviewPromptIncludesEntryTextAndOmitsNativePaths() throws {
    let item = Item(agent: "claude", nativeId: "native-identity", path: "/private/fixture/file.md",
                    scope: Scope(type: "project", path: "/private/project"), name: "Verify recovery",
                    description: "Requires backup", body: "Back up before writing.")
    let prompt = try ReviewAdvice.prompt(for: item)
    let payload = try #require(prompt.split(separator: "\n").last)
    let fields = try JSONDecoder().decode([String: String].self, from: Data(payload.utf8))
    #expect(fields == ["name": "Verify recovery", "description": "Requires backup", "body": "Back up before writing."])
}

@Test func skillAdviceRequestsACompleteSkillIncludingFrontMatter() throws {
    let text = "---\nname: verify-recovery\ndescription: Requires backup\n---\nCheck recovery."
    let item = Item(agent: "pi", nativeId: "verify-recovery", kind: Item.skillKind,
                    path: "/fixture/skills/verify-recovery", scope: Scope(type: "global"),
                    name: "verify-recovery", body: text)
    let prompt = try ReviewAdvice.prompt(for: item)
    #expect(prompt.contains("Preserve its YAML front matter"))
    #expect(prompt.contains("return the complete SKILL.md"))
    let payload = try #require(prompt.split(separator: "\n").last)
    let fields = try JSONDecoder().decode([String: String].self, from: Data(payload.utf8))
    #expect(fields["body"] == text)
}

@Test func aSkippedCardReturnsBeforeUnchangedEntries() throws {
    let scope = Scope(type: "global")
    func item(_ name: String) -> LogicalItem {
        LogicalItem(primary: Item(agent: "claude", nativeId: name, path: "/fixture/\(name)",
                                  scope: scope, name: name, hash: "same"), copies: [], originalMissing: false)
    }
    let entries = [item("kept"), item("skipped")]
    let baseline = try JSONDecoder().decode([ReviewRecord].self, from: Data("""
        [
            {"id":"claude|kept","name":"kept","primaryHash":"same","differingHashes":[]},
            {"id":"claude|skipped","name":"skipped","primaryHash":"same","differingHashes":[],"wasSkipped":true}
        ]
        """.utf8))
    let review = ContextReview(selection: .global, items: entries, report: MaintenanceReport(items: entries), baseline: baseline)
    #expect(review.cards.map(\.record.name) == ["skipped", "kept"])
    #expect(review.cards.allSatisfy { $0.change == nil })
    let updates = ContextReview(selection: .global, items: entries, report: MaintenanceReport(items: entries),
                                baseline: baseline, updatesOnly: true)
    #expect(updates.cards.map(\.record.name) == ["skipped"])
}

@Test func anInterruptedReviewResumesAndReopensChangedCards() throws {
    let scope = Scope(type: "global")
    func item(_ name: String, hash: String = "same") -> LogicalItem {
        LogicalItem(primary: Item(agent: "claude", nativeId: name, path: "/fixture/\(name)",
                                  scope: scope, name: name, hash: hash), copies: [], originalMissing: false)
    }
    let kept = item("kept")
    let edited = item("edited", hash: "saved")
    let visited = item("visited")
    let gone = item("gone")
    let restored = item("restored")
    let pending = item("pending")
    let answers: [String: ContextReview.Answer] = [
        kept.id: .init(decision: .keep, record: ReviewRecord(kept)),
        edited.id: .init(decision: .edit, record: ReviewRecord(edited)),
        visited.id: .init(decision: .skip, record: ReviewRecord(visited)),
        gone.id: .init(decision: .acknowledge, record: nil),
        restored.id: .init(decision: .remove, record: nil),
    ]
    let entries = [kept, edited, item("visited", hash: "changed"), restored, pending]
    let progress = try JSONDecoder().decode([String: ContextReview.Answer].self, from: JSONEncoder().encode(answers))
    let review = ContextReview(selection: .global, items: entries, report: MaintenanceReport(items: entries),
                               baseline: [kept, edited, visited, gone, restored, pending].map { ReviewRecord($0) },
                               progress: progress)
    #expect(review.cards.map(\.id) == [visited.id, restored.id, pending.id])
    #expect(review.cards.first?.change == .changed)
    #expect(Set(review.answers.keys) == [kept.id, edited.id, gone.id])
}

@Test func contextReviewOrdersChangesAndKeepsExistingEntriesInTheDeck() {
    let scope = Scope(type: "global")
    func item(_ name: String, hash: String) -> LogicalItem {
        LogicalItem(primary: Item(agent: "claude", nativeId: name, path: "/fixture/\(name)", scope: scope,
                                  name: name, hash: hash), copies: [], originalMissing: false)
    }
    let stable = item("stable", hash: "same")
    let changed = item("changed", hash: "after")
    let added = item("added", hash: "now")
    let records = [stable, item("changed", hash: "before"), item("removed", hash: "gone")].map { ReviewRecord($0) }
    let entries = [stable, changed, added]
    let report = MaintenanceReport(items: entries)
    let review = ContextReview(selection: .global, items: entries, report: report, baseline: records)
    #expect(review.cards.map(\.record.name) == ["changed", "added", "removed", "stable"])
    #expect(review.cards.map(\.change) == [.changed, .added, .removed, nil])
    #expect(review.cards[2].item == nil)
    let first = ContextReview(selection: .global, items: entries, report: report, baseline: nil)
    #expect(first.cards.allSatisfy { $0.change == nil })
    let updates = ContextReview(selection: .global, items: entries, report: report, baseline: records, updatesOnly: true)
    #expect(updates.cards.map(\.change) == [.changed, .added, .removed])
    let firstUpdates = ContextReview(selection: .global, items: entries, report: report, baseline: nil, updatesOnly: true)
    #expect(!firstUpdates.showsUpdatesOnly)
    #expect(firstUpdates.cards.map(\.id) == first.cards.map(\.id))

    let copy = Item(agent: "codex", nativeId: "stable-copy", path: "/fixture/stable-copy",
                    scope: scope, name: "stable", hash: "edited-copy")
    let diverged = LogicalItem(primary: stable.primary, copies: [copy], originalMissing: false)
    let replicaReview = ContextReview(selection: .global, items: [diverged],
                                      report: MaintenanceReport(items: [diverged]), baseline: [ReviewRecord(stable)])
    #expect(replicaReview.cards.first?.change == .changed)
    let flagged = ContextReview(selection: .global, items: [diverged], report: MaintenanceReport(items: [diverged]),
                                baseline: [ReviewRecord(diverged)], updatesOnly: true)
    #expect(flagged.cards.first?.findings == [.differentCopies])
    #expect(flagged.cards.first?.change == nil)
}

@Test func skillComparisonListsChangedAndMissingPaths() {
    let first = "- 1 first SKILL.md\n- 1 first file with spaces.txt"
    let second = "- 1 other SKILL.md\nx 2 another run.sh"
    #expect(SkillManifest.differences(first, second) == ["SKILL.md", "file with spaces.txt", "run.sh"])
}

@Test func maintenanceGroupsOriginalsAndShowsDifferingLinkedCopies() {
    let project = "/work/app"
    let scope = Scope(type: "project", path: project)
    let claude = Item(agent: "claude", nativeId: "a", path: "/fixture/a.md", scope: scope,
                      name: "Testing", body: "Run tests.\r\n", hash: "original")
    let copy = Item(agent: "pi", nativeId: "copy", path: "/fixture/copy.md", scope: scope,
                    name: "Testing", body: "Run integration tests.",
                    replicaOf: ItemRef(origin: "claude", nativeId: "a"), hash: "different")
    let opencode = Item(agent: "opencode", nativeId: "b", path: "/fixture/b.md", scope: scope,
                        name: "Verify", body: "Run tests.", hash: "another-note")
    let sameTextOtherProject = Item(agent: "pi", nativeId: "elsewhere", path: "/fixture/other.md",
                                    scope: Scope(type: "project", path: "/work/app/tools"),
                                    name: "Elsewhere", body: "Run tests.")
    let skill = Item(agent: "pi", nativeId: "skill", kind: Item.skillKind, path: "/fixture/skill",
                     scope: scope, name: "Check logs")
    let index = MemoryIndex(entries: [claude, copy, opencode, sameTextOtherProject, skill])

    let report = MaintenanceReport(index: index, scope: ProjectKey(path: project))
    #expect(report.notes.map(\.primary.nativeId) == ["a", "b"])
    #expect(report.duplicates.count == 1)
    #expect(report.duplicates.first?.notes.map(\.primary.nativeId) == ["a", "b"])
    #expect(report.differentCopies.map(\.primary.nativeId) == ["a"])
    #expect(report.notes.first?.copies == [copy])
    #expect(report.skills.map(\.primary.nativeId) == ["skill"])
    #expect(report.findings["claude|a"] == [.duplicate, .differentCopies])
    #expect(report.findings["opencode|b"] == [.duplicate])
    let allProjects = MaintenanceReport(items: index.items)
    #expect(allProjects.findings["pi|elsewhere"] == nil)
}

@Test func maintenanceReportsNativeMemoryIndexDiscrepancies() throws {
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: folder) }
    let memory = folder.appendingPathComponent("memory")
    try FileManager.default.createDirectory(at: memory, withIntermediateDirectories: true)
    let project = ProjectKey(path: "/work/app")
    let scope = Scope(type: "project", path: project.path)
    let first = memory.appendingPathComponent("Testing notes.md")
    let second = memory.appendingPathComponent("release.md")
    for path in [first, second] { try "note".write(to: path, atomically: true, encoding: .utf8) }
    let index = MemoryIndex(entries: [first, second].map {
        Item(agent: "claude", nativeId: $0.lastPathComponent, path: $0.path,
             scope: scope, name: $0.lastPathComponent, body: "note")
    })
    let missingIndex = MaintenanceReport(index: index, scope: project).indexIssues.first
    #expect(missingIndex?.indexMissing == true)
    #expect(missingIndex?.unlistedNotes == ["Testing notes.md", "release.md"])

    let memoryIndex = memory.appendingPathComponent("MEMORY.md")
    try """
    - [Testing](./Testing%20notes.md) — indexed
    - [Missing](missing.md)
    - [Nested link](subdir/link.md)
    - [External link](https://example.org/note.md)
    """.write(to: memoryIndex, atomically: true, encoding: .utf8)

    let issue = MaintenanceReport(index: index, scope: project).indexIssues.first
    #expect(issue?.indexMissing == false)
    #expect(issue?.missingTargets == ["missing.md"])
    #expect(issue?.unlistedNotes == ["release.md"])
    #expect(issue?.index == memoryIndex)
}
