import SwiftUI

/// What `mcp import` would add to the hub from the agents' own MCP settings.
struct McpImportSheet: View {
    let session: McpImportSession
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            content
            buttons
        }
        .padding(20)
        .frame(minWidth: 520, minHeight: 420)
        .task { await session.loadPreview() }
    }

    @ViewBuilder
    private var content: some View {
        switch session.phase {
        case .loading:
            ProgressView("Reading your agents' servers…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .applying:
            ProgressView("Importing…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .preview(let plan):
            planView(plan, headline: plan.headline)
        case .done(let plan):
            planView(plan, headline: plan.resultSummary)
        case .failed(let message):
            ContentUnavailableView("Import failed", systemImage: "xmark.circle", description: Text(message))
        }
    }

    private func planView(_ plan: McpImportPlan, headline: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(headline).font(.title3.bold())
            List {
                serverSection("New", plan.servers(.add), symbol: "plus.circle")
                serverSection("Already in the hub", plan.servers(.present), symbol: "checkmark.circle")
                serverSection(
                    "Conflicts: different settings in more than one place, not imported",
                    plan.servers(.conflict), symbol: "exclamationmark.triangle")
                if !plan.skipped.isEmpty {
                    Section("Skipped") {
                        ForEach(plan.skipped, id: \.self) { entry in
                            VStack(alignment: .leading, spacing: 4) {
                                Text("\(entry.name) · \(agentDisplayName(entry.agent))")
                                Text(entry.reason).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                if !plan.warnings.isEmpty {
                    Section("Notes") {
                        ForEach(plan.warnings, id: \.self) { warning in
                            Text(warning)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func serverSection(_ title: String, _ servers: [McpImportServer], symbol: String) -> some View {
        if !servers.isEmpty {
            Section(title) {
                ForEach(servers) { server in
                    HStack(spacing: 8) {
                        Label(server.name, systemImage: symbol)
                        Text(server.transport.title).foregroundStyle(.secondary)
                        Spacer()
                        VStack(alignment: .trailing, spacing: 4) {
                            Text(server.agentsText).foregroundStyle(.secondary)
                            if !server.secrets.isEmpty {
                                Text("\(server.secrets.count) \(server.secrets.count == 1 ? "secret" : "secrets") kept in your keychain")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        HStack {
            Spacer()
            switch session.phase {
            case .preview(let plan):
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Apply") { Task { await session.apply(plan: plan) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(plan.summary.add == 0)
            case .done, .failed:
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            case .loading, .applying:
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(isBusy)
            }
        }
    }

    private var isBusy: Bool {
        if case .applying = session.phase { return true }
        return false
    }
}
