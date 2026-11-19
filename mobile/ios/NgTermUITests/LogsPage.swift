import XCTest

/// Settings > Logs as the UI tests drive and read it, on the fake core (``LogsTests``) and on a real machine
/// (``LogsOnAMachineTests``).
internal enum LogsPage {
    /// How long a screen or a line may take to come.
    internal static let wait: TimeInterval = 15
    /// How long an offer of the system's (keep a password, notify) may take to come.
    internal static let offerWait: TimeInterval = 5
    /// How often a tap that can reach nothing is tried again.
    internal static let tries = 3
    // the button that closes the system's offer to keep a password
    private static let offerDismiss = "Not Now"
    private static let swipes = 12
    // Settings' page swiped down to its foot, then back up
    private static let revealSwipes = swipes + swipes
    // how often the list is swiped through at most, and after how many swipes that bring no new line it is at its foot
    private static let listSwipes = 60
    private static let stillSwipes = 2
    // how long a press on a field is held to bring up its edit menu
    private static let menuPress: TimeInterval = 1.2
    // where a tap outside an iPad's popover lands: near the screen's left edge and foot
    private static let outsideX: CGFloat = 0.03
    private static let outsideY: CGFloat = 0.9
    private static let outsidePopover = CGVector(dx: outsideX, dy: outsideY)
    // a log Cancel cleared shows its one line, saying so: a kept log shows more
    private static let clearedLines = 1
    // a line of a private key's body, which is in no form of the log: longer than this
    private static let keyLine = 16

    /// The log's lines as the page shows them (each row's words), top first, from one snapshot of the page: elements
    /// bound by index are found again one at a time, and a row that goes in between (Clear) fails the test.
    @MainActor
    internal static func shownLines(in app: XCUIApplication) -> [String] {
        guard let page = try? app.snapshot() else {
            return []
        }
        return lines(under: page)
    }

    /// Every line the page shows, top first, scrolled through.
    ///
    /// The list makes rows only as they come on screen. The page is then back at its top. A line that two rows show
    /// alike is taken once.
    @MainActor
    internal static func allShownLines(in app: XCUIApplication) -> [String] {
        let list = app.collectionViews["logs-list"]
        var seen: [String] = []
        var still = 0
        for _ in 0..<listSwipes where still < stillSwipes {
            let new = shownLines(in: app).filter { !seen.contains($0) }
            seen += new
            still = new.isEmpty ? still + 1 : 0
            list.swipeUp()
        }
        let top = app.buttons["logs-copy"]
        for _ in 0..<listSwipes where !(top.exists && top.isHittable) {
            list.swipeDown()
        }
        return seen
    }

    @MainActor
    private static func lines(under node: any XCUIElementSnapshot) -> [String] {
        node.identifier == "log-line" ? [node.label] : node.children.flatMap { lines(under: $0) }
    }

    /// Waits until the page's top line holds `text`.
    @MainActor
    internal static func expectNewest(_ text: String, in app: XCUIApplication) {
        ScreenRead.expect("the newest line is not \"\(text)\"", lasting: wait) {
            // a snapshot that fails shows no lines: it tells nothing
            guard let page = try? app.snapshot() else {
                return .failed
            }
            return ScreenRead.Answer(lines(under: page).first?.contains(text) == true)
        }
    }

