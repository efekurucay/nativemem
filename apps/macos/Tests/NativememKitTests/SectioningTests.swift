import Foundation
import Testing
@testable import NativememKit

@Suite struct SectioningTests {
    private func item(_ id: String, _ path: String) -> Item {
        Item(agent: "claude", nativeId: id, path: "/p/\(id)",
             scope: Scope(type: "project", path: path), name: id)
    }

    @Test func sectionsByProjectUnderFolder() throws {
        let index = MemoryIndex(entries: [
            item("x1", "/w/a/x"), item("x2", "/w/a/x"),
            item("y1", "/w/a/y"), item("z1", "/w/b/z"),
        ])
        let sections = index.sections(for: .folder(path: "/w/a"), matching: "")
        // Items under /w/a come from projects /w/a/x and /w/a/y: sectioned, headers are the
        // project paths relative to the selection.
        #expect(sections.count == 2)
        #expect(sections.compactMap(\.header).sorted() == ["x", "y"])
        let xSection = try #require(sections.first { $0.header == "x" })
        #expect(xSection.items.count == 2)
        #expect(sections.flatMap(\.items).count == 3)
    }

    @Test func singleProjectCollapsesToOneSection() {
        let index = MemoryIndex(entries: [
            item("x1", "/w/a/x"), item("x2", "/w/a/x"),
        ])
        let sections = index.sections(for: .folder(path: "/w/a"), matching: "")
        #expect(sections.count == 1)
        #expect(sections[0].header == nil)
        #expect(sections[0].items.count == 2)
    }

    @Test func folderOnlyListsExactProject() {
        let index = MemoryIndex(entries: [
            item("x1", "/w/a/x"), item("y1", "/w/a/y"),
        ])
        let sections = index.sections(for: .folderOnly(path: "/w/a/x"), matching: "")
        #expect(sections.count == 1)
        #expect(sections[0].items.count == 1)
    }

    @Test func ownItemsGetRelativeHeader() {
        let index = MemoryIndex(entries: [
            item("own", "/w/a"), item("child", "/w/a/x"),
        ])
        let sections = index.sections(for: .folder(path: "/w/a"), matching: "")
        #expect(sections.count == 2)
        let headers = Set(sections.compactMap(\.header))
        #expect(headers == ["This folder", "x"])
    }

    @Test func searchFiltersWithinSections() {
        let index = MemoryIndex(entries: [
            item("alpha", "/w/a/x"), item("beta", "/w/a/y"),
        ])
        let sections = index.sections(for: .folder(path: "/w/a"), matching: "alpha")
        #expect(sections.flatMap(\.items).count == 1)
        #expect(sections.flatMap(\.items).first?.primary.name == "alpha")
    }
}
