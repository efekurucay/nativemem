import Foundation
import Testing
@testable import NativememKit

@Suite struct McpContractTests {
    private func servers() throws -> [McpServer] {
        try decodeMcp(McpListDocument.self, from: fixture("mcp_list")).servers
    }

    @Test func decodesTheServerList() throws {
        let servers = try servers()
        #expect(servers.map(\.name) == ["github", "docs", "wiki"])
        let github = servers[0]
        #expect(github.transport == .stdio && github.enabled && github.auth == .none)
        #expect(github.authState == .none)
        #expect(github.command == "npx" && github.args == ["-y", "pkg"] && github.cwd == "/Users/x/app")
        #expect(github.secrets == ["github-GITHUB_TOKEN"])
        let docs = servers[1]
        #expect(docs.transport == .http && !docs.enabled && docs.auth == .oauth)
        #expect(docs.authState == .needsLogin)
        #expect(docs.url == "https://mcp.example.com/mcp" && docs.command == nil)
        #expect(servers[2].authState == .signedIn)
    }

    @Test func environmentAndHeaderNamesKeepTheirSpelling() throws {
        let servers = try servers()
        #expect(servers[0].env == ["GITHUB_TOKEN": "secret:github-GITHUB_TOKEN", "LOG": "<literal>"])
        #expect(servers[1].headers == ["X-Api-Key": "secret:docs-X-Api-Key"])
        #expect(McpServer.isSecretReference("secret:github-GITHUB_TOKEN"))
        #expect(!McpServer.isSecretReference("<literal>"))
    }

    @Test func decodesTestResults() throws {
        let ok = try decodeMcp(McpTestResult.self, from: fixture("mcp_test_ok"))
        #expect(ok == McpTestResult(ok: true, tools: ["echo", "env_matches"], error: nil))
        let failed = try decodeMcp(McpTestResult.self, from: fixture("mcp_test_failed"))
        #expect(!failed.ok && failed.tools.isEmpty && failed.error == "connection refused")
    }

    @Test func decodesTheManagementAnswers() throws {
        #expect(try decodeMcp(McpAdded.self, from: fixture("mcp_add")).added == "github")
        #expect(try decodeMcp(McpEdited.self, from: fixture("mcp_edit")).edited == "github")
        #expect(try decodeMcp(McpRemoved.self, from: fixture("mcp_remove")).removed == "github")
        let enabled = try decodeMcp(McpEnabled.self, from: fixture("mcp_enable"))
        #expect(enabled.name == "github" && enabled.enabled)
        #expect(try decodeMcp(McpSecretStored.self, from: fixture("mcp_set_secret")).secret == "github-GITHUB_TOKEN")
    }

    @Test func decodesSignInAndSignOut() throws {
        #expect(try decodeMcp(McpSignedIn.self, from: fixture("mcp_login")).signedIn == "wiki")
        let out = try decodeMcp(McpSignedOut.self, from: fixture("mcp_logout"))
        #expect(out.signedOut == "wiki" && out.wasSignedIn)
    }

    @Test func decodesInstallWithItsWarningsAndErrors() throws {
        let document = try decodeMcp(McpRegistrationDocument.self, from: fixture("mcp_install"))
        #expect(document.command == "/Users/x/.cargo/bin/nativemem")
        #expect(document.args?.prefix(2) == ["mcp", "serve"])
        #expect(document.agents.map(\.agent) == ["claude", "codex", "opencode", "pi"])
        #expect(document.agents.map(\.state) == [.installed, .failed, .agentMissing, .installed])
        #expect(document.agents[1].code == "parse")
        #expect(document.agents[1].message?.hasPrefix("cannot parse") == true)
        #expect(document.agents[2].path == nil)
        #expect(document.agents[3].warnings.count == 1)
        #expect(document.agents[0].displayName == "Claude Code")
    }

    @Test func decodesUninstallWithoutACommand() throws {
        let document = try decodeMcp(McpRegistrationDocument.self, from: fixture("mcp_uninstall"))
        #expect(document.command == nil && document.args == nil)
        #expect(document.agents.map(\.state) == [.notInstalled, .notInstalled])
    }

    @Test func statusCoversEveryRegistrationState() throws {
        let document = try decodeMcp(McpRegistrationDocument.self, from: fixture("mcp_status"))
        #expect(document.agents.map(\.state) == [.installed, .outdated, .notInstalled, .agentMissing])
    }

