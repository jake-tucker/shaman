// PROTOTYPE — throwaway. Variant A, "Terminal": the TUI's Transcript on a narrow screen.
// Everything the TUI folds is folded the same way and opens in place: a settled Turn behind
// its Turn Fold, a run of work behind its Group, a command's output behind a Peek. The
// Working Indicator is the Transcript's last row. An Intervention's panel takes the composer's
// place, as it does in the TUI, and one button sends or stops.

import SwiftUI

struct VariantA: View {
    @Environment(Store.self) private var store
    let live: Live
    let session: SessionRow
    private var reasoning: Bool { store.reasoning }
    private var diffs: Bool { store.diffs }
    @State private var openTurns: Set<Int> = []
    @State private var openGroups: Set<Int> = []
    @State private var stages: [Int: Int] = [:]  // an Activity's Fold: 0 folded, 1 Peek, 2 open
    @State private var openRecords: Set<Int> = []
    @State private var stopArmed = false
    @FocusState private var typing: Bool
    @AppStorage("unfold") private var unfold = false  // for screenshots: launch with every fold open

    private var unreachable: Bool { store.isUnreachable(session) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    ForEach(live.turns) { turn in turnRows(turn) }
                    if live.working || live.monitoring { workingIndicator }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 14)
                .padding(.top, 8)
            }
            .defaultScrollAnchor(.bottom)
            .defaultScrollAnchor(.top, for: .alignment)  // a short Transcript reads from the top
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: live.pulse) { withAnimation { proxy.scrollTo("end") } }
            .onChange(of: live.turns.count) { openTurns = [] }  // a newer Turn folds back the ones before it
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { foot }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .principal) { SessionTitle(session: session) } }
        .onAppear {
            guard unfold else { return }
            openTurns = Set(live.turns.map(\.id))
            openGroups = Set(live.turns.flatMap { blocks(of: $0.middle, reasoning: true).map(\.id) })
            openRecords = Set(live.turns.flatMap { $0.entries.map(\.id) })
        }
    }

    // MARK: A Turn

    @ViewBuilder private func turnRows(_ turn: Turn) -> some View {
        ForEach(turn.opening) { userBar($0) }
        if turn.outcome == nil {
            ForEach(blocks(of: turn.middle, reasoning: reasoning)) { block($0) }
        } else {
            if !turn.middle.isEmpty {
                let open = openTurns.contains(turn.id)
                marker(turn.outcome == .worked ? .done : turn.outcome == .failed ? .failed : .stopped, turnMarker(turn), open: open) {
                    openTurns.formSymmetricDifference([turn.id])
                }
                if open {
                    ForEach(blocks(of: turn.middle, reasoning: reasoning)) { block($0) }
                } else {
                    ForEach(turn.middle.filter(\.isUser)) { userBar($0) }
                }
            }
            ForEach(turn.closing) { entry in
                if entry.isError { errorRow(entry) } else { Prose(text: headline(entry)) }
            }
        }
    }

    /// The one row a reader taps to move a fold: Marker, words, and which way it stands.
    private func marker(_ state: RunState, _ words: String, open: Bool, dim: Bool = true, tap: @escaping () -> Void) -> some View {
        Button(action: tap) {
            HStack(alignment: open ? .firstTextBaseline : .center, spacing: 6) {
                StateMark(state: state)
                Text(words).multilineTextAlignment(.leading).lineLimit(open ? nil : 1)
                Spacer(minLength: 4)
                Image(systemName: open ? "chevron.down" : "chevron.right").font(.caption2).foregroundStyle(.tertiary)
            }
            .font(.footnote.monospaced())
            .foregroundStyle(dim ? .secondary : .primary)
            .padding(.vertical, 3)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func userBar(_ entry: Entry) -> some View {
        HStack(spacing: 0) {
            Rectangle().fill(.tint).frame(width: 3)
            Text(headline(entry)).padding(.horizontal, 10).padding(.vertical, 8)
            Spacer(minLength: 0)
        }
        .background(Color(.secondarySystemBackground))
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    private func errorRow(_ entry: Entry) -> some View {
        Text("Error: \(headline(entry))").font(.footnote.monospaced()).foregroundStyle(.red)
    }

    // MARK: Groups and Folds

    @ViewBuilder private func block(_ block: Block) -> some View {
        switch block {
        case .single(let entry):
            switch entry.body {
            case .user: userBar(entry)
            case .agent: Prose(text: headline(entry))
            case .error: errorRow(entry)
            case .subagent:
                marker(.done, headline(entry), open: false) {}
            case .approval, .questionnaire: interventionRow(entry)
            default: EmptyView()
            }
        case .work(let entries):
            let open = openGroups.contains(block.id)
            marker(worst(entries), groupMarker(entries), open: open) { openGroups.formSymmetricDifference([block.id]) }
            if open {
                VStack(alignment: .leading, spacing: 6) { ForEach(entries) { fold($0) } }
                    .padding(.leading, 14)
            }
        case .reasoning(let entries):
            let open = openGroups.contains(block.id)
            let words = headline(entries[entries.count - 1]) + (entries.count > 1 ? " · \(entries.count) steps" : "")
            marker(worst(entries), words, open: open) { openGroups.formSymmetricDifference([block.id]) }
            if open {
                ForEach(entries) { entry in
                    if case .reasoning(_, let text) = entry.body {
                        Text(text).font(.footnote).foregroundStyle(.secondary).padding(.leading, 20)
                    }
                }
            }
        }
    }

    /// One Activity in its kind's own Fold. A command opens in stages: folded, a Peek at the
    /// tail of its output, then all of it. A failed one starts at its Peek.
    @ViewBuilder private func fold(_ entry: Entry) -> some View {
        let stage = stages[entry.id] ?? (entry.state == .failed ? 1 : 0)
        switch entry.body {
        case .command(let line, let output, _, _):
            let failed = aside(entry).isEmpty ? "" : " (\(aside(entry)))"
            marker(entry.state, "$ \(line)\(failed)", open: stage > 0, dim: false) {
                stages[entry.id] = output.isEmpty ? (stage == 0 ? 2 : 0) : (stage + 1) % 3
            }
            let shown = entry.state == .running ? 3 : stage == 1 ? 6 : stage == 2 ? output.count : 0
            if shown > 0, !output.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    if output.count > shown {
                        Button("… +\(output.count - shown) lines") { stages[entry.id] = 2 }
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    OutputLines(lines: Array(output.suffix(shown)))
                }
                .padding(.leading, 20)
            }
        case .fileChange(let files):
            marker(entry.state, headline(entry), open: stage > 0, dim: false) { stages[entry.id] = stage == 0 ? 2 : 0 }
            if stage > 0 {
                ForEach(files) { file in
                    VStack(alignment: .leading, spacing: 3) {
                        if files.count > 1 { Text(file.path).font(.caption2.monospaced()).foregroundStyle(.secondary) }
                        if diffs { DiffLines(diff: file.diff).clipShape(RoundedRectangle(cornerRadius: 4)) }
                    }
                    .padding(.leading, 20)
                }
            }
        case .tool(_, _, let result):
            marker(entry.state, headline(entry), open: stage > 0, dim: false) { stages[entry.id] = stage == 0 ? 2 : 0 }
            if stage > 0 { OutputLines(lines: [result]).padding(.leading, 20) }
        default: EmptyView()
        }
    }

    /// An Approval or Questionnaire where it stands in the Transcript: a record once settled,
    /// and the way back to the panel while it waits.
    @ViewBuilder private func interventionRow(_ entry: Entry) -> some View {
        if entry.awaits {
            Button { live.setAside = false } label: {
                HStack(spacing: 6) {
                    Image(systemName: glyph(entry)).foregroundStyle(.orange)
                    Text((entry.isApprovalRow ? "Approval" : "Questionnaire") + " · waiting on you")
                    Spacer()
                    if live.setAside { Text("Answer").foregroundStyle(.tint) }
                }
                .font(.footnote.monospaced())
            }
            .buttonStyle(.plain)
        } else {
            let open = openRecords.contains(entry.id)
            marker(.done, settledLine(entry), open: open) { openRecords.formSymmetricDifference([entry.id]) }
            if open {
                Group {
                    if case .approval(let approval) = entry.body { ApprovalFacts(approval: approval) }
                    if case .questionnaire(let questionnaire) = entry.body { QuestionnaireRecord(questionnaire: questionnaire) }
                }
                .padding(.leading, 20)
            }
        }
    }

    /// "Working · Fable 5.2 (1m 05s)". The TUI's "Esc to interrupt" is the stop button here.
    private var workingIndicator: some View {
        HStack(spacing: 6) {
            ProgressView().controlSize(.mini)
            Shimmer(text: live.monitoring ? "Monitoring" : "Working")
            if let watch = live.watch, live.monitoring { Text(watch) }
            if live.working { Text("· Fable 5.2") }
            HStack(spacing: 0) {
                Text("(")
                Elapsed(since: live.began)
                Text(")")
            }
            Spacer()
        }
        .font(.footnote.monospaced())
        .foregroundStyle(.secondary)
        .padding(.vertical, 4)
    }

    // MARK: The foot: an Intervention's panel, or the composer

    @ViewBuilder private var foot: some View {
        VStack(spacing: 0) {
            Divider()
            if let entry = live.intervention {
                Group {
                    if case .approval(let approval) = entry.body { approvalPanel(entry, approval) }
                    if case .questionnaire(let questionnaire) = entry.body {
                        QuestionnairePanelA(live: live, entry: entry, questionnaire: questionnaire)
                    }
                }
                .id(entry.id)
                .padding(14)
            } else {
                composer
            }
        }
        .background(.bar)
    }

    private func panelTitle(_ words: String) -> some View {
        HStack {
            Text(words).font(.footnote.weight(.semibold)).foregroundStyle(.orange)
            Spacer()
            Button("Hide") { live.setAside = true }.font(.footnote)
        }
    }

    private func approvalPanel(_ entry: Entry, _ approval: Approval) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            panelTitle("Approval · Choose decision")
            ScrollView { ApprovalFacts(approval: approval) }.frame(maxHeight: 190).fixedSize(horizontal: false, vertical: true)
            VStack(spacing: 0) {
                ForEach(Array(Decision.allCases.enumerated()), id: \.element) { index, decision in
                    Button { live.decide(entry.id, decision) } label: {
                        HStack {
                            Text("\(index + 1).").foregroundStyle(.secondary).monospacedDigit()
                            Text(decision.label)
                            Spacer()
                        }
                        .padding(.vertical, 11)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    if index < 3 { Divider() }
                }
            }
            .armedAfterAMoment()
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if unreachable {
                Label("\(store.origin(session).id) is unreachable. Nothing sent goes anywhere until it answers.", systemImage: "wifi.slash")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if !live.pending.isEmpty {
                Button { live.setAside = false } label: {
                    Label("\(live.pending.count) waiting on you · Answer", systemImage: "hand.raised")
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(.orange)
                }
            }
            if !live.queued.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Pending").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(Array(live.queued.enumerated()), id: \.offset) { index, text in
                        HStack {
                            Text(text).font(.footnote).lineLimit(1)
                            Spacer()
                            Button("Steer now") { live.promote(index) }.font(.caption)
                            Button { live.queued.remove(at: index) } label: { Image(systemName: "xmark") }.font(.caption)
                        }
                    }
                }
                .padding(8)
                .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 8))
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField(live.working ? "Steer this turn" : "Prompt", text: Bindable(live).draft, axis: .vertical)
                    .lineLimit(1...6)
                    .focused($typing)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
                sendOrStop
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .disabled(unreachable)
    }

    /// One button: it sends when there are words, and stops the Turn when there are none.
    /// Stopping takes two taps, as the TUI takes two Esc.
    @ViewBuilder private var sendOrStop: some View {
        if live.draft.isEmpty && (live.working || live.monitoring) {
            Button {
                if stopArmed {
                    live.interrupt()
                    stopArmed = false
                } else {
                    stopArmed = true
                    Task {
                        try? await Task.sleep(for: .seconds(3))
                        stopArmed = false
                    }
                }
            } label: {
                if stopArmed {
                    Text("Tap again to interrupt").font(.caption.weight(.semibold)).padding(.horizontal, 10).frame(height: 34)
                        .background(.red, in: Capsule()).foregroundStyle(.white)
                } else {
                    Image(systemName: "stop.fill").frame(width: 34, height: 34)
                        .background(Color(.tertiarySystemFill), in: Circle())
                }
            }
            .buttonStyle(.plain)
        } else {
            Image(systemName: "arrow.up")
                .font(.body.weight(.semibold))
                .frame(width: 34, height: 34)
                .background(live.draft.isEmpty ? Color(.tertiarySystemFill) : Color.accentColor, in: Circle())
                .foregroundStyle(live.draft.isEmpty ? Color.secondary : Color.white)
                .onTapGesture { live.send() }
                .contextMenu {
                    if live.working {
                        Button("Steer now", systemImage: "arrow.up") { live.send() }
                        Button("Queue for after this turn", systemImage: "text.line.last.and.arrowtriangle.forward") { live.send(queue: true) }
                    }
                }
        }
    }
}

