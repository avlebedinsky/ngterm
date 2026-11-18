import XCTest

/// A passphrase typed into a secure field, as the UI tests on a real machine enter one.
///
/// On a loaded simulator a tap can leave the field without the keyboard, and typed keys can be lost (one of seven
/// held, on the tablet just after a reboot). So the field is tapped until it has the keyboard, and emptied and typed
/// again until it holds the whole text, each a counted number of times. A secure field reads as a dot for each
/// character it holds.
internal enum SecureEntry {
    private static let tries = 3
    private static let focusWait: TimeInterval = 5

    /// Empties `field` and types `text` into it, until it holds as many characters.
    @MainActor
    internal static func type(_ text: String, into field: XCUIElement) {
        var held = -1
        for _ in 0..<tries where held != text.count {
            focus(field)
            let old = count(in: field)
            if old > 0 {
                field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: old))
            }
            field.typeText(text)
            held = count(in: field)
        }
        XCTAssertEqual(held, text.count, "\(field) holds another text after \(tries) typings")
    }

    // taps the field until it has the keyboard
    @MainActor
    private static func focus(_ field: XCUIElement) {
        let hasKeyboard = NSPredicate(format: "hasKeyboardFocus == true")
        var focused = false
        for _ in 0..<tries where !focused {
            field.tap()
            let focus = XCTNSPredicateExpectation(predicate: hasKeyboard, object: field)
            focused = XCTWaiter.wait(for: [focus], timeout: focusWait) == .completed
        }
        XCTAssertTrue(focused, "\(field) never took the keyboard")
    }

    // the characters the field holds: an empty one reads as its placeholder
    @MainActor
    private static func count(in field: XCUIElement) -> Int {
        guard let value = field.value as? String, value != field.placeholderValue else {
            return 0
        }
        return value.count
    }
}
