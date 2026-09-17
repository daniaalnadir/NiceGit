import AppKit
import NiceGitCore
import SwiftUI

struct FileReviewView: View {
    let selection: DiffSelection
    @EnvironmentObject private var model: AppModel
    @State private var review: GitFileReview?
    @State private var document: GitEditableFile?
    @State private var text = ""
    @State private var editing = false
    @State private var selected: Set<Int> = []
    @State private var busy = false
    @State private var error: String?
    @State private var editError: String?
    @State private var query = ""
    @State private var lineEdits: [Int: String] = [:]
    @State private var sourceLines: [String] = []
    @State private var control = GitCommandControl()

    private var dirty: Bool { document.map { text != $0.text } ?? false }

    private var canEditDiff: Bool {
        guard let document, let review else { return false }
        var indexed = ""
        for (index, line) in review.lines.enumerated() where line.kind == .context || line.kind == .addition {
            indexed += String(line.text.dropFirst())
            if !review.lines.indices.contains(index + 1) || review.lines[index + 1].text != "\\ No newline at end of file" {
                indexed += "\n"
            }
        }
        return indexed == document.text
    }

    private func lineBinding(_ number: Int) -> Binding<String> {
        Binding {
            lineEdits[number] ?? originalLines()[number - 1]
        } set: { value in
            lineEdits[number] = value
            var lines = originalLines()
            for (number, replacement) in lineEdits where lines.indices.contains(number - 1) {
                lines[number - 1] = replacement
            }
            text = lines.joined(separator: "\n")
        }
    }

