import AppKit
import SwiftUI

struct OverviewView: View {
    @Bindable var model: AppModel

    var body: some View {
        Group {
            if model.cli == nil {
                ContentUnavailableView {
                    Label("nativemem not found", systemImage: "questionmark.folder")
                } description: {
                    Text("The app needs the nativemem command to read your memories. Choose where it is installed.")
                } actions: {
                    Button("Choose…", action: chooseBinary)
                }
            } else if let status = model.status {
                ScrollView {
                    VStack(alignment: .leading, spacing: 20) {
                        HStack(alignment: .top, spacing: 20) {
                            Figure(value: status.ledgerItems, caption: "items")
                            Figure(value: model.index.projects.count, caption: "projects")
                            Figure(value: model.agentsInSync, caption: "agents in sync")
                        }
                        ForEach(status.agents, id: \.id) { agent in
                            VStack(alignment: .leading, spacing: 8) {
                                Text(agentDisplayName(agent.id)).font(.headline)
                                GroupBox {
                                    VStack(alignment: .leading, spacing: 8) {
                                        LabeledContent("Originals", value: agent.originals.formatted())
                                        LabeledContent("Copies", value: agent.replicas.formatted())
                                        if !agent.skipped.isEmpty {
                                            DisclosureGroup("Skipped (\(agent.skipped.count))") {
                                                ForEach(agent.skipped, id: \.self) { entry in
                                                    VStack(alignment: .leading, spacing: 4) {
                                                        Text(entry.label)
                                                        Text(entry.reason)
                                                            .font(.caption)
                                                            .foregroundStyle(.secondary)
                                                    }
                                                    .frame(maxWidth: .infinity, alignment: .leading)
                                                }
                                            }
                                        }
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                }
                            }
                        }
                    }
                    .frame(maxWidth: 720, alignment: .leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(20)
                }
            } else if model.isLoading {
                ProgressView()
            } else {
                ContentUnavailableView("Nothing loaded yet", systemImage: "tray", description: Text("Press Reload to read your memories."))
            }
        }
    }

    private func chooseBinary() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = "Choose the nativemem command"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await model.chooseBinary(url) }
    }
}

private struct Figure: View {
    let value: Int
    let caption: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(value, format: .number)
                .font(.largeTitle.weight(.semibold))
                .monospacedDigit()
            Text(caption).foregroundStyle(.secondary)
        }
    }
}
