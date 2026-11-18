import XCTest

/// Settings > Logs on the fake core, with the core's own log in a directory the test reads (mobile/android's
/// `AppLogsE2eTest`, and `AppLogsPrivacyE2eTest` for the app's own lines).
///
/// The page shows its own newest line as it opens, copies the lines shown, shares the log through the system's share
/// sheet and clears it after asking. A troubleshooting session (a key imported with its passphrase, a chat message
/// sent to the agent, text typed into the terminal) is then told in the log, newest first, while none of what was
/// typed, sent or imported is in the lines shown, in what Copy takes, in the file Share writes or in the log's files.
///
/// This holds the app's own lines, and runs wherever the UI plan runs. The real core imports the key, and the session
/// runs on the fake core, which writes no line: the core's own lines about the connect, the chat message and the typed
/// text are not in the log here. ``LogsOnAMachineTests`` holds them: the same troubleshooting session through the real
/// core on a real machine (`scripts/test-ios-e2e.sh`).
internal final class LogsTests: XCTestCase {
    // what must be in no form of the log
    private static let typed = "qa-typed-mk9x"
    private static let chatMarker = "qa-chat-mk3p"
    private static let chatText = "\(chatMarker) please look at the build"
    private static let passphrase = TestKey.passphrase
    private static let key = TestKey.privateKey
    private static let fingerprint = "b2HWDf9FyXwRq12b914kfeYPm0qpzJF8WiUKuHlr8Ag"
    // what the log tells of the session, oldest first
    private static let told = [
        "destination=settings",
        "settings page=import",
        "settings page=key",
        "destination=session",
        "action result=sent",
        "settings page=logs",
    ]

    @MainActor
    private static func directory() -> URL {
        FileManager.default.temporaryDirectory.appending(path: "ngterm-logs-\(UUID().uuidString)")
    }

    // the log as the app keeps it: its file and the one it rotated to
    private static func logFiles(in directory: URL) -> String {
        LogsPage.files(under: directory.appending(path: "log")).joined(separator: "\n")
    }

    private static func lineCount(in directory: URL) -> Int {
        logFiles(in: directory).split(whereSeparator: \.isNewline).count
    }

    // the file the last Share wrote
    private static func exported(in directory: URL) -> String? {
        LogsPage.files(under: directory.appending(path: "export")).last
    }

    // nothing typed, sent or imported: no typed text, no chat text, no passphrase, no key line, no fingerprint
    private static func assertClean(_ what: String, _ log: String) {
        let secrets = [
            "what was typed in the terminal": typed,
            "the chat message": chatMarker,
            "the passphrase": passphrase,
            "the key's fingerprint": fingerprint,
        ]
        LogsPage.assertClean(what, log, secrets: secrets, key: key)
    }

    @MainActor
    internal func testLogsShowsItsOwnLineCopiesSharesAndClears() throws {
        let app = FakeApp.launch(withKey: true, for: self)
        LogsPage.open(in: app)
        LogsPage.expectNewest("settings page=logs", in: app)
        // on an iPad's sheet the list runs on past the sheet's foot
        try FakeApp.auditForm(app, page: app.collectionViews["logs-list"])

        app.buttons["logs-copy"].tap()
        FakeApp.expect(app.buttons["Copied"], "Copied")
        LogsPage.expectNewest("copied the shown lines", in: app)
        let copied = LogsPage.pasted(in: app)
        XCTAssertTrue(copied.contains(" I nav: settings page=logs"), "Copy took no line of the log:\n\(copied)")
        LogsPage.enter(in: app)
        LogsPage.expectNewest("settings page=logs", in: app)

        LogsPage.shareAndClose(in: app)

        LogsPage.clearAsksFirst(in: app)
    }

