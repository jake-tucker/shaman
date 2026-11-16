// PROTOTYPE — throwaway. Three answers to what the listing's compose button opens and how a
// first Prompt begins a Session from there, over the listing that won ticket 642 and the open
// Session that won ticket 643. Switched from the yellow pill (or `-variant B` at launch).
// Answers https://github.com/jake-tucker/shaman/issues/651.
//
//   A · Sheet            a compose sheet, as a mail app's: where it begins as header fields
//                        over a page to write on, the Workspace picker pushed inside the
//                        sheet. The sheet waits for the Server, then the Session opens.
//   B · Screen           a pushed screen shaped as an empty open Session: the Landing as a
//                        view of its own, the open Session's composer at the foot, the picker
//                        a half sheet by Server. Sending turns the same screen into the Session.
//   C · In the listing   nothing opens: the bottom bar becomes a composer with chips for
//                        where it begins. Sending leaves the reader in the listing, and the
//                        Session appears as a row, starting.

import SwiftUI

@main
struct NewSessionPrototypeApp: App {
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
        .onAppear {
            let arguments = UserDefaults.standard
            if let scenario = arguments.string(forKey: "scenario").flatMap(Scenario.init) { store.apply(scenario) }
            if let outlook = arguments.string(forKey: "outlook") { store.outlookWorkspace = outlook }
            store.open(open)
        }
        .task {
            // For driving the prototype without a finger; see run.sh.
            let arguments = UserDefaults.standard
            if arguments.bool(forKey: "compose") {
                try? await Task.sleep(for: .seconds(0.8))
                store.composeAsk.toggle()
            }
            if arguments.bool(forKey: "picker") {
                try? await Task.sleep(for: .seconds(1))
                store.pickerAsk.toggle()
            }
            if arguments.bool(forKey: "send") {
                try? await Task.sleep(for: .seconds(1.2))
                store.sendAsk.toggle()
            }
        }
    }
}

/// An open Session, as ticket 643 settled it.
struct SessionScreen: View {
    @Environment(Store.self) private var store
    let id: String

    var body: some View {
        OpenSession(live: store.live(id), session: store.row(id))
            .toolbar(.hidden, for: .bottomBar)
    }
}

/// Not part of any design: the high-contrast pill that flips variants and what the toy Server
/// does. Drag it out of the way of whatever it covers.
struct PrototypeSwitcher: View {
    @Environment(Store.self) private var store
    @State private var settled: CGSize = .zero
    @GestureState private var drag: CGSize = .zero

    static let variants = [("A", "Sheet"), ("B", "Screen"), ("C", "In the listing")]

    private var index: Int { Self.variants.firstIndex { $0.0 == store.variant } ?? 0 }

    private func cycle(_ step: Int) {
        let count = Self.variants.count
        store.variant = Self.variants[(index + step + count) % count].0
    }

    var body: some View {
        @Bindable var store = store
        VStack {
            HStack(spacing: 10) {
                Button { cycle(-1) } label: { Image(systemName: "chevron.left") }
                Text("\(Self.variants[index].0) · \(Self.variants[index].1)")
                    .font(.footnote.weight(.bold))
                Button { cycle(1) } label: { Image(systemName: "chevron.right") }
                Menu {
                    Picker("When a session is begun", selection: $store.startMode) {
                        ForEach(StartMode.allCases) { Text($0.label).tag($0) }
                    }
                    .pickerStyle(.inline)
                    Section("The servers this phone holds") {
                        Button("Three servers") { store.apply(.threeServers) }
                        Button("One server") { store.apply(.oneServer) }
                        Button("One server, no sessions") { store.apply(.empty) }
                        Button("homelab unreachable, outlook turned to it") {
                            store.apply(.unreachable)
                            store.outlookWorkspace = "lab/relay-infra"
                        }
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
