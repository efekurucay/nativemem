import SwiftUI

public struct ContentView: View {
    @Bindable var model: AppModel
    @State private var syncSession: SyncSession?
    @State private var showNewEntry = false

    public init(model: AppModel) {
        self.model = model
    }

    public var body: some View {
        NavigationSplitView {
            SidebarView(model: model)
        } detail: {
            detailColumn
        }
        .frame(minWidth: 900, minHeight: 560)
        .navigationTitle("nativemem")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("New", systemImage: "plus") {
                    showNewEntry = true
                }
                .labelStyle(.iconOnly)
                .help("New memory")
                .disabled(model.cli == nil || model.status == nil || model.isEditing)
            }
            ToolbarItem(placement: .primaryAction) {
                Button {
                    syncSession = model.makeSyncSession()
                } label: {
                    Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                        .labelStyle(.titleAndIcon)
                }
                .help("Sync")
                .disabled(model.cli == nil)
            }
            ToolbarItem {
                Button("Reload", systemImage: "arrow.clockwise") {
                    Task { await model.reload() }
                }
                .labelStyle(.iconOnly)
                .help("Reload")
                .disabled(model.cli == nil || model.isLoading)
            }
        }
        .sheet(item: $syncSession) { session in
            SyncSheet(session: session)
        }
        .sheet(isPresented: $showNewEntry) {
            NewEntrySheet(model: model)
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.errorMessage != nil }, set: { if !$0 { model.errorMessage = nil } })
        ) {
            Button("OK") {}
        } message: {
            Text(model.errorMessage ?? "")
        }
        .confirmationDialog(
            "Discard changes?",
            isPresented: $model.askDiscard,
            titleVisibility: .visible
        ) {
            Button("Discard", role: .destructive) { model.confirmDiscard() }
            Button("Cancel", role: .cancel) { model.cancelDiscard() }
        } message: {
            Text("You have unsaved edits. Discard them?")
        }
        .task { await model.reload() }
    }

    @ViewBuilder
    private var detailColumn: some View {
        switch model.sidebar {
        case .overview, nil:
            OverviewView(model: model)
        case .global, .skills, .folder, .folderOnly:
            HSplitView {
                ItemListView(model: model)
                    .frame(minWidth: 260, idealWidth: 340, maxWidth: 520)
                if let item = model.selectedItem {
                    ItemDetailView(model: model, item: item, onDelete: { await model.delete(item) })
                        .id(item.id)
                        .frame(minWidth: 380, maxWidth: .infinity, maxHeight: .infinity)
                        .layoutPriority(1)
                } else {
                    ContentUnavailableView(
                        model.sidebar == .skills ? "No skill selected" : "No memory selected",
                        systemImage: "doc.text"
                    )
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
        }
    }
}
