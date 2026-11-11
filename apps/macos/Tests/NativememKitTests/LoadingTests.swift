import Foundation
import Testing
@testable import NativememKit

/// Only this temporary script is executed; no native CLI, agent home or upstream is involved.
private struct LoadingCLI {
    let directory: URL
    let cli: NativememCLI

    init() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("nativemem-loading-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let snapshot: [String: Any] = [
            "items": try JSONSerialization.jsonObject(with: fixture("items")),
            "status": try JSONSerialization.jsonObject(with: fixture("status")),
            "doctor": try JSONSerialization.jsonObject(with: fixture("doctor_ok")), "sessions_included": false,
        ]
        try JSONSerialization.data(withJSONObject: snapshot).write(to: directory.appendingPathComponent("snapshot.json"))
        var sessions = try #require(JSONSerialization.jsonObject(with: fixture("items-sessions")) as? [String: Any])
        sessions["session_status"] = [["id": "claude", "originals": 0, "replicas": 0, "sessions": 3,
                                      "skipped": [["label": "synthetic.jsonl", "reason": "Synthetic skipped session"]]]]
        sessions["session_notice"] = "Synthetic retention notice"
        try JSONSerialization.data(withJSONObject: sessions).write(to: directory.appendingPathComponent("sessions.json"))
        let quoted = "'" + directory.path.replacingOccurrences(of: "'", with: "'\\''") + "'"
        let script = """
        #!/bin/sh
        cd \(quoted) || exit 1
        printf '%s\\n' "$*" >> calls
        case "$*" in
          '--no-sessions snapshot --json')
            if [ -f snapshot-fail ]; then echo '{"error":"Synthetic snapshot failure","code":"io"}'; exit 1; fi
            response="$(/bin/cat snapshot.json)"
            if [ -f snapshot-delay ]; then /usr/bin/touch snapshot-started; /bin/sleep 0.2; fi
            printf '%s\\n' "$response" ;;
          'items --kind session --json')
            if [ -f session-fail ]; then
              /bin/rm session-fail
              echo '{"error":"Synthetic read failure","code":"io"}'
              exit 1
            fi
            if [ -f session-delay ]; then /bin/sleep 0.2; fi
            /bin/cat sessions.json ;;
          'mcp list --json')
            echo '{"servers":[{"name":"demo","transport":"stdio","enabled":true,"auth":"none","auth_state":"none","args":[],"env":{},"headers":{},"secrets":[]}]}' ;;
          'mcp status --json') echo '{"agents":[]}' ;;
          'mcp test --json -- demo') echo '{"ok":true,"tools":[]}' ;;
          'subagent list --json') echo '{"subagents":[]}' ;;
          *) echo '{"error":"Unexpected command","code":"invalid"}'; exit 1 ;;
        esac
        """
        let url = directory.appendingPathComponent("nativemem")
        try Data(script.utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        cli = NativememCLI(binary: url)
    }

    var calls: [String] {
        ((try? String(contentsOf: directory.appendingPathComponent("calls"), encoding: .utf8)) ?? "")
            .split(separator: "\n").map(String.init)
    }

    func mark(_ name: String) throws { try Data().write(to: directory.appendingPathComponent(name)) }
    func remove(_ name: String) throws { try FileManager.default.removeItem(at: directory.appendingPathComponent(name)) }
    func clean() { try? FileManager.default.removeItem(at: directory) }
}

