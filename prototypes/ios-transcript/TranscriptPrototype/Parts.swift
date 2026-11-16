// PROTOTYPE — throwaway. Leaf views every variant may draw with: prose, output, a diff, the
// facts of an Intervention. How a screen is laid out stays each variant's own.

import SwiftUI

// MARK: Agent prose

private enum Piece {
    case para(String), bullets([String]), code(String), table([[String]])
}

private func pieces(_ text: String) -> [Piece] {
    var out: [Piece] = []
    var code: [String]?
    for line in text.components(separatedBy: "\n") {
        if line.hasPrefix("```") {
            if let lines = code { out.append(.code(lines.joined(separator: "\n"))); code = nil } else { code = [] }
        } else if code != nil {
            code?.append(line)
        } else if line.hasPrefix("- ") {
            let item = String(line.dropFirst(2))
            if case .bullets(let items) = out.last { out[out.count - 1] = .bullets(items + [item]) } else { out.append(.bullets([item])) }
        } else if line.hasPrefix("|") {
            let cells = line.split(separator: "|").map { $0.trimmingCharacters(in: .whitespaces) }
            if cells.allSatisfy({ $0.allSatisfy { $0 == "-" } }) { continue }
            if case .table(let rows) = out.last { out[out.count - 1] = .table(rows + [cells]) } else { out.append(.table([cells])) }
        } else if !line.isEmpty {
            out.append(.para(line))
        }
    }
    return out
}

func inline(_ text: String) -> AttributedString {
    (try? AttributedString(markdown: text)) ?? AttributedString(text)
}

/// An agent Message's Markdown: paragraphs, lists, code blocks and tables. Code and tables
/// scroll sideways rather than wrap.
struct Prose: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            ForEach(Array(pieces(text).enumerated()), id: \.offset) { _, piece in
                switch piece {
                case .para(let text):
                    Text(inline(text))
                case .bullets(let items):
                    VStack(alignment: .leading, spacing: 5) {
                        ForEach(items, id: \.self) { item in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("•").foregroundStyle(.secondary)
                                Text(inline(item))
                            }
                        }
                    }
                case .code(let code):
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(code).font(.footnote.monospaced()).padding(10)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 8))
                case .table(let rows):
                    ScrollView(.horizontal, showsIndicators: false) {
                        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                            ForEach(Array(rows.enumerated()), id: \.offset) { index, cells in
                                GridRow {
                                    ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                                        Text(inline(cell)).fontWeight(index == 0 ? .semibold : .regular)
                                    }
                                }
                                if index == 0 { Divider() }
                            }
                        }
                        .font(.subheadline)
                        .padding(.vertical, 2)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

// MARK: Output and diffs

struct OutputLines: View {
    let lines: [String]
    var wrap = true

    var body: some View {
        let text = Text(lines.joined(separator: "\n")).font(.caption2.monospaced()).foregroundStyle(.secondary)
        if wrap {
            text.frame(maxWidth: .infinity, alignment: .leading)
        } else {
            ScrollView(.horizontal, showsIndicators: false) { text.fixedSize() }
        }
    }
}

struct DiffLines: View {
    let diff: [DiffLine]
    var wrap = true

    var body: some View {
        let lines = VStack(alignment: .leading, spacing: 0) {
            ForEach(diff) { line in
                Text(line.mark == "@" ? "@\(line.text)" : "\(String(line.mark)) \(line.text)")
                    .font(.caption2.monospaced())
                    .foregroundStyle(line.mark == "@" ? .secondary : .primary)
                    .fixedSize(horizontal: !wrap, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(line.mark == "+" ? Color.green.opacity(0.16) : line.mark == "-" ? Color.red.opacity(0.16) : .clear)
            }
        }
        if wrap { lines } else { ScrollView(.horizontal, showsIndicators: false) { lines } }
    }
}

/// The TUI's Marker: a tick, a cross (red failed, yellow stopped) or a spinner.
struct StateMark: View {
    let state: RunState

    var body: some View {
        switch state {
        case .running: ProgressView().controlSize(.mini).frame(width: 14)
        case .done: Text("✓").foregroundStyle(.green).frame(width: 14)
        case .failed: Text("×").foregroundStyle(.red).frame(width: 14)
        case .stopped: Text("×").foregroundStyle(.yellow).frame(width: 14)
        }
    }
}

func clock(_ seconds: Int) -> String {
    let s = max(seconds, 0)
    if s < 60 { return "\(s)s" }
    if s < 3600 { return "\(s / 60)m \(String(format: "%02d", s % 60))s" }
    return "\(s / 3600)h \(String(format: "%02d", s % 3600 / 60))m"
}

/// Counts up from when the work began, a second at a time.
struct Elapsed: View {
    let since: Date?

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(clock(Int(context.date.timeIntervalSince(since ?? context.date)))).monospacedDigit()
        }
    }
}

/// The word that says a Session is live, breathing.
struct Shimmer: View {
    let text: String

    var body: some View {
        Text(text).phaseAnimator([1.0, 0.45]) { content, phase in
            content.opacity(phase)
        } animation: { _ in .easeInOut(duration: 0.9) }
    }
}

// MARK: The Session's name in the navigation bar

struct SessionTitle: View {
    @Environment(Store.self) private var store
    let session: SessionRow

