import SwiftUI

struct MemoryIndexRepair: Decodable, Sendable {
    let path: String
    let removed: [String]
    let added: [String]
    let createsIndex: Bool
    let token: String
    let applied: Bool

    var hasChanges: Bool { createsIndex || !removed.isEmpty || !added.isEmpty }
}

struct IndexRepairSheet: View {
    @Environment(\.dismiss) private var dismiss
    let model: AppModel
    let issue: MaintenanceReport.IndexIssue
    @State private var preview: MemoryIndexRepair?
    @State private var error: String?
    @State private var busy = false
    @State private var applied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Repair memory index").font(.title2.bold())
            Text(issue.index.path)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
            } else if busy {
                ProgressView()
            } else if applied {
                Label("Index repaired", systemImage: "checkmark.circle")
            } else if let preview {
                if preview.hasChanges {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 8) {
                            if preview.createsIndex { Text("Create MEMORY.md") }
                            ForEach(Array(preview.removed.enumerated()), id: \.offset) { _, file in
                                Label("Remove missing link to \(file)", systemImage: "minus.circle")
                            }
                            ForEach(preview.added, id: \.self) { file in
                                Label("Add link to \(file)", systemImage: "plus.circle")
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 210)
                    Text("Only the index changes. An existing index is backed up first.")
                        .font(.callout).foregroundStyle(.secondary)
                } else {
                    Text("The index needs no changes.")
                }
            }
            HStack {
                Spacer()
                Button(applied ? "Done" : "Close") { dismiss() }
                    .disabled(busy)
                if error != nil {
                    Button("Preview Again") { Task { await check() } }
                        .disabled(busy)
                } else if !applied, let preview, preview.hasChanges {
                    Button("Apply Repair") { Task { await check(token: preview.token) } }
                        .buttonStyle(.borderedProminent)
                        .disabled(busy)
                }
            }
        }
        .padding(22)
        .frame(width: 520)
        .interactiveDismissDisabled(busy)
        .task { await check() }
    }

    private func check(token: String? = nil) async {
        guard let cli = model.cli, !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        let project = issue.index.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent
        do {
            let repair = try await cli.repairMemoryIndex(projectDirectory: project, token: token)
            preview = repair
            if repair.applied {
                applied = true
                await model.reload()
            }
        } catch {
            self.error = error.localizedDescription
        }
    }
}
