import SwiftUI

struct SyncSheet: View {
    let session: SyncSession
    @Environment(\.dismiss) private var dismiss
    @State private var confirmingMassDelete = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            content
            buttons
        }
        .padding(20)
        .frame(minWidth: 520, minHeight: 420)
        .task { await session.loadPreview() }
    }

    @ViewBuilder
    private var content: some View {
        switch session.phase {
        case .loading:
            ProgressView("Checking what would change…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .applying:
            ProgressView("Applying changes…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .preview(let plan):
            planView(plan, headline: plan.headline)
        case .done(let plan):
            planView(plan, headline: plan.resultSummary, listChanges: false)
        case .needsMassDeleteApproval(let plan, let message):
            VStack(alignment: .leading, spacing: 8) {
                Label(message, systemImage: "exclamationmark.triangle")
                Text("Nothing has been changed.").foregroundStyle(.secondary)
                planView(plan, headline: plan.headline)
            }
        case .failed(let message):
            ContentUnavailableView("Sync failed", systemImage: "xmark.circle", description: Text(message))
        }
    }

    private func planView(_ plan: SyncPlan, headline: String, listChanges: Bool = true) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(headline).font(.title3.bold())
            List {
                if listChanges {
                    ForEach(plan.changesByProject, id: \.project) { group in
                        Section(group.project.path ?? group.project.title) {
                            ForEach(group.actions, id: \.self) { action in
                                HStack(spacing: 8) {
                                    Label(Self.verb(action.op), systemImage: Self.symbol(action.op))
                                        .frame(minWidth: 80, alignment: .leading)
                                    Text(action.name).lineLimit(1)
                                    Spacer()
                                    Text(agentDisplayName(action.agent)).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                if !plan.notes.isEmpty {
                    Section("Needs attention") {
                        ForEach(plan.notes, id: \.self) { note in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(note.kind.rawValue.capitalized)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text(note.message)
                            }
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        HStack {
            Spacer()
            switch session.phase {
            case .preview(let plan):
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await session.apply(plan: plan) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!plan.hasChanges)
            case .needsMassDeleteApproval(let plan, _):
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply anyway", role: .destructive) { confirmingMassDelete = true }
                    .confirmationDialog("Apply anyway?", isPresented: $confirmingMassDelete, titleVisibility: .visible) {
                        Button("Apply anyway", role: .destructive) {
                            Task { await session.apply(plan: plan, allowMassDelete: true) }
                        }
                        Button("Cancel", role: .cancel) {}
                    } message: {
                        Text("This removes many memories at once. Backups are kept in ~/.nativemem/backups.")
                    }
            case .done, .failed:
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            case .loading, .applying:
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(isBusy)
            }
        }
    }

    private var isBusy: Bool {
        if case .applying = session.phase { return true }
        return false
    }

    private static func verb(_ op: SyncOp) -> String {
        switch op {
        case .add: "Add"
        case .update: "Update"
        case .remove: "Remove"
        }
    }

    private static func symbol(_ op: SyncOp) -> String {
        switch op {
        case .add: "plus.circle"
        case .update: "arrow.triangle.2.circlepath"
        case .remove: "minus.circle"
        }
    }
}
