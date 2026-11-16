// PROTOTYPE — throwaway. Canned Transcripts, one per state the prototype has to show.
// `-open <state>` at launch opens straight into one; see `Canned.states`.

import SwiftUI

private var counter = 0
private func entry(_ body: Body, _ state: RunState = .done) -> Entry {
    counter += 1
    return Entry(id: counter, body: body, state: state)
}
private func turn(
    _ outcome: Outcome?, _ seconds: Int = 0, _ bodies: [Body], running: Body? = nil, stopped: Bool = false
) -> Turn {
    counter += 1
    var entries = bodies.map { body in
        if case .command(_, _, let exit?, _) = body, exit != 0 { return entry(body, .failed) }
        return entry(body)
    }
    if stopped { entries[entries.count - 1].state = .stopped }
    if let running { entries.append(entry(running, .running)) }
    return Turn(id: counter, entries: entries, outcome: outcome, seconds: seconds)
}

private func diff(_ text: String) -> [DiffLine] {
    text.split(separator: "\n", omittingEmptySubsequences: false).map { line in
        DiffLine(mark: line.first ?? " ", text: String(line.dropFirst()))
    }
}
private func file(_ path: String, _ added: Int, _ removed: Int, _ text: String) -> FileDelta {
    FileDelta(path: path, added: added, removed: removed, diff: diff(text))
}
private func read(_ path: String, _ lines: Int) -> Body {
    .tool(name: "Read", subject: path, result: "\(lines) lines")
}

/// An agent Message names its model only where the one that answered differs from the one selected.
private let model: String? = nil

extension Canned {
    /// The state a launch argument names, and the Session that shows it.
    static let states: [(key: String, label: String, session: String)] = [
        ("working", "A turn working", "s2"),
        ("approval", "Approval: a command", "s1"),
        ("fileApproval", "Approval: a file change", "s9"),
        ("questionnaire", "Questionnaire, three questions", "s6"),
        ("settled", "Settled, three turns", "s5"),
        ("failed", "A turn that failed", "s4"),
        ("unreachable", "Monitoring, origin unreachable", "s3"),
    ]

    static func live(_ id: String) -> Live {
        switch id {
        case "s1": approval
        case "s2": working
        case "s3": monitoring
        case "s4": failed
        case "s5": settled
        case "s6": questionnaire
        case "s9": fileApproval
        default: Live(id: id, turns: [
            turn(.worked, 48, [
                .user("Where does this stand?"),
                read("README.md", 120),
                .agent("Nothing is in flight here. The last change landed on `main` and the tree is clean.", model: nil),
            ])
        ])
        }
    }

    /// What plays when the reader sends a Prompt of their own.
    static var reply: [Step] {
        [
            .wait(1.2), .add(.reasoning(title: "Reading the ask", text: "The reader wants a small follow-up. Find the file, change it, check it.")),
            .wait(0.8), .begin(read("src/tui/qr.rs", 74)), .wait(0.9), .finish(.done),
            .begin(.tool(name: "Grep", subject: "\"quiet_zone\"", result: "3 matches in 2 files")), .wait(0.9), .finish(.done),
            .begin(.fileChange([file("src/tui/qr.rs", 4, 1, "@@ -18,7 +18,10 @@\n pub fn draw(invite: &str) -> Vec<Line> {\n-    let code = QrCode::new(invite)?;\n+    let code = QrCode::with_error_correction_level(\n+        invite,\n+        EcLevel::L,\n+    )?;\n     half_blocks(&code)")])),
            .wait(1.0), .finish(.done),
            .begin(.command("cargo check", output: [], exit: 0, seconds: 6)),
            .wait(1.0), .output("    Checking shaman v0.1.0 (~/Projects/shaman)"),
            .wait(2.5), .output("    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.02s"),
            .finish(.done), .wait(1.0),
            .add(.agent("Done. That was a canned turn: the prototype plays the same one for whatever you send.", model: model)),
            .settle(.worked),
        ]
    }

