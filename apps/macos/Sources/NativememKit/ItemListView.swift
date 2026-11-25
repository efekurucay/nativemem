import SwiftUI

struct CategoryTag: View {
    let category: String

    var body: some View {
        Text(category)
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

struct InstructionTag: View {
    var body: some View {
        Text("Instruction")
            .font(.caption)
            .foregroundStyle(.secondary)
    }
}

struct ItemListView: View {
    @Bindable var model: AppModel
    @State private var repairingIndex: MaintenanceReport.IndexIssue?
    @State private var reviewSession: ContextReview?

    var body: some View {
        VStack(spacing: 0) {
            if model.selectedFolderPath != nil {
                modePicker
            }
            if !model.showsSessions {
                reviewBar
            }
            list
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .searchable(text: $model.itemSearch, prompt: searchPrompt)
        .sheet(item: $repairingIndex) { issue in
            IndexRepairSheet(model: model, issue: issue)
        }
        .sheet(item: $reviewSession) { session in
            ContextReviewSheet(model: model, session: session)
        }
        .task(id: model.showsSessions ? model.dataRevision : nil) {
            if model.showsSessions { await model.loadSessions() }
        }
    }

    private var reviewBar: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Button("Review", systemImage: "wrench.and.screwdriver") {
                    Task { await model.reviewContext() }
                }
                Menu {
                    Button("Review Updates…") {
                        Task { reviewSession = await model.makeContextReview(updatesOnly: true) }
                    }
                } label: {
                    Label("Review Cards…", systemImage: "rectangle.on.rectangle")
                } primaryAction: {
                    Task { reviewSession = await model.makeContextReview() }
                }
                Spacer(minLength: 0)
            }
            .disabled(model.cli == nil || model.isLoading || model.isReviewingContext || model.isEditing)
            Group {
                if model.isReviewingContext {
                    ProgressView("Reviewing…").controlSize(.small)
                } else if let report = model.contextReport {
                    let count = report.findings.count
                    Text(count == 0 ? "No flagged items" : "\(count) flagged")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            if let report = model.contextReport {
                ForEach(report.indexIssues) { issue in
                    Button("Review memory index…", systemImage: "exclamationmark.triangle") {
                        repairingIndex = issue
                    }
                    .help(issue.index.path)
                    .disabled(model.isEditing || model.isLoading)
                }
                if (model.doctor?.agents ?? []).contains(where: { $0.paused.contains(Item.memoryKind) || $0.paused.contains(Item.skillKind) }) {
                    Button("Some items paused · Overview", systemImage: "exclamationmark.triangle") {
                        model.requestSidebar(.overview)
                    }
                    .buttonStyle(.link)
                    .font(.caption)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var modePicker: some View {
        Picker("Show", selection: modeBinding) {
            Text("Memory & skills").tag(ItemMode.memory)
            Text(model.sessionsLoaded ? "Sessions (\(model.folderSessionCount))" : "Sessions").tag(ItemMode.sessions)
        }
        .labelsHidden()
        .pickerStyle(.segmented)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var list: some View {
        let sections = model.visibleSections
        return Group {
            if model.showsSessions && model.isLoadingSessions {
                ProgressView("Loading sessions…")
            } else if model.showsSessions, let error = model.sessionError {
                ContentUnavailableView {
                    Label("Sessions could not be loaded", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(error)
                } actions: {
                    Button("Retry") { Task { await model.loadSessions() } }
                }
            } else if sections.isEmpty {
                if model.itemSearch.isEmpty {
                    ContentUnavailableView(emptyTitle, systemImage: "tray")
                } else {
                    ContentUnavailableView.search(text: model.itemSearch)
                }
            } else {
                List(selection: itemBinding) {
                    ForEach(sections) { section in
                        if let header = section.header {
                            Section(header) {
                                ForEach(section.items) { item in
                                    row(item)
                                }
                            }
                        } else {
                            ForEach(section.items) { item in
                                row(item)
                            }
                        }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var isSkills: Bool { model.sidebar == .skills }

    private var emptyTitle: String {
        if model.showsSessions { return "No sessions" }
        if model.selectedFolderPath != nil { return "No memories or skills" }
        return isSkills ? "No skills" : "No memories"
    }

    private var searchPrompt: String {
        if model.showsSessions { return "Search sessions" }
        if model.selectedFolderPath != nil { return "Search memories and skills" }
        return isSkills ? "Search skills" : "Search memories"
    }

    @ViewBuilder
    private func row(_ item: LogicalItem) -> some View {
        if model.showsSessions {
            sessionRow(item)
        } else {
            memoryRow(item)
        }
    }

    private func sessionRow(_ item: LogicalItem) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(item.title).lineLimit(2)
            if let stats = item.primary.session {
                HStack(spacing: 8) {
                    Text(stats.updated.formatted(.relative(presentation: .named)))
                    Text(stats.turns == 1 ? "1 turn" : "\(stats.turns) turns")
                }
                .font(.callout)
                .foregroundStyle(.secondary)
            }
            HStack(spacing: 8) {
                ForEach(item.livesIn, id: \.self) { residence in
                    Text(agentDisplayName(residence.agent))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .tag(item.id)
    }

    private func memoryRow(_ item: LogicalItem) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(item.primary.name).lineLimit(1)
                if item.primary.isSkill {
                    CategoryTag(category: "skill")
                } else if let category = item.primary.category {
                    CategoryTag(category: category)
                }
            }
            if let description = item.primary.description, !description.isEmpty {
                Text(description)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            if let findings = model.contextReport?.findings[item.id] {
                ForEach(findings, id: \.self) { finding in
                    Label(finding.rawValue, systemImage: "exclamationmark.circle")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.orange)
                }
            }
            if item.primary.isSkill {
                HStack(spacing: 8) {
                    ForEach(item.livesIn, id: \.self) { residence in
                        Text(agentDisplayName(residence.agent))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .tag(item.id)
    }

    private var modeBinding: Binding<ItemMode> {
        Binding(
            get: { model.itemMode },
            set: { model.requestMode($0) }
        )
    }

    private var itemBinding: Binding<String?> {
        Binding(
            get: { model.selectedItemID },
            set: { model.requestItem($0) }
        )
    }
}
