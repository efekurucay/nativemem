import AppKit
import SwiftUI

struct ItemDetailView: View {
    @Bindable var model: AppModel
    let item: LogicalItem
    let onDelete: () async -> Void
    @State private var confirmingDelete = false

    var body: some View {
        Group {
            if model.isEditing {
                editForm
            } else {
                readOnly
            }
        }
        .toolbar {
            ToolbarItemGroup {
                if !model.isEditing {
                    Button("Edit", systemImage: "pencil") {
                        model.beginEdit()
                    }
                    .disabled(editBlockedReason != nil)
                    .help(editBlockedReason ?? "Edit this memory")
                }
                Button("Show in Finder", systemImage: "folder") {
                    NSWorkspace.shared.activateFileViewerSelecting([
                        URL(fileURLWithPath: item.primary.path)
                    ])
                }
                .help("Show in Finder")
                if !model.isEditing {
                    Button("Delete…", systemImage: "trash", role: .destructive) {
                        confirmingDelete = true
                    }
                    .help("Delete this memory")
                }
            }
        }
        .confirmationDialog("Delete this memory?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await onDelete() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("It will be removed from Claude Code now and from the other agents at the next sync. A backup is kept in ~/.nativemem/backups.")
        }
    }

    private var editBlockedReason: String? {
        if item.originalMissing { return "The original is missing, so there is nothing to edit." }
        if model.uneditableItemIDs.contains(item.id) { return "Its agent regenerates this entry, so it cannot be edited." }
        return nil
    }

    // MARK: - Read-only

    private var readOnly: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        Text(item.primary.name).font(.title2.bold())
                        if let category = item.primary.category {
                            CategoryTag(category: category)
                        }
                    }
                    if let description = item.primary.description, !description.isEmpty {
                        Text(description).foregroundStyle(.secondary)
                    }
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text("Lives in").font(.headline)
                    HStack(spacing: 8) {
                        ForEach(item.livesIn, id: \.self) { residence in
                            Text("\(agentDisplayName(residence.agent)) · \(residence.label)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }

                BodyText(blocks: MarkdownBlocks.parse(item.primary.body))
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - Edit form

    private var editForm: some View {
        VStack(alignment: .leading, spacing: 12) {
            Form {
                TextField("Name", text: nameBinding)
                TextField("Description", text: descriptionBinding)
                Picker("Category", selection: categoryBinding) {
                    Text("None").tag(String?.none)
                    ForEach(categoryOptions, id: \.self) { category in
                        Text(category.capitalized).tag(String?.some(category))
                    }
                }
            }
            .formStyle(.columns)

            TextEditor(text: bodyBinding)
                .font(.body.monospaced())
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let error = model.editError {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout)
            }

            HStack(spacing: 12) {
                Text("Changes reach the other agents at the next sync.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") {
                    model.cancelEdit()
                }
                .keyboardShortcut(.cancelAction)
                Button("Save") {
                    Task { await model.saveEdit() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!model.canSave)
            }
        }
        .padding(20)
    }

    private var categoryOptions: [String] {
        let standard = ["user", "feedback", "project", "reference"]
        guard let current = model.editDraft?.category, !standard.contains(current) else { return standard }
        return standard + [current]
    }

    private var nameBinding: Binding<String> {
        Binding(
            get: { model.editDraft?.name ?? "" },
            set: { model.editDraft?.name = $0 }
        )
    }

    private var descriptionBinding: Binding<String> {
        Binding(
            get: { model.editDraft?.description ?? "" },
            set: { model.editDraft?.description = $0 }
        )
    }

    private var categoryBinding: Binding<String?> {
        Binding(
            get: { model.editDraft?.category },
            set: { model.editDraft?.category = $0 }
        )
    }

    private var bodyBinding: Binding<String> {
        Binding(
            get: { model.editDraft?.body ?? "" },
            set: { model.editDraft?.body = $0 }
        )
    }
}
