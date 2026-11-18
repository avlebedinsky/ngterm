import XCTest

/// Settings > Logs after a troubleshooting session on a real machine, through the app's real core (mobile/android's
/// `AppLogsPrivacyE2eTest`).
///
/// From a fresh install a person imports their key with its passphrase, adds the machine, trusts its host key and
/// opens a named session. They type a command at its shell, which prints what they typed, start Claude Code there
/// and send it a message from the chat. Settings > Logs then tells what the core did (the connect, the new session,
/// the attach, the input it sent, the agent it found, the chat message's action) and what the app did, while none of
/// what was typed, printed, sent or imported, nor the session's name, a key line or a fingerprint, is in the lines
/// shown, in what Copy takes, in the file Share writes or in the log's files. What is typed reaches the core a key at
/// a time, so the log gains fewer lines over the typing than the typed word has letters, and a line about input
/// holds how it was delivered, its kind and its pane, nothing else. Clear logs asks first.
///
/// The machine is a private `ngterm-headless` behind a private, unprivileged `sshd` on the Mac's 127.0.0.1, with the
/// mobile e2e's stand-in Claude Code on its PATH as `e2e-claude`: `scripts/test-ios-e2e.sh` starts it, installs the
/// app anew and hands the machine to the test through the test runner's environment. Without it the test is skipped.
/// The log is the app's own, in its data container on the simulator, which the test runner reads beside its own.
internal final class LogsOnAMachineTests: XCTestCase {
    /// The machine `scripts/test-ios-e2e.sh` started, as the test runner's environment names it.
    private struct Machine {
        let host: String
        let port: String
        let user: String
        let key: String
        let passphrase: String
        let fingerprint: String

        static func fromEnvironment() throws -> Self {
            let environment = ProcessInfo.processInfo.environment
            let names = ["HOST", "PORT", "USER", "KEY", "PASSPHRASE", "FINGERPRINT"].map { "IOS_E2E_\($0)" }
            let values = names.compactMap { environment[$0] }.filter { !$0.isEmpty }
            guard values.count == names.count else {
                throw XCTSkip("no machine: run it through scripts/test-ios-e2e.sh")
            }
            return Self(
                host: values[0],
                port: values[1],
                user: values[2],
                key: try String(contentsOfFile: values[3], encoding: .utf8),
                passphrase: values[4],
                fingerprint: values[5]
            )
        }
    }

    // where the app keeps its log and the files Share writes: its data container, a sibling of the test runner's
    private struct Place {
        let log: URL
        let export: URL

        // the log as the app keeps it: its file and the one it rotated to
        var files: String {
            LogsPage.files(under: log).joined(separator: "\n")
        }

        var lineCount: Int {
            files.split(whereSeparator: \.isNewline).count
        }

        // the file the last Share wrote
        var exported: String? {
            LogsPage.files(under: export).last
        }

        static func ofTheApp() -> Self? {
            let containers = URL(filePath: NSHomeDirectory()).deletingLastPathComponent()
            let manager = FileManager.default
            let all = (try? manager.contentsOfDirectory(at: containers, includingPropertiesForKeys: nil)) ?? []
            let app = all.first { container in
                let metadata = container.appending(path: ".com.apple.mobile_container_manager.metadata.plist")
                let plist = (try? Data(contentsOf: metadata)).flatMap { data in
                    try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
                }
                let identifier = plist?["MCMMetadataIdentifier"] as? String
                return identifier == appIdentifier
            }
            return app.map { container in
                Self(
                    log: container.appending(path: "Library/Application Support/logs", directoryHint: .isDirectory),
                    export: container.appending(path: "tmp/logs-export", directoryHint: .isDirectory)
                )
            }
        }
    }

    private static let connectWait: TimeInterval = 60
    private static let answerWait: TimeInterval = 20
    private static let appIdentifier = "com.nickalie.ngterm.ios"
    private static let label = "qa-logs-mac"
    private static let session = "qa-sess-mk5n"
    // typed at the shell: the shell prints `typed` back, so it is in the terminal twice
    private static let typed = "qa-typed-mk9x"
    private static let command = "echo \(typed)"
    private static let agent = "e2e-claude"
    private static let chatMarker = "qa-chat-mk3p"
    private static let chatText = "\(chatMarker) please look at the build"
    // what the log tells of the session: the core's lines, then the app's
    private static let told = [
        "host key trusted and pinned",
        "machine connected",
        "new session",
        "attached",
        "input delivery=sent",
        "pane agent announced",
        "action=prompt",
        "chat events",
        "destination=session",
        "settings page=logs",
    ]
    // a line about input in the log's file, whole: its time, its level, its tag, then its delivery, kind and pane
    private static let inputLine = try? NSRegularExpression(
        pattern: #"^\S+ [IWE] input: input delivery=[a-z-]+ kind=(key|text|paste) pane=[0-9A-Za-z-]+$"#
    )

