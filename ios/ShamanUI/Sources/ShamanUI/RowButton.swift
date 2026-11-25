import SwiftUI

/// A list's row that leads to another screen, drawn as a `NavigationLink` draws one: a link is greyed out in a hosted
/// screen, with no `NavigationStack` over it to push on.
struct RowButton<Row: View>: View {
  let action: () -> Void
  @ViewBuilder let row: Row

  var body: some View {
    Button(action: action) {
      // The chevron's size and where it stands are a link's own, measured in iOS 27.2.
      HStack(spacing: 10) {
        row
          .frame(maxWidth: .infinity, alignment: .leading)
        Image(systemName: "chevron.forward")
          .font(.body.weight(.semibold))
          .imageScale(.small)
          .foregroundStyle(.tertiary)
          .padding(.trailing, 5 / 3)
          .accessibilityHidden(true)
      }
    }
    // Not the tint, nor `Color.primary`, whose secondary is another grey than a link's.
    .foregroundStyle(Color(.label))
  }
}