    var body: some View {
        VStack(spacing: 1) {
            Text(session.title).font(.subheadline.weight(.semibold)).lineLimit(1)
            Text([store.place(session.workspace), session.checkout].compactMap { $0 }.joined(separator: " · "))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

// MARK: What an Intervention asks, and what was answered

/// The facts of an Approval, in the TUI panel's own lines.
struct ApprovalFacts: View {
    @Environment(Store.self) private var store
    let approval: Approval
    var wrap = true
    private var diffs: Bool { store.diffs }

    private func fact(_ name: String, _ value: String, mono: Bool = true) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(name).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(mono ? .callout.monospaced() : .callout).textSelection(.enabled)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            switch approval.subject {
            case .command(let line, let cwd):
                fact("Command", line)
                fact("Directory", cwd)
            case .fileChange(let files):
                ForEach(files) { file in
                    VStack(alignment: .leading, spacing: 4) {
                        fact("File change", file.path)
                        if diffs {
                            DiffLines(diff: file.diff, wrap: wrap).clipShape(RoundedRectangle(cornerRadius: 6))
                        }
                    }
                }
            }
            if let reason = approval.reason { fact("Reason", reason, mono: false) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A Questionnaire as the Transcript keeps it: each Question, then what was answered.
struct QuestionnaireRecord: View {
    let questionnaire: Questionnaire

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(questionnaire.questions) { question in
                VStack(alignment: .leading, spacing: 2) {
                    Text(question.text).font(.footnote).foregroundStyle(.secondary)
                    Text(questionnaire.answers?[question.id]?.joined(separator: ", ") ?? "Not answered")
                        .font(.callout)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// What the reader has chosen so far in a Questionnaire. Each variant lays the asking out its
/// own way and keeps its choices here.
@Observable
final class Answers {
    var picked: [String: [String]] = [:]
    var written: [String: String] = [:]

    func has(_ question: Question, _ choice: String) -> Bool { picked[question.id]?.contains(choice) ?? false }

    func toggle(_ question: Question, _ choice: String) {
        var now = picked[question.id] ?? []
        if let index = now.firstIndex(of: choice) {
            now.remove(at: index)
        } else if question.multiple {
            now.append(choice)
        } else {
            now = [choice]
        }
        picked[question.id] = now
    }

    func text(_ question: Question) -> Binding<String> {
        Binding(get: { self.written[question.id] ?? "" }, set: { self.written[question.id] = $0 })
    }

    func answer(_ question: Question) -> [String] {
        let text = (written[question.id] ?? "").trimmingCharacters(in: .whitespaces)
        return (picked[question.id] ?? []) + (text.isEmpty ? [] : [text])
    }

    func result(_ questions: [Question]) -> [String: [String]] {
        Dictionary(uniqueKeysWithValues: questions.map { ($0.id, answer($0)) }.filter { !$0.1.isEmpty })
    }
}

/// The whole of one Activity, for variants that open it somewhere of its own.
struct ActivityDetail: View {
    @Environment(Store.self) private var store
    let entry: Entry
    var wrap = false
    private var diffs: Bool { store.diffs }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            switch entry.body {
            case .command(let line, let output, _, _):
                Text(line).font(.callout.monospaced()).textSelection(.enabled)
                if !aside(entry).isEmpty {
                    Text(aside(entry)).font(.caption).foregroundStyle(entry.state == .failed ? .red : .yellow)
                }
                if output.isEmpty {
                    Text("No output yet.").font(.footnote).foregroundStyle(.tertiary)
                } else {
                    OutputLines(lines: output, wrap: wrap)
                }
            case .fileChange(let files):
                ForEach(files) { file in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(file.path).font(.callout.monospaced())
                            Spacer()
                            if diffs {
                                Text("+\(file.added) −\(file.removed)").font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                        }
                        if diffs { DiffLines(diff: file.diff, wrap: wrap).clipShape(RoundedRectangle(cornerRadius: 6)) }
                    }
                }
                if !diffs {
                    Text("A file change carries the paths it touched and nothing of what changed in them.")
                        .font(.footnote)
                        .foregroundStyle(.tertiary)
                }
            case .tool(let name, let subject, let result):
                Text("\(name) \(subject)").font(.callout.monospaced()).textSelection(.enabled)
                OutputLines(lines: [result], wrap: true)
            case .reasoning(_, let text):
                Text(text).font(.callout).foregroundStyle(.secondary)
            case .approval(let approval):
                ApprovalFacts(approval: approval, wrap: wrap)
                Text(settledLine(entry)).font(.footnote).foregroundStyle(.secondary)
            case .questionnaire(let questionnaire):
                QuestionnaireRecord(questionnaire: questionnaire)
                Text(settledLine(entry)).font(.footnote).foregroundStyle(.secondary)
            case .subagent(_, let task):
                Text(task)
                Text("A subagent row opens the subagent's own session.").font(.footnote).foregroundStyle(.tertiary)
            case .user(let text), .agent(let text, _), .error(let text):
                Prose(text: text)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// An Intervention takes no tap for a moment after appearing, so a reader mid-tap cannot
/// decide by accident.
struct Armed: ViewModifier {
    @State private var armed = false

    func body(content: Content) -> some View {
        content
            .disabled(!armed)
            .task {
                try? await Task.sleep(for: .seconds(0.6))
                armed = true
            }
    }
}

extension View {
    func armedAfterAMoment() -> some View { modifier(Armed()) }
}