    private static let qrDiff = """
        @@ -41,9 +41,21 @@ impl ServePanel {
             fn invite_lines(&self, invite: &Invite) -> Vec<Line<'static>> {
                 let mut lines = vec![Line::from(invite.to_string())];
        -        lines.push(Line::from("Expires in 10 minutes."));
        +        lines.push(Line::from(format!(
        +            "Expires in {}.",
        +            compact_duration(invite.remaining())
        +        )));
        +        if self.show_qr {
        +            match qr::draw(&invite.to_string(), self.width) {
        +                Ok(code) => lines.extend(code),
        +                Err(qr::TooNarrow { needs }) => lines.push(Line::from(
        +                    format!("Widen the panel to {needs} columns to draw the code."),
        +                )),
        +            }
        +        }
                 lines
             }
        """

    private static var working: Live {
        Live(
            id: "s2",
            turns: [
                turn(.worked, 192, [
                    .user("When /serve mints an invite, offer to draw it as a QR code right in the panel. A phone will scan it to pair."),
                    .reasoning(title: "Finding where an invite is drawn", text: "The serve panel builds its lines in one place. The invite is a few hundred characters of base64url, so the code will be dense; half blocks halve its height."),
                    read("src/tui/serve_panel.rs", 312),
                    .tool(name: "Grep", subject: "\"shaman-v1-\"", result: "6 matches in 4 files"),
                    read("src/serving.rs", 2140),
                    .command("cargo add qrcode --no-default-features", output: ["    Updating crates.io index", "      Adding qrcode v0.14.1 to dependencies", "             Features:", "             - image", "             - svg"], exit: 0, seconds: 2),
                    .agent("The invite fits a version 12 code at low error correction. I'll draw it with half blocks so it stands in 33 rows rather than 65.", model: model),
                    .fileChange([file("src/tui/serve_panel.rs", 15, 1, qrDiff)]),
                    .fileChange([file("src/tui/qr.rs", 61, 0, "@@ -0,0 +1,8 @@\n+//! Draws an Invite as a QR code out of half blocks.\n+\n+pub struct TooNarrow {\n+    pub needs: u16,\n+}\n+\n+pub fn draw(invite: &str, width: u16) -> Result<Vec<Line<'static>>, TooNarrow> {\n+    let code = QrCode::with_error_correction_level(invite, EcLevel::L).expect(\"an invite fits\");")]),
                    .command("cargo check", output: ["    Checking qrcode v0.14.1", "    Checking shaman v0.1.0 (~/Projects/shaman/.worktrees/serve-qr)", "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 14.31s"], exit: 0, seconds: 14),
                    .command("cargo nextest run serve_panel", output: (1...14).map { "        PASS [   0.0\($0 % 9 + 1)s] shaman tui::serve_panel::tests::case_\($0)" } + ["------------", "     Summary [   0.412s] 14 tests run: 14 passed, 1187 skipped"], exit: 0, seconds: 22),
                    .agent("`/serve` now offers **Draw as QR code** beneath a fresh invite.\n\n- The code is drawn from half blocks, so it stands 33 rows tall.\n- A panel narrower than 41 columns says how wide it needs to be instead.\n- The countdown beside it reads from the invite rather than saying ten minutes.\n\nNot done yet: the CLI. It would be:\n\n```sh\nshaman serve invite --qr\n```", model: model),
                ]),
                turn(.stopped, 41, [
                    .user("Add that CLI flag too."),
                    read("src/cli/serve.rs", 188),
                    .fileChange([file("src/cli/serve.rs", 9, 0, "@@ -22,6 +22,9 @@ pub struct InviteArgs {\n     #[arg(long)]\n     pub relay: Option<String>,\n+    /// Draw the invite as a QR code.\n+    #[arg(long)]\n+    pub qr: bool,\n }")]),
                    .command("cargo run -- serve invite --qr", output: ["   Compiling shaman v0.1.0 (~/Projects/shaman/.worktrees/serve-qr)"], exit: nil, seconds: 19),
                ], stopped: true),
                turn(nil, 0, [
                    .user("Sorry, carry on. But give it a quiet zone: my terminal has a light background and the phone couldn't find the corners."),
                    .reasoning(title: "Why a light background breaks it", text: "A scanner needs four modules of quiet around the code. On a dark terminal the background supplies it; on a light one the code must draw its own, inverted."),
                    read("src/tui/qr.rs", 74),
                    .fileChange([file("src/tui/qr.rs", 9, 2, "@@ -31,8 +31,15 @@ fn half_blocks(code: &QrCode) -> Vec<Line<'static>> {\n-    let width = code.width();\n-    let at = |x, y| code[(x, y)] == Color::Dark;\n+    const QUIET: usize = 4;\n+    let width = code.width() + QUIET * 2;\n+    let at = |x: usize, y: usize| {\n+        let inside = (QUIET..width - QUIET).contains(&x)\n+            && (QUIET..width - QUIET).contains(&y);\n+        inside && code[(x - QUIET, y - QUIET)] == Color::Dark\n+    };")]),
                ], running: .command("cargo nextest run qr", output: ["   Compiling shaman v0.1.0 (~/Projects/shaman/.worktrees/serve-qr)"], exit: 0, seconds: 31)),
            ],
            elapsed: 72,
            script: [
                .wait(6), .output("    Finished `test` profile [unoptimized + debuginfo] target(s) in 24.80s"),
                .wait(1), .output("    Starting 5 tests across 1 binary (1196 skipped)"),
                .wait(1), .output("        PASS [   0.004s] shaman tui::qr::tests::draws_a_quiet_zone"),
                .wait(0.6), .output("        PASS [   0.004s] shaman tui::qr::tests::halves_the_rows"),
                .wait(0.6), .output("        PASS [   0.005s] shaman tui::qr::tests::refuses_a_narrow_panel"),
                .wait(0.6), .output("        PASS [   0.006s] shaman tui::qr::tests::round_trips_an_invite"),
                .wait(0.6), .output("        PASS [   0.011s] shaman tui::qr::tests::inverts_on_request"),
                .output("     Summary [   0.031s] 5 tests run: 5 passed, 1196 skipped"),
                .finish(.done), .wait(2),
                .begin(.command("cargo run -- serve invite --qr", output: [], exit: 0, seconds: 8)),
                .wait(4), .output("shaman-v1-eyJ3YXlzIjpbIjE5Mi4xNjguMS4yMDo3NDQxIl0sImtleSI6…"),
                .wait(1), .finish(.done), .wait(2),
                .add(.agent("Both are in. The code now draws four modules of quiet around itself, and `shaman serve invite --qr` prints the same drawing.\n\nI scanned nothing myself: try it from the phone.", model: model)),
                .settle(.worked),
            ])
    }