extension Entry {
    var isApprovalRow: Bool { if case .approval = body { true } else { false } }
}

/// The TUI's Questionnaire panel: one Question to a page, then a review of every answer.
private struct QuestionnairePanelA: View {
    let live: Live
    let entry: Entry
    let questionnaire: Questionnaire
    @State private var answers = Answers()
    @State private var page = 0

    private var reviewing: Bool { page == questionnaire.questions.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(reviewing ? "Questionnaire · Review answer" : "Questionnaire · Question \(page + 1) of \(questionnaire.questions.count)")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.orange)
                Spacer()
                Button("Hide") { live.setAside = true }.font(.footnote)
            }
            if reviewing {
                ForEach(questionnaire.questions) { question in
                    VStack(alignment: .leading, spacing: 1) {
                        Text(question.text).font(.caption).foregroundStyle(.secondary)
                        let given = answers.answer(question)
                        Text(given.isEmpty ? "Not answered" : given.joined(separator: ", ")).font(.callout)
                    }
                }
            } else {
                let question = questionnaire.questions[page]
                Text(question.text).font(.callout.weight(.medium))
                VStack(spacing: 0) {
                    ForEach(question.choices) { choice in
                        Button { answers.toggle(question, choice.label) } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text(answers.has(question, choice.label) ? "[x]" : "[ ]").font(.callout.monospaced())
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(choice.label + (choice.recommended ? " (recommended)" : ""))
                                    Text(choice.detail).font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                            .padding(.vertical, 7)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                if question.freeform {
                    TextField("Text", text: answers.text(question))
                        .textFieldStyle(.roundedBorder)
                        .font(.callout)
                }
            }
            HStack {
                Button("Decline", role: .destructive) { live.answer(entry.id, nil) }
                Spacer()
                if page > 0 { Button("Back") { page -= 1 } }
                if reviewing {
                    Button("Submit") { live.answer(entry.id, answers.result(questionnaire.questions)) }
                        .buttonStyle(.borderedProminent)
                } else {
                    Button(page == questionnaire.questions.count - 1 ? "Review" : "Next") { page += 1 }
                        .buttonStyle(.bordered)
                }
            }
            .font(.callout)
            .armedAfterAMoment()
        }
    }
}
