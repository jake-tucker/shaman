// PROTOTYPE — throwaway. Variant C, "Timeline": no folds at all. Every Activity is one row on
// a rail, in the order it happened, and a tap opens it in a half sheet that leaves the
// Transcript in view behind. The Prompt that began a Turn stays pinned while its Turn is read,
// and the title menu jumps between Turns. Live work is the navigation bar's: the Standing under
// the title and the stop button beside it. An Intervention is answered where it stands in the
// Transcript, and the composer waits behind a button so the whole screen is for reading.

import SwiftUI

struct VariantC: View {
    @Environment(Store.self) private var store
    let live: Live
    let session: SessionRow
    @AppStorage("reasoning") private var reasoning = false
    @State private var inspected: Entry?
    @State private var composing = false
    @State private var confirmStop = false
    @State private var jump: Int?
    @AppStorage("drill") private var drill = false  // for screenshots: launch with a command open

    private var unreachable: Bool { store.isUnreachable(session) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                    ForEach(live.turns) { turn in
                        Section {
                            ForEach(turn.entries.dropFirst(turn.opening.count).filter { reasoning || !$0.isReasoning }) { row($0) }
                            if turn.outcome != nil {
                                Text(turnMarker(turn))
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 10)
                            }
                        } header: {
                            promptHeader(turn)
                        }
                        .id(turn.id)
                    }
                    Color.clear.frame(height: 76).id("end")
                }
            }
            .defaultScrollAnchor(.bottom)
            .defaultScrollAnchor(.top, for: .alignment)  // a short Transcript reads from the top
            .onChange(of: live.pulse) { withAnimation { proxy.scrollTo("end") } }
            .onChange(of: jump) {
                if let jump { withAnimation { proxy.scrollTo(jump, anchor: .top) } }
                jump = nil
            }
        }
        .overlay(alignment: .bottomTrailing) { composeButton }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) { titleMenu }
            if live.working || live.monitoring {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { confirmStop = true } label: { Image(systemName: "stop.fill") }.tint(.red)
                }
            }
        }
        .confirmationDialog(live.monitoring ? "Stop monitoring?" : "Interrupt this turn?", isPresented: $confirmStop, titleVisibility: .visible) {
            Button(live.monitoring ? "Stop" : "Interrupt", role: .destructive) { live.interrupt() }
        }
        .sheet(item: $inspected) { entry in
            ScrollView { ActivityDetail(entry: entry, wrap: true).padding(20) }
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationBackgroundInteraction(.enabled(upThrough: .medium))
                .presentationBackground(Color(.systemBackground))
        }
        .sheet(isPresented: $composing) { ComposeSheetC(live: live) }
        .onAppear {
            if drill { inspected = live.turns[0].entries.last { if case .command = $0.body { true } else { false } } }
        }
    }

    // MARK: The navigation bar: the Session's name, its Standing, and the way between Turns

    private var titleMenu: some View {
        Menu {
            Section("Jump to a turn") {
                ForEach(live.turns) { turn in
                    Button { jump = turn.id } label: {
                        Text(turn.opening.first.map(headline) ?? "Continuation")
                        Text(turnMarker(turn))
                    }
                }
            }
        } label: {
            VStack(spacing: 1) {
                HStack(spacing: 4) {
                    Text(session.title).font(.subheadline.weight(.semibold)).lineLimit(1)
                    Image(systemName: "chevron.down").font(.system(size: 9, weight: .bold))
                }
                .foregroundStyle(.primary)
                statusLine.font(.caption2)
            }
        }
    }

    @ViewBuilder private var statusLine: some View {
        if unreachable {
            Text("\(store.origin(session).id) unreachable").foregroundStyle(.secondary)
        } else if !live.pending.isEmpty {
            Text("Needs intervention").foregroundStyle(.orange)
        } else if live.working || live.monitoring {
            HStack(spacing: 4) {
                Shimmer(text: live.monitoring ? "Monitoring" : "Working")
                Elapsed(since: live.began)
            }
            .foregroundStyle(.blue)
        } else {
            Text([store.place(session.workspace), session.checkout].compactMap { $0 }.joined(separator: " · "))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }

    /// The Prompt that began the Turn, pinned to the top for as long as its Turn is on screen.
    private func promptHeader(_ turn: Turn) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "person.fill").font(.caption).foregroundStyle(.tint)
            Text(turn.opening.first.map(headline) ?? "Continuation").font(.subheadline.weight(.medium)).lineLimit(3)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .background(.bar)
        .overlay(alignment: .bottom) { Divider() }
    }

    // MARK: Rows

    @ViewBuilder private func row(_ entry: Entry) -> some View {
        switch entry.body {
        case .agent:
            Prose(text: headline(entry)).padding(.horizontal, 14).padding(.vertical, 8)
        case .user:
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "person.fill").font(.caption).foregroundStyle(.tint)
                Text(headline(entry)).font(.subheadline.weight(.medium))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        case .error:
            railRow(entry, tint: .red) { Text(headline(entry)).font(.footnote).foregroundStyle(.red) }
        case .approval(let approval):
            if entry.awaits { approvalCard(entry, approval) } else { settledRow(entry) }
        case .questionnaire(let questionnaire):
            if entry.awaits { QuestionnaireCardC(live: live, entry: entry, questionnaire: questionnaire) } else { settledRow(entry) }
        default:
            Button { inspected = entry } label: {
                railRow(entry, tint: entry.state == .failed ? .red : entry.state == .stopped ? .yellow : .secondary) {
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(headline(entry)).font(.footnote.monospaced()).lineLimit(1)
                            Spacer(minLength: 6)
                            Text(aside(entry)).font(.caption2).foregroundStyle(entry.state == .failed ? .red : .secondary).lineLimit(1)
                        }
                        // A running command shows the tail of what it has said so far.
                        if entry.state == .running, case .command(_, let output, _, _) = entry.body, !output.isEmpty {
                            OutputLines(lines: Array(output.suffix(3)))
                        }
                    }
                }
            }
            .buttonStyle(.plain)
        }
    }

    private func settledRow(_ entry: Entry) -> some View {
        Button { inspected = entry } label: {
            railRow(entry, tint: .secondary) {
                Text(settledLine(entry)).font(.footnote).foregroundStyle(.secondary)
            }
        }
        .buttonStyle(.plain)
    }

    /// One row on the rail: the line running down the left, the kind's glyph on it.
    private func railRow(_ entry: Entry, tint: Color, @ViewBuilder content: () -> some View) -> some View {
        HStack(alignment: .top, spacing: 10) {
            ZStack {
                Rectangle().fill(Color(.separator)).frame(width: 1)
                Group {
                    if entry.state == .running {
                        ProgressView().controlSize(.mini)
                    } else {
                        Image(systemName: glyph(entry)).font(.system(size: 11)).foregroundStyle(tint)
                    }
                }
                .frame(width: 20, height: 20)
                .background(Color(.systemBackground))
                .padding(.top, 5)
                .frame(maxHeight: .infinity, alignment: .top)
            }
            .frame(width: 20)
            content().padding(.top, 7).padding(.bottom, 5)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .contentShape(Rectangle())
    }

    // MARK: Interventions, answered where they stand

    private func card(@ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 12) { content() }
            .padding(14)
            .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.orange.opacity(0.5)))
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
    }

    private func approvalCard(_ entry: Entry, _ approval: Approval) -> some View {
        card {
            Label("Approval", systemImage: "hand.raised.fill").font(.footnote.weight(.semibold)).foregroundStyle(.orange)
            ApprovalFacts(approval: approval)
            HStack(spacing: 8) {
                Button("Decline") { live.decide(entry.id, .decline) }.buttonStyle(.bordered)
                Button("Accept once") { live.decide(entry.id, .acceptOnce) }.buttonStyle(.borderedProminent)
                Spacer()
                Menu {
                    Button("Accept for session") { live.decide(entry.id, .acceptForSession) }
                    Button("Decline and interrupt", role: .destructive) { live.decide(entry.id, .declineAndInterrupt) }
                } label: {
                    Image(systemName: "ellipsis.circle").font(.title3)
                }
            }
            .armedAfterAMoment()
        }
    }

    // MARK: The composer, behind a button

    private var composeButton: some View {
        Button { composing = true } label: {
            HStack(spacing: 6) {
                Image(systemName: "square.and.pencil")
                Text(live.working ? "Steer" : "Prompt")
                if !live.queued.isEmpty { Text("· \(live.queued.count) queued").fontWeight(.regular) }
            }
            .font(.subheadline.weight(.semibold))
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(Color.accentColor, in: Capsule())
            .foregroundStyle(.white)
            .shadow(radius: 5, y: 2)
        }
        .padding(16)
        .disabled(unreachable)
        .opacity(unreachable ? 0.4 : 1)
    }
}

