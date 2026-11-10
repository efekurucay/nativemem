import Foundation

public enum MarkdownBlock: Equatable, Sendable {
    case heading(level: Int, text: String)
    case bullet(String)
    case numbered(Int, String)
    case code(String)
    case paragraph(String)

    fileprivate func continued(with line: String) -> MarkdownBlock {
        switch self {
        case .bullet(let text): .bullet(text + " " + line)
        case .numbered(let number, let text): .numbered(number, text + " " + line)
        case .paragraph(let text): .paragraph(text + " " + line)
        case .heading, .code: self
        }
    }
}

/// Simple block structure for memory bodies. Hard-wrapped source lines are joined so text flows to the pane width.
public enum MarkdownBlocks {
    public static func parse(_ source: String) -> [MarkdownBlock] {
        var blocks: [MarkdownBlock] = []
        var open: MarkdownBlock?
        var fence: [String]?

        func flush() {
            if let open { blocks.append(open) }
            open = nil
        }

        for slice in source.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline) {
            let line = String(slice)
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if fence != nil {
                if trimmed.hasPrefix("```") {
                    blocks.append(.code((fence ?? []).joined(separator: "\n")))
                    fence = nil
                } else {
                    fence?.append(line)
                }
            } else if trimmed.hasPrefix("```") {
                flush()
                fence = []
            } else if trimmed.isEmpty {
                flush()
            } else if let (level, text) = heading(trimmed) {
                flush()
                blocks.append(.heading(level: level, text: text))
            } else if let text = bullet(trimmed) {
                flush()
                open = .bullet(text)
            } else if let (number, text) = numbered(trimmed) {
                flush()
                open = .numbered(number, text)
            } else {
                open = open?.continued(with: trimmed) ?? .paragraph(trimmed)
            }
        }
        flush()
        if let fence { blocks.append(.code(fence.joined(separator: "\n"))) }
        return blocks
    }

    /// Inline bold, italic, code and links; plain text when the markdown cannot be parsed.
    public static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnly)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }

    private static func heading(_ line: String) -> (Int, String)? {
        let hashes = line.prefix { $0 == "#" }.count
        let rest = line.dropFirst(hashes)
        guard (1...6).contains(hashes), rest.first == " " else { return nil }
        return (min(hashes, 3), rest.trimmingCharacters(in: .whitespaces))
    }

    private static func bullet(_ line: String) -> String? {
        guard line.hasPrefix("- ") || line.hasPrefix("* ") else { return nil }
        return line.dropFirst(2).trimmingCharacters(in: .whitespaces)
    }

    private static func numbered(_ line: String) -> (Int, String)? {
        let digits = line.prefix { $0.isASCII && $0.isNumber }
        let rest = line.dropFirst(digits.count)
        guard !digits.isEmpty, rest.hasPrefix(". "), let number = Int(digits) else { return nil }
        return (number, rest.dropFirst(2).trimmingCharacters(in: .whitespaces))
    }
}
