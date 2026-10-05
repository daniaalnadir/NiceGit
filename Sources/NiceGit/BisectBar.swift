import NiceGitCore
import SwiftUI

/// Shows a bisect in progress: the commit to test, roughly how many marks remain, and the
/// first bad commit once Git has found it.
struct BisectBar: View {
    let bisect: GitBisectStatus
    let commits: [GitCommit]
    let inspect: (GitCommit) -> Void
    @EnvironmentObject private var model: AppModel

    var body: some View {
        // Side by side when there is room; otherwise the buttons move to a second row.
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) { summary; Spacer(minLength: 8); actions }
            VStack(alignment: .leading, spacing: 8) { summary; HStack { Spacer(minLength: 0); actions } }
        }
        .disabled(model.isLoading)
        .padding(12)
        .background(Color.purple.opacity(0.12))
    }

    private var summary: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "scope").foregroundStyle(.purple)
            VStack(alignment: .leading, spacing: 2) {
                if let firstBad = bisect.firstBad {
                    Text("First bad commit: \(String(firstBad.prefix(7))) \(commits.first { $0.hash == firstBad }?.subject ?? "")")
                        .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                } else {
                    let testing = bisect.testing.flatMap { hash in commits.first { $0.hash == hash } }
                    Text("Test \(bisect.testing.map { String($0.prefix(7)) } ?? "this checkout")\(testing.map { ": \($0.subject)" } ?? "")")
                        .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                    Text(bisect.remainingSteps.map { $0 == 0 ? "One more mark should find the first bad commit" : "About \($0) more \($0 == 1 ? "mark" : "marks") after this one" } ?? "Mark this commit good or bad")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var actions: some View {
        HStack(spacing: 8) {
            if let firstBad = bisect.firstBad {
                if let commit = commits.first(where: { $0.hash == firstBad }) { Button("Show commit") { inspect(commit) } }
            } else {
                Button("Good") { model.markBisect(.good) }.help("This commit does not have the problem")
                Button("Bad") { model.markBisect(.bad) }.help("This commit has the problem")
                Button("Skip") { model.markBisect(.skip) }.help("This commit cannot be tested")
            }
            Button("End bisect") { model.endBisect() }
                .help("Return to \(bisect.originalCheckout.isEmpty ? "the starting checkout" : bisect.originalCheckout)")
        }.fixedSize()
    }
}
