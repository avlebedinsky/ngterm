import XCTest

/// Settings' Export private key on a real machine, as the UI tests open and drive it (``ExportKeyOnAMachineTests``,
/// ``KeyPassphraseOnAMachineTests``): past the system's owner check, under a passphrase.
internal enum ExportPage {
    private static let offerWait: TimeInterval = 5
    private static let beat: TimeInterval = 1
    // how many swipes a row far down the form may take to come into view
    private static let swipes = 12
    // how many times a face or finger is shown before the system's prompt is known to be up
    private static let looks = 10
    private static let springboardIdentifier = "com.apple.springboard"

    /// Waits `seconds`, with nothing to wait for.
    internal static func pause(_ seconds: TimeInterval) {
        _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: seconds)
    }

    /// Taps Settings' Export private key of the app's one key.
    @MainActor
    internal static func tapExport(in app: XCUIApplication) {
        let export = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'export-key-'")).firstMatch
        let form = app.collectionViews["settings-form"]
        for _ in 0..<swipes where !(export.exists && export.isHittable) {
            form.swipeUp()
        }
        XCTAssertTrue(export.isHittable, "Export private key never came into view")
        export.tap()
    }

    /// Allows the app Face ID, the first time it asks (Touch ID asks nothing).
    @MainActor
    internal static func allowBiometrics() {
        let allow = XCUIApplication(bundleIdentifier: springboardIdentifier).alerts.buttons["OK"]
        if allow.waitForExistence(timeout: offerWait) {
            allow.tap()
        }
    }

    /// Shows the system's prompt a face or finger that doesn't match, then cancels it.
    @MainActor
    internal static func refuseOwnerCheck() {
        allowBiometrics()
        let cancel = XCUIApplication(bundleIdentifier: springboardIdentifier).buttons["Cancel"].firstMatch
        for _ in 0..<looks where !cancel.exists {
            pause(beat)
            Biometrics.noMatch()
        }
        FakeApp.expect(cancel, "the system prompt's Cancel")
        cancel.tap()
    }

    /// Shows the system's prompt a matching face or finger, until the export opens.
    @MainActor
    internal static func passOwnerCheck(in app: XCUIApplication) {
        allowBiometrics()
        let page = app.navigationBars["Export private key"]
        for _ in 0..<looks where !page.exists {
            pause(beat)
            Biometrics.match()
        }
        FakeApp.expect(page, "the export page after a match")
    }

    /// Types `passphrase` into both fields, each emptied first, and closes the keyboard after the second.
    @MainActor
    internal static func typePassphrase(_ passphrase: String, in app: XCUIApplication) {
        for identifier in ["export-passphrase", "export-passphrase-again"] {
            let field = app.secureTextFields[identifier]
            FakeApp.expect(field, identifier)
            field.tap()
            let old = (field.value as? String ?? "").count
            if old > 0, field.value as? String != field.placeholderValue {
                field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: old))
            }
            field.typeText(passphrase + (identifier.hasSuffix("again") ? "\n" : ""))
            // a secure field reads as a dot for each character it holds
            XCTAssertEqual((field.value as? String)?.count, passphrase.count, "\(identifier) holds another text")
        }
    }
}
