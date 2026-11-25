import AppKit
import SwiftUI

struct ItemDetailView: View {
    @Bindable var model: AppModel
    let item: LogicalItem
    let onDelete: () async -> Void
    @State private var confirmingDelete = false
    @State private var showingSkillEditor = false

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
                if !model.isEditing && model.contextReport == nil {
                    if item.primary.isSkill {
                        Button("Edit", systemImage: "pencil") { showingSkillEditor = true }
                    } else {
                        Button("Edit", systemImage: "pencil") { model.beginEdit() }
                            .disabled(editBlockedReason != nil)
                            .help(editBlockedReason ?? "Edit this memory")
                    }
                }
                Button("Show in Finder", systemImage: "folder") {
                    NSWorkspace.shared.activateFileViewerSelecting([
                        URL(fileURLWithPath: item.primary.path)
                    ])
                }
                .help("Show in Finder")
                if !model.isEditing && model.contextReport == nil {
                    Button("Delete…", systemImage: "trash", role: .destructive) {
                        confirmingDelete = true
                    }
                    .disabled(isGeneratedMemory)
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
            Text(deleteImpact)
        }
        .sheet(isPresented: $showingSkillEditor) {
            SkillEditorSheet(model: model, item: item)
        }
    }

    private var isGeneratedMemory: Bool {
        item.primary.agent == "codex" && item.primary.kind == Item.memoryKind && item.primary.nativeId.contains("#")
    }

    private var deleteImpact: String {
        let source = "This entry will be removed from \(agentDisplayName(item.primary.agent)) now. A backup will be kept."
        guard !item.copies.isEmpty else { return source }
        let copies = item.copies.map { "\(agentDisplayName($0.agent)) · \($0.path)" }.joined(separator: "\n")
        if item.primary.kind == Item.memoryKind {
            return "\(source)\n\nThese linked copies remain:\n\(copies)"
        }
        return "\(source)\n\nThe next sync can remove these linked copies:\n\(copies)"
    }

    private var editBlockedReason: String? {
        if item.primary.isSkill { return "Skills are edited as files: edit the skill files directly." }
        if item.originalMissing { return "The original is missing, so there is nothing to edit." }
        if isGeneratedMemory || model.uneditableItemIDs.contains(item.id) {
            return "Its agent regenerates this entry, so it cannot be edited."
        }
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

                if let report = model.contextReport {
                    reviewActions(report)
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text("Lives in").font(.headline)
                    if item.primary.isSkill || model.contextReport != nil {
                        Text(item.project.path ?? "Global · available across projects")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        ForEach([item.primary] + item.copies, id: \.agent) { copy in
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(agentDisplayName(copy.agent)).font(.subheadline)
                                    Text(copy.agent == item.primary.agent
                                         ? (item.originalMissing ? "copy (original missing)" : "original")
                                         : "copy")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Button("Show in Finder", systemImage: "folder") {
                                        NSWorkspace.shared.activateFileViewerSelecting([
                                            URL(fileURLWithPath: copy.path)
                                        ])
                                    }
                                    .labelStyle(.iconOnly)
                                    .help("Show \(agentDisplayName(copy.agent)) file in Finder")
                                }
                                Text(copy.path)
                                    .font(.caption.monospaced())
                                    .foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                            }
                        }
                    } else {
                        HStack(spacing: 8) {
                            ForEach(item.livesIn, id: \.self) { residence in
                                Text("\(agentDisplayName(residence.agent)) · \(residence.label)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }

                if item.primary.isSkill {
                    SkillTextView(model: model, item: item)
                } else {
                    BodyText(blocks: MarkdownBlocks.parse(item.primary.body))
                    if model.contextReport != nil {
                        ForEach(item.copies.filter { !$0.hash.isEmpty && $0.hash != item.primary.hash }, id: \.self) { copy in
                            DisclosureGroup("\(agentDisplayName(copy.agent)) copy") {
                                BodyText(blocks: MarkdownBlocks.parse(copy.body))
                            }
                        }
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func reviewActions(_ report: MaintenanceReport) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if let duplicates = report.duplicates.first(where: { $0.notes.contains(where: { $0.id == item.id }) }) {
                Text("This note shares its body with another note. Compare their descriptions and categories before removing either.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                ForEach(duplicates.notes.filter { $0.id != item.id }) { other in
                    Button("Open \(other.primary.name) · \(agentDisplayName(other.primary.agent))", systemImage: "doc.on.doc") {
                        model.requestItem(other.id)
                    }
                }
            }
            if item.primary.isSkill {
                let differentAgents = item.copies.filter { $0.hash != item.primary.hash }.map { agentDisplayName($0.agent) }
                if !differentAgents.isEmpty {
                    Label("Different copies · \(differentAgents.joined(separator: ", "))", systemImage: "exclamationmark.circle")
                        .font(.callout)
                        .foregroundStyle(.orange)
                }
            }
            HStack {
                if item.primary.isSkill {
                    Button("Compare & Edit", systemImage: "pencil") { showingSkillEditor = true }
                } else {
                    Button("Edit", systemImage: "pencil") { model.beginEdit() }
                        .disabled(editBlockedReason != nil)
                        .help(editBlockedReason ?? "Edit the source entry")
                }
                Button("Remove…", systemImage: "trash", role: .destructive) {
                    confirmingDelete = true
                }
                .disabled(isGeneratedMemory)
                .help(isGeneratedMemory ? "Codex regenerates this entry" : "Review the impact before removing this entry")
            }
            .buttonStyle(.bordered)
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
