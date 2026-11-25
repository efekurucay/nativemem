import SwiftUI

/// The SKILL.md of a skill, read-only, as the CLI prints it.
struct SkillTextView: View {
    @Bindable var model: AppModel
    let item: LogicalItem
    @State private var text: String?
    @State private var failure: String?

    var body: some View {
        Group {
            if let text {
                Text(text)
                    .font(.body.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .font(.callout)
            } else {
                ProgressView()
            }
        }
        .task(id: item.primary.hash) {
            text = nil
            failure = nil
            do {
                let loaded = try await model.skillText(of: item)
                if !Task.isCancelled { text = loaded }
            } catch {
                if !Task.isCancelled { failure = error.localizedDescription }
            }
        }
    }
}