    /// Taps `control` until `isDone`, and closes the system's offer to keep a password whenever it shows.
    ///
    /// Before Import a key emptied its passphrase field as the import starts, the system offered to keep the passphrase
    /// in Passwords after the import (``KeyPassphraseOfferTests`` now fails if it does). The close stays as a guard:
    /// the offer came at no fixed time (on a loaded simulator 11 s after the import, over Settings), and a tap while
    /// it shows or closes reaches only the offer. The tap is then tried again, while `control` is on screen.
    /// `isDone` reads the app on each poll, and only a read that completed tells whether it holds: a tap is tried
    /// again only when one told it does not. An offer that shows when `isDone` holds is closed before the return,
    /// because it takes the next tap. Returns true when it closed an offer.
    @MainActor
    @discardableResult
    internal static func tap(
        _ control: XCUIElement,
        until isDone: @escaping () -> ScreenRead.Answer,
        in app: XCUIApplication,
        _ what: String
    ) -> Bool {
        let offer = app.buttons[offerDismiss]
        var closed = false
        for _ in 0..<tries where ScreenRead.now(isDone) == .doesNotHold {
            closed = closeOffer(in: app, within: 0) || closed
            // a control that went off screen did its work: wait for it
            if ScreenRead.now(isDone) == .doesNotHold, control.exists, control.isHittable {
                control.tap()
            }
            ScreenRead.step(lasting: wait).poll {
                switch isDone() {
                case .holds:
                    .holds

                case .failed:
                    .failed

                case .doesNotHold:
                    ScreenRead.isShown(offer)
                }
            }
        }
        closed = closeOffer(in: app, within: 0) || closed
        let done = ScreenRead.now(isDone)
        XCTAssertEqual(done, .holds, done == .failed ? "\(what): no read completed" : what)
        return closed
    }

    /// Closes the system's offer to keep a password, if it comes until `timeout` ends.
    ///
    /// Returns true when it closed an offer.
    @MainActor
    @discardableResult
    private static func closeOffer(in app: XCUIApplication, within timeout: TimeInterval) -> Bool {
        let offer = app.buttons[offerDismiss]
        guard offer.exists || (timeout > 0 && offer.waitForExistence(timeout: timeout)) else {
            return false
        }
        // a tap on Not Now while the offer comes in reaches nothing: tap it again while the offer stays
        for _ in 0..<tries where offer.exists {
            offer.tap()
            _ = offer.waitForNonExistence(timeout: offerWait)
        }
        XCTAssertFalse(offer.exists, "the offer to keep the passphrase stayed after \(tries) taps on \(offerDismiss)")
        return true
    }

    /// Closes the offer to keep the imported key's passphrase if it comes after the import's last tap (a guard, as
    /// for ``tap(_:until:in:_:)``).
    ///
    /// The system made the offer one time for each import. When the import's taps closed no offer, it can still
    /// come, and take a tap of the steps after the import: wait for it until `wait` ends.
    @MainActor
    internal static func closeLateOffer(unless closed: Bool, in app: XCUIApplication) {
        if !closed {
            closeOffer(in: app, within: wait)
        }
    }

    /// Settings open, then Logs.
    @MainActor
    internal static func open(in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        enter(in: app)
    }

    /// Settings' Logs, from Settings' own page.
    @MainActor
    internal static func enter(in app: XCUIApplication) {
        let row = app.buttons["settings-logs"]
        reveal(row, in: app)
        row.tap()
        FakeApp.expect(app.navigationBars["Logs"], "Logs")
        FakeApp.expect(FakeApp.element(containing: "It never holds what you type", in: app), "what the log never holds")
    }

    /// Scrolls Settings' page to `element`: down to its foot, then back up.
    @MainActor
    internal static func reveal(_ element: XCUIElement, in app: XCUIApplication) {
        let form = app.collectionViews["settings-form"]
        for swipe in 0..<revealSwipes where !(element.exists && element.isHittable) {
            if swipe < swipes {
                form.swipeUp()
            } else {
                form.swipeDown()
            }
        }
    }

    /// The clipboard's text, which Copy put there.
    ///
    /// Settings' Import a key takes it in its field: the test runner, in the background, can't read the clipboard
    /// itself. Logs is left for Settings' own page, where it ends.
    @MainActor
    internal static func pasted(in app: XCUIApplication) -> String {
        app.buttons["BackButton"].firstMatch.tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        let start = app.buttons["import-key"]
        reveal(start, in: app)
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        let menu = NSPredicate(
            format: "label == 'Paste' AND (elementType == %lu OR elementType == %lu)",
            XCUIElement.ElementType.menuItem.rawValue,
            XCUIElement.ElementType.button.rawValue
        )
        let paste = app.descendants(matching: .any).matching(menu).firstMatch
        // a press while the field takes the focus can bring up no menu: press again
        field.tap()
        for _ in 0..<tries where !paste.exists {
            field.press(forDuration: menuPress)
            _ = paste.waitForExistence(timeout: offerWait)
        }
        FakeApp.expect(paste, "the edit menu's Paste")
        paste.tap()
        ScreenRead.Wait(lasting: wait).poll { ScreenRead.shows(field) { !($0.value as? String ?? "").isEmpty } }
        let text = field.value as? String ?? ""
        app.buttons["BackButton"].firstMatch.tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        return text
    }

