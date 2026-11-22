import SwiftUI

enum SkillManifest {
    static func differences(_ first: String, _ second: String) -> [String] {
        func files(_ manifest: String) -> [String: String] {
            var entries: [String: String] = [:]
            for line in manifest.split(separator: "\n") {
                let fields = line.split(separator: " ", maxSplits: 3, omittingEmptySubsequences: false)
                guard fields.count == 4 else { continue }
                entries[String(fields[3])] = fields.prefix(3).joined(separator: " ")
            }
            return entries
        }
        let firstFiles = files(first)
        let secondFiles = files(second)
        return Set(firstFiles.keys).union(secondFiles.keys)
            .filter { firstFiles[$0] != secondFiles[$0] }.sorted()
    }
}

struct SkillEditorSheet: View {
    @Environment(\.dismiss) private var dismiss
    let model: AppModel
    let item: LogicalItem
    var onSaved: () -> Void = {}
    @State private var copies: [String: SkillText] = [:]
    @State private var failures: [String: String] = [:]
    @State private var selectedAgent: String?
    @State private var draft = ""
    @State private var showAdvice = false
    @State private var error: String?
    @State private var isSaving = false

    private var agents: [Item] { [item.primary] + item.copies }
    private var current: SkillText? { selectedAgent.flatMap { copies[$0] } }
    private var dirty: Bool { current.map { $0.text != draft } ?? false }
    private var reviewEntry: Item? {
        guard var copy = agents.first(where: { $0.agent == selectedAgent }) else { return nil }
        copy.body = draft
        return copy
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(item.primary.name).font(.title2.bold())
            if selectedAgent == nil {
                ProgressView("Loading skill copies…")
            } else if let current {
                editor(current)
            } else if let selectedAgent, let failure = failures[selectedAgent] {
                Text(failure).foregroundStyle(.secondary)
            }
            if let error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.disabled(isSaving)
                Button("Save") { Task { await save() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(!dirty || current?.textHash == nil || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isSaving)
            }
        }
        .padding(22)
        .frame(width: 720, height: 540)
        .interactiveDismissDisabled(dirty || isSaving)
        .task { await load() }
        .sheet(isPresented: $showAdvice) {
            if let cli = model.cli, let entry = reviewEntry {
                ReviewAdviceSheet(cli: cli, item: entry) { draft = $0 }
            }
        }
    }

    private func editor(_ current: SkillText) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Picker("Edit copy", selection: Binding(
                    get: { selectedAgent ?? item.primary.agent },
                    set: { agent in
                        selectedAgent = agent
                        draft = copies[agent]?.text ?? ""
                        error = nil
                    }
                )) {
                    ForEach(agents.filter { copies[$0.agent] != nil }, id: \.agent) { copy in
                        Text(agentDisplayName(copy.agent)).tag(copy.agent)
                    }
                }
                .disabled(dirty || isSaving)
                if copies.count > 1 {
                    Menu("Use text from") {
                        ForEach(agents.filter { $0.agent != selectedAgent && copies[$0.agent] != nil }, id: \.agent) { copy in
                            Button(agentDisplayName(copy.agent)) {
                                if let text = copies[copy.agent]?.text { draft = text }
                            }
                        }
                    }
                    .disabled(isSaving || current.textHash == nil)
                }
            }
            HStack {
                Text(current.path).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                Spacer()
                Button("Ask Subagent…") { showAdvice = true }
                    .disabled(isSaving || current.textHash == nil || draft.trimmed.isEmpty)
                    .fixedSize()
            }
            ForEach(agents.filter { $0.agent != selectedAgent }, id: \.agent) { copy in
                if let manifest = current.manifest, let other = copies[copy.agent]?.manifest {
                    let names = SkillManifest.differences(manifest, other)
                    if !names.isEmpty {
                        Text("\(agentDisplayName(copy.agent)) differs in \(names.joined(separator: ", "))")
                            .font(.callout).foregroundStyle(.secondary).lineLimit(2)
                            .help(names.joined(separator: "\n"))
                    }
                } else if let failure = failures[copy.agent] {
                    Text("\(agentDisplayName(copy.agent)) · \(failure)").font(.caption).foregroundStyle(.secondary)
                }
            }
            TextEditor(text: $draft)
                .font(.body.monospaced())
                .accessibilityLabel("Skill instructions")
                .disabled(isSaving || current.textHash == nil)
            Text("Save updates this SKILL.md with a backup. Review propagation with Sync.")
                .font(.callout).foregroundStyle(.secondary)
        }
    }

    private func load() async {
        guard let cli = model.cli else { error = CLIError.notFound.localizedDescription; return }
        for copy in agents {
            do {
                copies[copy.agent] = try await cli.skillShow(agent: copy.agent, nativeId: copy.nativeId)
            } catch {
                failures[copy.agent] = error.localizedDescription
            }
            if Task.isCancelled { return }
        }
        selectedAgent = copies[item.primary.agent] != nil ? item.primary.agent : agents.first(where: { copies[$0.agent] != nil })?.agent ?? item.primary.agent
        draft = current?.text ?? ""
    }

    private func save() async {
        guard let cli = model.cli, let selectedAgent, let current, let hash = current.textHash,
              let copy = agents.first(where: { $0.agent == selectedAgent }), !isSaving else { return }
        isSaving = true
        defer { isSaving = false }
        do {
            _ = try await cli.editSkill(agent: selectedAgent, nativeId: copy.nativeId, expected: hash, text: draft)
            await model.reload()
            onSaved()
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
