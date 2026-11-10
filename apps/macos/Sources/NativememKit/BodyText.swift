import SwiftUI

/// Renders parsed markdown blocks with native SwiftUI text, keeping the text selectable.
struct BodyText: View {
    let blocks: [MarkdownBlock]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(blocks.indices, id: \.self) { index in
                block(blocks[index])
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .textSelection(.enabled)
    }

    @ViewBuilder
    private func block(_ block: MarkdownBlock) -> some View {
        switch block {
        case .heading(let level, let text):
            Text(MarkdownBlocks.inline(text))
                .font(headingFont(level))
                .frame(maxWidth: .infinity, alignment: .leading)
        case .bullet(let text):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("•")
                Text(MarkdownBlocks.inline(text))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        case .numbered(let number, let text):
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("\(number).").monospacedDigit()
                Text(MarkdownBlocks.inline(text))
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        case .code(let code):
            Text(code)
                .font(.body.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
        case .paragraph(let text):
            Text(MarkdownBlocks.inline(text))
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: .title2.bold()
        case 2: .title3.bold()
        default: .headline
        }
    }
}
