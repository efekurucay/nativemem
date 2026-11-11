import Foundation
import Observation

public enum SidebarSelection: Hashable, Sendable {
    case overview
    case global
    case skills
    case mcp
    case subagents
    case folder(path: String)
    case folderOnly(path: String)
}

/// What the list of a selected project folder shows.
public enum ItemMode: Hashable, Sendable {
    case memory
    case sessions
}

/// Draft of an item being edited in the detail pane.
public struct EditDraft: Equatable, Sendable {
    public var name: String
    public var description: String
    public var category: String?
    public var body: String

    public init(name: String, description: String, category: String?, body: String) {
        self.name = name
        self.description = description
        self.category = category
        self.body = body
    }

    public init(item: LogicalItem) {
        self.name = item.primary.name
        self.description = item.primary.description ?? ""
        self.category = item.primary.category
        self.body = item.primary.body
    }

    public var document: EditDocument {
        EditDocument(name: name, description: description, category: category, body: body)
    }
}

@MainActor @Observable
public final class AppModel {
    public private(set) var index = MemoryIndex.empty
    public private(set) var sessionIndex = SessionIndex.empty
    public private(set) var status: StatusDocument?
    public private(set) var doctor: DoctorDocument?
    public private(set) var isLoading = false
    public private(set) var isLoadingSessions = false
    public private(set) var sessionsLoaded = false
    public private(set) var sessionError: String?
    public private(set) var dataRevision = 0
    private var reloadGeneration = 0
    public private(set) var cli: NativememCLI? {
        didSet {
            mcp.cli = cli
            subagents.cli = cli
            mcp.invalidate()
            subagents.invalidate()
        }
    }
    public let mcp = McpModel()
    public let subagents = SubagentModel()
    public var errorMessage: String?

    public var sidebar: SidebarSelection? = .overview
    public var selectedItemID: String?
    public var projectSearch = ""
    public var itemSearch = ""
    /// Applies to project folders only; every other row lists memory.
    public private(set) var itemMode = ItemMode.memory

    public var editDraft: EditDraft?
    public var editError: String?
    public var askDiscard = false
    public private(set) var uneditableItemIDs: Set<String> = []
    private var pendingSidebar: SidebarSelection?
    private var pendingItemID: String?
    private var pendingMode: ItemMode?

    public init(cli: NativememCLI? = nil) {
        self.cli = cli ?? NativememCLI.locateOnThisMachine().map(NativememCLI.init(binary:))
        mcp.cli = self.cli
        subagents.cli = self.cli
    }

