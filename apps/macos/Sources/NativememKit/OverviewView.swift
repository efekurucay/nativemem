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
                        if let doctor = model.doctor, doctor.hasNotices {
                            ForEach(doctor.needingAction) { agent in
                                CompatibilityCard(agent: agent) { await model.markReviewed(agent.id) }
                            }
                            ForEach(doctor.updated) { agent in
                                Text(agent.updatedLine)
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        HStack(alignment: .top, spacing: 20) {
                            Figure(value: status.logicalItems, caption: "items")
                            Figure(value: model.index.projects.count, caption: "projects")
                            Figure(value: status.agentsInSync, caption: "agents in sync")
                        }
                        ForEach(status.agents, id: \.id) { agent in
                            VStack(alignment: .leading, spacing: 8) {
                                Text(agentDisplayName(agent.id)).font(.headline)
                                GroupBox {
                                    VStack(alignment: .leading, spacing: 8) {
                                        LabeledContent("Originals") {
                                            Text(agent.originals, format: .number).monospacedDigit()
                                        }
                                        LabeledContent("Copies") {
                                            Text(agent.replicas, format: .number).monospacedDigit()
                                        }
                                        ForEach(model.doctor?.agents.first { $0.id == agent.id }?.noticeLines ?? [], id: \.self) { notice in
                                            Text(notice)
                                                .font(.caption)
                                                .foregroundStyle(.secondary)
                                                .frame(maxWidth: .infinity, alignment: .leading)
                                        }
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
                                    .labeledContentStyle(TrailingValueStyle())
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

/// An agent updated and nativemem paused part of its sync until the adapter is brought up to date.
private struct CompatibilityCard: View {
    let agent: DoctorAgent
    let markReviewed: () async -> Void
    @State private var confirming = false

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Label("\(agent.displayName) needs attention", systemImage: "exclamationmark.triangle")
                    .font(.headline)
                if let version = agent.version {
                    Text("Updated to \(version)")
                }
                if !agent.paused.isEmpty {
                    Text("Paused: \(agent.pausedText)")
                        .foregroundStyle(.secondary)
                }
                DisclosureGroup("Failing checks (\(agent.failingChecks.count))") {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(agent.failingChecks) { check in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(check.summary)
                                Text("Expected: \(check.expected)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text("Observed: \(check.observed)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                        }
                    }
                    .padding(.top, 4)
                }
                HStack {
                    Button("Copy prompt") { copyPrompt() }
                        .disabled(agent.prompt == nil)
                    Button("Mark as reviewed") { confirming = true }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .confirmationDialog(
            "Mark \(agent.displayName) as reviewed?",
            isPresented: $confirming,
            titleVisibility: .visible
        ) {
            Button("Mark as reviewed") { Task { await markReviewed() } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("nativemem will keep syncing \(agent.pausedText) with what \(agent.displayName) writes now. Do this only when you have checked that the files still look right.")
        }
    }

    private func copyPrompt() {
        guard let prompt = agent.prompt else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(prompt, forType: .string)
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

private struct TrailingValueStyle: LabeledContentStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack {
            configuration.label
            Spacer()
            configuration.content
        }
    }
}
