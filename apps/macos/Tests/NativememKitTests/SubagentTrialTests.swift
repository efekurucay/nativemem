import Foundation
import Testing
@testable import NativememKit

/// Runs against a script that stands in for the `nativemem` binary; no agent is involved.
@Suite struct SubagentTrialTests {
    private func script(_ body: String) throws -> NativememCLI {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nativemem-trial-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("nativemem")
        try Data("#!/bin/sh\n\(body)\n".utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
        return NativememCLI(binary: url)
    }

    @Test func theRunNeedsAPrompt() async throws {
        let trial = await SubagentTrial(cli: try script("exit 1"), name: "scout")
        #expect(await !trial.canRun)
        await MainActor.run { trial.prompt = "  \n" }
        #expect(await !trial.canRun)
        await MainActor.run { trial.prompt = "hello" }
        #expect(await trial.canRun)
    }

    @Test func thePromptTravelsOnStdinAndTheFolderAsAnOption() async throws {
        let cli = try script(#"printf '{"ok":true,"text":"%s|%s","exit_code":0,"duration_ms":5,"error":null}' "$*" "$(cat)""#)
        let result = try await cli.subagentTry("scout", prompt: "hello", folder: "/tmp/work")
        #expect(result.ok)
        #expect(result.text == "subagent run --prompt-file - --json --cwd=/tmp/work -- scout|hello")
    }

    @Test func aFailedRunIsAResultAndAMissingSubagentIsAnError() async throws {
        let failed = try script(#"echo '{"ok":false,"text":"","exit_code":4,"duration_ms":7,"error":"claude exited with code 4"}'; exit 2"#)
        let result = try await failed.subagentTry("scout", prompt: "hi", folder: nil)
        #expect(!result.ok && result.exitCode == 4)
        let missing = try script(#"echo '{"error":"no subagent named \"scout\"","code":"not_found"}'; exit 1"#)
        await #expect(throws: CLIError.failed(message: "no subagent named \"scout\"", code: "not_found")) {
            _ = try await missing.subagentTry("scout", prompt: "hi", folder: nil)
        }
    }

    @Test func cancellingTerminatesTheCommand() async throws {
        let cli = try script("exec /bin/sleep 30")
        let started = Date()
        let task = Task { try await cli.subagentTry("scout", prompt: "hi", folder: nil) }
        try await Task.sleep(for: .milliseconds(300))
        task.cancel()
        await #expect(throws: CancellationError.self) { _ = try await task.value }
        #expect(Date().timeIntervalSince(started) < 10)
    }

    @Test func theTrialShowsRunningThenTheAnswer() async throws {
        let cli = try script(##"cat >/dev/null; echo '{"ok":true,"text":"# Hi","exit_code":0,"duration_ms":5,"error":null}'"##)
        let trial = await SubagentTrial(cli: cli, name: "scout")
        await MainActor.run {
            trial.prompt = "hello"
            trial.run()
            #expect(trial.phase == .running)
        }
        for _ in 0..<100 where await trial.phase == .running {
            try await Task.sleep(for: .milliseconds(50))
        }
        #expect(await trial.phase == .finished(SubagentRunResult(ok: true, text: "# Hi", exitCode: 0, durationMs: 5)))
    }

    @Test func cancellingTheTrialStopsTheCommandAndShowsCancelled() async throws {
        let cli = try script("exec /bin/sleep 30")
        let trial = await SubagentTrial(cli: cli, name: "scout")
        await MainActor.run {
            trial.prompt = "hello"
            trial.run()
        }
        try await Task.sleep(for: .milliseconds(300))
        await MainActor.run { trial.cancel() }
        #expect(await trial.phase == .cancelled)
        try await Task.sleep(for: .milliseconds(300))
        #expect(await trial.phase == .cancelled)
    }
}