/// Every Question at once, in the Transcript, with one button to send them all.
private struct QuestionnaireCardC: View {
    let live: Live
    let entry: Entry
    let questionnaire: Questionnaire
    @State private var answers = Answers()

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("Questionnaire · \(questionnaire.kindWord)", systemImage: "list.bullet.rectangle.fill")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.orange)
            ForEach(questionnaire.questions) { question in
                VStack(alignment: .leading, spacing: 6) {
                    Text(question.text).font(.subheadline.weight(.medium))
                    ForEach(question.choices) { choice in
                        let on = answers.has(question, choice.label)
                        Button { answers.toggle(question, choice.label) } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Image(systemName: question.multiple ? (on ? "checkmark.square.fill" : "square") : (on ? "largecircle.fill.circle" : "circle"))
                                    .foregroundStyle(on ? Color.accentColor : .secondary)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(choice.label + (choice.recommended ? " · recommended" : "")).font(.subheadline)
                                    Text(choice.detail).font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer(minLength: 0)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    if question.freeform {
                        TextField("Or write your own", text: answers.text(question))
                            .textFieldStyle(.roundedBorder)
                            .font(.subheadline)
                    }
                }
            }
            HStack {
                Button("Decline") { live.answer(entry.id, nil) }.buttonStyle(.bordered)
                Button("Send answers") { live.answer(entry.id, answers.result(questionnaire.questions)) }
                    .buttonStyle(.borderedProminent)
            }
            .armedAfterAMoment()
        }
        .padding(14)
        .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(.orange.opacity(0.5)))
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }
}

