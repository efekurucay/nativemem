import Foundation
import Testing
@testable import NativememKit

@Suite struct SyncHeadlineTests {
    private func plan(add: Int = 0, update: Int = 0, remove: Int = 0, conflicts: Int = 0, skipped: Int = 0) -> SyncPlan {
        SyncPlan(applied: false, actions: [], notes: [],
                 summary: SyncSummary(add: add, update: update, remove: remove, conflicts: conflicts, skipped: skipped))
    }

    @Test func inSync() {
        #expect(plan().headline == "Everything is in sync.")
    }

    @Test func counts() {
        #expect(plan(add: 12, update: 3, remove: 1).headline == "12 to add, 3 to update, 1 to remove")
        #expect(plan(add: 2).headline == "2 to add")
    }

    @Test func resultSummary() {
        #expect(plan(add: 12, remove: 1).resultSummary == "Added 12, removed 1.")
        #expect(plan().resultSummary == "Nothing needed to change.")
    }

    @Test func unresolvedItemsAppearInPreviewAndResult() {
        #expect(plan(conflicts: 1).headline == "1 conflict")
        #expect(plan(skipped: 2).headline == "2 skipped")
        #expect(plan(update: 1, conflicts: 2).headline == "1 to update, 2 conflicts")
        #expect(plan(update: 1, conflicts: 2, skipped: 1).resultSummary == "Updated 1, 2 conflicts, 1 skipped.")
    }
}
