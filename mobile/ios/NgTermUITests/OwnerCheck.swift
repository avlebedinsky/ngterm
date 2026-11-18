import XCTest

/// The system's owner check (LocalAuthentication) as Springboard shows it over the app, passed or refused with the
/// simulator's face or finger (``Biometrics``), each step taken on what the screen shows.
///
/// The prompt can come seconds after the tap that asks for it (5 s on the tablet), and a face or finger shown before
/// it is up reaches nothing. Face ID's prompt has no buttons until a face doesn't match; Touch ID's alert has Cancel
/// at once, and a second finger that doesn't match brings the passcode screen. Each wait is a count of reads, and a
/// step that did not take is done again a counted number of times.
internal enum OwnerCheck {
    private static let springboardIdentifier = "com.apple.springboard"
    // the prompt: Touch ID's alert, Face ID's view, and the passcode screen's view
    private static let promptIdentifiers = [
        "com.apple.localauthentication.ax.authentication.alert",
        "authentication_ui",
    ]
    // shown once a face or finger didn't match: Face ID's Try Face ID Again, Touch ID's Enter Passcode
    private static let refusedIdentifiers = [
        "com.apple.localauthentication.ax.authentication.button.try-again",
        "com.apple.localauthentication.ax.authentication.button.fallback",
    ]
    // the prompt's Cancel, and the passcode screen's
    private static let cancelIdentifiers = ["com.apple.localauthentication.ax.authentication.button.cancel", "Cancel"]
    private static let passcodeField = "Passcode field"
    // the app's alert when the system didn't confirm the owner (ExportNotice)
    private static let failedTitle = "Couldn't confirm it's you"
    private static let tryAgain = "Try again"
    private static let beat: TimeInterval = 0.5
    // how many reads the prompt may take to come or to go
    private static let promptReads = 30
    // how many reads a face or finger may take to show its outcome before it is shown again
    private static let outcomeReads = 8
    // how many times a step that did not take is done again
    private static let tries = 5

    @MainActor private static var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: springboardIdentifier)
    }

    @MainActor private static var prompt: XCUIElement {
        springboard.descendants(matching: .any).matching(NSPredicate(format: "identifier IN %@", promptIdentifiers))
            .firstMatch
    }

    @MainActor private static var passcode: XCUIElement {
        springboard.secureTextFields[passcodeField]
    }

    /// Does `ask` until the system's prompt shows, and fails the test when it never does.
    @MainActor
    internal static func open(by ask: () -> Void) {
        for _ in 0..<tries {
            ask()
            if awaitPrompt() {
                return
            }
        }
        XCTFail("the system's owner check never showed, after \(tries) asks")
    }

    /// Opens the prompt with `ask` and shows it a matching face or finger until `opened` shows.
    ///
    /// A match that brought nothing is shown again. The app's alert that the system didn't confirm the owner gets
    /// Try again, and the passcode screen gets Cancel and `ask` again.
    @MainActor
    internal static func pass(by ask: () -> Void, in app: XCUIApplication, until opened: XCUIElement) {
        open(by: ask)
        let failed = app.staticTexts[failedTitle]
        for _ in 0..<tries where !opened.exists {
            Biometrics.match()
            for _ in 0..<outcomeReads {
                if opened.exists {
                    return
                }
                if failed.exists {
                    app.buttons[tryAgain].tap()
                    _ = awaitPrompt()
                    break
                }
                if passcode.exists {
                    cancel()
                    open(by: ask)
                    break
                }
                pause()
            }
        }
        XCTAssertTrue(opened.exists, "the owner check didn't open \(opened) after \(tries) matches")
    }

    /// Opens the prompt with `ask`, shows it a face or finger that doesn't match until it says so, then cancels it.
    @MainActor
    internal static func refuse(by ask: () -> Void) {
        open(by: ask)
        let refused = springboard.buttons.matching(NSPredicate(format: "identifier IN %@", refusedIdentifiers))
            .firstMatch
        for _ in 0..<tries where !(refused.exists || passcode.exists) {
            Biometrics.noMatch()
            for _ in 0..<outcomeReads where !(refused.exists || passcode.exists) {
                pause()
            }
        }
        XCTAssertTrue(refused.exists || passcode.exists, "the prompt never showed that a face or finger didn't match")
        cancel()
    }

    // taps the prompt's Cancel, or the passcode screen's, and waits for the prompt to go
    @MainActor
    private static func cancel() {
        let cancel = springboard.buttons.matching(NSPredicate(format: "identifier IN %@", cancelIdentifiers)).firstMatch
        XCTAssertTrue(cancel.exists, "the system's owner check has no Cancel")
        cancel.tap()
        for _ in 0..<promptReads where prompt.exists {
            pause()
        }
        XCTAssertFalse(prompt.exists, "the system's owner check stayed after Cancel")
    }

    // waits for the prompt, and allows the app Face ID when the system asks that first (Touch ID asks nothing)
    @MainActor
    private static func awaitPrompt() -> Bool {
        let allow = springboard.alerts.buttons["OK"]
        for _ in 0..<promptReads {
            if allow.exists {
                allow.tap()
            } else if prompt.exists {
                return true
            }
            pause()
        }
        return false
    }

    private static func pause() {
        _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: beat)
    }
}