    private static var approval: Live {
        Live(
            id: "s1",
            turns: [
                turn(nil, 0, [
                    .user("Implement ADR-0078: each side states one version and accepts the other at or above its floor."),
                    read("docs/adr/0078-hold-what-a-peer-and-a-serving-server-say-to-each-other-to-compatibility.md", 96),
                    read("src/protocol.rs", 4410),
                    .tool(name: "Grep", subject: "PROTOCOL_VERSION", result: "23 matches in 9 files"),
                    .fileChange([file("src/protocol.rs", 31, 9, "@@ -12,7 +12,14 @@\n-pub const PROTOCOL_VERSION: u32 = 116;\n+/// The one version this build states.\n+pub const PROTOCOL_VERSION: u32 = 117;\n+/// The oldest version this build still answers.\n+pub const PROTOCOL_FLOOR: u32 = 117;\n+\n+pub fn accepts(theirs: u32) -> bool {\n+    theirs >= PROTOCOL_FLOOR\n+}")]),
                    .fileChange([file("src/serving.rs", 18, 6, "@@ -880,9 +880,12 @@ fn check_peer_version(theirs: u32) -> Result<(), PairingError> {\n-    if theirs != PROTOCOL_VERSION {\n-        return Err(PairingError::VersionMismatch { ours: PROTOCOL_VERSION, theirs });\n-    }\n+    if !protocol::accepts(theirs) {\n+        return Err(PairingError::BelowFloor {\n+            floor: PROTOCOL_FLOOR,\n+            theirs,\n+        });\n+    }\n     Ok(())")]),
                    .command("cargo check", output: ["    Checking shaman v0.1.0 (~/Projects/shaman/.worktrees/peer-version-floor)", "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.77s"], exit: 0, seconds: 19),
                    .agent("The handshake compiles against the floor. Next I want the relay crate's tests, which open loopback sockets.", model: model),
                    .approval(Approval(
                        subject: .command("cargo nextest run -p shaman-relay", cwd: "~/Projects/shaman/.worktrees/peer-version-floor"),
                        reason: "Needs network access outside the sandbox.")),
                ])
            ],
            elapsed: 250,
            allowed: [
                .begin(.command("cargo nextest run -p shaman-relay", output: [], exit: 0, seconds: 9)),
                .wait(2), .output("    Starting 41 tests across 3 binaries"),
                .wait(3), .output("     Summary [   2.906s] 41 tests run: 41 passed, 0 skipped"),
                .finish(.done), .wait(1.5),
                .add(.agent("The relay's 41 tests pass with the floor in place. A peer below it is refused with **below floor**, naming both versions.", model: model)),
                .settle(.worked),
            ],
            denied: [
                .wait(1.5),
                .add(.agent("I have not run the relay's tests. The change compiles; the handshake's own tests are the ones that would catch a mistake here.", model: model)),
                .settle(.worked),
            ])
    }

