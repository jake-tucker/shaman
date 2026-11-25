import ShamanCore
import SwiftUI

/// The button in an open Session's navigation bar that opens the tree it belongs to, wearing how many of its
/// Subagents work as a badge, where any do. A plain label, as a bar standing down the side takes no other, whose title
/// the overflow menu and VoiceOver read.
struct TreeButtonView: View {
  let button: TreeButton
  let open: () -> Void

  var body: some View {
    Button(action: open) {
      Label {
        Text(verbatim: button.label)
      } icon: {
        Image(systemName: "point.3.connected.trianglepath.dotted")
      }
    }
    .badge(button.working.map { Text(verbatim: $0) })
    .accessibilityIdentifier("session.tree")
  }
}

/// The tree an open Session belongs to, as a sheet, where an act's failure is said as well.
struct TreeSheet: View {
  let model: SessionModel
  let choose: (String) -> Void

  var body: some View {
    NavigationStack {
      TreeScreen(model: model, saysFailures: true, choose: choose)
    }
    .presentationDetents([.medium, .large])
    // Opaque, so nothing of the conversation beneath reads as part of an entry.
    .presentationBackground(Color(.systemBackground))
    // It has nothing to put in a bar down its side, which would only narrow it.
    .toolbarVerticalBehavior(.disabled)
    .accessibilityIdentifier("tree.sheet")
  }
}

/// The tree an open Session belongs to, in its sheet or the Aside: its top-level Session first, then every Subagent
/// beneath the one that spawned it — and beneath a Sidekick's, the Sessions it has a hand in, on whichever Server — in
/// the order the core gives, the open Session highlighted. Choosing an entry hands its Session to `choose`; one the
/// core gives none for, on a Server this phone cannot open it on, takes no tap, and is greyed only where the core says
/// nothing of its work is current. A working Subagent that can be stopped alone is stopped by a swipe, which asks
/// nothing first. It crosses from the core only while it is up.
struct TreeScreen: View {
  let model: SessionModel
  /// Whether an act's failure is said here, as it is where nothing else on screen says it.
  var saysFailures = false
  let choose: (String) -> Void
  @State private var tree: SubagentTree?
  @Environment(\.asideAppearances) private var appearances

