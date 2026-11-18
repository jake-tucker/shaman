import ShamanCore
import SwiftUI
import Textual

/// A Turn's work, pushed from its card: every Activity in the order it happened, the Agent's Messages along the way
/// among them, each row drawn from the one shape the core fills for every kind, and each pushing the whole of its
/// Activity where there is more to it than its row. It crosses from the core only while it is up.
struct TurnWorkScreen: View {
  let session: SessionModel
  let turn: String
  @State private var work: TurnWork?

  var body: some View {
    List {
      ForEach(work?.items ?? [], id: \.id) { item in
        switch item.kind {
        case .row(let row):
          if case .detail(let activity) = row.tap {
            NavigationLink(value: Pushed.detail(session.route, activity: activity)) {
              WorkRowView(row: row)
            }
            .accessibilityIdentifier("work.row")
          } else {
            WorkRowView(row: row)
              .accessibilityIdentifier("work.row")
          }
        case .agentMessage(let markdown, _):
          StructuredText(markdown: markdown)
            .textual.overflowMode(.scroll)
            .textual.tableStyle(.overflow)
            .font(.subheadline)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("work.agentMessage")
        }
      }
    }
    .listStyle(.plain)
    .overlay {
      if let empty = work?.empty {
        ContentUnavailableView {
          Label {
            Text(verbatim: empty)
              .accessibilityIdentifier("work.empty")
          } icon: {
            Image(systemName: "list.bullet")
          }
        }
      }
    }
    .navigationTitle(work?.title ?? "")
    .navigationBarTitleDisplayMode(.inline)
    .accessibilityIdentifier("work.list")
    .task {
      for await work in session.work(turn) {
        self.work = work
      }
    }
  }
}

/// One Activity among a Turn's work: its glyph in its tone, its title over the line beneath, and its trailing words.
struct WorkRowView: View {
  let row: WorkRow

  var body: some View {
    HStack(alignment: .center, spacing: 10) {
      ToneGlyph(glyph: row.glyph, tone: row.tone)
        .frame(width: 24)
      VStack(alignment: .leading, spacing: 2) {
        // What was run or touched in monospace, as it was written; words in prose.
        Text(verbatim: row.title)
          .font(row.titleStyle == .monospace ? .subheadline.monospaced() : .subheadline)
          .lineLimit(2)
        if let line = row.line {
          Text(verbatim: line)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.middle)
        }
      }
      Spacer(minLength: 0)
      if let trailing = row.trailing {
        Text(verbatim: trailing)
          .font(.caption)
          .monospacedDigit()
          .foregroundStyle(row.tone == .failed ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
          .fixedSize()
      }
    }
    .accessibilityElement(children: .combine)
  }
}

/// The glyph a row is led by, coloured by how its Activity stands; a spinner while it runs.
struct ToneGlyph: View {
  let glyph: String?
  let tone: Tone

  var body: some View {
    Group {
      if tone == .running {
        ProgressView().controlSize(.small)
      } else if let glyph {
        Glyph(glyph: glyph, style: .body)
          .foregroundStyle(tone.style)
      } else {
        Image(systemName: "circle.fill")
          .font(.caption2)
          .foregroundStyle(tone.style)
      }
    }
    .accessibilityHidden(true)
  }
}

extension Tone {
  /// The colour a glyph standing so is drawn in: the plain one for what went as asked, and a colour of its own for
  /// what waits on the reader, failed or was stopped.
  var style: AnyShapeStyle {
    switch self {
    case .running: AnyShapeStyle(.tint)
    case .waiting: AnyShapeStyle(.orange)
    case .failed: AnyShapeStyle(.red)
    case .stopped: AnyShapeStyle(.yellow)
    case .succeeded, .quiet, .unknown: AnyShapeStyle(.secondary)
    }
  }
}

/// The whole of one Activity, pushed from its row: its labelled facts, then its sections of body, prose wrapping and
/// monospace keeping its lines and scrolling sideways. It crosses from the core only while it is up, so a running
/// command's output grows in it.
struct ActivityDetailScreen: View {
  let session: SessionModel
  let activity: String
  @State private var detail: ActivityDetail?

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 16) {
        ForEach(Array((detail?.facts ?? []).enumerated()), id: \.offset) { _, fact in
          FactView(fact: fact)
        }
        ForEach(Array((detail?.sections ?? []).enumerated()), id: \.offset) { _, section in
          SectionView(section: section)
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(16)
    }
    .navigationTitle(detail?.title ?? "")
    .navigationBarTitleDisplayMode(.inline)
    .accessibilityIdentifier("detail.screen")
    .task {
      for await detail in session.detail(activity) {
        self.detail = detail
      }
    }
  }
}

/// One labelled fact: its label over its value.
private struct FactView: View {
  let fact: Fact

  var body: some View {
    VStack(alignment: .leading, spacing: 3) {
      Text(verbatim: fact.label)
        .font(.caption)
        .foregroundStyle(.secondary)
      Text(verbatim: fact.value)
        .font(fact.style == .monospace ? .callout.monospaced() : .callout)
        .textSelection(.enabled)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("detail.fact")
  }
}

/// One section of an Activity's body, under its heading where it has one.
private struct SectionView: View {
  let section: DetailSection

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      if let heading = section.heading {
        Text(verbatim: heading)
          .font(.caption)
          .foregroundStyle(.secondary)
      }
      switch section.style {
      case .monospace:
        // Its lines kept as they are, scrolling sideways rather than wrapping.
        ScrollView(.horizontal) {
          Text(verbatim: section.body)
            .font(.footnote.monospaced())
            .fixedSize(horizontal: true, vertical: false)
            .textSelection(.enabled)
            .padding(10)
            .accessibilityIdentifier("detail.monospace")
        }
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityIdentifier("detail.monospaceScroll")
      case .prose:
        Text(verbatim: section.body)
          .font(.callout)
          .textSelection(.enabled)
          .accessibilityIdentifier("detail.prose")
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

#if DEBUG
  #Preview("A settled turn's work") {
    NavigationStack {
      TurnWorkScreen(session: SessionModel(core: FakeCore(.paired), session: "workstation-key/7"), turn: "work")
    }
  }

  #Preview("A running command") {
    NavigationStack {
      ActivityDetailScreen(
        session: SessionModel(core: FakeCore(.paired), session: "workstation-key/1"), activity: "run-qr")
    }
  }
#endif