    private static var fileApproval: Live {
        Live(
            id: "s9",
            turns: [
                turn(nil, 0, [
                    .user("Upgrade Home Assistant to 2026.11 and fix whatever the release notes say breaks."),
                    .tool(name: "WebFetch", subject: "home-assistant.io/blog/2026/11/04/release-202611", result: "Breaking: `platform: template` sensors under `sensor:` are removed."),
                    read("configuration.yaml", 214),
                    .command("ha core check", output: ["Processing... Done.", "Command completed successfully."], exit: 0, seconds: 31),
                    .approval(Approval(
                        subject: .fileChange([
                            file("configuration.yaml", 6, 4, "@@ -88,10 +88,12 @@\n-sensor:\n-  - platform: template\n-    sensors:\n-      attic_humidity_smoothed:\n+template:\n+  - sensor:\n+      - name: Attic humidity smoothed\n+        unique_id: attic_humidity_smoothed\n+        unit_of_measurement: \"%\"\n+        state: >\n           {{ states('sensor.attic_humidity') | float(0) | round(0) }}"),
                            file("automations.yaml", 2, 2, "@@ -14,7 +14,7 @@\n   trigger:\n-    platform: numeric_state\n-    entity_id: sensor.attic_humidity_smoothed\n+    trigger: numeric_state\n+    entity_id: sensor.attic_humidity_smoothed\n     above: 70"),
                        ]),
                        reason: nil)),
                ])
            ],
            elapsed: 740,
            allowed: [
                .begin(.fileChange([file("configuration.yaml", 6, 4, "@@ -88,10 +88,12 @@\n-sensor:\n+template:"), file("automations.yaml", 2, 2, "@@ -14,7 +14,7 @@\n-    platform: numeric_state\n+    trigger: numeric_state")])),
                .wait(1), .finish(.done), .wait(1.5),
                .add(.agent("Both files are changed. Restart Home Assistant to pick them up.", model: model)),
                .settle(.worked),
            ],
            denied: [
                .wait(1.5), .add(.agent("I've left both files alone. The template sensor will stop loading after the upgrade.", model: model)),
                .settle(.worked),
            ])
    }

