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
    // Read once at launch and never written, so a launch argument is safe to read this way.
    @AppStorage("open") private var open = "none"
    @AppStorage("pill") private var pill = true

    var body: some View {
        ZStack {
            Listing()
            if pill { PrototypeSwitcher() }
        }
        .onAppear { store.open(open) }
        .task {
            // For checking the pill without a finger: `-cycle YES` steps to the next variant.
            guard UserDefaults.standard.bool(forKey: "cycle") else { return }
            try? await Task.sleep(for: .seconds(2))
            PrototypeSwitcher.cycle(store, 1)
        }
    }
}

/// An open Session, drawn by whichever variant the pill names.
struct SessionScreen: View {
    @Environment(Store.self) private var store
    let id: String

    var body: some View {
        let live = store.live(id)
        let session = store.row(id)
        Group {
            switch store.variant {
            case "B": VariantB(live: live, session: session)
            case "C": VariantC(live: live, session: session)
            default: VariantA(live: live, session: session)
            }
        }
        .id(store.variant)
        .toolbar(.hidden, for: .bottomBar)
    }
}

/// Not part of any design: the high-contrast pill that flips variants and canned states.
/// Drag it out of the way of whatever it covers.
struct PrototypeSwitcher: View {
    @Environment(Store.self) private var store
    @State private var settled: CGSize = .zero
    @GestureState private var drag: CGSize = .zero

    static let variants = [("A", "Terminal"), ("B", "Chat"), ("C", "Timeline")]

    private var index: Int { Self.variants.firstIndex { $0.0 == store.variant } ?? 0 }

    static func cycle(_ store: Store, _ step: Int) {
        let count = variants.count
        let index = variants.firstIndex { $0.0 == store.variant } ?? 0
        store.variant = variants[(index + step + count) % count].0
    }

    private func cycle(_ step: Int) { Self.cycle(store, step) }

    var body: some View {
        @Bindable var store = store
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
                        Toggle("Reasoning shown", isOn: $store.reasoning)
                        Toggle("Diffs (not on the wire today)", isOn: $store.diffs)
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
