// PROTOTYPE — throwaway. Three variants of the screen a phone shows when no Session is open,
// switched from the yellow pill (or `-variant B -scenario unreachable` at launch).
// Answers https://github.com/jake-tucker/shaman/issues/642.

import SwiftUI

@main
struct SessionListPrototypeApp: App {
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
    @AppStorage("scenario") private var scenarioRaw = Scenario.threeServers.rawValue

    var body: some View {
        ZStack {
            Group {
                switch variant {
                case "B": VariantB()
                case "C": VariantC()
                default: VariantA()
                }
            }
            .id(variant + scenarioRaw)

            PrototypeSwitcher(variant: $variant, scenarioRaw: $scenarioRaw)
        }
        .onChange(of: scenarioRaw, initial: true) {
            store.apply(Scenario(rawValue: scenarioRaw) ?? .threeServers)
        }
    }
}

/// Not part of any design: the high-contrast pill that flips variants and canned scenarios.
/// Drag it out of the way of whatever it covers.
struct PrototypeSwitcher: View {
    @Binding var variant: String
    @Binding var scenarioRaw: String
    @State private var settled: CGSize = .zero
    @GestureState private var drag: CGSize = .zero

    static let variants = [("A", "Inbox"), ("B", "Places"), ("C", "Attention")]

    private var index: Int { Self.variants.firstIndex { $0.0 == variant } ?? 0 }

    private func cycle(_ step: Int) {
        let count = Self.variants.count
        variant = Self.variants[(index + step + count) % count].0
    }

    var body: some View {
        VStack {
            Spacer()
            HStack(spacing: 10) {
                Button { cycle(-1) } label: { Image(systemName: "chevron.left") }
                Text("\(Self.variants[index].0) · \(Self.variants[index].1)")
                    .font(.footnote.weight(.bold))
                Button { cycle(1) } label: { Image(systemName: "chevron.right") }
                Menu {
                    Picker("Scenario", selection: $scenarioRaw) {
                        ForEach(Scenario.allCases) { Text($0.label).tag($0.rawValue) }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "slider.horizontal.3")
                        Text(Scenario(rawValue: scenarioRaw)?.label ?? "")
                            .font(.caption2.weight(.semibold))
                            .lineLimit(1)
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
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
            .padding(.bottom, 110)
        }
    }
}