@MainActor @Suite struct LoadingTests {
    @Test func startupReadsOneSnapshotAndLeavesExpensiveScreensUnloaded() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        let model = AppModel(cli: fake.cli)
        await model.reload()
        #expect(fake.calls == ["--no-sessions snapshot --json"])
        #expect(model.index.items.count == 2)
        #expect(model.status != nil && model.doctor != nil)
        #expect(!model.isLoading && model.errorMessage == nil)
        #expect(!model.sessionsLoaded && !model.mcp.isLoaded && !model.subagents.isLoaded)
    }

    @Test func sessionsLoadOnceAndAreInvalidatedByReload() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        let model = AppModel(cli: fake.cli)
        await model.reload()
        await model.loadSessions()
        await model.loadSessions()
        #expect(model.sessionsLoaded && model.sessionIndex.sessions.count == 4)
        #expect(model.status?.agents.first?.sessions == 3)
        #expect(model.status?.agents.first?.skipped.filter { $0.label == "synthetic.jsonl" }.count == 1)
        #expect(model.doctor?.agents.first?.noticeLines == ["Synthetic retention notice"])
        #expect(fake.calls.filter { $0 == "items --kind session --json" }.count == 1)
        await model.reload()
        #expect(!model.sessionsLoaded)
        await model.loadSessions()
        #expect(fake.calls.filter { $0 == "items --kind session --json" }.count == 2)
    }

    @Test func sessionFailuresCanBeRetriedAndAreNotEmptyResults() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        try fake.mark("session-fail")
        let model = AppModel(cli: fake.cli)
        await model.loadSessions()
        #expect(!model.sessionsLoaded && model.sessionError != nil && !model.isLoadingSessions)
        await model.loadSessions()
        #expect(model.sessionsLoaded && model.sessionError == nil)
    }

    @Test func cancelledSessionsStayUnloadedWithoutAnError() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        try fake.mark("session-delay")
        let model = AppModel(cli: fake.cli)
        let loading = Task { await model.loadSessions() }
        for _ in 0..<100 where !model.isLoadingSessions { try await Task.sleep(for: .milliseconds(5)) }
        #expect(model.isLoadingSessions)
        loading.cancel()
        await loading.value
        #expect(!model.isLoadingSessions && !model.sessionsLoaded && model.sessionError == nil)
    }

    @Test func aFailedReloadDoesNotLeaveSupersededSessionsLoading() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        let model = AppModel(cli: fake.cli)
        await model.reload()
        try fake.mark("session-delay")
        let loading = Task { await model.loadSessions() }
        for _ in 0..<100 where !model.isLoadingSessions { try await Task.sleep(for: .milliseconds(5)) }
        try fake.mark("snapshot-fail")
        await model.reload()
        await loading.value
        #expect(model.errorMessage != nil && !model.isLoadingSessions && !model.sessionsLoaded)
        try fake.remove("session-delay")
        await model.loadSessions()
        #expect(model.sessionsLoaded && model.sessionError == nil)
    }

    @Test func hubReadsAndUpstreamTestsStartOnlyOnDemand() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        let model = AppModel(cli: fake.cli)
        await model.reload()
        #expect(fake.calls.count == 1)
        await model.mcp.loadIfNeeded()
        await model.mcp.loadIfNeeded()
        for _ in 0..<500 where model.mcp.tests["demo"] == nil { try await Task.sleep(for: .milliseconds(10)) }
        #expect(model.mcp.tests["demo"]?.ok == true)
        #expect(fake.calls.filter { $0 == "mcp list --json" }.count == 1)
        #expect(fake.calls.filter { $0 == "mcp test --json -- demo" }.count == 1)
        await model.subagents.loadIfNeeded()
        await model.subagents.loadIfNeeded()
        #expect(model.subagents.isLoaded)
        #expect(fake.calls.filter { $0 == "subagent list --json" }.count == 1)
        await model.reload()
        #expect(!model.mcp.isLoaded && !model.subagents.isLoaded)
        #expect(fake.calls.filter { $0 == "mcp list --json" }.count == 1)
    }

    @Test func aSlowSnapshotCannotOverwriteANewerReload() async throws {
        let fake = try LoadingCLI()
        defer { fake.clean() }
        try fake.mark("snapshot-delay")
        let model = AppModel(cli: fake.cli)
        let loading = Task { await model.reload() }
        let marker = fake.directory.appendingPathComponent("snapshot-started").path
        for _ in 0..<1000 where !FileManager.default.fileExists(atPath: marker) { try await Task.sleep(for: .milliseconds(5)) }
        #expect(FileManager.default.fileExists(atPath: marker))
        try fake.remove("snapshot-delay")
        let snapshotURL = fake.directory.appendingPathComponent("snapshot.json")
        var document = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: snapshotURL)) as? [String: Any])
        document["items"] = ["items": []]
        try JSONSerialization.data(withJSONObject: document).write(to: snapshotURL)
        await model.reload()
        await loading.value
        #expect(model.index.items.isEmpty && model.dataRevision == 1 && !model.isLoading)
    }
}
