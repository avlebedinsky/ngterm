import XCTest

/// A key imported with its passphrase on the fake core brings no offer of the system's to keep the passphrase.
///
/// The offer to keep it in Passwords comes neither while the key's public half shows nor once that page is closed
/// (``ExportKeyTests`` holds the export's side). The import empties the passphrase field while it runs, and a
/// refused passphrase comes back into it.
internal final class KeyPassphraseOfferTests: XCTestCase {
    @MainActor
    internal func testAKeyImportedWithItsPassphraseBringsNoOfferToKeepIt() {
        // an offer takes the next tap: the steps after it are not what they say
        continueAfterFailure = false
        let app = FakeApp.launch(withKey: false, for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(TestKey.privateKey)
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(TestKey.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        FakeApp.expect(page, "the imported key's public half")
        PasswordOffer.expectNone(in: app, "after the import")

        app.buttons["key-done"].tap()
        FakeApp.showMachines(in: app)
        FakeApp.expect(FakeApp.element(containing: "No machines yet", in: app), "the machines")
        PasswordOffer.expectNone(in: app, "once the public key page closed")
    }

    // the field is empty while the import runs: a refused passphrase comes back into it, for another try
    @MainActor
    internal func testARefusedPassphraseStaysInItsField() {
        let wrong = "not-the-passphrase"
        let app = FakeApp.launch(withKey: false, for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(TestKey.privateKey)
        let secret = app.secureTextFields["passphrase"]
        secret.tap()
        secret.typeText(wrong + "\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(
            FakeApp.element(containing: "That passphrase doesn't open this key.", in: app),
            "that the wrong passphrase was refused"
        )
        // a secure field's value is a dot for each character
        XCTAssertEqual((secret.value as? String)?.count, wrong.count, "the refused passphrase is not in its field")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
