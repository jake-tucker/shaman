// PROTOTYPE — throwaway. What an open Session holds, the Transcript's own rules every variant
// shares (what a Group gathers, what a Turn Fold hides), and a toy engine that plays a Turn.
// Answers https://github.com/jake-tucker/shaman/issues/643 and nothing else.

import SwiftUI

enum RunState { case running, done, failed, stopped }

struct DiffLine: Identifiable {
    let id = UUID()
    let mark: Character  // "+", "-", " " or "@" for a hunk header
    let text: String
}

/// One file of a File Change. The wire carries only the path today: the counts and the diff
/// are here to show what a phone could draw if a Server sent them (the pill's "Diffs" switch).
struct FileDelta: Identifiable {
    let id = UUID()
    let path: String
    let added: Int
    let removed: Int
    let diff: [DiffLine]
    var created: Bool { diff.first?.text.hasPrefix(" -0,0") ?? false }
}

enum Decision: CaseIterable, Identifiable {
    case acceptOnce, acceptForSession, decline, declineAndInterrupt
    var id: Self { self }

    /// The TUI panel's own words for each choice.
    var label: String {
        switch self {
        case .acceptOnce: "Accept once"
        case .acceptForSession: "Accept for session"
        case .decline: "Decline"
        case .declineAndInterrupt: "Decline and interrupt"
        }
    }

    /// How a decided Approval reads in the Transcript.
    var outcome: String {
        switch self {
        case .acceptOnce: "Accepted once"
        case .acceptForSession: "Accepted for session"
        case .decline: "Declined"
        case .declineAndInterrupt: "Declined and interrupted"
        }
    }

    var accepts: Bool { self == .acceptOnce || self == .acceptForSession }
}

struct Approval {
    enum Subject {
        case command(String, cwd: String)
        case fileChange([FileDelta])
    }
    let subject: Subject
    var reason: String?
    var decision: Decision?
}

struct Question: Identifiable {
    struct Choice: Identifiable {
        let label: String
        let detail: String
        var recommended = false
        var id: String { label }
    }
    let id: String
    let title: String
    let text: String
    let choices: [Choice]
    var multiple = false
    var freeform = false  // the reader may write an answer of their own
}

struct Questionnaire {
    let questions: [Question]
    var answers: [String: [String]]?  // by Question; nil while it waits, a missing key is "Not answered"
    var declined = false

    var kindWord: String { "\(questions.count) question\(questions.count == 1 ? "" : "s")" }
}

enum Body {
    case user(String)
    case agent(String, model: String?)
    case command(String, output: [String], exit: Int?, seconds: Int)
    case fileChange([FileDelta])
    case tool(name: String, subject: String, result: String)
    case reasoning(title: String?, text: String)
    case subagent(name: String, task: String)
    case error(String)
    case approval(Approval)
    case questionnaire(Questionnaire)
}

struct Entry: Identifiable {
    let id: Int
    var body: Body
    var state: RunState = .done

    var isUser: Bool { if case .user = body { true } else { false } }
    var isAgent: Bool { if case .agent = body { true } else { false } }
    var isError: Bool { if case .error = body { true } else { false } }
    var isReasoning: Bool { if case .reasoning = body { true } else { false } }
    var isWork: Bool {
        switch body {
        case .command, .fileChange, .tool: true
        default: false
        }
    }
    /// An Intervention: an Approval awaiting a Decision or a Questionnaire awaiting an Answer.
    var awaits: Bool {
        switch body {
        case .approval(let approval): approval.decision == nil
        case .questionnaire(let questionnaire): questionnaire.answers == nil && !questionnaire.declined
        default: false
        }
    }
}

enum Outcome { case worked, stopped, failed }

struct Turn: Identifiable {
    let id: Int
    var entries: [Entry]
    var outcome: Outcome?
    var seconds = 0

    /// The user Message that began it.
    var opening: [Entry] { Array(entries.prefix { $0.isUser }) }

    /// What a settled Turn keeps outside its Turn Fold at the end: the final agent Message,
    /// and a failed Turn's terminal Error.
    var closing: [Entry] {
        guard outcome != nil else { return [] }
        var kept: [Entry] = []
        var rest = entries.dropFirst(opening.count)
        if let last = rest.last, last.isError { kept.insert(last, at: 0); rest = rest.dropLast() }
        if let last = rest.last, last.isAgent { kept.insert(last, at: 0) }
        return kept
    }

