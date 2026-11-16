// PROTOTYPE — throwaway. Variant B, "Chat": a messaging app. What was said reads as a
// conversation, and a Turn's work is one card between the ask and the answer. Nothing opens
// in place: the card pushes a screen listing the work, and each Activity pushes a screen of
// its own with room to scroll sideways. Live work rides a bar above the composer, where the
// stop button is. An Intervention presents itself as a sheet.

import SwiftUI

private struct WorkRoute: Hashable { let turn: Int }
private struct EntryRoute: Hashable { let turn: Int; let entry: Int }

struct VariantB: View {
    @Environment(Store.self) private var store
    let live: Live
    let session: SessionRow
    @AppStorage("drill") private var drill = false  // for screenshots: launch on the first Turn's work

    private var unreachable: Bool { store.isUnreachable(session) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    ForEach(live.turns) { turn in turnRows(turn) }
                    ForEach(Array(live.queued.enumerated()), id: \.offset) { index, text in queuedBubble(index, text) }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 14)
                .padding(.top, 8)
            }
            .defaultScrollAnchor(.bottom)
            .defaultScrollAnchor(.top, for: .alignment)  // a short Transcript reads from the top
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: live.pulse) { withAnimation { proxy.scrollTo("end") } }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { foot }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .principal) { SessionTitle(session: session) } }
        .onAppear { if drill, store.path.count == 1 { store.path.append(WorkRoute(turn: live.turns[0].id)) } }
        .navigationDestination(for: WorkRoute.self) { WorkScreen(live: live, turn: $0.turn) }
        .navigationDestination(for: EntryRoute.self) { EntryScreen(live: live, route: $0) }
        .sheet(isPresented: Binding(
            get: { live.intervention != nil },
            set: { shown in if !shown, !live.pending.isEmpty { live.setAside = true } }
        )) {
            if let entry = live.intervention { InterventionSheet(live: live, entry: entry) }
        }
    }

    // MARK: A Turn

    @ViewBuilder private func turnRows(_ turn: Turn) -> some View {
        ForEach(turn.opening) { bubble(headline($0)) }
        let closing = turn.closing
        let work = turn.middle.filter { !$0.isUser }
        if !work.isEmpty || turn.outcome == nil {
            NavigationLink(value: WorkRoute(turn: turn.id)) { workCard(turn, work) }.buttonStyle(.plain)
        }
        ForEach(turn.middle.filter(\.isUser)) { bubble(headline($0)) }
        ForEach(closing) { entry in
            if entry.isError {
                Label(headline(entry), systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .padding(10)
                    .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
            } else {
                Prose(text: headline(entry)).padding(.trailing, 24)
            }
        }
    }

    private func bubble(_ text: String) -> some View {
        Text(text)
            .padding(.horizontal, 13)
            .padding(.vertical, 9)
            .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 18))
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .padding(.leading, 44)
    }

    /// A queued Prompt waits at the foot of the conversation, where it will be said.
    private func queuedBubble(_ index: Int, _ text: String) -> some View {
        VStack(alignment: .trailing, spacing: 3) {
            Text(text)
                .padding(.horizontal, 13)
                .padding(.vertical, 9)
                .overlay(RoundedRectangle(cornerRadius: 18).strokeBorder(.tint, style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
                .foregroundStyle(.secondary)
            HStack(spacing: 12) {
                Text("Queued").foregroundStyle(.secondary)
                Button("Steer now") { live.promote(index) }
                Button("Cancel") { live.queued.remove(at: index) }
            }
            .font(.caption)
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .padding(.leading, 44)
    }

    /// The whole of a Turn's work as one card: how it settled, and the Group's count of it.
    private func workCard(_ turn: Turn, _ work: [Entry]) -> some View {
        HStack(spacing: 10) {
            Group {
                switch turn.outcome {
                case nil: ProgressView().controlSize(.small)
                case .worked: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                case .stopped: Image(systemName: "stop.circle.fill").foregroundStyle(.yellow)
                case .failed: Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                }
            }
            .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                if turn.outcome == nil {
                    Text(live.pending.isEmpty ? (live.running.map(headline) ?? "Working") : "Waiting on you")
                        .font(.subheadline.weight(.medium))
                        .lineLimit(1)
                } else {
                    Text(turnMarker(turn)).font(.subheadline.weight(.medium))
                }
                let counts = tally(work)
                if !counts.isEmpty { Text(counts).font(.caption).foregroundStyle(.secondary) }
            }
            Spacer()
            Image(systemName: "chevron.right").font(.caption.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
        .padding(.trailing, 24)
    }

    // MARK: The foot: the live bar and the composer

    private var foot: some View {
        VStack(spacing: 8) {
            if unreachable {
                Label("\(store.origin(session).id) is unreachable. Nothing sent goes anywhere until it answers.", systemImage: "wifi.slash")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if !live.pending.isEmpty {
                liveBar(color: .orange) {
                    Image(systemName: "hand.raised.fill")
                    Text(live.pending[0].isApprovalRow ? "Approval waiting" : "Questionnaire waiting").fontWeight(.medium)
                } trailing: {
                    Button("Review") { live.setAside = false }.buttonStyle(.borderedProminent).tint(.orange)
                }
            } else if live.working || live.monitoring {
                liveBar(color: .blue) {
                    ProgressView().controlSize(.mini)
                    Shimmer(text: live.monitoring ? "Monitoring" : "Working").fontWeight(.medium)
                    Elapsed(since: live.began).foregroundStyle(.secondary)
                    if let watch = live.watch, live.monitoring { Text(watch).foregroundStyle(.secondary).lineLimit(1) }
                } trailing: {
                    Button { live.interrupt() } label: { Label("Stop", systemImage: "stop.fill") }
                        .buttonStyle(.bordered)
                        .tint(.red)
                }
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField(live.working ? "Steer, or hold send to queue" : "Message", text: Bindable(live).draft, axis: .vertical)
                    .lineLimit(1...6)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 20))
                Menu {
                    if live.working {
                        Button("Steer now", systemImage: "arrow.up") { live.send() }
                        Button("Queue for after this turn", systemImage: "clock") { live.send(queue: true) }
                    }
                } label: {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 34))
                } primaryAction: {
                    live.send()
                }
                .disabled(live.draft.isEmpty)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
        .disabled(unreachable)
    }

    private func liveBar(
        color: Color, @ViewBuilder leading: () -> some View, @ViewBuilder trailing: () -> some View
    ) -> some View {
        HStack(spacing: 8) {
            leading()
            Spacer()
            trailing()
        }
        .font(.footnote)
        .controlSize(.small)
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(color.opacity(0.12), in: Capsule())
    }
}

/// A Turn's work on a screen of its own: every Activity in the order it happened, each one
/// leading to its whole content.
private struct WorkScreen: View {
    let live: Live
    let turn: Int
    @AppStorage("reasoning") private var reasoning = false

    var body: some View {
        if let turn = live.turns.first(where: { $0.id == turn }) {
            List {
                ForEach(turn.middle.filter { !$0.isUser && (reasoning || !$0.isReasoning) }) { entry in
                    if entry.isAgent {
                        Prose(text: headline(entry)).font(.subheadline)
                    } else {
                        NavigationLink(value: EntryRoute(turn: turn.id, entry: entry.id)) {
                            HStack(spacing: 10) {
                                Group {
                                    if entry.state == .running {
                                        ProgressView().controlSize(.small)
                                    } else {
                                        Image(systemName: glyph(entry))
                                            .foregroundStyle(entry.state == .failed ? .red : entry.state == .stopped ? .yellow : .secondary)
                                    }
                                }
                                .frame(width: 24)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(entry.awaits ? "Waiting on you" : entry.isApprovalRow || isQuestionnaire(entry) ? settledLine(entry) : headline(entry))
                                        .font(.subheadline.monospaced())
                                        .lineLimit(2)
                                    let note = aside(entry)
                                    if !note.isEmpty {
                                        Text(note).font(.caption).foregroundStyle(entry.state == .failed ? .red : .secondary).lineLimit(1)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .listStyle(.plain)
            .navigationTitle(turn.outcome == nil ? "Working" : turnMarker(turn))
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func isQuestionnaire(_ entry: Entry) -> Bool { if case .questionnaire = entry.body { true } else { false } }
}

private struct EntryScreen: View {
    let live: Live
    let route: EntryRoute

    var body: some View {
        if let entry = live.turns.first(where: { $0.id == route.turn })?.entries.first(where: { $0.id == route.entry }) {
            ScrollView {
                ActivityDetail(entry: entry, wrap: false).padding(16)
            }
            .navigationTitle(title(entry))
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func title(_ entry: Entry) -> String {
        switch entry.body {
        case .command: "Command"
        case .fileChange: "File change"
        case .tool(let name, _, _): name
        case .reasoning: "Thought"
        case .approval: "Approval"
        case .questionnaire: "Questionnaire"
        case .subagent: "Subagent"
        default: ""
        }
    }
}

/// An Intervention as a sheet over the conversation. Swiping it away sets it aside; the
/// orange bar above the composer brings it back.
private struct InterventionSheet: View {
    let live: Live
    let entry: Entry
    @State private var answers = Answers()
    @State private var page = 0

    var body: some View {
        NavigationStack {
            Group {
                if case .approval(let approval) = entry.body { approvalBody(approval) }
                if case .questionnaire(let questionnaire) = entry.body { questionnaireBody(questionnaire) }
            }
            .navigationBarTitleDisplayMode(.inline)
        }
        .presentationDetents(entry.isApprovalRow ? [.medium, .large] : [.large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Color(.systemBackground))
    }

    private func approvalBody(_ approval: Approval) -> some View {
        VStack(spacing: 14) {
            ScrollView { ApprovalFacts(approval: approval, wrap: false).padding(.horizontal, 20) }
            VStack(spacing: 10) {
                HStack(spacing: 10) {
                    Button { live.decide(entry.id, .decline) } label: { Text("Decline").frame(maxWidth: .infinity) }
                        .buttonStyle(.bordered)
                    Button { live.decide(entry.id, .acceptOnce) } label: { Text("Accept once").frame(maxWidth: .infinity) }
                        .buttonStyle(.borderedProminent)
                }
                .controlSize(.large)
                HStack {
                    Button("Decline and interrupt", role: .destructive) { live.decide(entry.id, .declineAndInterrupt) }
                    Spacer()
                    Button("Accept for session") { live.decide(entry.id, .acceptForSession) }
                }
                .font(.subheadline)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 12)
            .armedAfterAMoment()
        }
        .navigationTitle({ if case .command = approval.subject { "Run this command?" } else { "Make this file change?" } }())
    }

    private func questionnaireBody(_ questionnaire: Questionnaire) -> some View {
        let last = questionnaire.questions.count - 1
        return VStack(spacing: 0) {
            TabView(selection: $page) {
                ForEach(Array(questionnaire.questions.enumerated()), id: \.offset) { index, question in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 12) {
                            Text(question.title.uppercased()).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                            Text(question.text).font(.title3.weight(.semibold))
                            if question.multiple { Text("Choose any").font(.footnote).foregroundStyle(.secondary) }
                            ForEach(question.choices) { choice in
                                let on = answers.has(question, choice.label)
                                Button { answers.toggle(question, choice.label) } label: {
                                    HStack {
                                        VStack(alignment: .leading, spacing: 2) {
                                            HStack {
                                                Text(choice.label).fontWeight(.medium)
                                                if choice.recommended {
                                                    Text("Recommended").font(.caption2).padding(.horizontal, 6).padding(.vertical, 2)
                                                        .background(.tint.opacity(0.15), in: Capsule())
                                                }
                                            }
                                            Text(choice.detail).font(.footnote).foregroundStyle(.secondary)
                                        }
                                        Spacer()
                                        Image(systemName: on ? "checkmark.circle.fill" : "circle").foregroundStyle(on ? Color.accentColor : .secondary)
                                    }
                                    .padding(12)
                                    .background(on ? Color.accentColor.opacity(0.12) : Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
                                }
                                .buttonStyle(.plain)
                            }
                            if question.freeform {
                                TextField("Or write your own", text: answers.text(question))
                                    .textFieldStyle(.roundedBorder)
                            }
                        }
                        .padding(20)
                    }
                    .tag(index)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: .always))
            .indexViewStyle(.page(backgroundDisplayMode: .always))
            Button {
                if page < last { withAnimation { page += 1 } } else { live.answer(entry.id, answers.result(questionnaire.questions)) }
            } label: {
                Text(page < last ? "Next" : "Send answers").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .armedAfterAMoment()
        }
        .navigationTitle("Question \(page + 1) of \(last + 1)")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Decline", role: .destructive) { live.answer(entry.id, nil) }
            }
        }
    }
}
