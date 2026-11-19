import XCTest

/// Typing into a terminal on the fake core, through the system's own input (mobile/android's `TerminalKeyboardTest`).
///
/// The soft keyboard and the extra keys, and a hardware keyboard's keys and chords. The app shows a terminal on a fake
/// pane alone (`-ngterm-fake-terminal`), with what the pane got written over it.
internal final class KeyboardTests: XCTestCase {
    private static let wait: TimeInterval = 15
    // how long the slide-to-type introduction gets to come up after the keyboard, which it follows within a moment
    private static let introductionWait: TimeInterval = 5
    // how far apart a wait reads a line of the bench, which draws its lines again five times a second
    private static let readBeat: TimeInterval = 0.2

    // the extra key named `name`
    @MainActor
    private static func key(_ name: String, in app: XCUIApplication) -> XCUIElement {
        app.descendants(matching: .any)["extra-key-\(name)"]
    }

    // Closes the system's slide-to-type introduction if it came up over the soft keyboard. Its Continue is the only way
    // past it: it covers the keyboard's lower half and the extra keys, so every tap lands on it and types nothing.
    // build-ios.sh sets the preference that should stop it, but a simulator's first keyboard now and then shows it
    // anyway.
    @MainActor
    private static func closeTheKeyboardIntroduction(in app: XCUIApplication) {
        let introduction = app.descendants(matching: .any)["UIContinuousPathIntroductionView"]
        guard introduction.waitForExistence(timeout: introductionWait) else {
            return
        }
        introduction.buttons["Continue"].tap()
        FakeApp.expectGone(introduction, "the slide-to-type introduction", lasting: wait)
    }

    // Reads the bench's `line` again until its text `matches` or the wait ends, and returns the last text read: nil
    // when no read completed. The test reads the line itself: an XCTNSPredicateExpectation that reads the line once
    // before the app updates it then captures the element's debug description, and on a loaded simulator that capture
    // can use all the wait, so the expectation reads the line no second time. The wait is a number of reads, as
    // FakeApp's waits are.
    @MainActor
    private static func read(_ line: String, in app: XCUIApplication, until matches: (String) -> Bool) -> String? {
        let element = app.staticTexts[line]
        var text: String?
        ScreenRead.Wait(lasting: wait, beat: readBeat).poll {
            ScreenRead.shows(element) { snapshot in
                text = snapshot.label
                return matches(snapshot.label)
            }
        }
        return text
    }

    // waits until the bench's `line` reads `text`
    @MainActor
    private static func expect(_ line: String, reads text: String, in app: XCUIApplication) {
        let label = read(line, in: app) { $0 == text }
        XCTAssertEqual(label, text, "\(line) reads '\(label ?? "nothing: no read completed")', not '\(text)'")
    }

    @MainActor
    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-ngterm-fake-core", "-ngterm-fake-terminal", FakeApp.stillArgument]
        app.launch()
        // a tap on the terminal gives it the focus and opens the soft keyboard
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        FakeApp.expect(Self.key("escape", in: app), "the extra keys")
        return app
    }

    @MainActor
    internal func testTheSoftKeyboardAndTheExtraKeysType() throws {
        let app = launch()
        let keyboard = app.keyboards.firstMatch
        // only a read that completed and found no soft keyboard is a reason to skip: a wait on time alone can end with
        // no read on a loaded runner
        let shown = ScreenRead.Wait(reads: 1, lasting: Self.wait).poll { ScreenRead.isShown(keyboard) }
        guard shown.held else {
            guard shown.reads > 0 else {
                FakeApp.attachScreen("no read of the soft keyboard")
                XCTFail("no read of the soft keyboard completed in \(Int(shown.seconds)) s (\(shown.failed) failed)")
                return
            }
            throw XCTSkip("the simulator has the Mac's keyboard attached, so no soft keyboard shows")
        }
        Self.closeTheKeyboardIntroduction(in: app)
        // each key reaches the program as it goes down, not when a space or Return ends the word
        keyboard.keys["l"].tap()
        Self.expect("bench-typed", reads: "l", in: app)
        keyboard.keys["s"].tap()
        Self.expect("bench-typed", reads: "ls", in: app)
        Self.key("control", in: app).tap()
        XCTAssertEqual(Self.key("control", in: app).value as? String, "On for the next key")
        keyboard.keys["c"].tap()
        Self.key("escape", in: app).tap()
        Self.key("upArrow", in: app).tap()
        Self.key("pipe", in: app).tap()
        Self.expect("bench-keys", reads: "key_c/2 escape/0 arrow_up/0", in: app)
        Self.expect("bench-typed", reads: "ls|", in: app)
        XCTAssertEqual(Self.key("control", in: app).value as? String, "Off")
        // the keyboard's key closes the soft keyboard, and the extra keys stay
        Self.key("keyboard", in: app).tap()
        FakeApp.expectGone(keyboard, "the soft keyboard", lasting: Self.wait)
        XCTAssertTrue(Self.key("escape", in: app).exists)
        Self.key("keyboard", in: app).tap()
        FakeApp.expect(keyboard, "the soft keyboard again", lasting: Self.wait)
    }

    @MainActor
    internal func testAHardwareKeyboardTypesAndItsChordsRunInTheApp() {
        let app = launch()
        app.typeKey("e", modifierFlags: [])
        app.typeKey("c", modifierFlags: .control)
        app.typeKey(XCUIKeyboardKey.leftArrow.rawValue, modifierFlags: [])
        Self.expect("bench-typed", reads: "e", in: app)
        Self.expect("bench-keys", reads: "key_c/2 arrow_left/0", in: app)
        let grid = app.staticTexts["bench-grid"].label
        app.typeKey("=", modifierFlags: .command)
        let bigger = Self.read("bench-grid", in: app) { $0 != grid }
        XCTAssertNotNil(bigger, "no read of bench-grid completed after ⌘=")
        XCTAssertNotEqual(bigger ?? grid, grid, "⌘= made the text no bigger")
        app.typeKey("0", modifierFlags: .command)
        Self.expect("bench-grid", reads: grid, in: app)
        app.typeKey(XCUIKeyboardKey.rightArrow.rawValue, modifierFlags: [.command, .option])
        app.typeKey(XCUIKeyboardKey.upArrow.rawValue, modifierFlags: [.command, .option])
        Self.expect("bench-moves", reads: "next pane, previous session", in: app)
        Self.expect("bench-keys", reads: "key_c/2 arrow_left/0", in: app)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