    private static var questionnaire: Live {
        Live(
            id: "s6",
            turns: [
                turn(nil, 0, [
                    .user("Give the button palette proper tokens. Ask me what you need."),
                    read("tokens/colors.json", 540),
                    .tool(name: "Grep", subject: "\"button-\"", result: "41 matches in 12 files"),
                    .agent("Three things I can't settle from the code.", model: model),
                    .questionnaire(Questionnaire(questions: [
                        Question(id: "palette", title: "Palette", text: "Which palette should buttons draw from?", choices: [
                            .init(label: "Brand", detail: "The six brand hues. Matches the marketing site."),
                            .init(label: "Neutral", detail: "Greys only, with brand kept for links.", recommended: true),
                            .init(label: "Semantic", detail: "Success, warning, danger and info, nothing else."),
                        ]),
                        Question(id: "states", title: "States", text: "Which states get a token of their own?", choices: [
                            .init(label: "Hover", detail: "Pointer devices only."),
                            .init(label: "Pressed", detail: "Shown for the length of a tap."),
                            .init(label: "Disabled", detail: "Today this is 40% opacity over the resting colour."),
                            .init(label: "Focus ring", detail: "Shares the ring token unless given its own."),
                        ], multiple: true),
                        Question(id: "naming", title: "Naming", text: "How should the tokens be named?", choices: [
                            .init(label: "button.primary.bg", detail: "Component first."),
                            .init(label: "color.action.primary", detail: "Role first, shared with links."),
                        ], freeform: true),
                    ])),
                ])
            ],
            elapsed: 96,
            allowed: [
                .wait(1.2),
                .begin(.fileChange([file("tokens/buttons.json", 38, 0, "@@ -0,0 +1,6 @@\n+{\n+  \"button\": {\n+    \"primary\": {\n+      \"bg\": { \"value\": \"{brand.indigo.600}\" },\n+      \"bg-pressed\": { \"value\": \"{brand.indigo.700}\" },\n+      \"bg-disabled\": { \"value\": \"{neutral.300}\" }")])),
                .wait(1.2), .finish(.done), .wait(1.5),
                .add(.agent("The tokens are in `tokens/buttons.json`, named and scoped as you answered.", model: model)),
                .settle(.worked),
            ],
            denied: [
                .wait(1.2),
                .add(.agent("Without those answers I've changed nothing. Tell me the palette, the states and the naming whenever you like.", model: model)),
                .settle(.worked),
            ])
    }

    private static var settled: Live {
        Live(id: "s5", turns: [
            turn(.worked, 134, [
                .user("My zsh takes over a second to start. Find out why."),
                .command("for i in 1 2 3; do /usr/bin/time zsh -i -c exit; done", output: ["        1.21 real         0.71 user         0.44 sys", "        1.18 real         0.70 user         0.43 sys", "        1.19 real         0.70 user         0.44 sys"], exit: 0, seconds: 4),
                read(".zshrc", 148),
                .command("zsh -i -c 'zmodload zsh/zprof; source ~/.zshrc; zprof' | head -12", output: ["num  calls                time                       self            name", "-----------------------------------------------------------------------------------", " 1)    1         612.40   612.40   51.3%    612.40   612.40   51.3%  nvm_auto", " 2)    2         301.77   150.89   25.3%    301.77   150.89   25.3%  compinit", " 3)    1         122.05   122.05   10.2%    122.05   122.05   10.2%  _pyenv_init"], exit: 0, seconds: 3),
                .agent("Three things account for nearly all of it:\n\n| Cost | What | Share |\n| --- | --- | --- |\n| 612 ms | `nvm` loading at startup | 51% |\n| 302 ms | `compinit` running twice | 25% |\n| 122 ms | `pyenv init` | 10% |\n\n`nvm` can load the first time you call `node`. `compinit` runs once in `.zshrc` and again from the Homebrew completions snippet.", model: model),
            ]),
            turn(.worked, 251, [
                .user("Fix the first two. Leave pyenv."),
                .fileChange([file(".zshrc", 14, 6, "@@ -31,9 +31,17 @@\n-export NVM_DIR=\"$HOME/.nvm\"\n-[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\"\n+export NVM_DIR=\"$HOME/.nvm\"\n+# Load nvm the first time something asks for node.\n+for cmd in nvm node npm npx; do\n+  eval \"$cmd() { unset -f nvm node npm npx; . \\\"$NVM_DIR/nvm.sh\\\"; $cmd \\\"\\$@\\\"; }\"\n+done")]),
                .fileChange([file(".zsh/completions.zsh", 1, 3, "@@ -4,5 +4,3 @@\n fpath=(/opt/homebrew/share/zsh/site-functions $fpath)\n-autoload -Uz compinit\n-compinit\n+# compinit runs once, from .zshrc.")]),
                .command("for i in 1 2 3; do /usr/bin/time zsh -i -c exit; done", output: ["        0.31 real         0.19 user         0.11 sys", "        0.29 real         0.18 user         0.10 sys", "        0.29 real         0.18 user         0.10 sys"], exit: 0, seconds: 2),
                .agent("Startup is down from 1.19 s to 0.29 s. `node`, `npm` and `npx` load `nvm` the first time one is called, which costs that one call about 600 ms.", model: model),
            ]),
            turn(.worked, 38, [
                .user("Commit it."),
                .command("git add -A && git commit -m 'perf(zsh): load nvm lazily and run compinit once'", output: ["[main 4be1a02] perf(zsh): load nvm lazily and run compinit once", " 2 files changed, 15 insertions(+), 9 deletions(-)"], exit: 0, seconds: 1),
                .agent("Committed as `4be1a02` on `main`.", model: model),
            ]),
        ])
    }

