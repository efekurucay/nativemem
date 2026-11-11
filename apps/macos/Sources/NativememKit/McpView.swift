import SwiftUI

/// "MCP & Integrations": which agents use the hub, and the servers the hub holds.
struct McpView: View {
    @Bindable var model: McpModel
    @State private var serverSheet: McpServerDraft?
    @State private var importSession: McpImportSession?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            McpAgentsBox(model: model)
                .padding(20)
            if model.cli == nil {
                ContentUnavailableView("nativemem not found", systemImage: "questionmark.folder")
            } else if model.isLoading || !model.isLoaded {
                ProgressView("Loading integrations…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let loadError = model.loadError {
                ContentUnavailableView("The servers could not be read", systemImage: "xmark.circle", description: Text(loadError))
            } else if model.isLoaded && model.servers.isEmpty {
                ContentUnavailableView(
                    "No servers yet", systemImage: "puzzlepiece.extension",
                    description: Text("Add a server, or import the ones your agents already use."))
            } else {
                HSplitView {
                    McpServerList(model: model)
                        .frame(minWidth: 260, idealWidth: 340, maxWidth: 520)
                    if let server = model.selectedServer {
                        McpServerDetail(model: model, server: server, edit: { serverSheet = McpServerDraft(editing: server) })
                            .id(server.name)
                            .frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
                            .layoutPriority(1)
                    } else {
                        ContentUnavailableView("No server selected", systemImage: "puzzlepiece.extension")
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    serverSheet = McpServerDraft()
                } label: {
                    Label("Add server…", systemImage: "plus.rectangle.on.folder")
                        .labelStyle(.titleAndIcon)
                }
                .help("Add a server to the hub")
                .disabled(model.cli == nil)
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    importSession = model.makeImportSession()
                } label: {
                    Label("Import from agents…", systemImage: "square.and.arrow.down")
                        .labelStyle(.titleAndIcon)
                }
                .help("Copy the servers your agents already use into the hub")
                .disabled(model.cli == nil)
            }
        }
        .sheet(item: $serverSheet) { draft in
            McpServerSheet(model: model, draft: draft)
        }
        .sheet(item: $importSession) { session in
            McpImportSheet(session: session)
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

extension McpServerDraft: Identifiable {
    public var id: String { isEditing ? "edit-\(name)" : "add" }
}

// MARK: - Agents

private struct McpAgentsBox: View {
    @Bindable var model: McpModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Agents using the hub").font(.headline)
            GroupBox {
                VStack(alignment: .leading, spacing: 8) {
                    Text("An agent that has the hub can use every enabled server below.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    if let error = model.registrationError {
                        Label(error, systemImage: "exclamationmark.triangle").font(.callout)
                    } else if model.visibleRegistrations.isEmpty {
                        Text(model.isLoaded ? "No agents found." : "Looking for agents…")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(model.visibleRegistrations) { registration in
                        AgentRow(model: model, registration: registration)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

private struct AgentRow: View {
    @Bindable var model: McpModel
    let registration: McpAgentRegistration

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 12) {
                Text(registration.displayName)
                Spacer()
                if model.busyAgents.contains(registration.agent) {
                    ProgressView().controlSize(.small)
                }
                Text(stateText)
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 96, alignment: .trailing)
                if let action {
                    Button(action.title) { Task { await action.run() } }
                        .frame(minWidth: 72)
                        .disabled(model.busyAgents.contains(registration.agent))
                } else {
                    Color.clear.frame(width: 72, height: 1)
                }
            }
            if let message = registration.message {
                Text(message).font(.caption).foregroundStyle(.secondary)
            }
            ForEach(registration.warnings, id: \.self) { warning in
                Label(warning, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var stateText: String {
        switch registration.state {
        case .installed: "Installed"
        case .outdated: "Out of date"
        case .notInstalled, .agentMissing: "Not installed"
        case .failed: "Problem"
        }
    }

    private var action: (title: String, run: () async -> Void)? {
        let agent = registration.agent
        switch registration.state {
        case .installed: return ("Remove", { await model.uninstall(agent) })
        case .outdated: return ("Update", { await model.install(agent) })
        case .notInstalled: return ("Install", { await model.install(agent) })
        case .agentMissing, .failed: return nil
        }
    }
}

// MARK: - Servers

private struct McpServerList: View {
    @Bindable var model: McpModel

    var body: some View {
        List(model.servers, selection: $model.selectedName) { server in
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(server.name).lineLimit(1)
                    Text(subtitle(server))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                McpStateLabel(state: model.state(of: server))
            }
            .tag(server.name)
        }
    }

    private func subtitle(_ server: McpServer) -> String {
        guard let count = model.toolCount(of: server) else { return server.transport.title }
        return "\(server.transport.title) · \(count) \(count == 1 ? "tool" : "tools")"
    }
}

private struct McpStateLabel: View {
    let state: McpServerState

    var body: some View {
        switch state {
        case .checking:
            HStack(spacing: 4) {
                ProgressView().controlSize(.small)
                Text(state.title).font(.caption).foregroundStyle(.secondary)
            }
        case .connected:
            Label(state.title, systemImage: "checkmark.circle.fill")
                .font(.caption)
                .foregroundStyle(.green)
        case .needsSignIn:
            Label(state.title, systemImage: "person.crop.circle.badge.exclamationmark")
                .font(.caption)
                .foregroundStyle(.orange)
        case .error:
            Label(state.title, systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(.red)
        case .disabled:
            Label(state.title, systemImage: "minus.circle")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}

private struct McpServerDetail: View {
    @Bindable var model: McpModel
    let server: McpServer
    let edit: () -> Void
    @State private var confirmingRemove = false

    private var isSigningIn: Bool { model.signingIn.contains(server.name) }
    private var isTesting: Bool { model.testing.contains(server.name) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                if isSigningIn {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Waiting for you to sign in in the browser…").foregroundStyle(.secondary)
                    }
                }
                buttons
                GroupBox {
                    VStack(alignment: .leading, spacing: 8) {
                        LabeledContent("Type", value: server.transport.title)
                        if let command = server.command { LabeledContent("Command") { mono(command) } }
                        if !server.args.isEmpty { LabeledContent("Arguments") { mono(server.args.joined(separator: "\n")) } }
                        if let cwd = server.cwd { LabeledContent("Folder") { mono(cwd) } }
                        if let url = server.url { LabeledContent("Address") { mono(url) } }
                        if server.transport == .http {
                            LabeledContent("Sign-in", value: server.auth == .oauth ? "OAuth" : "None")
                        }
                        valueRows(server.transport == .stdio ? "Environment" : "Headers",
                                  server.transport == .stdio ? server.env : server.headers)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let tools = model.tests[server.name]?.tools, !tools.isEmpty {
                    DisclosureGroup("Tools (\(tools.count))") {
                        Text(tools.joined(separator: "\n"))
                            .font(.body.monospaced())
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.top, 4)
                    }
                }
            }
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(20)
        }
        .confirmationDialog(
            "Remove \(server.name)?", isPresented: $confirmingRemove, titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) { Task { await model.remove(server.name) } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The hub stops offering its tools to your agents. Its stored secrets stay in your keychain.")
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(server.name).font(.title2.bold())
            let state = model.state(of: server)
            McpStateLabel(state: state)
            if case .error(let message) = state {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
        }
    }

    private var buttons: some View {
        HStack(spacing: 12) {
            Button("Test") { Task { await model.test(server.name) } }
                .disabled(!server.enabled || isTesting || isSigningIn)
            if server.auth == .oauth {
                if server.authState == .signedIn {
                    Button("Sign out") { Task { await model.signOut(server.name) } }
                        .disabled(isSigningIn)
                } else {
                    Button("Sign in") { Task { await model.signIn(server.name) } }
                        .disabled(isSigningIn)
                }
            }
            Toggle("Enabled", isOn: Binding(
                get: { server.enabled },
                set: { value in Task { await model.setEnabled(server.name, value) } }
            ))
            .toggleStyle(.switch)
            .disabled(isSigningIn)
            Spacer()
            Button("Edit…", action: edit).disabled(isSigningIn)
            Button("Remove…", role: .destructive) { confirmingRemove = true }.disabled(isSigningIn)
        }
    }

    @ViewBuilder
    private func valueRows(_ title: String, _ values: [String: String]) -> some View {
        if !values.isEmpty {
            LabeledContent(title) {
                VStack(alignment: .trailing, spacing: 4) {
                    ForEach(values.keys.sorted(), id: \.self) { key in
                        mono("\(key) = \(values[key] ?? "")")
                    }
                }
            }
        }
    }

    private func mono(_ text: String) -> some View {
        Text(text)
            .font(.body.monospaced())
            .textSelection(.enabled)
            .multilineTextAlignment(.trailing)
    }
}