    /// The work between: what a Turn Fold hides once the Turn has settled.
    var middle: [Entry] {
        let kept = Set(closing.map(\.id))
        return entries.dropFirst(opening.count).filter { !kept.contains($0.id) }
    }
}

/// A Transcript's rows once Groups have formed.
enum Block: Identifiable {
    case single(Entry)
    case work([Entry])
    case reasoning([Entry])

    var id: Int {
        switch self {
        case .single(let entry): entry.id
        case .work(let entries), .reasoning(let entries): entries[0].id
        }
    }
}

/// Adjacent commands, Tool Calls and File Changes form a work Group; adjacent Reasoning a
/// Reasoning Group; anything else ends a run. Hidden Reasoning neither joins nor ends one.
func blocks(of entries: [Entry], reasoning: Bool) -> [Block] {
    var out: [Block] = []
    for entry in entries {
        if entry.isReasoning && !reasoning { continue }
        if entry.isWork, case .work(let run) = out.last {
            out[out.count - 1] = .work(run + [entry])
        } else if entry.isWork {
            out.append(.work([entry]))
        } else if entry.isReasoning, case .reasoning(let run) = out.last {
            out[out.count - 1] = .reasoning(run + [entry])
        } else if entry.isReasoning {
            out.append(.reasoning([entry]))
        } else {
            out.append(.single(entry))
        }
    }
    return out
}

func duration(_ seconds: Int) -> String {
    switch seconds {
    case ..<60: "\(seconds)s"
    case ..<3600: seconds % 60 == 0 ? "\(seconds / 60)m" : "\(seconds / 60)m \(seconds % 60)s"
    default: "\(seconds / 3600)h \(seconds % 3600 / 60)m"
    }
}

private func counted(_ n: Int, _ noun: String) -> String? {
    n == 0 ? nil : "\(n) \(noun)\(n == 1 ? "" : "s")"
}

/// "3 tools · 1 command · 6 edits": kind by kind, in an order that never changes.
func tally(_ entries: [Entry]) -> String {
    var tools = 0, commands = 0, edits = 0
    for entry in entries {
        switch entry.body {
        case .tool: tools += 1
        case .command: commands += 1
        case .fileChange: edits += 1
        default: break
        }
    }
    return [counted(tools, "tool"), counted(commands, "command"), counted(edits, "edit")]
        .compactMap { $0 }.joined(separator: " · ")
}

/// One line for an Activity, as its Fold reads closed.
func headline(_ entry: Entry) -> String {
    switch entry.body {
    case .user(let text), .agent(let text, _), .error(let text): text
    case .command(let line, _, _, _): line
    case .fileChange(let files):
        fileVerb(entry.state, created: files[0].created) + " "
            + (files.count == 1 ? files[0].path : "\(files[0].path) and \(files.count - 1) more")
    case .tool(let name, let subject, _): "\(name) \(subject)"
    case .reasoning(let title, _):
        (entry.state == .running ? "Thinking" : "Thought") + (title.map { ": \($0)" } ?? "")
    case .subagent(let name, let task): "Subagent: \(name): \(task)"
    case .approval(let approval):
        switch approval.subject {
        case .command(let line, _): line
        case .fileChange(let files): files.map(\.path).joined(separator: ", ")
        }
    case .questionnaire(let questionnaire): questionnaire.questions.map(\.title).joined(separator: " · ")
    }
}

func fileVerb(_ state: RunState, created: Bool) -> String {
    switch state {
    case .running: created ? "Creating" : "Editing"
    case .failed: "Failed to edit"
    case .stopped: "Stopped editing"
    case .done: created ? "Created" : "Edited"
    }
}

/// How a settled Intervention reads in the Transcript, in the TUI's words.
func settledLine(_ entry: Entry) -> String {
    switch entry.body {
    case .approval(let approval):
        let kind = if case .command = approval.subject { "Command" } else { "File change" }
        let outcome = approval.decision?.outcome ?? "Turn ended before a decision"
        return "Approval · \(kind) · \(outcome)"
    case .questionnaire(let questionnaire):
        let outcome = questionnaire.declined ? "Declined" : questionnaire.answers == nil ? "Turn ended" : "Answered"
        return "Questionnaire · \(outcome) · \(questionnaire.kindWord)"
    default: return ""
    }
}

