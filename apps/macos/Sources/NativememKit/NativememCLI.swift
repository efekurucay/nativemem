import Foundation

public enum CLIError: Error, LocalizedError, Equatable {
    case notFound
    case failed(message: String, code: String)
    case unreadable(exitCode: Int32)

    public var errorDescription: String? {
        switch self {
        case .notFound: "The nativemem command could not be found."
        case .failed(let message, _): message
        case .unreadable(let exitCode): "nativemem stopped with exit code \(exitCode) and gave no readable answer."
        }
    }

    public var isMassDelete: Bool {
        if case .failed(_, "mass_delete") = self { return true }
        return false
    }

    public var isUnsupported: Bool {
        if case .failed(_, "unsupported") = self { return true }
        return false
    }
}

public struct NativememCLI: Sendable {
    public static let savedPathKey = "binaryPath"

    public let binary: URL

    public init(binary: URL) {
        self.binary = binary
    }

    /// Search order matters: a GUI app launched from Finder has a minimal PATH, so nothing is looked up there.
    public static func locate(
        savedPath: String?,
        environment: [String: String],
        executableDirectory: URL?,
        home: URL,
        isExecutable: (String) -> Bool
    ) -> URL? {
        var candidates: [String] = []
        if let savedPath, !savedPath.isEmpty { candidates.append(savedPath) }
        if let fromEnv = environment["NATIVEMEM_BIN"], !fromEnv.isEmpty { candidates.append(fromEnv) }
        if let executableDirectory {
            candidates.append(executableDirectory.appendingPathComponent("nativemem-cli").path)
        }
        candidates.append(home.appendingPathComponent(".cargo/bin/nativemem").path)
        candidates.append("/usr/local/bin/nativemem")
        candidates.append("/opt/homebrew/bin/nativemem")
        return candidates.first(where: isExecutable).map { URL(fileURLWithPath: $0) }
    }

    public static func locateOnThisMachine() -> URL? {
        locate(
            savedPath: UserDefaults.standard.string(forKey: savedPathKey),
            environment: ProcessInfo.processInfo.environment,
            executableDirectory: Bundle.main.executableURL?.deletingLastPathComponent(),
            home: FileManager.default.homeDirectoryForCurrentUser,
            isExecutable: { FileManager.default.isExecutableFile(atPath: $0) }
        )
    }

    public func status() async throws -> StatusDocument {
        try await run(["status", "--json"])
    }

    public func items() async throws -> [Item] {
        try await run(["items", "--json"], as: ItemsDocument.self).items
    }

    /// Chat sessions, without their turns.
    public func sessions() async throws -> [Item] {
        try await run(["items", "--kind", "session", "--json"], as: ItemsDocument.self).items
    }

    /// The transcript of one session; native ids often begin with "-", so the value is attached with "=".
    public func sessionShow(agent: String, nativeId: String) async throws -> SessionTranscript {
        try await run(["session", "show", "--agent", agent, "--id=\(nativeId)", "--json"])
    }

    /// Exits with 3 while an agent needs action; the document is complete either way.
    public func doctor() async throws -> DoctorDocument {
        try await run(["doctor", "--json"])
    }

    /// Records the agent's current observed values as reviewed and returns the document after that.
    public func accept(agent: String) async throws -> DoctorDocument {
        try await run(["doctor", "--accept", agent, "--json"])
    }

    public func syncPreview() async throws -> SyncPlan {
        try await run(["sync", "--json"])
    }

    public func syncApply(allowMassDelete: Bool) async throws -> SyncPlan {
        var arguments = ["sync", "--apply", "--json"]
        if allowMassDelete { arguments.append("--allow-mass-delete") }
        return try await run(arguments)
    }

    /// The SKILL.md text of a skill in one agent's own skills directory.
    public func skillShow(agent: String, name: String) async throws -> SkillText {
        try await run(["skill", "show", "--agent", agent, "--name=\(name)", "--json"])
    }

    /// Native ids often begin with "-", so the value is attached with "=" to stay one option argument.
    public func delete(agent: String, nativeId: String) async throws -> DeleteResult {
        try await run(["delete", "--agent", agent, "--id=\(nativeId)", "--json"])
    }

    /// The new content goes to the process's stdin as one JSON document.
    public func edit(agent: String, nativeId: String, document: EditDocument) async throws -> EditResult {
        try await run(
            ["edit", "--agent", agent, "--id=\(nativeId)", "--json"],
            stdin: try Self.encode(document)
        )
    }

    /// Creates an original in the agent's own store; `project` is nil for a global instruction section.
    /// The content goes to stdin like an edit.
    public func add(
        agent: String, kind: String, project: String?, document: EditDocument
    ) async throws -> AddResult {
        var arguments = ["add", "--agent", agent, "--kind", kind, "--json"]
        if let project { arguments.append("--project=\(project)") }
        return try await run(arguments, stdin: try Self.encode(document))
    }

    private static func encode(_ document: EditDocument) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(document)
    }

    func run<T: Decodable & Sendable>(
        _ arguments: [String], stdin: Data? = nil, as type: T.Type = T.self,
        decode: (Data) throws -> T = { try decodeContract(T.self, from: $0) }
    ) async throws -> T {
        let (data, exitCode) = try await execute(arguments, stdin: stdin)
        switch exitCode {
        case 0, 2, 3:
            return try decode(data)
        default:
            if let failure = try? decodeContract(ErrorDocument.self, from: data) {
                throw CLIError.failed(message: failure.error, code: failure.code)
            }
            throw CLIError.unreadable(exitCode: exitCode)
        }
    }

    private func execute(_ arguments: [String], stdin: Data?) async throws -> (Data, Int32) {
        let binary = binary
        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = binary
                process.arguments = arguments
                let pipe = Pipe()
                process.standardOutput = pipe
                process.standardError = FileHandle.nullDevice
                let input = Pipe()
                process.standardInput = stdin == nil ? FileHandle.nullDevice : input
                do {
                    try process.run()
                    if let stdin {
                        try? input.fileHandleForWriting.write(contentsOf: stdin)
                        try? input.fileHandleForWriting.close()
                    }
                    // Drain before waiting: output can exceed the pipe buffer.
                    let data = pipe.fileHandleForReading.readDataToEndOfFile()
                    process.waitUntilExit()
                    continuation.resume(returning: (data, process.terminationStatus))
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }
}
