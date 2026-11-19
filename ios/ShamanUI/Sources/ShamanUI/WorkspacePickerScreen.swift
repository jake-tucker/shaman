import ShamanCore
import SwiftUI

/// The Workspace Picker, pushed over the new-session screen by "Begin somewhere else": one searchable list of every
/// Remote's Workspaces, under a header for each Remote where the phone holds more than one. A row is the Icon, name
/// and Description, the Outlook's ticked; one its Server cannot read is dimmed with the reason and takes no tap.
/// Choosing a Workspace turns the Outlook toward it and pops back to the screen, which then begins there.
struct WorkspacePickerScreen: View {
  @State var model: WorkspacePickerModel
  @Environment(\.dismiss) private var dismiss
  @State private var query = ""

  var body: some View {
    List {
      if let picker = model.picker {
        ForEach(Array(picker.sections.enumerated()), id: \.offset) { _, section in
          Section {
            ForEach(section.rows, id: \.workspace) { row in
              WorkspaceRowView(row: row, invalid: model.invalid?.row == row.workspace ? model.invalid?.message : nil) {
                Task {
                  if await model.choose(row) {
                    dismiss()
                  }
                }
              }
            }
            if let line = section.line {
              Text(verbatim: line)
                .font(.callout)
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("picker.line")
            }
          } header: {
            SectionHeader(section: section)
          }
        }
      }
    }
    .accessibilityIdentifier("picker.screen")
    .overlay {
      if let empty = model.picker?.empty {
        ContentUnavailableView {
          Label {
            Text(verbatim: empty)
              .accessibilityIdentifier("picker.empty")
          } icon: {
            Image(systemName: "magnifyingglass")
          }
        }
      }
    }
    .safeAreaInset(edge: .top) {
      if let banner = model.banner {
        Banner(text: banner)
          .onTapGesture { model.dismissBanner() }
          .accessibilityAddTraits(.isButton)
          .accessibilityHint("Dismisses this")
      }
    }
    .navigationTitle(model.picker?.title ?? "")
    .navigationBarTitleDisplayMode(.inline)
    .searchable(
      text: $query, placement: .navigationBarDrawer(displayMode: .always),
      prompt: Text(verbatim: model.picker?.searchPrompt ?? "")
    )
    .onChange(of: query) { model.search(query) }
    .task { await model.follow() }
  }
}

/// A Remote's header: its name where the phone holds more than one, and that it is Unreachable where it is.
private struct SectionHeader: View {
  let section: WorkspaceSection

  var body: some View {
    if section.header != nil || section.unreachable != nil {
      HStack(spacing: 6) {
        if let header = section.header {
          Image(systemName: "server.rack")
            .accessibilityHidden(true)
          Text(verbatim: header)
        }
        if let unreachable = section.unreachable {
          Label {
            Text(verbatim: unreachable)
          } icon: {
            Image(systemName: "wifi.slash")
          }
          .foregroundStyle(.orange)
          .accessibilityIdentifier("picker.unreachable")
        }
      }
      .accessibilityElement(children: .combine)
      .accessibilityIdentifier("picker.header")
    }
  }
}

/// One Workspace: its Icon, name and Description, ticked where the Outlook stands in it; where its Server cannot read
/// it, its Icon and name dimmed and the reason in their place.
private struct WorkspaceRowView: View {
  let row: WorkspaceRow
  /// What the core refused of choosing it, shown beside it.
  let invalid: String?
  let choose: () -> Void

  var body: some View {
    if row.unavailable == nil {
      Button(action: choose) { content }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(row.chosen ? .isSelected : [])
        .accessibilityIdentifier(row.workspace)
    } else {
      // No choice at all, rather than a disabled one, which would dim the reason with the rest.
      content
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(row.workspace)
    }
  }

  private var content: some View {
    HStack(spacing: 12) {
      Group {
        if let glyph = row.glyph {
          Glyph(glyph: glyph, style: .title3)
        } else {
          Image(systemName: "folder")
            .font(.title3)
        }
      }
      .frame(width: 28)
      .foregroundStyle(row.unavailable == nil ? .secondary : .tertiary)
      .accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 2) {
        Text(verbatim: row.name)
          .foregroundStyle(row.unavailable == nil ? .primary : .tertiary)
        if let unavailable = row.unavailable {
          HStack(spacing: 4) {
            Image(systemName: "exclamationmark.triangle.fill")
              .foregroundStyle(.orange)
              .accessibilityHidden(true)
            Text(verbatim: unavailable)
              .foregroundStyle(.secondary)
          }
          .font(.caption)
        } else if let description = row.description {
          Text(verbatim: description)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
        if let invalid {
          Text(verbatim: invalid)
            .font(.caption)
            .foregroundStyle(.red)
        }
      }
      Spacer(minLength: 0)
      if row.chosen {
        Image(systemName: "checkmark")
          .foregroundStyle(.tint)
          .accessibilityHidden(true)
      }
    }
    .contentShape(Rectangle())
  }
}

/// What the Workspace Picker holds while it is up: the picker the core shows, and what choosing a Workspace failed
/// with.
@MainActor @Observable
final class WorkspacePickerModel {
  private let core: any Core
  private(set) var picker: WorkspacePicker?
  /// What the core refused of choosing a row, shown beside that row.
  private(set) var invalid: (row: String, message: String)?
  /// Why choosing failed otherwise, in the core's words, shown over the screen.
  private(set) var banner: String?

  init(core: any Core) {
    self.core = core
  }

  /// Follows the picker until the screen goes.
  func follow() async {
    for await picker in core.workspaces() {
      self.picker = picker
    }
  }

  /// Hands the core what the search field holds.
  func search(_ text: String) {
    core.searchWorkspaces(text)
  }

  /// Chooses the Workspace `row` carries, turning the Outlook toward it; whether it was, so the screen pops back.
  func choose(_ row: WorkspaceRow) async -> Bool {
    guard row.unavailable == nil else { return false }
    invalid = nil
    banner = nil
    do {
      try await core.chooseWorkspace(row.workspace)
      return true
    } catch {
      switch error {
      case .Invalid(let message): invalid = (row.workspace, message)
      case .Unreachable(let message), .Refused(let message), .Gone(let message): banner = message
      }
      return false
    }
  }

  /// Takes the banner away.
  func dismissBanner() {
    banner = nil
  }
}

#if DEBUG
  #Preview("Every remote's workspaces") {
    NavigationStack {
      WorkspacePickerScreen(model: WorkspacePickerModel(core: FakeCore(.several)))
    }
  }
#endif
