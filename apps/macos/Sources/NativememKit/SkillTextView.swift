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
        .task(id: item.id) {
            do {
                text = try await model.skillText(of: item)
            } catch {
                failure = error.localizedDescription
            }
        }
    }
}
