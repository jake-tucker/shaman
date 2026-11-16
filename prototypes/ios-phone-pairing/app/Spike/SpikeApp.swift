import SwiftUI
import UIKit

@main
struct SpikeApp: App {
    @State private var spike = Spike()

    var body: some Scene {
        WindowGroup {
            Console(spike: spike)
        }
    }
}

/// A console and nothing more: steps to run, and the log of what each found.
struct Console: View {
    @Bindable var spike: Spike
    @State private var step = ""

    private let steps = [
        "identity", "preview", "redeem:direct", "redeem:relay", "connect:direct", "connect:relay", "probe",
        "standing", "login", "nap", "stop-streams", "drop-connections", "reset",
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("Invite", text: $spike.invite)
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("invite")
                Button("Paste") { spike.invite = UIPasteboard.general.string ?? "" }
            }
            ScrollView(.horizontal) {
                HStack {
                    ForEach(steps, id: \.self) { step in
                        Button(step) { spike.run(step) }
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier(step)
                    }
                }
            }
            HStack {
                TextField("step[:argument], comma separated", text: $step)
                    .textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("step")
                Button("Run") { spike.run(step) }
            }
            if let code = spike.loginCode {
                Text("Login code \(code) (copied)")
                    .font(.headline)
            }
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(spike.lines.enumerated()), id: \.offset) { index, line in
                            Text(line)
                                .font(.system(size: 10, design: .monospaced))
                                .id(index)
                        }
                    }
                }
                .onChange(of: spike.lines.count) { _, count in
                    reader.scrollTo(count - 1, anchor: .bottom)
                }
            }
        }
        .padding(8)
    }
}
