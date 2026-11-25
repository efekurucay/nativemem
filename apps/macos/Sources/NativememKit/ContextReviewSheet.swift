import SwiftUI

struct ContextReviewSheet: View {
    @Environment(\.dismiss) private var dismiss
    let model: AppModel
    let session: ContextReview
    @State private var position = 0
    @State private var answers: [String: ContextReview.Answer]
    @State private var draft: EditDraft?
    @State private var adviceFor: LogicalItem?
    @State private var skillToEdit: LogicalItem?
    @State private var showRemove = false
    @State private var error: String?
    @State private var busy = false
    @State private var sync: SyncSession?

    init(model: AppModel, session: ContextReview) {
        self.model = model
        self.session = session
        _answers = State(initialValue: session.answers)
    }

    private var answeredCount: Int { session.answers.count + position }
    private var totalCount: Int { session.answers.count + session.cards.count }
    private var hasChanges: Bool { answers.values.contains { $0.decision == .edit || $0.decision == .remove } }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(session.showsUpdatesOnly ? "Review Updates" : "Context Review").font(.title2.bold())
                Spacer()
                if position < session.cards.count {
                    Text("\(answeredCount + 1) of \(totalCount)")
                        .monospacedDigit().foregroundStyle(.secondary)
                }
            }
            if totalCount > 0 {
                ProgressView(value: Double(answeredCount), total: Double(totalCount))
            }
            if position >= session.cards.count {
                finishPanel
            } else {
                card(session.cards[position])
            }
            if let error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
        }
        .padding(24)
        .frame(width: 660, height: 530)
        .interactiveDismissDisabled(draft != nil || busy)
        .task {
            if session.cards.isEmpty { saveProgress() }
        }
        .sheet(item: $skillToEdit) { item in
            SkillEditorSheet(model: model, item: item, onSaved: {
                advance(.edit, updatedItem: model.index.item(id: item.id))
            })
        }
        .sheet(item: $adviceFor) { item in
            if let cli = model.cli {
                ReviewAdviceSheet(cli: cli, item: item.primary) { body in
                    var proposal = EditDraft(item: item)
                    proposal.body = body
                    draft = proposal
                }
            }
        }
        .sheet(item: $sync) { SyncSheet(session: $0) }
    }

    private var finishPanel: some View {
        VStack(alignment: .leading, spacing: 12) {
            if totalCount == 0 {
                Text(session.showsUpdatesOnly ? "No updates to review" : "No cards to review").font(.headline)
            } else {
                Text("Review complete").font(.headline)
                Text("\(count(.keep)) kept · \(count(.edit)) edited · \(count(.remove)) removed · \(count(.skip)) skipped")
                    .foregroundStyle(.secondary)
            }
            if hasChanges {
                Text("Preview the changes that will reach linked copies.")
                    .foregroundStyle(.secondary)
            }
            Spacer()
            HStack {
                Spacer()
                if hasChanges {
                    Button("Preview Sync") { sync = model.makeSyncSession() }
                }
                Button("Done") { dismiss() }.buttonStyle(.borderedProminent)
            }
        }
    }

    private func card(_ value: ContextReview.Card) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(value.record.name).font(.title2.bold())
            if let change = value.change { Text(change.rawValue).font(.callout).foregroundStyle(.secondary) }
            if let item = value.item {
                if value.record.wasSkipped == true {
                    Text("Skipped last review").font(.callout).foregroundStyle(.secondary)
                }
                Text(([item.primary] + item.copies).map { agentDisplayName($0.agent) }.joined(separator: " · "))
                    .font(.caption).foregroundStyle(.secondary)
                if draft != nil {
                    noteEditor(item)
                } else {
                    itemText(item, findings: value.findings)
                    controls(item)
                }
            } else {
                Text("This entry is absent from the current scope.")
                    .foregroundStyle(.secondary)
                Spacer()
                HStack {
                    Button("Close") { dismiss() }
                    Spacer()
                    Button("Next") { advance(.acknowledge) }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                }
            }
        }
        .id(value.id)
    }

    private func itemText(_ item: LogicalItem, findings: [MaintenanceReport.Finding]) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(findings, id: \.self) { finding in
                    Label(finding.rawValue, systemImage: "exclamationmark.circle").foregroundStyle(.orange)
                    Text(guidance(for: finding, item: item)).font(.callout).foregroundStyle(.secondary)
                }
                if let description = item.primary.description, !description.isEmpty {
                    Text(description).foregroundStyle(.secondary)
                }
                if item.primary.isSkill {
                    SkillTextView(model: model, item: item)
                } else {
                    BodyText(blocks: MarkdownBlocks.parse(item.primary.body))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func guidance(for finding: MaintenanceReport.Finding, item: LogicalItem) -> String {
        switch finding {
        case .duplicate:
            "Another note in this scope has the same text. Check whether the titles and descriptions serve different purposes before removing a copy."
        case .differentCopies where item.primary.isSkill:
            "Open Compare & Edit to see which files differ, then choose the skill text to keep."
        case .differentCopies:
            "The linked copies differ. Check their content before choosing which version to keep."
        case .unlisted:
            "Claude Code’s memory index doesn’t link to this note. Review the index repair from the project list if the note should be discoverable."
        case .sourceMissing:
            "The source note is missing. Read the remaining copies and choose which ones to keep."
        }
    }

    private func controls(_ item: LogicalItem) -> some View {
        HStack {
            Button("Close") { dismiss() }
            if !item.primary.isSkill {
                Button("Ask Subagent…") { adviceFor = item }
                    .disabled(item.originalMissing || generated(item) || model.cli == nil)
            }
            Spacer()
            Button("Skip") {
                advance(.skip)
            }
            Button("Remove…", role: .destructive) { showRemove = true }
                .disabled(generated(item) || busy)
            Button(item.primary.isSkill ? "Compare & Edit" : "Edit") {
                if item.primary.isSkill {
                    skillToEdit = item
                } else {
                    draft = EditDraft(item: item)
                }
            }
            .disabled(item.originalMissing || generated(item) || busy)
            Button("Keep") {
                advance(.keep)
            }
            .buttonStyle(.borderedProminent)
            .keyboardShortcut(.defaultAction)
        }
        .disabled(busy)
        .confirmationDialog("Remove \(item.primary.name)?", isPresented: $showRemove, titleVisibility: .visible) {
            Button("Remove from \(agentDisplayName(item.primary.agent))", role: .destructive) {
                Task { await remove(item) }
            }
            if item.primary.kind == Item.memoryKind && !item.copies.isEmpty {
                Button("Remove from all linked agents", role: .destructive) {
                    Task { await remove(item, includingCopies: true) }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            let copies = item.copies.map { "\(agentDisplayName($0.agent)) · \($0.path)" }.joined(separator: "\n")
            if item.primary.kind == Item.memoryKind {
                Text(copies.isEmpty ? "The selected note is backed up before removal."
                    : "Choose which copies to remove. Each selected copy is backed up first.\n\n\(copies)")
            } else {
                Text("The source in \(agentDisplayName(item.primary.agent)) is backed up and removed."
                    + (copies.isEmpty ? "" : "\n\nThe next sync can remove these linked copies:\n\(copies)"))
            }
        }
    }

    private func noteEditor(_ item: LogicalItem) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if !item.primary.isInstruction {
                TextField("Name", text: editBinding(\.name))
                TextField("Description", text: editBinding(\.description))
            }
            TextEditor(text: editBinding(\.body))
                .font(.body.monospaced()).accessibilityLabel("Note text")
            HStack {
                Spacer()
                Button("Cancel Edit") { draft = nil; error = nil }
                Button("Save & Next") { Task { await save(item) } }
                    .buttonStyle(.borderedProminent)
                    .disabled(draft?.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty != false || draft == EditDraft(item: item))
            }
        }
        .disabled(busy)
    }

    private func editBinding(_ key: WritableKeyPath<EditDraft, String>) -> Binding<String> {
        Binding(get: { draft?[keyPath: key] ?? "" }, set: { draft?[keyPath: key] = $0 })
    }

    private func generated(_ item: LogicalItem) -> Bool {
        item.primary.agent == "codex" && item.primary.kind == Item.memoryKind && item.primary.nativeId.contains("#")
    }

    private func save(_ item: LogicalItem) async {
        guard let cli = model.cli, let draft, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try await cli.edit(agent: item.primary.agent, nativeId: item.primary.nativeId, document: draft.document)
            await model.reload()
            let id = "\(result.edited.agent)|\(result.edited.nativeId)"
            advance(.edit, updatedItem: model.index.item(id: id))
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func remove(_ item: LogicalItem, includingCopies: Bool = false) async {
        guard let cli = model.cli, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let selected = includingCopies ? item.copies + [item.primary] : [item.primary]
            for entry in selected {
                _ = try await cli.delete(agent: entry.agent, nativeId: entry.nativeId)
            }
            await model.reload()
            advance(.remove)
        } catch {
            await model.reload()
            self.error = error.localizedDescription
        }
    }

    private func count(_ decision: ContextReview.Decision) -> Int {
        answers.values.filter { $0.decision == decision }.count
    }

    private func advance(_ decision: ContextReview.Decision, updatedItem: LogicalItem? = nil) {
        let card = session.cards[position]
        let record: ReviewRecord?
        switch decision {
        case .keep, .skip: record = card.record
        case .edit: record = updatedItem.map { ReviewRecord($0) } ?? card.record
        case .remove, .acknowledge: record = nil
        }
        answers[record?.id ?? card.id] = ContextReview.Answer(decision: decision, record: record)
        draft = nil
        error = nil
        position += 1
        saveProgress()
    }

    private func saveProgress() {
        model.saveContextReview(session.selection, answers: answers, complete: position == session.cards.count)
    }
}
