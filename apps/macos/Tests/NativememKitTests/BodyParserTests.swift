import Foundation
import Testing
@testable import NativememKit

@Suite struct BodyParserTests {
    @Test func joinsParagraphLines() {
        let blocks = MarkdownBlocks.parse("This is a sentence\nthat wraps onto\nthree lines.")
        #expect(blocks == [.paragraph("This is a sentence that wraps onto three lines.")])
    }

    @Test func parsesHeadings() {
        let blocks = MarkdownBlocks.parse("# Title\n## Sub\n### Deep")
        #expect(blocks == [
            .heading(level: 1, text: "Title"),
            .heading(level: 2, text: "Sub"),
            .heading(level: 3, text: "Deep"),
        ])
    }

    @Test func joinsBulletItemContinuations() {
        let blocks = MarkdownBlocks.parse("- first item\n  continues here\n- second")
        #expect(blocks == [
            .bullet("first item continues here"),
            .bullet("second"),
        ])
    }

    @Test func joinsNumberedItemContinuations() {
        let blocks = MarkdownBlocks.parse("1. one\n2. two\n   more")
        #expect(blocks == [
            .numbered(1, "one"),
            .numbered(2, "two more"),
        ])
    }

    @Test func codeFenceLeftUntouched() {
        let blocks = MarkdownBlocks.parse("```swift\nlet x = 1\nfunc foo() {}\n```")
        #expect(blocks == [.code("let x = 1\nfunc foo() {}")])
    }

    @Test func mixesBlocks() {
        let source = """
        # Heading

        A paragraph with
        two lines.

        - bullet one
        """
        #expect(MarkdownBlocks.parse(source) == [
            .heading(level: 1, text: "Heading"),
            .paragraph("A paragraph with two lines."),
            .bullet("bullet one"),
        ])
    }
}