/// "✓ Worked for 1m 23s", "× Stopped after 47s", "× Failed after 12s".
func turnMarker(_ turn: Turn) -> String {
    switch turn.outcome {
    case .worked: "Worked for \(duration(turn.seconds))"
    case .stopped: "Stopped after \(duration(turn.seconds))"
    case .failed: "Failed after \(duration(turn.seconds))"
    case nil: "Working"
    }
}

/// A work Group's marker: the counts, the work still running, and how many failed or stopped.
func groupMarker(_ entries: [Entry]) -> String {
    var parts = [tally(entries)]
    if let running = entries.last(where: { $0.state == .running }) {
        parts.append(headline(running))
    }
    let failed = entries.filter { $0.state == .failed }.count
    let stopped = entries.filter { $0.state == .stopped }.count
    if failed > 0 { parts.append("\(failed) failed") }
    if stopped > 0 { parts.append("\(stopped) stopped") }
    return parts.joined(separator: " · ")
}

/// The worst of a run: failed over stopped over running over done.
func worst(_ entries: [Entry]) -> RunState {
    if entries.contains(where: { $0.state == .failed }) { return .failed }
    if entries.contains(where: { $0.state == .stopped }) { return .stopped }
    if entries.contains(where: { $0.state == .running }) { return .running }
    return .done
}

/// The few words at the right of a closed Activity.
func aside(_ entry: Entry) -> String {
    switch entry.body {
    case .command(_, _, let exit, _):
        entry.state == .failed ? "exit \(exit ?? 1)" : entry.state == .stopped ? "stopped" : ""
    case .tool(_, _, let result): result
    default: ""
    }
}

func glyph(_ entry: Entry) -> String {
    switch entry.body {
    case .user: "person"
    case .agent: "sparkle"
    case .command: "terminal"
    case .fileChange: "pencil.line"
    case .tool: "wrench.and.screwdriver"
    case .reasoning: "brain"
    case .subagent: "person.2"
    case .error: "exclamationmark.triangle"
    case .approval: "hand.raised"
    case .questionnaire: "list.bullet.rectangle"
    }
}

enum Step {
    case wait(Double)
    case begin(Body)  // an Activity starts running
    case output(String)  // a line arrives for the running command
    case finish(RunState)  // the running Activity settles
    case add(Body)  // something arrives whole
    case settle(Outcome)
}

/// One Session as a Client holds it open: its Turns, and the toy engine that plays the rest
/// of a canned Turn so live work, interrupting and answering can be felt rather than read.
@Observable
final class Live {
    let id: String
    var turns: [Turn]
    var began: Date?  // when the live Turn's work began
    var watch: String?  // the Watch a Monitoring Session waits on
    var draft = ""
    var queued: [String] = []  // Prompts held for after the live Turn
    var setAside = false  // the reader dismissed the Interventions pending just now
    var allowed: [Step] = []  // what plays once an Intervention is approved or answered
    var denied: [Step] = []
    private var script: [Step] = []
    private var task: Task<Void, Never>?
    private var nextID = 10_000

    init(id: String, turns: [Turn], elapsed: Int = 0, watch: String? = nil, script: [Step] = [],
         allowed: [Step] = [], denied: [Step] = []) {
        self.id = id
        self.turns = turns
        self.watch = watch
        self.script = script
        self.allowed = allowed
        self.denied = denied
        if working || watch != nil { began = Date().addingTimeInterval(-Double(elapsed)) }
        if working { play() }
    }

    var working: Bool { !turns.isEmpty && turns.last?.outcome == nil }
    var monitoring: Bool { !working && watch != nil }

    /// The open Session's own Interventions, oldest first in Transcript order.
    var pending: [Entry] { working ? turns[turns.count - 1].entries.filter(\.awaits) : [] }
    var intervention: Entry? { setAside ? nil : pending.first }

    /// The Activity running now, for surfaces that name the live work.
    var running: Entry? { working ? turns[turns.count - 1].entries.last { $0.state == .running } : nil }