    /// The share sheet over the app, open and then closed.
    @MainActor
    internal static func shareAndClose(in app: XCUIApplication) {
        app.buttons["logs-share"].tap()
        let sheet = app.otherElements["ActivityListView"]
        FakeApp.expect(sheet, "the share sheet")
        if UIDevice.current.userInterfaceIdiom == .pad {
            // an iPad's share sheet is a popover, which a tap outside it dismisses
            tapOutsidePopover(in: app)
        } else {
            // a phone's is a sheet over the screen's lower half, which a tap above it dismisses
            app.navigationBars["Logs"].tap()
        }
        FakeApp.expectGone(sheet, "the share sheet", lasting: wait)
    }

    /// Clear logs asks first: Cancel keeps the lines, Clear logs leaves only the line that says so.
    @MainActor
    internal static func clearAsksFirst(in app: XCUIApplication) {
        app.buttons["logs-clear"].tap()
        // the confirmation dialog's own Clear logs, not the page's
        let confirm = app.buttons
            .matching(NSPredicate(format: "label == 'Clear logs' AND identifier != 'logs-clear'"))
            .firstMatch
        FakeApp.expect(confirm, "Clear logs' question")
        if app.buttons["Cancel"].exists {
            app.buttons["Cancel"].tap()
        } else {
            // an iPad's dialog is a popover, which a tap outside it dismisses
            tapOutsidePopover(in: app)
        }
        FakeApp.expectGone(confirm, "Clear logs' question", lasting: wait)
        let kept = shownLines(in: app)
        let isCleared = kept.count <= clearedLines || kept.contains { $0.contains("log cleared") }
        XCTAssertFalse(isCleared, "Cancel cleared the log: \(kept)")

        app.buttons["logs-clear"].tap()
        FakeApp.expect(confirm, "Clear logs' question")
        confirm.tap()
        expectNewest("log cleared", in: app)
        XCTAssertEqual(shownLines(in: app).count, clearedLines, "lines from before the clear: \(shownLines(in: app))")
    }

    // a tap outside an iPad's popover: near the screen's left edge, beside the sheet Settings is in and far from what
    // the popover points at, where its dismiss region takes the tap
    @MainActor
    private static func tapOutsidePopover(in app: XCUIApplication) {
        app.coordinate(withNormalizedOffset: outsidePopover).tap()
    }

    /// The text of each file under `directory` and its folders.
    internal static func files(under directory: URL) -> [String] {
        let walk = FileManager.default.enumerator(at: directory, includingPropertiesForKeys: nil)
        return (walk?.allObjects as? [URL] ?? [])
            .filter { !$0.hasDirectoryPath }
            .compactMap { try? String(contentsOf: $0, encoding: .utf8) }
    }

    /// Fails when `log` holds a secret.
    ///
    /// The secrets are `secrets` (what each is → its text), a line of `key`'s body, a key's armor and a fingerprint.
    internal static func assertClean(_ what: String, _ log: String, secrets: [String: String], key: String) {
        for (name, secret) in secrets {
            XCTAssertFalse(log.contains(secret), "\(what) hold \(name):\n\(log)")
        }
        for line in key.split(whereSeparator: \.isNewline) where line.count > keyLine && !line.hasPrefix("-----") {
            XCTAssertFalse(log.contains(line), "\(what) hold key material")
        }
        XCTAssertFalse(log.contains("PRIVATE KEY"), "\(what) hold a key's armor")
        XCTAssertFalse(log.contains("SHA256:"), "\(what) hold a fingerprint")
    }

    /// `log` holds each of `told`.
    internal static func assertTold(_ what: String, _ log: String, _ told: [String]) {
        for line in told {
            XCTAssertTrue(log.contains(line), "\(what) hold no \"\(line)\":\n\(log)")
        }
    }
}