/// A sheet with room to write. While a Turn is live it says outright which of the two
/// deliveries a Prompt gets.
private struct ComposeSheetC: View {
    let live: Live
    @Environment(\.dismiss) private var dismiss
    @FocusState private var typing: Bool

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 12) {
                TextEditor(text: Bindable(live).draft)
                    .focused($typing)
                    .scrollContentBackground(.hidden)
                    .padding(8)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
                if !live.queued.isEmpty {
                    Text("Queued for after this turn").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(Array(live.queued.enumerated()), id: \.offset) { index, text in
                        HStack {
                            Text(text).font(.footnote).lineLimit(1)
                            Spacer()
                            Button("Steer now") { live.promote(index) }.font(.caption)
                            Button { live.queued.remove(at: index) } label: { Image(systemName: "xmark") }.font(.caption)
                        }
                    }
                }
                HStack {
                    if live.working {
                        Button { live.send(queue: true); dismiss() } label: { Text("Queue").frame(maxWidth: .infinity) }
                            .buttonStyle(.bordered)
                        Button { live.send(); dismiss() } label: { Text("Steer now").frame(maxWidth: .infinity) }
                            .buttonStyle(.borderedProminent)
                    } else {
                        Button { live.send(); dismiss() } label: { Text("Send").frame(maxWidth: .infinity) }
                            .buttonStyle(.borderedProminent)
                    }
                }
                .controlSize(.large)
                .disabled(live.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(16)
            .navigationTitle(live.working ? "Steer this turn" : "Prompt")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
            .onAppear { typing = true }
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(Color(.systemBackground))
    }
}
