import SwiftUI

/// New subagent and Edit: one sheet for both.
struct SubagentSheet: View {
    @Bindable var model: SubagentModel
    @State private var draft: SubagentDraft
    @State private var errorMessage: String?
    @State private var isSaving = false
    let installedAgents: [String]
    @Environment(\.dismiss) private var dismiss

    init(model: SubagentModel, draft: SubagentDraft, installedAgents: [String]) {
        self.model = model
        self.installedAgents = installedAgents
        _draft = State(initialValue: draft)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    field("Name") {
                        TextField("Name", text: $draft.name)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                            .disabled(draft.isEditing)
                    }
                    field("Agent") {
                        Picker("Agent", selection: $draft.harness) {
                            ForEach(draft.harnessChoices(installed: installedAgents), id: \.self) { agent in
                                Text(agentDisplayName(agent)).tag(agent)
                            }
                        }
                        .labelsHidden()
                        .pickerStyle(.menu)
                        .fixedSize()
                    }
                    field("Model") {
                        TextField("Model", text: $draft.model)
                            .labelsHidden()
                            .font(.body.monospaced())
                            .textFieldStyle(.roundedBorder)
                    }
                    field("Description") {
                        TextField("Description", text: $draft.description)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                    }
                    editor("Instructions", text: $draft.instructions, height: 120,
                           note: "Put in front of every prompt. Optional.")
                    field("Timeout") {
                        TextField("Minutes", text: $draft.timeoutText)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                            .frame(width: 64)
                        Text("minutes").foregroundStyle(.secondary)
                    }
                    editor("Extra arguments", text: $draft.argumentLines, height: 64,
                           note: "One argument per line, passed to the agent before the prompt. Optional.")
                }
                .padding(.bottom, 4)
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .textSelection(.enabled)
            }

            HStack(spacing: 12) {
                Text("Every agent that uses the hub can call it as a tool.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if isSaving { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(draft.isEditing ? "Save" : "Create") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!draft.isValid || isSaving)
                    .help(draft.problem ?? "")
            }
        }
        .padding(20)
        .frame(minWidth: 560, minHeight: 520)
    }

    private func save() {
        isSaving = true
        errorMessage = nil
        Task {
            do {
                try await model.save(draft)
                dismiss()
            } catch {
                errorMessage = error.localizedDescription
            }
            isSaving = false
        }
    }

    private func editor(_ title: String, text: Binding<String>, height: CGFloat, note: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline).foregroundStyle(.secondary)
            TextEditor(text: text)
                .font(.body.monospaced())
                .frame(height: height)
            Text(note).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline).foregroundStyle(.secondary)
            HStack(spacing: 8) { content() }
        }
    }
}
