import Foundation
import Testing
@testable import NativememKit

@Suite struct ProjectTreeTests {
    private func node(_ label: String, _ count: Int, _ children: [ProjectTree.Node] = [], listsOnlyOwnItems: Bool = false) -> ProjectTree.Node {
        ProjectTree.Node(id: label, label: label, totalCount: count, children: children, listsOnlyOwnItems: listsOnlyOwnItems)
    }

    @Test func homePrefixShownForProjectAtHome() {
        let tree = ProjectTree.build(projects: [("/Users/x", 5)], home: "/Users/x")
        #expect(tree.count == 1)
        #expect(tree[0].label == "~")
        #expect(tree[0].totalCount == 5)
        #expect(tree[0].listsOnlyOwnItems)
        #expect(tree[0].children.isEmpty)
    }

    @Test func commonAncestorIsHoistedNotItsOwnLevel() {
        let tree = ProjectTree.build(
            projects: [("/Users/x/app", 2), ("/Users/x/other", 1)],
            home: "/Users/x"
        )
        #expect(tree.map(\.label) == ["app", "other"])
        #expect(tree.allSatisfy { !$0.listsOnlyOwnItems })
        #expect(tree[0].totalCount == 2)
        #expect(tree[1].totalCount == 1)
    }

    @Test func commonAncestorHoldingItemsAppearsAsLeaf() {
        let tree = ProjectTree.build(
            projects: [("/Users/x", 3), ("/Users/x/app", 2)],
            home: "/Users/x"
        )
        #expect(tree.count == 2)
        #expect(tree[0].label == "~")
        #expect(tree[0].totalCount == 3)
        #expect(tree[0].listsOnlyOwnItems)
        #expect(tree[1].label == "app")
        #expect(tree[1].totalCount == 2)
        #expect(!tree[1].listsOnlyOwnItems)
    }

    @Test func singleChildFolderMergesWithChild() {
        let tree = ProjectTree.build(
            projects: [("/Users/x/acme/apps", 2), ("/Users/x/other", 1)],
            home: "/Users/x"
        )
        #expect(tree[0].label == "acme/apps")
        #expect(tree[0].totalCount == 2)
        #expect(tree[0].children.isEmpty)
        #expect(tree[1].label == "other")
    }

    @Test func badgeCountsItemsInTheFolderAndBelow() {
        let tree = ProjectTree.build(
            projects: [("/w/a", 2), ("/w/a/sub", 3), ("/w/b", 1)],
            home: "/h"
        )
        #expect(tree[0].label == "a")
        #expect(tree[0].totalCount == 5)
        #expect(tree[0].children.count == 1)
        #expect(tree[0].children[0].label == "sub")
        #expect(tree[0].children[0].totalCount == 3)
        #expect(tree[1].label == "b")
        #expect(tree[1].totalCount == 1)
    }

    @Test func siblingsSortedByCountThenName() {
        let tree = ProjectTree.build(
            projects: [("/w/z", 1), ("/w/a", 1), ("/w/m", 3)],
            home: "/h"
        )
        #expect(tree.map(\.label) == ["m", "a", "z"])
    }

    @Test func filterKeepsMatchesAndAncestors() {
        let tree = ProjectTree.build(
            projects: [("/w/a", 2), ("/w/a/sub", 3), ("/w/b", 1)],
            home: "/h"
        )
        let filtered = ProjectTree.filter(tree, matching: "sub")
        #expect(filtered.count == 1)
        #expect(filtered[0].label == "a")
        #expect(filtered[0].children.count == 1)
        #expect(filtered[0].children[0].label == "sub")

        #expect(ProjectTree.filter(tree, matching: "nomatch").isEmpty)
        #expect(ProjectTree.filter(tree, matching: "").map(\.label) == ["a", "b"])
    }

    @Test func displayPathUsesHomePrefix() {
        #expect(ProjectTree.displayPath("/Users/x", home: "/Users/x") == "~")
        #expect(ProjectTree.displayPath("/Users/x/app", home: "/Users/x") == "~/app")
        #expect(ProjectTree.displayPath("/opt/other", home: "/Users/x") == "/opt/other")
    }
}
