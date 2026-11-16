// PROTOTYPE — throwaway. Three variants of an open Session on a phone, pushed from the listing
// that won ticket 642 and switched from the yellow pill (or `-variant B -open approval` at launch).
// Answers https://github.com/jake-tucker/shaman/issues/643.
//
//   A · Terminal   the TUI's Transcript on a narrow screen: Turn Folds, Groups and Folds open in
//                  place, the Working Indicator is the last row, an Intervention's panel takes
//                  the composer's place.
//   B · Chat       bubbles and one card for a Turn's work; nothing opens in place, work is
//                  pushed onto screens of its own; a live bar above the composer; an
//                  Intervention is a sheet.
//   C · Timeline   no folds at all: every Activity one row on a rail, opened in a half sheet;
//                  the Turn's Prompt pinned as you read; live work in the navigation bar; an
//                  Intervention answered where it stands; the composer behind a button.

import SwiftUI

@main
struct TranscriptPrototypeApp: App {
    @State private var store = Store()

    var body: some Scene {
        WindowGroup {
            Root().environment(store)
        }
    }
}

struct Root: View {
    @Environment(Store.self) private var store
    @AppStorage("variant") private var variant = "A"
    @AppStorage("open") private var open = "none"
    @AppStorage("pill") private var pill = true

    var body: some View {
        ZStack {
            Listing()
            if pill { PrototypeSwitcher(variant: $variant) }
        }
        .onAppear { store.open(open) }
    }
}

/// An open Session, drawn by whichever variant the pill names.
struct SessionScreen: View {
    @Environment(Store.self) private var store
    @AppStorage("variant") private var variant = "A"
    let id: String

    var body: some View {
        let live = store.live(id)
        let session = store.row(id)
        Group {
            switch variant {
            case "B": VariantB(live: live, session: session)
            case "C": VariantC(live: live, session: session)
            default: VariantA(live: live, session: session)
            }
        }
        .id(variant)
        .toolbar(.hidden, for: .bottomBar)
    }
}

/// Not part of any design: the high-contrast pill that flips variants and canned states.
/// Drag it out of the way of whatever it covers.
struct PrototypeSwitcher: View {
    @Environment(Store.self) private var store
    @Binding var variant: String
    @AppStorage("reasoning") private var reasoning = false
    @AppStorage("diffs") private var diffs = false
    @State private var settled: CGSize = .zero
    @GestureState private var drag: CGSize = .zero

    static let variants = [("A", "Terminal"), ("B", "Chat"), ("C", "Timeline")]

    private var index: Int { Self.variants.firstIndex { $0.0 == variant } ?? 0 }

    private func cycle(_ step: Int) {
        let count = Self.variants.count
        variant = Self.variants[(index + step + count) % count].0
    }

    var body: some View {
        VStack {
            HStack(spacing: 10) {
                Button { cycle(-1) } label: { Image(systemName: "chevron.left") }
                Text("\(Self.variants[index].0) · \(Self.variants[index].1)")
                    .font(.footnote.weight(.bold))
                Button { cycle(1) } label: { Image(systemName: "chevron.right") }
                Menu {
                    Section("Open a state") {
                        ForEach(Canned.states, id: \.key) { state in
                            Button(state.label) { store.open(state.key) }
                        }
                    }
                    Section("Settings a reader could turn") {
                        Toggle("Reasoning shown", isOn: $reasoning)
                        Toggle("Diffs (not on the wire today)", isOn: $diffs)
                    }
                } label: {
                    Image(systemName: "slider.horizontal.3")
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .foregroundStyle(.black)
            .background(Color.yellow, in: Capsule())
            .shadow(radius: 6, y: 2)
            .offset(x: settled.width + drag.width, y: settled.height + drag.height)
            .gesture(
                DragGesture()
                    .updating($drag) { value, state, _ in state = value.translation }
                    .onEnded { value in
                        settled.width += value.translation.width
                        settled.height += value.translation.height
                    }
            )
            .padding(.top, 48)
            Spacer()
        }
    }
}