    private static var failed: Live {
        Live(id: "s4", turns: [
            turn(.worked, 310, [
                .user("Port the invoice rounding fix from v1 to v2."),
                read("billing/v1/rounding.py", 88),
                read("billing/v2/money.py", 240),
                .fileChange([file("billing/v2/money.py", 12, 4, "@@ -57,8 +57,12 @@ def round_line(amount: Decimal) -> Decimal:\n-    return amount.quantize(CENT)\n+    # Bankers' rounding drifts a cent on .5 lines; v1 rounds half up.\n+    return amount.quantize(CENT, rounding=ROUND_HALF_UP)")]),
                .command("pytest billing/v2 -q", output: ["........................F...", "=================================== FAILURES ===================================", "_____________________ test_credit_note_rounds_toward_zero ______________________", "    def test_credit_note_rounds_toward_zero():", ">       assert round_line(Decimal(\"-0.125\")) == Decimal(\"-0.12\")", "E       AssertionError: assert Decimal('-0.13') == Decimal('-0.12')", "billing/v2/test_money.py:91: AssertionError", "=========================== short test summary info ============================", "FAILED billing/v2/test_money.py::test_credit_note_rounds_toward_zero", "1 failed, 27 passed in 3.41s"], exit: 1, seconds: 4),
                .agent("The fix is ported, and it breaks one test: credit notes are expected to round toward zero, which v1 never handled. Should negative lines keep that rule?", model: model),
            ]),
            turn(.failed, 12, [
                .user("Yes, keep it for negatives."),
                .tool(name: "Grep", subject: "round_line", result: "9 matches in 4 files"),
                .error("The provider stopped answering: 529 overloaded. Nothing was lost. Send a prompt to carry on."),
            ]),
        ])
    }

    private static var monitoring: Live {
        Live(
            id: "s3",
            turns: [
                turn(.worked, 420, [
                    .user("The nightly backup skipped Tuesday. Find out why, and watch tonight's run."),
                    .command("journalctl -u nightly-backup --since '3 days ago' | tail -20", output: ["Nov 10 02:00:01 homelab systemd[1]: Starting nightly-backup.service", "Nov 10 02:00:01 homelab restic[41220]: Fatal: unable to create lock in backend: repository is already locked by PID 38811", "Nov 10 02:00:01 homelab systemd[1]: nightly-backup.service: Failed with result 'exit-code'."], exit: 0, seconds: 1),
                    .subagent(name: "Explorer", task: "find what held the restic lock on Monday night"),
                    .command("restic unlock --remove-all", output: ["successfully removed 1 locks"], exit: 0, seconds: 2),
                    .agent("Monday's prune was still holding the repository lock when Tuesday's backup began, so it gave up. I removed the stale lock.\n\nI'm watching `nightly-backup.timer` and will read the journal when it fires at 02:00.", model: model),
                ])
            ],
            elapsed: 2280,
            watch: "nightly-backup.timer")
    }
}