  var body: some View {
    List {
      ForEach(tree?.rows ?? [], id: \.id) { row in
        Group {
          if let session = row.session {
            Button {
              choose(session)
            } label: {
              TreeRowView(row: row)
            }
            .buttonStyle(.plain)
          } else {
            TreeRowView(row: row)
          }
        }
        .listRowBackground(row.open ? Color.accentColor.opacity(0.14) : nil)
        .accessibilityAddTraits(row.open ? .isSelected : [])
        .accessibilityIdentifier("tree.row")
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
          if let stop = row.stop, let session = row.session {
            Button(role: .destructive) {
              Task { await model.stopSubagent(session) }
            } label: {
              Label {
                Text(verbatim: stop)
              } icon: {
                Image(systemName: "stop.fill")
              }
            }
            .accessibilityIdentifier("tree.stop")
          }
        }
      }
    }
    .listStyle(.plain)
    .accessibilityIdentifier("tree.list")
    .overlay {
      if let loading = tree?.loading {
        ProgressView {
          Text(verbatim: loading)
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("tree.loading")
      } else if let failed = tree?.failed {
        ContentUnavailableView {
          Label {
            Text(verbatim: failed)
              .accessibilityIdentifier("tree.failed")
          } icon: {
            Image(systemName: "exclamationmark.triangle")
          }
        }
      }
    }
    .safeAreaInset(edge: .top) {
      if saysFailures, let banner = model.banner {
        Banner(text: banner)
          .onTapGesture { model.dismissBanner() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .navigationTitle(Text(verbatim: tree?.title ?? ""))
    .navigationBarTitleDisplayMode(.inline)
    .toolbar {
      // The count stands beneath the title, as words rather than a control.
      ToolbarItem(placement: .principal) {
        VStack(spacing: 1) {
          Text(verbatim: tree?.title ?? "")
            .font(.headline)
          if let count = tree?.count {
            Text(verbatim: count)
              .font(.caption.monospacedDigit())
              .foregroundStyle(.secondary)
              .accessibilityIdentifier("tree.count")
          }
        }
        .accessibilityElement(children: .contain)
      }
    }
    // Not until the Session is framed, which says whether it has a tree: the one read last stands meanwhile.
    .task(id: Reading(what: model.frame == nil ? nil : model.route, appearances: appearances)) {
      guard model.frame != nil else { return }
      for await tree in model.tree() {
        self.tree = tree
      }
    }
  }
}

/// One entry of the tree: its Marker and Title; where it is a Session a Sidekick has a hand in, where it works; its
/// name and Model beneath; why it opens nothing, where it opens nothing; and at the right how long it has worked,
/// counting while it works, or what it says in the time's place; indented as deep as it stands, and dimmed where
/// nothing of its work is current.
private struct TreeRowView: View {
  let row: TreeRow

  var body: some View {
    HStack(alignment: .center, spacing: 10) {
      MarkerSymbol(tone: row.marker)
        .frame(width: 22)
      VStack(alignment: .leading, spacing: 2) {
        Text(verbatim: row.title)
          .font(.subheadline.weight(row.open ? .semibold : .regular))
          .foregroundStyle(row.open ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
          .lineLimit(2)
        if let place = row.place {
          PlaceLine(place: place)
        }
        if let line = row.line {
          Text(verbatim: line)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        if let note = row.note {
          Text(verbatim: note)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(2)
            .accessibilityIdentifier("tree.note")
        }
      }
      Spacer(minLength: 0)
      Group {
        if let since = row.since {
          TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(verbatim: RunningTime.working(since: since, now: context.date))
          }
        } else if let trailing = row.trailing {
          Text(verbatim: trailing)
            .foregroundStyle(row.needsYou ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
        }
      }
      .font(.caption.monospacedDigit())
      .foregroundStyle(.secondary)
      .fixedSize()
    }
    .padding(.leading, CGFloat(row.depth) * 16)
    .opacity(row.dimmed ? 0.55 : 1)
    .contentShape(Rectangle())
    .accessibilityElement(children: .combine)
  }
}

/// Where a Session a Sidekick has a hand in works: its Remote, where it lives on another Server, and its Workspace,
/// each led by its own glyph.
private struct PlaceLine: View {
  let place: Place

  var body: some View {
    HStack(spacing: 4) {
      if let remote = place.remote {
        if let glyph = place.remoteGlyph {
          Glyph(glyph: glyph, style: .caption)
        }
        Text(verbatim: remote)
        if place.workspace != nil {
          Text(verbatim: "·")
        }
      }
      if let workspace = place.workspace {
        if let glyph = place.workspaceGlyph {
          Glyph(glyph: glyph, style: .caption)
        }
        Text(verbatim: workspace)
      }
    }
    .font(.caption)
    .foregroundStyle(.secondary)
    .lineLimit(1)
    .accessibilityElement(children: .combine)
    .accessibilityIdentifier("tree.place")
  }
}

/// The symbol an entry's Marker stands as: a spinner while it works, and its outcome once it settles.
private struct MarkerSymbol: View {
  let tone: Tone?

  var body: some View {
    Group {
      switch tone {
      case .running: ProgressView().controlSize(.small)
      case .succeeded: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
      case .failed: Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
      case .stopped: Image(systemName: "stop.circle.fill").foregroundStyle(.yellow)
      case .waiting: Image(systemName: "hand.raised.fill").foregroundStyle(.orange)
      case .quiet, .unknown: Image(systemName: "circle").foregroundStyle(.secondary)
      case nil: Color.clear.frame(width: 1, height: 1)
      }
    }
    .accessibilityHidden(true)
  }
}

#if DEBUG
  #Preview("A tree with working and settled subagents") {
    let model = SessionModel(core: FakeCore(.paired), session: "workstation-key/12.1")
    TreeSheet(model: model) { _ in }
      .task { await model.follow() }
  }
#endif
