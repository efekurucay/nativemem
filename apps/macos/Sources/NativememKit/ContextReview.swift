import Foundation

struct ReviewRecord: Codable, Hashable, Sendable {
    let id: String
    let name: String
    let primaryHash: String
    let differingHashes: Set<String>
    let wasSkipped: Bool?

    init(_ item: LogicalItem, wasSkipped: Bool = false) {
        id = item.id
        name = item.primary.name
        primaryHash = item.primary.hash
        differingHashes = Set(item.copies.map(\.hash)).subtracting([primaryHash])
        self.wasSkipped = wasSkipped ? true : nil
    }

    func matchesContent(of other: Self) -> Bool {
        id == other.id && name == other.name && primaryHash == other.primaryHash && differingHashes == other.differingHashes
    }
}

struct ContextReview: Identifiable {
    enum Decision: String, Codable {
        case keep, edit, remove, skip, acknowledge
    }

    struct Answer: Codable {
        let decision: Decision
        let record: ReviewRecord?

        func stillApplies(to current: ReviewRecord?) -> Bool {
            if let record, let current { return record.matchesContent(of: current) }
            return record == nil && current == nil
        }
    }

    enum Change: String {
        case added = "New since last review"
        case changed = "Changed since last review"
        case removed = "Removed since last review"
    }

    struct Card: Identifiable {
        let record: ReviewRecord
        let item: LogicalItem?
        let change: Change?
        let findings: [MaintenanceReport.Finding]
        var id: String { record.id }
        var needsAttention: Bool { change != nil || !findings.isEmpty || record.wasSkipped == true }
    }

    let id = UUID()
    let selection: SidebarSelection
    let cards: [Card]
    let answers: [String: Answer]
    let showsUpdatesOnly: Bool

    init(selection: SidebarSelection, items: [LogicalItem], report: MaintenanceReport, baseline: [ReviewRecord]?,
         progress: [String: Answer] = [:], updatesOnly: Bool = false) {
        self.selection = selection
        showsUpdatesOnly = updatesOnly && baseline != nil
        let previous = Dictionary((baseline ?? []).map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let present = items.map { item -> Card in
            let old = previous[item.id]
            let record = ReviewRecord(item, wasSkipped: old?.wasSkipped == true)
            let change: Change?
            if let old {
                change = record.matchesContent(of: old) ? nil : .changed
            } else {
                change = baseline == nil ? nil : .added
            }
            return Card(record: record, item: item, change: change, findings: report.findings[item.id] ?? [])
        }
        let current = Dictionary(present.map { ($0.id, $0.record) }, uniquingKeysWith: { first, _ in first })
        let answers = progress.filter { $0.value.stillApplies(to: current[$0.key]) }
        let removed = (baseline ?? []).filter { current[$0.id] == nil }
            .map { Card(record: $0, item: nil, change: .removed, findings: []) }
        let undecided = present.filter { answers[$0.id] == nil }
        let attention = undecided.filter(\.needsAttention) + removed.filter { answers[$0.id] == nil }
        cards = showsUpdatesOnly ? attention : attention + undecided.filter { !$0.needsAttention }
        self.answers = answers
    }

    static func key(for selection: SidebarSelection) -> String? {
        switch selection {
        case .global: "context-review.global"
        case .skills: "context-review.skills"
        case .folder(let path): "context-review.folder:\(path)"
        case .folderOnly(let path): "context-review.project:\(path)"
        default: nil
        }
    }
}