    private func originalLines() -> [String] {
        sourceLines
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "doc.text")
                Text(selection.path ?? selection.title).lineLimit(1).truncationMode(.middle)
                if dirty { Circle().fill(.orange).frame(width: 7, height: 7).accessibilityLabel("Unsaved edits") }
                Spacer(minLength: 0)
                Button { model.closeFileReview() } label: { Image(systemName: "xmark") }
                    .help("Close file and return to graph")
            }.font(.system(size: 12)).padding(10).background(AppPalette.toolbar)
            Divider()
            HStack(spacing: 8) {
                Picker("View", selection: $editing) {
                    Text("Diff").tag(false)
                    Text("Edit").tag(true)
                }.pickerStyle(.segmented).frame(width: 120).disabled(dirty)
                Text(editing ? "Working file" : selection.staged ? "Staged" : "Unstaged")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer(minLength: 0)
                if editing || dirty {
                    Button { save() } label: { Label("Save", systemImage: "square.and.arrow.down") }
                        .disabled(!dirty || document == nil).keyboardShortcut("s", modifiers: .command)
                }
                if !editing {
                    Button { changeIndex() } label: {
                        Label(selection.staged ? "Unstage selected" : "Stage selected", systemImage: selection.staged ? "minus.circle" : "plus.circle")
                    }.disabled(selected.isEmpty || review?.lineStagingUnavailable != nil || dirty)
                }
                Button {
                    guard model.confirmDiscardFileEdits() else { return }
                    Task { await load() }
                } label: { Image(systemName: "arrow.clockwise") }.help("Reload file and diff")
            }.controlSize(.small).padding(8)
            Divider()
            if let error {
                HStack {
                    Image(systemName: "exclamationmark.triangle").foregroundStyle(.orange)
                    Text(error).font(.caption).textSelection(.enabled)
                    Spacer()
                    Button { self.error = nil } label: { Image(systemName: "xmark") }.help("Dismiss error")
                }.padding(8)
                Divider()
            }
            if editing {
                if document != nil {
                    WorkingCodeEditor(text: $text)
                } else {
                    ContentUnavailableView("Cannot edit this file", systemImage: "doc", description: Text(editError ?? "Loading file..."))
                }
            } else {
                diffBody
            }
        }
        .disabled(busy)
        .overlay { if busy { ProgressView().padding().background(.regularMaterial) } }
        .onChange(of: text) { model.fileReviewHasEdits = dirty }
        .task { await load() }
        .onDisappear { control.cancel() }
    }

    private var diffBody: some View {
        let editable = canEditDiff
        return VStack(spacing: 0) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Find in diff", text: $query).textFieldStyle(.plain)
                if !selected.isEmpty { Text("\(selected.count) selected").font(.caption).monospacedDigit() }
            }.padding(8)
            if let reason = review?.lineStagingUnavailable {
                Text(reason).font(.caption).foregroundStyle(.secondary).padding(8)
            }
            if selection.staged && document != nil && !editable {
                Text("Staged snapshot. Open the unstaged file to edit its newer working changes.")
                    .font(.caption).foregroundStyle(.secondary).padding(8)
            }
            Divider()
            GeometryReader { geometry in
                ScrollView([.horizontal, .vertical]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if let review {
                            ForEach(GitDiffHunk.grouped(review.lines)) { hunk in
                                HStack {
                                    let first = review.lines[hunk.lineIndices.lowerBound]
                                    Text("@@ Line \(first.newNumber ?? first.oldNumber ?? 1) @@")
                                        .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                                    Spacer()
                                    Button {
                                        changeIndex(indexes: hunk.changedIndices)
                                    } label: {
                                        Label(selection.staged ? "Unstage Hunk" : "Stage Hunk", systemImage: selection.staged ? "minus.circle" : "plus.circle")
                                    }.controlSize(.small)
                                        .disabled(review.lineStagingUnavailable != nil || dirty)
                                }.padding(.horizontal, 10).frame(height: 32)
                                    .frame(width: geometry.size.width)
                                    .background(AppPalette.toolbar)
                            ForEach(hunk.lineIndices, id: \.self) { index in
                                let line = review.lines[index]
                                HStack(spacing: 0) {
                                    if line.kind == .addition || line.kind == .deletion {
                                        Button {
                                            if !selected.insert(index).inserted { selected.remove(index) }
                                        } label: {
                                            Image(systemName: selected.contains(index) ? "checkmark.square.fill" : "square")
                                                .frame(width: 28, height: 22)
                                        }.buttonStyle(.plain)
                                            .disabled(review.lineStagingUnavailable != nil || dirty)
                                            .help("Select line for \(selection.staged ? "unstaging" : "staging")")
                                            .accessibilityLabel("Select change at diff line \(index + 1)")
                                    } else { Color.clear.frame(width: 28, height: 22) }
                                    Text(line.oldNumber.map(String.init) ?? "").foregroundStyle(.secondary).frame(width: 42, alignment: .trailing)
                                    Text(line.newNumber.map(String.init) ?? "").foregroundStyle(.secondary).frame(width: 42, alignment: .trailing)
                                    if editable, let number = line.newNumber,
                                       originalLines().indices.contains(number - 1),
                                       line.kind == .context || line.kind == .addition {
                                        Text(line.kind == .addition ? "+" : " ").padding(.leading, 12)
                                        TextField("", text: lineBinding(number), axis: .vertical)
                                            .textFieldStyle(.plain)
                                            .frame(minWidth: max(240, geometry.size.width - 160), alignment: .leading)
                                            .padding(.trailing, 12)
                                            .accessibilityLabel("Edit working line \(number)")
                                    } else {
                                        Text(line.text.isEmpty ? " " : line.text).padding(.horizontal, 12).textSelection(.enabled)
                                    }
                                }
                                .font(.system(size: 12, design: .monospaced))
                                .fixedSize(horizontal: true, vertical: false)
                                .frame(minWidth: geometry.size.width, alignment: .leading)
                                .background(line.newNumber.flatMap { lineEdits[$0] } != nil ? Color.green.opacity(0.18) : color(line))
                                .overlay(alignment: .leading) {
                                    if !query.isEmpty && line.text.localizedCaseInsensitiveContains(query) {
                                        Rectangle().fill(.yellow).frame(width: 3)
                                    }
                                }
                            }
                            }
                            if GitDiffHunk.grouped(review.lines).isEmpty {
                                Text(review.patch.isEmpty ? "No differences" : "No text changes. Stage or unstage this file from the file list.")
                                    .foregroundStyle(.secondary).padding()
                            }
                        }
                    }.frame(minWidth: geometry.size.width, minHeight: geometry.size.height, alignment: .topLeading)
                }
            }
        }
    }

    private func color(_ line: GitDiffLine) -> Color {
        switch line.kind {
        case .addition: .green.opacity(0.18)
        case .deletion: .red.opacity(0.18)
        case .hunk: .blue.opacity(0.12)
        default: .clear
        }
    }

    @MainActor private func load() async {
        busy = true
        error = nil
        selected = []
        let path = selection.path ?? ""
        let staged = selection.staged
        let url = selection.repositoryURL
        let commandControl = control
        defer { busy = false }
        do {
            let result = try await Task.detached {
                try GitClient(control: commandControl).fileReview(path: path, staged: staged, in: url)
            }.value
            guard !Task.isCancelled, model.fileReviewSelection?.id == selection.id else { return }
            review = result
            do {
                let loaded = try await Task.detached {
                    try GitClient(control: commandControl).editableFile(path: path, in: url)
                }.value
                guard model.fileReviewSelection?.id == selection.id else { return }
                document = loaded
                sourceLines = loaded.text.components(separatedBy: "\n")
                lineEdits = [:]
                text = loaded.text
                editError = nil
            } catch {
                guard model.fileReviewSelection?.id == selection.id else { return }
                document = nil
                editError = error.localizedDescription
            }
            model.fileReviewHasEdits = false
        } catch { self.error = error.localizedDescription }
    }

    private func changeIndex(indexes: Set<Int>? = nil) {
        guard let review else { return }
        let indexes = indexes ?? selected
        let url = selection.repositoryURL
        let commandControl = control
        busy = true
        model.isLoading = true
        Task {
            do {
                try await Task.detached { try GitClient(control: commandControl).stageLines(indexes, from: review, in: url) }.value
                await load()
                model.isLoading = false
                model.refreshWorkingTree()
            } catch { self.error = error.localizedDescription; busy = false; model.isLoading = false }
        }
    }

    private func save() {
        guard let document else { return }
        let content = text
        let url = selection.repositoryURL
        busy = true
        model.isLoading = true
        Task {
            do {
                try await Task.detached { try GitClient().saveFile(document, text: content, in: url) }.value
                model.fileReviewHasEdits = false
                await load()
                model.isLoading = false
                model.refreshWorkingTree()
            } catch { self.error = error.localizedDescription; busy = false; model.isLoading = false }
        }
    }
}