    // a form's field, its old text (a default port) replaced by `text`. An iPad's number pad (the port's) is a
    // popover: a tap outside it only closes it, so the next field is tapped again until it has the keyboard
    @MainActor
    private static func fill(_ identifier: String, with text: String, in app: XCUIApplication) {
        let field = app.textFields[identifier]
        FakeApp.expect(field, identifier)
        var focused = false
        for _ in 0..<LogsPage.tries where !focused {
            field.tap()
            let hasKeyboard = NSPredicate(format: "hasKeyboardFocus == true")
            let focus = XCTNSPredicateExpectation(predicate: hasKeyboard, object: field)
            focused = XCTWaiter.wait(for: [focus], timeout: LogsPage.offerWait) == .completed
        }
        XCTAssertTrue(focused, "\(identifier) never took the keyboard")
        let old = field.value as? String ?? ""
        let placeholder = field.placeholderValue ?? ""
        if !old.isEmpty, old != placeholder {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: old.count))
        }
        field.typeText(text)
    }

    // a line about input: how it was delivered, its kind and its pane, and there is one
    private static func assertInputLinesHoldNoText(_ what: String, _ log: String) {
        let input = log.split(whereSeparator: \.isNewline).map(String.init).filter { $0.contains(" input: ") }
        XCTAssertFalse(input.isEmpty, "\(what) tell nothing of the input:\n\(log)")
        for line in input {
            let whole = NSRange(line.startIndex..., in: line)
            XCTAssertNotNil(
                inputLine?.firstMatch(in: line, range: whole),
                "\(what) hold a line about input with more than its delivery:\n\(line)"
            )
        }
    }

    @MainActor
    internal func testOnARealMachineTheLogTellsWhatTheCoreDidAndHoldsNothingTypedSentOrImported() throws {
        let machine = try Machine.fromEnvironment()
        let app = FakeApp.launchOnTheRealCore(for: self)
        let place = try XCTUnwrap(Place.ofTheApp(), "the app's data container is not beside the test runner's")
        signIn(with: machine, in: app)
        addMachine(machine, in: app)
        openANewSession(in: app)
        typeAtTheShell(in: app, logIn: place)
        startTheAgentAndSend(in: app)
        let secrets = [
            "what was typed at the shell": Self.typed,
            "the chat message": Self.chatMarker,
            "the session's name": Self.session,
            "the passphrase": machine.passphrase,
            "the key's fingerprint": machine.fingerprint,
        ]

        LogsPage.open(in: app)
        LogsPage.expectNewest("settings page=logs", in: app)
        let shown = LogsPage.allShownLines(in: app).joined(separator: "\n")
        attach("the lines shown", shown)
        LogsPage.assertTold("the lines shown", shown, Self.told)
        LogsPage.assertClean("the lines shown", shown, secrets: secrets, key: machine.key)

        app.buttons["logs-copy"].tap()
        FakeApp.expect(app.buttons["Copied"], "Copied")
        let copied = LogsPage.pasted(in: app)
        LogsPage.assertTold("the copied lines", copied, Self.told)
        LogsPage.assertClean("the copied lines", copied, secrets: secrets, key: machine.key)
        LogsPage.enter(in: app)

        LogsPage.shareAndClose(in: app)
        let written = NSPredicate { _, _ in place.exported != nil }
        let came = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: written, object: nil)],
            timeout: LogsPage.wait
        )
        XCTAssertEqual(came, .completed, "Share wrote no file")
        let whole = place.exported ?? ""
        attach("the shared file", whole)
        LogsPage.assertTold("the shared file", whole, Self.told)
        LogsPage.assertClean("the shared file", whole, secrets: secrets, key: machine.key)
        Self.assertInputLinesHoldNoText("the shared file", whole)
        let files = place.files
        LogsPage.assertTold("the log's files", files, Self.told)
        LogsPage.assertClean("the log's files", files, secrets: secrets, key: machine.key)
        Self.assertInputLinesHoldNoText("the log's files", files)

        LogsPage.clearAsksFirst(in: app)
        shot("cleared", app)
    }

    // the welcome's Import a key, with the machine's key and its passphrase, to its public half, then Done
    @MainActor
    private func signIn(with machine: Machine, in app: XCUIApplication) {
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(machine.key.trimmingCharacters(in: .whitespacesAndNewlines))
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(machine.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(app.descendants(matching: .any)["public-key-page"].firstMatch, "the imported key's public half")
        // a guard: the system's offer to keep the passphrase in Passwords (gone since the import empties its field)
        // came at no fixed time, and took the next tap
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        let closed = LogsPage.tap(done, until: { !page.exists }, in: app, "the public key page stayed after Done")
        LogsPage.closeLateOffer(unless: closed, in: app)
    }

    // the machine through the form, its host key trusted, connected
    @MainActor
    private func addMachine(_ machine: Machine, in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        Self.fill("machine-label", with: Self.label, in: app)
        Self.fill("machine-host", with: machine.host, in: app)
        Self.fill("machine-port", with: machine.port, in: app)
        Self.fill("machine-user", with: machine.user, in: app)
        app.buttons["machine-save"].tap()
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "the host key sheet")
        trust.tap()
        let row = NSPredicate(format: "identifier == %@ AND label CONTAINS 'Connected'", "machine-row-\(Self.label)")
        let connected = app.descendants(matching: .any).matching(row).firstMatch
        XCTAssertTrue(connected.waitForExistence(timeout: Self.connectWait), "the machine never read Connected")
    }

    // New named session on the machine, landing in its terminal; the first attach asks about notifying: not now
    @MainActor
    private func openANewSession(in app: XCUIApplication) {
        app.descendants(matching: .any)["machine-row-\(Self.label)"].firstMatch.tap()
        let new = app.buttons["new-session"]
        FakeApp.expect(new, "New session")
        new.press(forDuration: 1)
        let named = app.buttons["new-named-session"]
        FakeApp.expect(named, "New named session in the button's menu")
        named.tap()
        let name = app.textFields["session-name"]
        FakeApp.expect(name, "the New session sheet")
        name.tap()
        name.typeText(Self.session)
        app.buttons["create-session"].tap()
        let later = app.buttons["notifications-later"]
        if later.waitForExistence(timeout: LogsPage.offerWait) {
            later.tap()
        }
        FakeApp.expect(app.descendants(matching: .any)["session-title"].firstMatch, "the new session's view")
    }

    // a command at the shell, which prints what it was given; the log gains fewer lines over the typing than the
    // command has letters, so no line was written for a key
    @MainActor
    private func typeAtTheShell(in app: XCUIApplication, logIn place: Place) {
        let terminal = app.textViews.firstMatch
        FakeApp.expect(terminal, "the session's terminal")
        expectTerminal("a prompt", in: app) { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        terminal.tap()
        let before = place.lineCount
        app.typeText(Self.command + "\n")
        expectTerminal("the shell's echo", in: app) { text in text.components(separatedBy: Self.typed).count > 2 }
        let gained = place.lineCount - before
        XCTAssertLessThan(gained, Self.command.count, "the log gained \(gained) lines over \(Self.command.count) keys")
        shot("terminal", app)
    }

    // Claude Code started at the shell: the pane turns to its chat, where a message goes and its answer comes
    @MainActor
    private func startTheAgentAndSend(in app: XCUIApplication) {
        app.typeText(Self.agent + "\n")
        let prompt = app.descendants(matching: .any)["chat-prompt"].firstMatch
        if !prompt.waitForExistence(timeout: Self.answerWait) {
            let switcher = app.buttons["session-view-switch"]
            FakeApp.expect(switcher, "the chat, or the switch to it")
            switcher.tap()
        }
        FakeApp.expect(prompt, "the chat's prompt field")
        prompt.tap()
        prompt.typeText(Self.chatText)
        app.buttons["chat-send"].tap()
        FakeApp.expect(FakeApp.element(containing: Self.chatMarker, in: app), "the message in the chat")
        shot("chat", app)
        // a phone's session is a page of its own over the machines: back to them
        for _ in 0..<LogsPage.tries where !app.buttons["add-machine"].exists {
            app.buttons["BackButton"].firstMatch.tap()
        }
    }

    @MainActor
    private func expectTerminal(_ what: String, in app: XCUIApplication, shows: @escaping (String) -> Bool) {
        let terminal = app.textViews.firstMatch
        let shown = NSPredicate { _, _ in shows((terminal.value as? String) ?? "") }
        let came = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: shown, object: nil)],
            timeout: Self.connectWait
        )
        XCTAssertEqual(came, .completed, "the terminal never showed \(what): \(terminal.value ?? "")")
    }

    @MainActor
    private func shot(_ name: String, _ app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func attach(_ name: String, _ text: String) {
        let attachment = XCTAttachment(string: text)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
