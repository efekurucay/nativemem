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

    var body: some View {
        let sections = model.visibleSections
        Group {
            if sections.isEmpty {
                if model.itemSearch.isEmpty {
                    ContentUnavailableView("No memories", systemImage: "tray")
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
        .searchable(text: $model.itemSearch, prompt: "Search memories")
    }

    private func row(_ item: LogicalItem) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(item.primary.name).lineLimit(1)
                if let category = item.primary.category {
                    CategoryTag(category: category)
                }
            }
            if let description = item.primary.description, !description.isEmpty {
                Text(description)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .tag(item.id)
    }

    private var itemBinding: Binding<String?> {
        Binding(
            get: { model.selectedItemID },
            set: { model.requestItem($0) }
        )
    }
}
