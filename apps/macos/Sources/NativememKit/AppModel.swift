import Foundation
import Observation

public enum SidebarSelection: Hashable, Sendable {
    case overview
    case global
    case folder(path: String)
    case folderOnly(path: String)
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
    public private(set) var status: StatusDocument?
    public private(set) var isLoading = false
    public private(set) var cli: NativememCLI?
    public var errorMessage: String?

    public var sidebar: SidebarSelection? = .overview
    public var selectedItemID: String?
    public var projectSearch = ""
    public var itemSearch = ""

    public var editDraft: EditDraft?
    public var editError: String?
    public var askDiscard = false
    public private(set) var uneditableItemIDs: Set<String> = []
    private var pendingSidebar: SidebarSelection?
    private var pendingItemID: String?

    public init() {
        cli = NativememCLI.locateOnThisMachine().map(NativememCLI.init(binary:))
    }

    public var isFiltering: Bool {
        !projectSearch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public var hasGlobalItems: Bool {
        index.items.contains { $0.project.path == nil }
    }

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

    public var visibleSections: [ItemSection] {
        guard let selection = sidebar else { return [] }
        return index.sections(for: selection, matching: itemSearch)
    }

    public var selectedItem: LogicalItem? {
        selectedItemID.flatMap { index.item(id: $0) }
    }

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
        isLoading = true
        defer { isLoading = false }
        do {
            async let loadedItems = cli.items()
            async let loadedStatus = cli.status()
            let (entries, statusDocument) = try await (loadedItems, loadedStatus)
            index = MemoryIndex(entries: entries)
            status = statusDocument
            if selectedItem == nil { selectedItemID = nil }
        } catch {
            errorMessage = error.localizedDescription
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

    public func requestSidebar(_ value: SidebarSelection?) {
        if editDirty, value != sidebar {
            pendingSidebar = value
            pendingItemID = nil
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
        } else if let id = pendingItemID {
            selectedItemID = id
        }
        pendingSidebar = nil
        pendingItemID = nil
        askDiscard = false
    }

    public func cancelDiscard() {
        pendingSidebar = nil
        pendingItemID = nil
        askDiscard = false
    }
}
