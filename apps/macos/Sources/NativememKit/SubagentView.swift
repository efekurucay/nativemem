import AppKit
import SwiftUI

/// "Subagents": the coding agents defined once and offered to every agent that uses the hub.
struct SubagentView: View {
    @Bindable var model: SubagentModel
    let installedAgents: [String]
    @State private var sheet: SubagentDraft?

    var body: some View {
        Group {
            if model.cli == nil {
                ContentUnavailableView("nativemem not found", systemImage: "questionmark.folder")
            } else if model.isLoading || !model.isLoaded {
                ProgressView("Loading subagents…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let loadError = model.loadError {
                ContentUnavailableView(
                    "The subagents could not be read", systemImage: "xmark.circle", description: Text(loadError))
            } else if model.isLoaded && model.subagents.isEmpty {
                ContentUnavailableView(
                    "No subagents yet", systemImage: "person.2",
                    description: Text("Define a coding agent with the model you choose. Every agent that uses the hub can call it as a tool."))
            } else {
                HSplitView {
                    SubagentList(model: model)
                        .frame(minWidth: 260, idealWidth: 340, maxWidth: 520)
                    if let subagent = model.selectedSubagent {
                        SubagentDetail(
                            model: model, subagent: subagent,
                            edit: { sheet = SubagentDraft(editing: subagent) }
                        )
                        .id(subagent.name)
                        .frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
                        .layoutPriority(1)
                    } else {
                        ContentUnavailableView("No subagent selected", systemImage: "person.2")
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    sheet = SubagentDraft(harness: installedAgents.first ?? "")
                } label: {
                    Label("New subagent…", systemImage: "person.badge.plus")
                        .labelStyle(.titleAndIcon)
                }
                .help("Define a subagent")
                .disabled(model.cli == nil)
            }
        }
        .sheet(item: $sheet) { draft in
            SubagentSheet(model: model, draft: draft, installedAgents: installedAgents)
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })
        ) {
            Button("OK") {}
        } message: {
            Text(model.errorMessage ?? "")
        }
    }
}

extension SubagentDraft: Identifiable {
    public var id: String { isEditing ? "edit-\(name)" : "new" }
}

private struct SubagentList: View {
    @Bindable var model: SubagentModel

    var body: some View {
        List(model.subagents, selection: $model.selectedName) { subagent in
            VStack(alignment: .leading, spacing: 4) {
                Text(subagent.name).lineLimit(1)
                Text("\(subagent.harnessName) · \(subagent.model)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .tag(subagent.name)
        }
    }
}

private struct SubagentDetail: View {
    @Bindable var model: SubagentModel
    let subagent: Subagent
    let edit: () -> Void
    @State private var confirmingRemove = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(subagent.name).font(.title2.bold())
                    Text(subagent.description)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                HStack(spacing: 12) {
                    Spacer()
                    Button("Edit…", action: edit)
                    Button("Remove…", role: .destructive) { confirmingRemove = true }
                }
                GroupBox {
                    VStack(alignment: .leading, spacing: 8) {
                        LabeledContent("Agent", value: subagent.harnessName)
                        LabeledContent("Model") { mono(subagent.model) }
                        LabeledContent("Timeout", value: subagent.timeoutText)
                        if !subagent.args.isEmpty {
                            LabeledContent("Extra arguments") { mono(subagent.args.joined(separator: "\n")) }
                        }
                        if let instructions = subagent.instructions {
                            VStack(alignment: .leading, spacing: 4) {
                                Text("Instructions").foregroundStyle(.secondary)
                                Text(instructions)
                                    .font(.body.monospaced())
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let trial = model.makeTrial(for: subagent) {
                    SubagentTryIt(trial: trial)
                }
            }
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(20)
        }
        .confirmationDialog(
            "Remove \(subagent.name)?", isPresented: $confirmingRemove, titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) { Task { await model.remove(subagent.name) } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Your agents can no longer call it as a tool.")
        }
    }

    private func mono(_ text: String) -> some View {
        Text(text)
            .font(.body.monospaced())
            .textSelection(.enabled)
            .multilineTextAlignment(.trailing)
    }
}

/// The prompt, an optional working folder and the answer of one run.
private struct SubagentTryIt: View {
    @State var trial: SubagentTrial

    init(trial: SubagentTrial) {
        _trial = State(initialValue: trial)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Try it").font(.headline)
            GroupBox {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Runs the subagent once with this prompt. Nothing is saved.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    TextEditor(text: $trial.prompt)
                        .font(.body)
                        .frame(height: 96)
                        .disabled(trial.isRunning)
                    folderRow
                    runRow
                    result
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .onDisappear { trial.cancel() }
    }

    private var folderRow: some View {
        HStack(spacing: 8) {
            Text("Working folder").foregroundStyle(.secondary)
            Text(trial.folder.map { ($0 as NSString).abbreviatingWithTildeInPath } ?? "Not set")
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
            Spacer()
            if trial.folder != nil {
                Button("Clear") { trial.folder = nil }
                    .disabled(trial.isRunning)
            }
            Button("Choose…", action: chooseFolder)
                .disabled(trial.isRunning)
        }
    }

    private var runRow: some View {
        HStack(spacing: 12) {
            if trial.isRunning {
                ProgressView().controlSize(.small)
                Text("Running…").foregroundStyle(.secondary)
                Spacer()
                Button("Cancel") { trial.cancel() }
                    .keyboardShortcut(.cancelAction)
            } else {
                Spacer()
                Button("Run") { trial.run() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!trial.canRun)
            }
        }
    }

    @ViewBuilder
    private var result: some View {
        switch trial.phase {
        case .idle, .running:
            EmptyView()
        case .cancelled:
            Text("Cancelled.").foregroundStyle(.secondary)
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .font(.callout)
                .textSelection(.enabled)
        case .finished(let result):
            VStack(alignment: .leading, spacing: 8) {
                if result.ok {
                    Text("Done in \(result.durationText).")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    BodyText(blocks: MarkdownBlocks.parse(result.text))
                } else {
                    Label(result.error ?? "The run failed.", systemImage: "exclamationmark.triangle")
                        .font(.callout)
                        .textSelection(.enabled)
                    Text("Stopped after \(result.durationText).")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        trial.folder = url.path
    }
}