    @MainActor
    internal func testTheLogTellsWhatHappenedAndHoldsNothingTypedSentOrImported() {
        let directory = Self.directory()
        addTeardownBlock {
            try? FileManager.default.removeItem(at: directory)
        }
        let app = FakeApp.launch(
            withKey: true,
            arguments: ["-ngterm-fake-session", "-ngterm-fake-chat", "-ngterm-fake-log", directory.path],
            for: self
        )
        importKey(in: app)
        sendAndType(in: app, logIn: directory)

        LogsPage.open(in: app)
        LogsPage.expectNewest("settings page=logs", in: app)
        let shown = LogsPage.shownLines(in: app).joined(separator: "\n")
        Self.assertClean("the lines shown", shown)
        XCTAssertTrue(shown.contains("settings page=logs"), "the page shows no line of its own:\n\(shown)")

        app.buttons["logs-copy"].tap()
        FakeApp.expect(app.buttons["Copied"], "Copied")
        let copied = LogsPage.pasted(in: app)
        LogsPage.assertTold("the copied lines", copied, Self.told)
        Self.assertClean("the copied lines", copied)
        LogsPage.enter(in: app)

        app.buttons["logs-share"].tap()
        let written = NSPredicate { _, _ in Self.exported(in: directory) != nil }
        let came = XCTWaiter.wait(
            for: [XCTNSPredicateExpectation(predicate: written, object: nil)],
            timeout: LogsPage.wait
        )
        XCTAssertEqual(came, .completed, "Share wrote no file")
        let whole = Self.exported(in: directory) ?? ""
        LogsPage.assertTold("the shared file", whole, Self.told)
        Self.assertClean("the shared file", whole)
        let files = Self.logFiles(in: directory)
        LogsPage.assertTold("the log's files", files, Self.told)
        Self.assertClean("the log's files", files)
    }

    // Settings > Keys > Import a key, with the test's key and passphrase, to its public half, then Settings closed
    @MainActor
    private func importKey(in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["settings"].tap()
        let start = app.buttons["import-key"]
        LogsPage.reveal(start, in: app)
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(Self.key)
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(Self.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(app.descendants(matching: .any)["public-key-page"].firstMatch, "the imported key's public half")
        // a guard: the system's offer to keep the passphrase in Passwords (gone since the import empties its field)
        // came at no fixed time, and took the next tap
        let done = app.buttons["settings-done"]
        let back = app.buttons["BackButton"].firstMatch
        let closedOnBack = LogsPage.tap(back, until: { done.exists }, in: app, "no Settings' Done")
        let settings = app.navigationBars["Settings"]
        let closedOnDone = LogsPage.tap(done, until: { !settings.exists }, in: app, "Settings stayed")
        LogsPage.closeLateOffer(unless: closedOnBack || closedOnDone, in: app)
    }

    // a message to `work`'s agent, then text typed into its terminal; the log gains fewer lines over the typing than
    // the text has letters. The fake core writes no line, so this holds the app's own lines: a line for each key fails
    @MainActor
    private func sendAndType(in app: XCUIApplication, logIn directory: URL) {
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        let machine = app.descendants(matching: .any)["machine-row-split"].firstMatch
        let row = app.descendants(matching: .any)["session-work"].firstMatch
        LogsPage.tap(machine, until: { row.exists }, in: app, "no row for the session work")
        row.tap()
        let prompt = app.descendants(matching: .any)["chat-prompt"].firstMatch
        FakeApp.expect(prompt, "the prompt field")
        prompt.tap()
        prompt.typeText(Self.chatText)
        app.buttons["chat-send"].tap()
        FakeApp.expect(FakeApp.element(containing: "Done: \(Self.chatText).", in: app), "the agent's reply")

        app.buttons["session-view-switch"].tap()
        let terminal = app.textViews.matching(NSPredicate(format: "value CONTAINS %@", "P1 ready")).firstMatch
        FakeApp.expect(terminal, "the first pane's terminal")
        terminal.tap()
        let before = Self.lineCount(in: directory)
        app.typeText(Self.typed)
        let gained = Self.lineCount(in: directory) - before
        XCTAssertLessThan(gained, Self.typed.count, "the log gained \(gained) lines over \(Self.typed.count) keys")
        // a phone's session is a page of its own over the machines: back to them
        for _ in 0..<LogsPage.tries where !app.buttons["add-machine"].exists {
            app.buttons["BackButton"].firstMatch.tap()
        }
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
