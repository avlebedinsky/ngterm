import XCTest

/// The machine `scripts/test-ios-e2e.sh` started, as the test runner's environment names it.
internal struct E2EMachine {
    private static let tries = 3
    private static let focusWait: TimeInterval = 5

    /// The machine's address, port and user.
    internal let host: String
    internal let port: String
    internal let user: String
    /// The client key's file, its text.
    internal let key: String
    /// The passphrase the client key is under.
    internal let passphrase: String
    /// The client key's fingerprint, without `SHA256:`.
    internal let fingerprint: String

    /// The machine the test runner's environment names.
    ///
    /// - Throws: `XCTSkip` when the environment names none: the test runs only through the script.
    internal static func fromEnvironment() throws -> Self {
        let environment = ProcessInfo.processInfo.environment
        func read(_ name: String) -> String? {
            environment["IOS_E2E_\(name)"].flatMap { $0.isEmpty ? nil : $0 }
        }
        guard let address = read("HOST"), let sshPort = read("PORT"), let account = read("USER"),
            let keyFile = read("KEY"), let secret = read("PASSPHRASE"), let digest = read("FINGERPRINT")
        else {
            throw XCTSkip("no machine: run it through scripts/test-ios-e2e.sh")
        }
        return Self(
            host: address,
            port: sshPort,
            user: account,
            key: try String(contentsOfFile: keyFile, encoding: .utf8),
            passphrase: secret,
            fingerprint: digest
        )
    }

    /// Replaces the text of the machine form's field `identifier` (a default port, say) with `text`.
    ///
    /// An iPad's number pad (the port's) is a popover: a tap outside it only closes it, so the field is tapped again
    /// until it has the keyboard.
    @MainActor
    internal static func fill(_ identifier: String, with text: String, in app: XCUIApplication) {
        let field = app.textFields[identifier]
        FakeApp.expect(field, identifier)
        var focused = false
        for _ in 0..<tries where !focused {
            field.tap()
            let hasKeyboard = NSPredicate(format: "hasKeyboardFocus == true")
            let read = ScreenRead.step(lasting: focusWait).poll {
                ScreenRead.shows(field, matching: hasKeyboard)
            }
            focused = read.held
        }
        XCTAssertTrue(focused, "\(identifier) never took the keyboard")
        let old = field.value as? String ?? ""
        let placeholder = field.placeholderValue ?? ""
        if !old.isEmpty, old != placeholder {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: old.count))
        }
        field.typeText(text)
    }
}
