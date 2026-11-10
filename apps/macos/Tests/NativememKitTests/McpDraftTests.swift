import Foundation
import Testing
@testable import NativememKit

@Suite struct McpDraftTests {
    private func json(_ draft: McpServerDraft) throws -> [String: Any] {
        let data = try draft.document.encoded()
        return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func row(_ key: String, _ value: String, secret: Bool = false) -> McpKeyValueRow {
        var row = McpKeyValueRow()
        row.key = key
        row.value = value
        row.isSecret = secret
        return row
    }

    @Test func anEmptyDraftNeedsANameAndACommand() {
        var draft = McpServerDraft()
        #expect(draft.problem == "Enter a name.")
        draft.name = " github "
        #expect(draft.problem == "Enter the command.")
        draft.command = "npx"
        #expect(draft.isValid)
        draft.kind = .remote
        #expect(draft.problem == "Enter the address.")
        draft.url = "https://mcp.example.com/mcp"
        #expect(draft.isValid)
    }

    @Test func aLocalServerIsSentWithItsArgumentsAndEnvironment() throws {
        var draft = McpServerDraft()
        draft.name = " github "
        draft.command = " npx "
        draft.argumentLines = "-y\n\n  \n@scope/pkg"
        draft.rows = [row("LOG", "debug"), row("", "")]
        let document = try json(draft)
        #expect(document["name"] as? String == "github")
        #expect(document["transport"] as? String == "stdio")
        #expect(document["command"] as? String == "npx")
        #expect(document["args"] as? [String] == ["-y", "@scope/pkg"])
        #expect(document["env"] as? [String: String] == ["LOG": "debug"])
        #expect(document["auth"] as? String == "none")
        #expect(document["enabled"] as? Bool == true)
        #expect(document["url"] == nil && document["headers"] == nil)
    }

    @Test func aSecretRowStoresTheReferenceAndKeepsTheValueOutOfTheDocument() throws {
        var draft = McpServerDraft()
        draft.name = "github"
        draft.command = "npx"
        draft.rows = [row("GITHUB_TOKEN", "ghp_value", secret: true), row("LOG", "debug")]
        #expect(draft.isValid)
        let data = try draft.document.encoded()
        let text = String(decoding: data, as: UTF8.self)
        #expect(!text.contains("ghp_value"))
        let document = try json(draft)
        #expect(document["env"] as? [String: String] == [
            "GITHUB_TOKEN": "secret:github-GITHUB_TOKEN", "LOG": "debug"])
        let secrets = draft.secretValues
        #expect(secrets.count == 1)
        #expect(secrets[0].id == "github-GITHUB_TOKEN" && secrets[0].value == "ghp_value")
    }

    @Test func aRemoteServerIsSentWithItsHeadersAndSignIn() throws {
        var draft = McpServerDraft()
        draft.kind = .remote
        draft.name = "docs"
        draft.url = "https://mcp.example.com/mcp"
        draft.usesOAuth = true
        draft.rows = [row("X-Api-Key", "abc", secret: true)]
        let document = try json(draft)
        #expect(document["transport"] as? String == "http")
        #expect(document["url"] as? String == "https://mcp.example.com/mcp")
        #expect(document["headers"] as? [String: String] == ["X-Api-Key": "secret:docs-X-Api-Key"])
        #expect(document["auth"] as? String == "oauth")
        #expect(document["command"] == nil && document["args"] == nil && document["env"] == nil)
        #expect(draft.secretValues.map(\.id) == ["docs-X-Api-Key"])
    }

    @Test func rowsAreCheckedForNamesDuplicatesAndValues() {
        var draft = McpServerDraft()
        draft.name = "s"
        draft.command = "c"
        draft.rows = [row("", "v")]
        #expect(draft.problem == "Every row needs a name.")
        draft.rows = [row("A", "1"), row("A", "2")]
        #expect(draft.problem == "A appears twice.")
        draft.rows = [row("A", "")]
        #expect(draft.problem == "Enter a value for A.")
        draft.rows = [row("A", "", secret: true)]
        #expect(draft.problem == "Enter a value for A.")
        draft.rows = [row("", "")]
        #expect(draft.isValid)
    }

    @Test func switchingTheTypeClearsTheRowsAndTheSignIn() {
        var draft = McpServerDraft()
        draft.kind = .remote
        draft.usesOAuth = true
        draft.rows = [row("A", "1")]
        draft.kind = .local
        #expect(draft.rows.isEmpty && !draft.usesOAuth)
        draft.rows = [row("B", "2")]
        draft.kind = .local
        #expect(draft.rows.count == 1)
    }

    @Test func editingStartsFromTheServerWithoutItsValues() throws {
        let server = try decodeMcp(McpListDocument.self, from: fixture("mcp_list")).servers[0]
        var draft = McpServerDraft(editing: server)
        #expect(draft.isEditing && draft.name == "github" && draft.kind == .local)
        #expect(draft.command == "npx" && draft.arguments == ["-y", "pkg"])
        #expect(draft.rows.map(\.key) == ["GITHUB_TOKEN", "LOG"])
        #expect(draft.rows[0].isSecret && draft.rows[0].reference == "secret:github-GITHUB_TOKEN")
        #expect(draft.rows.allSatisfy { $0.value.isEmpty })
        #expect(draft.rows[1].isHiddenLiteral && !draft.rows[1].isSecret)
        #expect(draft.hasHiddenValues)
        #expect(draft.problem == "Type the value of LOG again, or remove the row.")
        draft.rows[1].value = "debug"
        #expect(draft.isValid)
        #expect(draft.secretValues.isEmpty)
        let document = try json(draft)
        #expect(document["env"] as? [String: String] == [
            "GITHUB_TOKEN": "secret:github-GITHUB_TOKEN", "LOG": "debug"])
        #expect(document["cwd"] as? String == "/Users/x/app")
        #expect(document["enabled"] as? Bool == true)
    }

    @Test func anEditedSecretKeepsItsReferenceAndStoresANewValueUnderIt() throws {
        let server = try decodeMcp(McpListDocument.self, from: fixture("mcp_list")).servers[0]
        var draft = McpServerDraft(editing: server)
        draft.rows.remove(at: 1)
        draft.rows[0].value = "new-token"
        let secrets = draft.secretValues
        #expect(secrets.count == 1 && secrets[0].id == "github-GITHUB_TOKEN" && secrets[0].value == "new-token")
    }

    @Test func editingKeepsADisabledServerDisabled() throws {
        let server = try decodeMcp(McpListDocument.self, from: fixture("mcp_list")).servers[1]
        let draft = McpServerDraft(editing: server)
        #expect(draft.kind == .remote && draft.usesOAuth && draft.url == "https://mcp.example.com/mcp")
        #expect(draft.rows.map(\.key) == ["X-Api-Key"] && draft.rows[0].isSecret)
        #expect(draft.isValid)
        let document = try json(draft)
        #expect(document["enabled"] as? Bool == false)
        #expect(document["headers"] as? [String: String] == ["X-Api-Key": "secret:docs-X-Api-Key"])
    }
}
