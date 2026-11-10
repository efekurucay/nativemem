import SwiftUI

/// Add server and Edit: one sheet for both.
struct McpServerSheet: View {
    @Bindable var model: McpModel
    @State private var draft: McpServerDraft
    @State private var errorMessage: String?
    @State private var isSaving = false
    @Environment(\.dismiss) private var dismiss

    init(model: McpModel, draft: McpServerDraft) {
        self.model = model
        _draft = State(initialValue: draft)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Picker("Type", selection: $draft.kind) {
                        ForEach(McpServerDraft.Kind.allCases, id: \.self) { kind in
                            Text(kind.title).tag(kind)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    field("Name") {
                        TextField("Name", text: $draft.name)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                            .disabled(draft.isEditing)
                    }
                    switch draft.kind {
                    case .local: localFields
                    case .remote: remoteFields
                    }
                    rowsEditor
                }
                .padding(.bottom, 4)
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .textSelection(.enabled)
            }

            HStack(spacing: 12) {
                Text(draft.problem ?? "")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if isSaving { ProgressView().controlSize(.small) }
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(draft.isEditing ? "Save" : "Add") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!draft.isValid || isSaving)
            }
        }
        .padding(20)
        .frame(minWidth: 560, minHeight: 480)
    }

    @ViewBuilder
    private var localFields: some View {
        field("Command") {
            TextField("Command", text: $draft.command)
                .labelsHidden()
                .font(.body.monospaced())
                .textFieldStyle(.roundedBorder)
        }
        VStack(alignment: .leading, spacing: 4) {
            Text("Arguments").font(.subheadline).foregroundStyle(.secondary)
            TextEditor(text: $draft.argumentLines)
                .font(.body.monospaced())
                .frame(height: 64)
            Text("One argument per line.").font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var remoteFields: some View {
        field("Address") {
            TextField("Address", text: $draft.url)
                .labelsHidden()
                .font(.body.monospaced())
                .textFieldStyle(.roundedBorder)
        }
        field("Sign-in") {
            Picker("Sign-in", selection: $draft.usesOAuth) {
                Text("None").tag(false)
                Text("OAuth").tag(true)
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .fixedSize()
        }
    }

    private var rowsEditor: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(draft.rowsTitle).font(.subheadline).foregroundStyle(.secondary)
                Spacer()
                Button("Add row", systemImage: "plus") { draft.rows.append(McpKeyValueRow()) }
                    .labelStyle(.titleAndIcon)
            }
            ForEach($draft.rows) { $row in
                HStack(spacing: 8) {
                    TextField("Name", text: $row.key)
                        .textFieldStyle(.roundedBorder)
                    Group {
                        if row.isSecret {
                            SecureField(row.reference == nil ? "Value" : "Keep the stored value", text: $row.value)
                        } else {
                            TextField(row.isHiddenLiteral ? "Type the value again" : "Value", text: $row.value)
                        }
                    }
                    .textFieldStyle(.roundedBorder)
                    Toggle("Secret", isOn: $row.isSecret)
                        .toggleStyle(.checkbox)
                    Button("Remove row", systemImage: "minus.circle") {
                        draft.rows.removeAll { $0.id == row.id }
                    }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                }
            }
            if draft.hasHiddenValues {
                Text("Values that are not secret are not shown. Type them again to keep them.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text("A secret is kept in your keychain, never in the settings file.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
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

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline).foregroundStyle(.secondary)
            HStack(spacing: 8) { content() }
        }
    }
}
