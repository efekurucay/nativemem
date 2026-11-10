import AppKit
import SwiftUI

struct NewEntrySheet: View {
    @Bindable var model: AppModel
    @State private var draft: NewEntryDraft
    @State private var errorMessage: String?
    @State private var isCreating = false
    @State private var projectChoice: String?
    @Environment(\.dismiss) private var dismiss

    private static let chooseFolder = "\u{0}choose-folder"

    init(model: AppModel) {
        let draft = model.makeNewEntryDraft()
        self.model = model
        _draft = State(initialValue: draft)
        _projectChoice = State(initialValue: draft.projectPath)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 12) {
                field("Where") {
                    Picker("Where", selection: $draft.isGlobal) {
                        Text("Global").tag(true)
                        Text("Project").tag(false)
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .fixedSize()
                    if !draft.isGlobal { projectPicker }
                }
                field("Agent") {
                    Picker("Agent", selection: $draft.agent) {
                        ForEach(model.installedAgents, id: \.self) { agent in
                            Text(agentDisplayName(agent)).tag(agent)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.menu)
                    .fixedSize()
                }
                if draft.isGlobal {
                    field("Heading") {
                        TextField("Heading", text: $draft.description)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                    }
                } else {
                    field("Name") {
                        TextField("Name", text: $draft.name)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                    }
                    field("Description") {
                        TextField("Description", text: $draft.description)
                            .labelsHidden()
                            .textFieldStyle(.roundedBorder)
                    }
                    field("Category") {
                        Picker("Category", selection: $draft.category) {
                            Text("None").tag(String?.none)
                            ForEach(NewEntryDraft.categories, id: \.self) { category in
                                Text(category.capitalized).tag(String?.some(category))
                            }
                        }
                        .labelsHidden()
                        .pickerStyle(.menu)
                        .fixedSize()
                    }
                }
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Body").font(.subheadline).foregroundStyle(.secondary)
                TextEditor(text: $draft.body)
                    .font(.body.monospaced())
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle")
                    .font(.callout)
            }

            HStack(spacing: 12) {
                Text("It reaches the other agents at the next sync.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!draft.isValid || isCreating)
            }
        }
        .padding(20)
        .frame(minWidth: 520, minHeight: 480)
    }

    private var projectPicker: some View {
        Picker("Project", selection: $projectChoice) {
            if projectChoice == nil { Text("Choose a folder…").tag(String?.none) }
            ForEach(projectPaths, id: \.self) { path in
                Text((path as NSString).abbreviatingWithTildeInPath).tag(String?.some(path))
            }
            Divider()
            Text("Choose folder…").tag(String?.some(Self.chooseFolder))
        }
        .labelsHidden()
        .pickerStyle(.menu)
        .onChange(of: projectChoice) { previous, choice in
            if choice == Self.chooseFolder {
                projectChoice = previous
                chooseProjectFolder()
            } else {
                draft.projectPath = choice
            }
        }
    }

    /// Known projects, plus the current choice when it came from the folder panel.
    private var projectPaths: [String] {
        var paths = model.knownProjectPaths
        if let current = draft.projectPath, !paths.contains(current) { paths.append(current) }
        return paths
    }

    private func chooseProjectFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        draft.projectPath = url.path
        projectChoice = url.path
    }

    private func create() {
        isCreating = true
        errorMessage = nil
        Task {
            do {
                try await model.add(draft)
                dismiss()
            } catch {
                errorMessage = error.localizedDescription
            }
            isCreating = false
        }
    }

    private func field<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.subheadline).foregroundStyle(.secondary)
            HStack(spacing: 8) { content() }
        }
    }
}
