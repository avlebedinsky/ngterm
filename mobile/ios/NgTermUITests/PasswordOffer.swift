import XCTest

/// The system's offer to keep a password in Passwords, as the UI tests look for it.
internal enum PasswordOffer {
    /// How long the offer may take to come: on a loaded simulator it came 11 s after a key's import.
    internal static let wait: TimeInterval = 15

    /// Fails the test if the offer comes until ``wait`` ends: a key's passphrase is not a password to keep.
    @MainActor
    internal static func expectNone(in app: XCUIApplication, _ when: String) {
        let offer = app.buttons["Not Now"]
        XCTAssertFalse(
            offer.waitForExistence(timeout: wait),
            "the system offered to keep the passphrase in Passwords \(when)"
        )
    }
}