    public var isFiltering: Bool {
        !projectSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public var hasGlobalItems: Bool {
        index.items.contains { !$0.primary.isSkill && $0.project.path == nil }
    }

    public var skillCount: Int { index.skills.count }

    public var projectTree: [ProjectTree.Node] {
        let entries = index.projects.compactMap { project -> (path: String, count: Int)? in
            guard let path = project.key.path else { return nil }
            return (path: path, count: project.count)
        }
        return ProjectTree.build(projects: entries, home: homePath)
    }

    public var visibleTree: [ProjectTree.Node] {
        ProjectTree.filter(projectTree, matching: projectSearch)
    }

    public var selectedFolderPath: String? {
        switch sidebar {
        case .folder(let path), .folderOnly(let path): path
        default: nil
        }
    }

    public var showsSessions: Bool { itemMode == .sessions && selectedFolderPath != nil }

    /// Sessions of the selected folder and below, whatever the search says.
    public var folderSessionCount: Int {
        guard let selection = sidebar else { return 0 }
        return sessionIndex.sessions(in: selection).count
    }

    public var visibleSections: [ItemSection] {
        guard let selection = sidebar else { return [] }
        if showsSessions { return sessionIndex.sections(for: selection, matching: itemSearch) }
        return index.sections(for: selection, matching: itemSearch)
    }

    public var selectedItem: LogicalItem? {
        guard let id = selectedItemID else { return nil }
        return showsSessions ? sessionIndex.item(id: id) : index.item(id: id)
    }

    /// Shown as the badge of the Overview row.
    public var agentsNeedingAction: Int { doctor?.needingAction.count ?? 0 }

    public var isEditing: Bool { editDraft != nil }

    public var editDirty: Bool {
        guard let draft = editDraft, let item = selectedItem else { return false }
        return draft != EditDraft(item: item)
    }

    public var canSave: Bool {
        guard editDirty else { return false }
        let name = editDraft?.name.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !name.isEmpty
    }

    private var homePath: String {
        FileManager.default.homeDirectoryForCurrentUser.path
    }

    public func chooseBinary(_ url: URL) async {
        UserDefaults.standard.set(url.path, forKey: NativememCLI.savedPathKey)
        cli = NativememCLI(binary: url)
        await reload()
    }

    public func reload() async {
        guard let cli else { return }
        reloadGeneration += 1
        let generation = reloadGeneration
        isLoadingSessions = false
        isLoading = true
        defer { if generation == reloadGeneration { isLoading = false } }
        do {
            let (document, loadedIndex) = try await Self.readSnapshot(cli)
            guard generation == reloadGeneration, !Task.isCancelled else { return }
            index = loadedIndex
            status = document.status
            doctor = document.doctor
            errorMessage = nil
            sessionIndex = .empty
            sessionsLoaded = false
            isLoadingSessions = false
            sessionError = nil
            dataRevision += 1
            mcp.invalidate()
            subagents.invalidate()
            if showsSessions { await loadSessions() }
            if selectedItem == nil { selectedItemID = nil }
            if sidebar == .mcp { await mcp.reload() }
            if sidebar == .subagents { await subagents.reload() }
        } catch is CancellationError {
        } catch {
            if generation == reloadGeneration { errorMessage = error.localizedDescription }
        }
    }

    // Index construction, like CLI decoding, must not occupy the UI's main actor.
    private nonisolated static func readSnapshot(_ cli: NativememCLI) async throws -> (SnapshotDocument, MemoryIndex) {
        let document = try await cli.snapshot()
        return (document, MemoryIndex(entries: document.items.items))
    }

    private nonisolated static func readSessions(_ cli: NativememCLI) async throws -> (ItemsDocument, SessionIndex) {
        let document = try await cli.sessions()
        return (document, SessionIndex(entries: document.items))
    }

    public func loadSessions() async {
        guard !sessionsLoaded, !isLoadingSessions, let cli else { return }
        let generation = reloadGeneration
        let revision = dataRevision
        isLoadingSessions = true
        sessionError = nil
        defer { if generation == reloadGeneration, revision == dataRevision { isLoadingSessions = false } }
        do {
            let (document, loaded) = try await Self.readSessions(cli)
            guard generation == reloadGeneration, revision == dataRevision, !Task.isCancelled else { return }
            sessionIndex = loaded
            for summary in document.sessionStatus ?? [] {
                if let position = status?.agents.firstIndex(where: { $0.id == summary.id }) {
                    status?.agents[position].sessions = summary.sessions
                    status?.agents[position].skipped.append(contentsOf: summary.skipped)
                }
            }
            if let notice = document.sessionNotice,
               let position = doctor?.agents.firstIndex(where: { $0.id == "claude" }),
               doctor?.agents[position].noticeLines.contains(notice) != true {
                let notices = (doctor?.agents[position].noticeLines ?? []) + [notice]
                doctor?.agents[position].notices = notices
            }
            sessionsLoaded = true
        } catch is CancellationError {
        } catch {
            if generation == reloadGeneration, revision == dataRevision { sessionError = error.localizedDescription }
        }
    }

    public func delete(_ item: LogicalItem) async {
        guard let cli else { return }
        do {
            _ = try await cli.delete(agent: item.primary.agent, nativeId: item.primary.nativeId)
            selectedItemID = nil
            editDraft = nil
            editError = nil
        } catch {
            errorMessage = error.localizedDescription
        }
        await reload()
    }

    // MARK: - Creating

    public var installedAgents: [String] { status?.agents.map(\.id) ?? [] }

    /// The project paths that already hold entries, for the New sheet's picker.
    public var knownProjectPaths: [String] {
        index.projects.compactMap(\.key.path).sorted()
    }

    /// A draft that starts in the folder selected in the sidebar, for the agent that holds the most originals.
    public func makeNewEntryDraft() -> NewEntryDraft {
        let agent = status?.agentWithMostOriginals ?? ""
        switch sidebar {
        case .folder(let path), .folderOnly(let path):
            return NewEntryDraft(agent: agent, isGlobal: false, projectPath: path)
        default:
            return NewEntryDraft(agent: agent)
        }
    }

    /// Creates the entry, reloads and selects it. A CLI error is thrown for the sheet to show.
    public func add(_ draft: NewEntryDraft) async throws {
        guard let cli else { throw CLIError.notFound }
        let result = try await cli.add(
            agent: draft.agent, kind: draft.kind, project: draft.project, document: draft.document
        )
        editDraft = nil
        editError = nil
        await reload()
        sidebar = draft.project.map { .folder(path: $0) } ?? .global
        selectedItemID = "\(result.agent)|\(result.nativeId)"
    }

    public func skillText(of item: LogicalItem) async throws -> String {
        guard let cli else { throw CLIError.notFound }
        return try await cli.skillShow(agent: item.primary.agent, nativeId: item.primary.nativeId).text
    }

    public func sessionTranscript(of item: LogicalItem) async throws -> SessionTranscript {
        guard let cli else { throw CLIError.notFound }
        return try await cli.sessionShow(agent: item.primary.agent, nativeId: item.primary.nativeId)
    }

    public func markReviewed(_ agent: String) async {
        guard let cli else { return }
        do {
            _ = try await cli.accept(agent: agent)
        } catch {
            errorMessage = error.localizedDescription
        }
        await reload()
    }

    public func makeSyncSession() -> SyncSession? {
        guard let cli else { return nil }
        return SyncSession(cli: cli, onApplied: { [weak self] in await self?.reload() })
    }

    // MARK: - Editing

    public func beginEdit() {
        guard let item = selectedItem else { return }
        editDraft = EditDraft(item: item)
        editError = nil
    }

    public func cancelEdit() {
        editDraft = nil
        editError = nil
    }

    public func saveEdit() async {
        guard let draft = editDraft, let item = selectedItem, let cli else { return }
        do {
            _ = try await cli.edit(agent: item.primary.agent, nativeId: item.primary.nativeId, document: draft.document)
            editDraft = nil
            editError = nil
            await reload()
        } catch {
            if let failure = error as? CLIError, failure.isUnsupported { uneditableItemIDs.insert(item.id) }
            editError = error.localizedDescription
        }
    }

    // MARK: - Selection with unsaved-edit guard

    public func requestMode(_ value: ItemMode) {
        if editDirty, value != itemMode {
            pendingMode = value
            pendingSidebar = nil
            pendingItemID = nil
            askDiscard = true
        } else {
            if itemMode != value { selectedItemID = nil }
            itemMode = value
            editDraft = nil
            editError = nil
        }
    }

    public func requestSidebar(_ value: SidebarSelection?) {
        if editDirty, value != sidebar {
            pendingSidebar = value
            pendingItemID = nil
            pendingMode = nil
            askDiscard = true
        } else {
            if sidebar != value { selectedItemID = nil }
            sidebar = value
            editDraft = nil
            editError = nil
        }
    }

    public func requestItem(_ id: String?) {
        if editDirty, id != selectedItemID {
            pendingItemID = id
            pendingSidebar = nil
            pendingMode = nil
            askDiscard = true
        } else {
            selectedItemID = id
            editDraft = nil
            editError = nil
        }
    }

    public func confirmDiscard() {
        editDraft = nil
        editError = nil
        if let sidebar = pendingSidebar {
            if self.sidebar != sidebar { selectedItemID = nil }
            self.sidebar = sidebar
        } else if let mode = pendingMode {
            if itemMode != mode { selectedItemID = nil }
            itemMode = mode
        } else if let id = pendingItemID {
            selectedItemID = id
        }
        pendingSidebar = nil
        pendingItemID = nil
        pendingMode = nil
        askDiscard = false
    }

    public func cancelDiscard() {
        pendingSidebar = nil
        pendingItemID = nil
        pendingMode = nil
        askDiscard = false
    }
}