    var standing: Standing {
        if !pending.isEmpty { return .needsIntervention }
        if working { return .working }
        if turns.last?.outcome == .failed { return .failed }
        if monitoring { return .monitoring }
        return .nothing
    }

    /// Changes whenever the Transcript grows, for views that follow its end.
    var pulse: Int {
        turns.count * 1000 + (turns.last?.entries.count ?? 0) * 50 + queued.count
            + (turns.last?.entries.last.map { entry in
                if case .command(_, let output, _, _) = entry.body { output.count } else { 0 }
            } ?? 0)
    }

    private func append(_ body: Body, _ state: RunState) {
        nextID += 1
        let entry = Entry(id: nextID, body: body, state: state)
        if entry.awaits { setAside = false }  // a new Intervention presents itself
        turns[turns.count - 1].entries.append(entry)
    }

    private func mutateRunning(_ change: (inout Entry) -> Void) {
        let turn = turns.count - 1
        guard let index = turns[turn].entries.lastIndex(where: { $0.state == .running }) else { return }
        change(&turns[turn].entries[index])
    }

    private func play() {
        task?.cancel()
        task = Task { @MainActor [weak self] in
            while let self, !self.script.isEmpty, self.working {
                if !self.pending.isEmpty { return }  // an Intervention holds the Turn
                switch self.script.removeFirst() {
                case .wait(let seconds):
                    try? await Task.sleep(for: .seconds(seconds))
                    if Task.isCancelled { return }
                case .begin(let body): self.append(body, .running)
                case .output(let line):
                    self.mutateRunning { entry in
                        if case .command(let command, let output, let exit, let seconds) = entry.body {
                            entry.body = .command(command, output: output + [line], exit: exit, seconds: seconds)
                        }
                    }
                case .finish(let state): self.mutateRunning { $0.state = state }
                case .add(let body): self.append(body, .done)
                case .settle(let outcome): self.settle(outcome)
                }
            }
        }
    }

    private func settle(_ outcome: Outcome) {
        let turn = turns.count - 1
        turns[turn].outcome = outcome
        turns[turn].seconds = Int(Date().timeIntervalSince(began ?? .now))
        began = nil
        setAside = false
        if outcome == .worked, !queued.isEmpty {
            let text = queued.removeFirst()
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(0.6))
                if !self.working { self.begin(text) }
            }
        }
    }

    /// A Prompt begins a Turn when the Session is idle. While a Turn is live it steers that
    /// Turn, or waits its turn in the queue.
    func send(queue: Bool = false) {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        draft = ""
        if working && queue {
            queued.append(text)
        } else if working {
            append(.user(text), .done)
        } else {
            begin(text)
        }
    }

    private func begin(_ text: String) {
        nextID += 1
        turns.append(Turn(id: nextID, entries: []))
        append(.user(text), .done)
        began = .now
        script = Canned.reply
        play()
    }

    /// A queued Prompt promoted steers the live Turn now.
    func promote(_ index: Int) {
        let text = queued.remove(at: index)
        if working { append(.user(text), .done) } else { begin(text) }
    }

    func interrupt() {
        guard working else { return }
        task?.cancel()
        script = []
        mutateRunning { $0.state = .stopped }
        settle(.stopped)
    }

    func decide(_ id: Int, _ decision: Decision) {
        change(id) { entry in
            if case .approval(var approval) = entry.body {
                approval.decision = decision
                entry.body = .approval(approval)
            }
        }
        if decision == .declineAndInterrupt { return interrupt() }
        script = decision.accepts ? allowed : denied
        play()
    }

    /// Answers a Questionnaire, or declines it when `answers` is nil.
    func answer(_ id: Int, _ answers: [String: [String]]?) {
        change(id) { entry in
            if case .questionnaire(var questionnaire) = entry.body {
                questionnaire.answers = answers
                questionnaire.declined = answers == nil
                entry.body = .questionnaire(questionnaire)
            }
        }
        script = answers == nil ? denied : allowed
        play()
    }

    private func change(_ id: Int, _ change: (inout Entry) -> Void) {
        let turn = turns.count - 1
        guard let index = turns[turn].entries.firstIndex(where: { $0.id == id }) else { return }
        change(&turns[turn].entries[index])
    }
}
