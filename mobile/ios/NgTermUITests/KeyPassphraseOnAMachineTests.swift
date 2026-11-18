import XCTest

/// On a real machine, through the app's real core, no screen brings the system's offer to keep the key's passphrase in
/// Passwords (``KeyPassphraseOfferTests`` holds the same on the fake core).
///
/// From a fresh install a person imports their key with a wrong passphrase: the core refuses it, and the passphrase
/// comes back into its field for another try. They go Back to the welcome, import the key again with its passphrase,
/// close its public half, and add the machine, which trusts only this key and reads Connected. In Settings they export
/// the key under a passphrase to a file, past the owner check, and close the export. The test waits for the offer
/// after each of these steps (``PasswordOffer/wait``), and takes a screenshot of each.
///
/// The machine is a private `ngterm-headless` behind a private, unprivileged `sshd` on the Mac's 127.0.0.1:
/// `scripts/test-ios-e2e.sh` starts it, installs the app anew and hands the machine to the test through the test
/// runner's environment. Without it the test is skipped.
internal final class KeyPassphraseOnAMachineTests: XCTestCase {
    // a passphrase the machine's key does not open with, and the one the export is saved under
    private static let wrong = "qa-wrong-4hv8-gravel"
    private static let exportPassphrase = "qa-export-4hv8-gravel"
    private static let label = "qa-offer-mac"
    private static let connectWait: TimeInterval = 60
    private static let pageWait: TimeInterval = 5
    private static let tries = 3

    // past this a test fails: on a simulator that answers a minute late, waits counted in reads take hours
    private static let timeLimit: TimeInterval = 1_200

    @MainActor
    internal func testOnARealMachineNoScreenOffersToKeepTheKeysPassphrase() throws {
        executionTimeAllowance = Self.timeLimit
        // an offer takes the next tap: the steps after it are not what they say
        continueAfterFailure = false
        let machine = try E2EMachine.fromEnvironment()
        ExportedKeyFiles.removeAll()
        addTeardownBlock {
            ExportedKeyFiles.removeAll()
        }
        Biometrics.enroll(for: self)
        let app = FakeApp.launchOnTheRealCore(for: self)
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")

        start.tap()
        importKey(machine.key, with: Self.wrong, in: app)
        FakeApp.expect(
            FakeApp.element(containing: "That passphrase doesn't open this key.", in: app),
            "that the wrong passphrase was refused"
        )
        let secret = app.secureTextFields["passphrase"]
        XCTAssertEqual((secret.value as? String)?.count, Self.wrong.count, "the refused passphrase is not in its field")
        shot("1-refused", app)
        PasswordOffer.expectNone(in: app, "after a refused passphrase")

        app.navigationBars["Import a key"].buttons.element(boundBy: 0).tap()
        FakeApp.expect(start, "the welcome again")
        shot("2-after-back", app)
        PasswordOffer.expectNone(in: app, "after Back from the import")

        start.tap()
        importKey(machine.key, with: machine.passphrase, in: app)
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        FakeApp.expect(page, "the imported key's public half")
        FakeApp.expect(FakeApp.element(containing: machine.fingerprint, in: app), "the key's fingerprint")
        shot("3-public-key", app)
        PasswordOffer.expectNone(in: app, "after the import")

        closePublicKey(page, in: app)
        shot("4-public-key-closed", app)
        PasswordOffer.expectNone(in: app, "once the public key page closed")

        addMachine(machine, in: app)
        shot("5-connected", app)
        PasswordOffer.expectNone(in: app, "after the machine signed the key in")

        exportUnderAPassphrase(in: app)
        shot("6-exported", app)
        PasswordOffer.expectNone(in: app, "after the export under a passphrase")

        app.navigationBars["Export private key"].buttons.element(boundBy: 0).tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings again")
        shot("7-export-closed", app)
        PasswordOffer.expectNone(in: app, "once the export page closed")
    }

    // on the import screen: `key` typed into an empty field, the passphrase field emptied, `passphrase` typed, Import
    @MainActor
    private func importKey(_ key: String, with passphrase: String, in app: XCUIApplication) {
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        if (field.value as? String ?? "").isEmpty || field.value as? String == field.placeholderValue {
            field.tap()
            field.typeText(key.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        let secret = app.secureTextFields["passphrase"]
        SecureEntry.type(passphrase, into: secret)
        secret.typeText("\n")
        app.buttons["import-key-action"].tap()
    }

    // the public key page's Done, until the page goes, with no offer to close on the way
    @MainActor
    private func closePublicKey(_ page: XCUIElement, in app: XCUIApplication) {
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        for _ in 0..<Self.tries where page.exists {
            done.tap()
            _ = page.waitForNonExistence(timeout: Self.pageWait)
        }
        XCTAssertFalse(page.exists, "the public key page stayed after Done")
    }

    // the machine through the form, its host key trusted, connected
    @MainActor
    private func addMachine(_ machine: E2EMachine, in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        E2EMachine.fill("machine-label", with: Self.label, in: app)
        E2EMachine.fill("machine-host", with: machine.host, in: app)
        E2EMachine.fill("machine-port", with: machine.port, in: app)
        E2EMachine.fill("machine-user", with: machine.user, in: app)
        app.buttons["machine-save"].tap()
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "the host key sheet")
        trust.tap()
        let row = NSPredicate(format: "identifier == %@ AND label CONTAINS 'Connected'", "machine-row-\(Self.label)")
        let connected = app.descendants(matching: .any).matching(row).firstMatch
        XCTAssertTrue(connected.waitForExistence(timeout: Self.connectWait), "the machine never read Connected")
    }

    // Settings' Export private key past the owner check, saved under a passphrase through the system's file screen
    @MainActor
    private func exportUnderAPassphrase(in app: XCUIApplication) {
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")
        ExportPage.passOwnerCheck(in: app)
        ExportPage.typePassphrase(Self.exportPassphrase, in: app)
        FileScreen.open(with: app.buttons["export-save"], in: app)
        FileScreen.save(in: app, back: app.buttons["export-copy"])
        FakeApp.expect(app.staticTexts["Saved"], "that the key was saved")
        XCTAssertEqual(ExportedKeyFiles.saved().count, 1, "the export saved no file, or more than one")
    }

    @MainActor
    private func shot(_ name: String, _ app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