private struct WorkingCodeEditor: NSViewRepresentable {
    @Binding var text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        let editor = NSTextView()
        editor.isRichText = false
        editor.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        editor.allowsUndo = true
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticTextReplacementEnabled = false
        editor.isAutomaticSpellingCorrectionEnabled = false
        editor.isHorizontallyResizable = true
        editor.isVerticallyResizable = true
        editor.textContainer?.widthTracksTextView = false
        editor.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        editor.textContainerInset = NSSize(width: 12, height: 12)
        editor.minSize = NSSize(width: 0, height: 0)
        editor.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        editor.autoresizingMask = [.width]
        editor.delegate = context.coordinator
        editor.string = text
        scroll.documentView = editor
        scroll.verticalRulerView = CodeLineRuler(scrollView: scroll, orientation: .verticalRuler)
        scroll.hasVerticalRuler = true
        scroll.rulersVisible = true
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let editor = scroll.documentView as? NSTextView, editor.string != text else { return }
        editor.string = text
        editor.undoManager?.removeAllActions()
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: WorkingCodeEditor
        init(_ parent: WorkingCodeEditor) { self.parent = parent }
        func textDidChange(_ notification: Notification) {
            guard let editor = notification.object as? NSTextView else { return }
            parent.text = editor.string
            editor.enclosingScrollView?.verticalRulerView?.needsDisplay = true
        }
    }
}

private final class CodeLineRuler: NSRulerView {
    override init(scrollView: NSScrollView?, orientation: NSRulerView.Orientation) {
        super.init(scrollView: scrollView, orientation: orientation)
        ruleThickness = 52
    }
    required init(coder: NSCoder) { super.init(coder: coder) }

    override func drawHashMarksAndLabels(in rect: NSRect) {
        guard let editor = scrollView?.documentView as? NSTextView,
              let layout = editor.layoutManager, let container = editor.textContainer else { return }
        let content = editor.string as NSString
        let attributes: [NSAttributedString.Key: Any] = [.font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular), .foregroundColor: NSColor.secondaryLabelColor]
        var location = 0
        var number = 1
        while location < content.length {
            let range = content.lineRange(for: NSRange(location: location, length: 0))
            let glyph = layout.glyphRange(forCharacterRange: NSRange(location: location, length: 1), actualCharacterRange: nil)
            let bounds = layout.boundingRect(forGlyphRange: glyph, in: container)
            let y = convert(NSPoint(x: 0, y: bounds.minY + editor.textContainerOrigin.y), from: editor).y
            if y > self.bounds.maxY { break }
            if y + bounds.height >= self.bounds.minY {
                let label = String(number) as NSString
                label.draw(at: NSPoint(x: ruleThickness - label.size(withAttributes: attributes).width - 10, y: y), withAttributes: attributes)
            }
            location = NSMaxRange(range)
            number += 1
        }
        if content.length == 0 {
            ("1" as NSString).draw(at: NSPoint(x: 32, y: editor.textContainerInset.height), withAttributes: attributes)
        }
    }
}
