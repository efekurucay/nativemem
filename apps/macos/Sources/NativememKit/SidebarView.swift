import SwiftUI

struct SidebarView: View {
    @Bindable var model: AppModel

    var body: some View {
        List(selection: selectionBinding) {
            Label("Overview", systemImage: "square.grid.2x2")
                .badge(model.agentsNeedingAction)
                .tag(SidebarSelection.overview)
            if model.hasGlobalItems {
                Label("Global", systemImage: "globe")
                    .tag(SidebarSelection.global)
            }
            if model.skillCount > 0 {
                Label("Skills", systemImage: "sparkles")
                    .badge(model.skillCount)
                    .tag(SidebarSelection.skills)
            }
            Label("MCP & Integrations", systemImage: "puzzlepiece.extension")
                .badge(model.mcp.attentionCount)
                .tag(SidebarSelection.mcp)
            Section("Projects") {
                ForEach(model.visibleTree) { node in
                    TreeNodeView(node: node, depth: 0, forceExpanded: model.isFiltering)
                }
            }
        }
        .listStyle(.sidebar)
        .searchable(text: $model.projectSearch, placement: .sidebar, prompt: "Filter projects")
        .navigationSplitViewColumnWidth(min: 200, ideal: 240)
    }

    private var selectionBinding: Binding<SidebarSelection?> {
        Binding(
            get: { model.sidebar },
            set: { model.requestSidebar($0) }
        )
    }
}

/// Recursive view for one folder node of the project tree.
struct TreeNodeView: View {
    let node: ProjectTree.Node
    let depth: Int
    let forceExpanded: Bool
    @State private var expanded: Bool

    init(node: ProjectTree.Node, depth: Int, forceExpanded: Bool) {
        self.node = node
        self.depth = depth
        self.forceExpanded = forceExpanded
        _expanded = State(initialValue: forceExpanded || depth == 0)
    }

    var body: some View {
        let selection: SidebarSelection = node.listsOnlyOwnItems
            ? .folderOnly(path: node.id)
            : .folder(path: node.id)
        if node.children.isEmpty {
            Label(node.label, systemImage: "folder")
                .badge(node.totalCount)
                .tag(selection)
        } else {
            DisclosureGroup(isExpanded: $expanded) {
                ForEach(node.children) { child in
                    TreeNodeView(node: child, depth: depth + 1, forceExpanded: forceExpanded)
                }
            } label: {
                Label(node.label, systemImage: "folder")
                    .badge(node.totalCount)
            }
            .tag(selection)
            .onChange(of: forceExpanded) { _, newValue in
                if newValue { expanded = true }
            }
        }
    }
}
