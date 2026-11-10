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
                    .help(item.primary.isSkill ? "Delete this skill" : "Delete this memory")
                }
            }
        }
        .confirmationDialog(
            item.primary.isSkill ? "Delete this skill?" : "Delete this memory?",
            isPresented: $confirmingDelete,
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                Task { await onDelete() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("It will be removed from \(agentDisplayName(item.primary.agent)) now and from the other agents at the next sync. A backup is kept in ~/.nativemem/backups.")
        }
    }

    private var editBlockedReason: String? {
        if item.primary.isSkill { return "Skills are edited as files: edit the skill files directly." }
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
                        if item.primary.isInstruction {
                            InstructionTag()
                        } else if let category = item.primary.category {
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

                if item.primary.isSkill {
                    SkillTextView(model: model, item: item)
                } else {
                    BodyText(blocks: MarkdownBlocks.parse(item.primary.body))
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - Edit form

    private var editForm: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 12) {
                field("Name") {
                    TextField("Name", text: nameBinding)
                        .labelsHidden()
                        .textFieldStyle(.roundedBorder)
                        .disabled(isInstruction)
                }
                field("Description") {
                    TextField("Description", text: descriptionBinding, axis: .vertical)
                        .labelsHidden()
                        .lineLimit(1...4)
                        .textFieldStyle(.roundedBorder)
                        .disabled(isInstruction)
                }
                if isInstruction {
                    Text("Edit the heading in the file itself")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    field("Category") {
                        Picker("Category", selection: categoryBinding) {
                            Text("None").tag(String?.none)
                            ForEach(categoryOptions, id: \.self) { category in
                                Text(category.capitalized).tag(String?.some(category))
                            }
                        }
                        .labelsHidden()
                        .pickerStyle(.menu)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }

            field("Body") {
                TextEditor(text: bodyBinding)
                    .font(.body.monospaced())
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxHeight: .infinity)

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

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline).foregroundStyle(.secondary)
            content()
        }
    }

    private var isInstruction: Bool { item.primary.isInstruction }

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
