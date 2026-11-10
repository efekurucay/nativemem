import Foundation
import Testing
@testable import NativememKit

@Suite struct SyncHeadlineTests {
    private func plan(add: Int = 0, update: Int = 0, remove: Int = 0) -> SyncPlan {
        SyncPlan(applied: false, actions: [], notes: [],
                 summary: SyncSummary(add: add, update: update, remove: remove, conflicts: 0, skipped: 0))
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
}
