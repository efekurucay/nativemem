import Foundation
import Observation

/// The Subagents screen: the definitions and the actions on them. Everything goes through
/// `nativemem subagent ... --json`; nothing here knows how a harness is started.
@MainActor @Observable
public final class SubagentModel {
    public var cli: NativememCLI?
    public private(set) var subagents: [Subagent] = []
    public private(set) var loadError: String?
    public private(set) var isLoaded = false
    public var selectedName: String?
    public var errorMessage: String?

    public init() {}

    public var selectedSubagent: Subagent? {
        subagents.first { $0.name == selectedName }
    }

    public func reload() async {
        guard let cli else { return }
        do {
            subagents = try await cli.subagentList()
            loadError = nil
        } catch {
            subagents = []
            loadError = error.localizedDescription
        }
        isLoaded = true
        if let selectedName, !subagents.contains(where: { $0.name == selectedName }) {
            self.selectedName = nil
        }
    }

    public func remove(_ name: String) async {
        guard let cli else { return }
        do {
            _ = try await cli.subagentRemove(name)
        } catch {
            errorMessage = error.localizedDescription
        }
        await reload()
    }

    /// Adds or replaces the definition; a CLI error (`exists`, `not_installed`, `invalid`) is thrown for the sheet to show.
    public func save(_ draft: SubagentDraft) async throws {
        guard let cli else { throw CLIError.notFound }
        if draft.isEditing {
            _ = try await cli.subagentEdit(draft.trimmedName, document: draft.document)
        } else {
            _ = try await cli.subagentAdd(draft.document)
        }
        await reload()
        selectedName = draft.trimmedName
    }

    public func makeTrial(for subagent: Subagent) -> SubagentTrial? {
        cli.map { SubagentTrial(cli: $0, name: subagent.name) }
    }
}

/// The "Try it" box of one subagent: the prompt, an optional working folder and the run.
@MainActor @Observable
public final class SubagentTrial {
    public enum Phase: Equatable, Sendable {
        case idle
        case running
        /// The command ran; `ok` is false when the harness failed.
        case finished(SubagentRunResult)
        /// The command itself failed, for example because the subagent no longer exists.
        case failed(String)
        case cancelled
    }

    public var prompt = ""
    /// An absolute folder path; nil runs in the folder of the app.
    public var folder: String?
    public private(set) var phase = Phase.idle

    private let cli: NativememCLI
    private let name: String
    private var task: Task<Void, Never>?
    /// Tells a finished run from one that was cancelled and replaced.
    private var runNumber = 0

    public init(cli: NativememCLI, name: String) {
        self.cli = cli
        self.name = name
    }

    public var isRunning: Bool { phase == .running }

    public var canRun: Bool { !isRunning && !prompt.trimmed.isEmpty }

    public func run() {
        guard canRun else { return }
        phase = .running
        runNumber += 1
        let (cli, name, prompt, folder, number) = (cli, name, prompt, folder, runNumber)
        task = Task {
            let outcome: Phase
            do {
                outcome = .finished(try await cli.subagentTry(name, prompt: prompt, folder: folder))
            } catch is CancellationError {
                outcome = .cancelled
            } catch {
                outcome = .failed(error.localizedDescription)
            }
            if number == runNumber, phase == .running { phase = outcome }
        }
    }

    /// Ends the command, which ends the harness it started.
    public func cancel() {
        guard isRunning else { return }
        task?.cancel()
        phase = .cancelled
    }
}