    @Test func decodesTheImportPreview() throws {
        let plan = try decodeMcp(McpImportPlan.self, from: fixture("mcp_import_preview"))
        #expect(!plan.applied)
        #expect(plan.servers(.add).map(\.name) == ["shared"])
        #expect(plan.servers(.present).map(\.name) == ["known"])
        #expect(plan.servers(.conflict).map(\.name) == ["clash"])
        #expect(plan.servers[0].secrets == ["shared-API_KEY"])
        #expect(plan.servers[0].agentsText == "Claude Code, Codex, Pi")
        #expect(plan.servers[2].agentsText == "Claude Code, the hub")
        #expect(plan.skipped == [McpImportSkipped(
            agent: "claude", name: "sse-server", reason: "sse transport is not supported by the hub")])
        #expect(plan.warnings.count == 1)
        #expect(plan.summary == McpImportSummary(add: 1, present: 1, conflicts: 1, skipped: 1))
        #expect(plan.headline == "1 to add, 1 already in the hub, 1 conflict")
    }

    @Test func decodesTheAppliedImport() throws {
        let plan = try decodeMcp(McpImportPlan.self, from: fixture("mcp_import_apply"))
        #expect(plan.applied)
        #expect(plan.resultSummary == "Imported 1 server.")
        var nothing = plan
        nothing.summary.add = 0
        #expect(nothing.resultSummary == "Nothing was imported.")
        #expect(nothing.headline == "Your agents have no servers to import.")
    }
}

@Suite struct McpStateTests {
    private func servers() throws -> [McpServer] {
        try decodeMcp(McpListDocument.self, from: fixture("mcp_list")).servers
    }

    private func test(_ name: String) throws -> McpTestResult {
        try decodeMcp(McpTestResult.self, from: fixture(name))
    }

    @Test func aDisabledServerIsDisabledWhateverElseIsTrue() throws {
        let docs = try servers()[1]
        #expect(McpServerState.derive(docs, test: try test("mcp_test_failed")) == .disabled)
        #expect(McpServerState.derive(docs, test: nil) == .disabled)
    }

    @Test func aServerWithoutAnswerIsChecking() throws {
        let github = try servers()[0]
        #expect(McpServerState.derive(github, test: nil) == .checking)
    }

    @Test func aPassingTestMeansConnected() throws {
        let github = try servers()[0]
        #expect(McpServerState.derive(github, test: try test("mcp_test_ok")) == .connected)
    }

    @Test func aFailingTestIsAnErrorWithItsMessage() throws {
        let github = try servers()[0]
        #expect(McpServerState.derive(github, test: try test("mcp_test_failed")) == .error("connection refused"))
        #expect(McpServerState.derive(github, test: McpTestResult(ok: false)) == .error("The server did not answer."))
    }

    @Test func signInIsNeededWithoutTokensOrWhenTheTestSaysSo() throws {
        var wiki = try servers()[2]
        #expect(McpServerState.derive(wiki, test: try test("mcp_test_sign_in")) == .needsSignIn)
        #expect(McpServerState.derive(wiki, test: try test("mcp_test_ok")) == .connected)
        wiki.authState = .needsLogin
        #expect(McpServerState.derive(wiki, test: nil) == .needsSignIn)
        #expect(McpServerState.derive(wiki, test: try test("mcp_test_ok")) == .needsSignIn)
    }

    @Test func onlySignInAndErrorNeedAttention() {
        #expect(McpServerState.needsSignIn.needsAttention)
        #expect(McpServerState.error("x").needsAttention)
        #expect(!McpServerState.checking.needsAttention)
        #expect(!McpServerState.connected.needsAttention)
        #expect(!McpServerState.disabled.needsAttention)
    }

    @Test func theBadgeCountsServersThatNeedSignInOrAreInError() throws {
        var list = try servers()
        list[1].enabled = true
        let tests = [
            "github": try test("mcp_test_failed"),
            "wiki": try test("mcp_test_sign_in"),
        ]
        // github is in error, docs needs sign-in (no tokens), wiki needs sign-in (its test says so).
        #expect(McpServerState.attentionCount(servers: list, tests: tests) == 3)
        let healthy = ["github": try test("mcp_test_ok"), "wiki": try test("mcp_test_ok")]
        #expect(McpServerState.attentionCount(servers: list, tests: healthy) == 1)
    }

    @Test func theBadgeIgnoresDisabledCheckingAndConnectedServers() throws {
        let list = try servers()
        #expect(McpServerState.attentionCount(servers: list, tests: [:]) == 0)
        #expect(McpServerState.attentionCount(servers: [], tests: [:]) == 0)
    }

    @Test func titlesAreShortPlainWords() {
        #expect(McpServerState.connected.title == "Connected")
        #expect(McpServerState.needsSignIn.title == "Needs sign-in")
        #expect(McpServerState.error("x").title == "Error")
        #expect(McpServerState.disabled.title == "Disabled")
    }
}
