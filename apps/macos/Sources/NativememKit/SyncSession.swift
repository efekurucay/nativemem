import Foundation
import Observation

extension SyncPlan {
    public var headline: String {
        let parts = [
            (summary.add, "to add"),
            (summary.update, "to update"),
            (summary.remove, "to remove"),
        ].filter { $0.0 > 0 }.map { "\($0.0) \($0.1)" }
        let status = parts + unresolvedCounts
        return status.isEmpty ? "Everything is in sync." : status.joined(separator: ", ")
    }

    public var resultSummary: String {
        let parts = [
            (summary.add, "added"),
            (summary.update, "updated"),
            (summary.remove, "removed"),
        ].filter { $0.0 > 0 }.map { "\($0.1) \($0.0)" }
        let status = parts + unresolvedCounts
        guard !status.isEmpty else { return "Nothing needed to change." }
        let text = status.joined(separator: ", ")
        return text.prefix(1).uppercased() + text.dropFirst() + "."
    }

    private var unresolvedCounts: [String] {
        [
            (summary.conflicts, summary.conflicts == 1 ? "conflict" : "conflicts"),
            (summary.skipped, "skipped"),
        ].filter { $0.0 > 0 }.map { "\($0.0) \($0.1)" }
    }

    public var hasChanges: Bool { !actions.isEmpty }

    public var changesByProject: [(project: ProjectKey, actions: [SyncAction])] {
        Dictionary(grouping: actions, by: { ProjectKey(scope: $0.scope) })
            .map { (project: $0.key, actions: $0.value) }
            .sorted { ($0.project.path ?? "") < ($1.project.path ?? "") }
    }
}

@MainActor @Observable
public final class SyncSession: Identifiable {
    public enum Phase {
        case loading
        case preview(SyncPlan)
        case applying(SyncPlan)
        case needsMassDeleteApproval(plan: SyncPlan, message: String)
        case done(SyncPlan)
        case failed(String)
    }

    public private(set) var phase: Phase = .loading
    private let cli: NativememCLI
    private let onApplied: @MainActor () async -> Void

    public init(cli: NativememCLI, onApplied: @escaping @MainActor () async -> Void) {
        self.cli = cli
        self.onApplied = onApplied
    }

    public func loadPreview() async {
        phase = .loading
        do {
            phase = .preview(try await cli.syncPreview())
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    public func apply(plan: SyncPlan, allowMassDelete: Bool = false) async {
        phase = .applying(plan)
        do {
            let result = try await cli.syncApply(allowMassDelete: allowMassDelete)
            phase = .done(result)
            await onApplied()
        } catch let error as CLIError where error.isMassDelete {
            phase = .needsMassDeleteApproval(plan: plan, message: error.localizedDescription)
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }
}
