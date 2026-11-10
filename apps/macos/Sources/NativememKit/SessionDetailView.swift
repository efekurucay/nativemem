import AppKit
import SwiftUI

/// A chat session, read-only: sessions are logs the user manages in their own agent.
struct SessionDetailView: View {
    @Bindable var model: AppModel
    let item: LogicalItem
    @State private var transcript: SessionTranscript?
    @State private var failure: String?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                header
                if let transcript {
                    ForEach(transcript.turns.indices, id: \.self) { index in
                        turn(transcript.turns[index])
                    }
                } else if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle")
                        .font(.callout)
                } else {
                    ProgressView()
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .toolbar {
            if item.storedAsFile {
                ToolbarItem {
                    Button("Show in Finder", systemImage: "folder") {
                        NSWorkspace.shared.activateFileViewerSelecting([
                            URL(fileURLWithPath: item.primary.path)
                        ])
                    }
                    .help("Show in Finder")
                }
            }
        }
        .task(id: item.id) {
            do {
                transcript = try await model.sessionTranscript(of: item)
            } catch {
                failure = error.localizedDescription
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(item.title).font(.title2.bold())
            Text("Lives in").font(.headline)
            HStack(spacing: 8) {
                ForEach(item.livesIn, id: \.self) { residence in
                    Text("\(agentDisplayName(residence.agent)) · \(residence.label)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func turn(_ turn: SessionTurn) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(turn.speaker)
                .font(.subheadline.bold())
                .foregroundStyle(.secondary)
            BodyText(blocks: MarkdownBlocks.parse(turn.text))
        }
    }
}
