import XCTest

/// A key's export on a real machine, through the app's real core and the simulator's own owner check
/// (mobile/android's `KeyExport*E2eTest`s).
///
/// From a fresh install a person imports their key with its passphrase and adds the machine, which reads Connected.
/// In Settings, Export private key asks the device's owner first: a face or finger that doesn't match, then Cancel,
/// opens nothing; a match opens the export. It shows the key's fingerprint, refuses a passphrase too short, and saves
/// the key under a passphrase through the system's file screen, as an encrypted OpenSSH private key. Export without a
/// passphrase warns and asks once more; Copy then puts the key on the clipboard as a concealed item, which leaving the
/// page clears. Then, after the app is installed anew (the
/// second test), Import a key reads that file back: a wrong passphrase is refused, the right one gives the same key,
/// and the same machine signs it in again.
///
/// The machine is a private `ngterm-headless` behind a private, unprivileged `sshd` on the Mac's 127.0.0.1, which
/// trusts only the machine's key: `scripts/test-ios-e2e.sh` starts it, installs the app anew before each test and
/// hands the machine to the test through the test runner's environment. Run the two tests in one run of the script,
/// in their order (`ExportKeyOnAMachineTests/testA… ExportKeyOnAMachineTests/testB…`): the second reads the file the
/// first saved. Without a machine both are skipped.
internal final class ExportKeyOnAMachineTests: XCTestCase {
    // the passphrase the export is saved under, and one a character too short
    private static let passphrase = "qa-export-7kq2-ladder"
    private static let short = "qa-7kq2"
    private static let wrong = "qa-wrong-7kq2-ladder"
    private static let opensshHeader = "-----BEGIN OPENSSH PRIVATE KEY-----"
    private static let concealedType = "org.nspasteboard.ConcealedType"
    private static let firstLabel = "qa-export-mac"
    private static let secondLabel = "qa-import-mac"
    private static let connectWait: TimeInterval = 60
    private static let offerWait: TimeInterval = 5
    private static let beat: TimeInterval = 1
    // how long a switch is held: a plain tap on a form's switch can miss
    private static let switchPress: TimeInterval = 0.6
    private static let tries = 3

    // past this a test fails: on a simulator that answers a minute late, waits counted in reads take hours
    private static let timeLimit: TimeInterval = 1_200

    @MainActor
    internal func testAOnARealMachineAKeyIsExportedBehindTheOwnerCheckUnderAPassphrase() throws {
        executionTimeAllowance = Self.timeLimit
        let machine = try E2EMachine.fromEnvironment()
        ExportedKeyFiles.removeAll()
        Biometrics.enroll(for: self)
        let app = launch()
        signIn(with: machine, in: app)
        addMachine(machine, label: Self.firstLabel, in: app)
        app.buttons["settings"].tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings")

        // a face or finger that doesn't match, then Cancel: nothing opens
        ExportPage.refuseOwnerCheck(in: app)
        ExportPage.pause(Self.beat)
        XCTAssertFalse(app.navigationBars["Export private key"].exists, "a refused check opened the export")
        shot("refused", app)

        // a match opens the export, which shows the key's fingerprint
        ExportPage.passOwnerCheck(in: app)
        let page = app.scrollViews["export-key-page"]
        FakeApp.expect(page, "the export page")
        FakeApp.expect(FakeApp.element(containing: machine.fingerprint, in: app), "the key's fingerprint")

        // nothing is saved under a passphrase too short
        ExportPage.typePassphrase(Self.short, in: app)
        app.buttons["export-save"].tap()
        FakeApp.expect(FakeApp.element(containing: "Use at least 8 characters.", in: app), "the short passphrase")
        XCTAssertFalse(app.buttons["DOCPicker.actionButton"].exists, "a short passphrase opened the file screen")
        shot("short", app)

        ExportPage.typePassphrase(Self.passphrase, in: app)
        FileScreen.open(with: app.buttons["export-save"], in: app)
        FileScreen.save(in: app, back: app.buttons["export-copy"])
        FakeApp.expect(app.staticTexts["Saved"], "that the key was saved")
        assertSavedFileIsAnEncryptedOpenSSHKey()

        // without a passphrase only past the switch's warning and one more question; Copy then puts the key on the
        // clipboard as a concealed item, and leaving the page clears it
        let unprotected = app.switches["export-unprotected"]
        let isOn = NSPredicate(format: "value == '1'")
        for attempt in 0..<Self.tries {
            // the switch can turn on after the last read of the step: a read that completed tells it is off before a
            // press again, as a second press turns it off
            if attempt > 0, ScreenRead.holdsNow({ ScreenRead.shows(unprotected, matching: isOn) }) {
                break
            }
            unprotected.switches.firstMatch.press(forDuration: Self.switchPress)
            let read = ScreenRead.step(lasting: Self.offerWait).poll {
                ScreenRead.shows(unprotected, matching: isOn)
            }
            if read.held {
                break
            }
        }
        XCTAssertEqual(unprotected.value as? String, "1", "Export without a passphrase never turned on")
        FakeApp.expect(app.staticTexts["The exported key won't be encrypted"], "the warning")
        XCTAssertFalse(app.secureTextFields["export-passphrase"].exists)
        app.buttons["export-copy"].tap()
        FakeApp.expect(app.staticTexts["Export without a passphrase?"], "the last question")
        XCTAssertFalse(app.staticTexts["Copied"].exists, "the key was copied before the last question was answered")
        shot("last-question", app)
        app.buttons["Export without a passphrase"].firstMatch.tap()
        FakeApp.expect(app.staticTexts["Copied"], "that the key was copied")
        XCTAssertTrue(UIPasteboard.general.types.contains(Self.concealedType), "the copied key is not concealed")
        app.navigationBars["Export private key"].buttons.element(boundBy: 0).tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings again")
        ScreenRead.expect("the copied key stayed on the clipboard after the page was left", lasting: Self.offerWait) {
            !UIPasteboard.general.types.contains(Self.concealedType)
        }
    }

    @MainActor
    internal func testBAfterAReinstallTheExportedFileImportsBackAndSignsInToTheSameMachine() throws {
        executionTimeAllowance = Self.timeLimit
        let machine = try E2EMachine.fromEnvironment()
        addTeardownBlock {
            ExportedKeyFiles.removeAll()
        }
        XCTAssertFalse(
            ExportedKeyFiles.saved().isEmpty,
            "no exported file: run testA first, in the same run of the script"
        )
        let app = launch()
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")
        start.tap()
        FileScreen.choose(
            fileStartingWith: ExportedKeyFiles.name,
            opening: app.buttons["Choose file"],
            then: FakeApp.element(containing: "This key is protected: enter its passphrase.", in: app),
            in: app
        )

        // a wrong passphrase is refused, and the key stays out
        let field = app.secureTextFields["passphrase"]
        SecureEntry.type(Self.wrong, into: field)
        field.typeText("\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(
            FakeApp.element(containing: "That passphrase doesn't open this key.", in: app),
            "that the wrong passphrase was refused"
        )
        XCTAssertFalse(app.descendants(matching: .any)["public-key-page"].firstMatch.exists)
        shot("wrong-passphrase", app)

        SecureEntry.type(Self.passphrase, into: field)
        field.typeText("\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(app.descendants(matching: .any)["public-key-page"].firstMatch, "the imported key's public half")
        FakeApp.expect(FakeApp.element(containing: machine.fingerprint, in: app), "the exported key's fingerprint")
        closePublicKey(in: app)

        // the machine, which trusts only this key, signs it in
        addMachine(machine, label: Self.secondLabel, in: app)
        shot("connected-again", app)
    }

    // the app on its real core; an iPad on its side, as the other UI tests hold it
    @MainActor
    private func launch() -> XCUIApplication {
        FakeApp.launchOnTheRealCore(for: self)
    }

    // the welcome's Import a key, with the machine's key and its passphrase, to its public half, then Done
    @MainActor
    private func signIn(with machine: E2EMachine, in app: XCUIApplication) {
        let start = app.buttons["import-key"]
        FakeApp.expect(start, "the welcome's Import a key: is the install fresh?")
        start.tap()
        let field = app.textViews["private-key"]
        FakeApp.expect(field, "the private key's field")
        field.tap()
        field.typeText(machine.key.trimmingCharacters(in: .whitespacesAndNewlines))
        let secret = app.secureTextFields["passphrase"]
        SecureEntry.type(machine.passphrase, into: secret)
        secret.typeText("\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(app.descendants(matching: .any)["public-key-page"].firstMatch, "the imported key's public half")
        closePublicKey(in: app)
    }

    // the public key page's Done, past the offer to keep the passphrase in Passwords: a guard, as the offer is gone
    // since the import empties its field
    @MainActor
    private func closePublicKey(in app: XCUIApplication) {
        let notNow = app.buttons["Not Now"]
        if notNow.waitForExistence(timeout: Self.offerWait) {
            notNow.tap()
        }
        // a tap while the offer goes can reach nothing: tap Done again until the page goes
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        // a read that completed tells whether the page stayed: a failed read is no reason to stop, or to tap a Done
        // that went with the page
        for _ in 0..<Self.tries where !ScreenRead.holdsNow({ ScreenRead.isGone(page) }) {
            done.tap()
            ScreenRead.step(lasting: Self.offerWait).poll { ScreenRead.isGone(page) }
        }
        FakeApp.expectGone(page, "the public key page, after Done,")
    }

    // the machine through the form, its host key trusted, connected
    @MainActor
    private func addMachine(_ machine: E2EMachine, label: String, in app: XCUIApplication) {
        FakeApp.showMachines(in: app)
        app.buttons["add-machine"].tap()
        E2EMachine.fill("machine-label", with: label, in: app)
        E2EMachine.fill("machine-host", with: machine.host, in: app)
        E2EMachine.fill("machine-port", with: machine.port, in: app)
        E2EMachine.fill("machine-user", with: machine.user, in: app)
        app.buttons["machine-save"].tap()
        let trust = app.buttons["trust-host-key"]
        FakeApp.expect(trust, "the host key sheet")
        trust.tap()
        let row = NSPredicate(format: "identifier == %@ AND label CONTAINS 'Connected'", "machine-row-\(label)")
        let connected = app.descendants(matching: .any).matching(row).firstMatch
        FakeApp.expect(connected, "Connected on the machine's row", lasting: Self.connectWait)
    }

    // the one saved file is an OpenSSH private key under a cipher, never in the clear; only its cipher is reported
    private func assertSavedFileIsAnEncryptedOpenSSHKey() {
        let files = ExportedKeyFiles.saved()
        XCTAssertEqual(files.count, 1, "the export saved \(files.count) files")
        guard let file = files.first, let text = try? String(contentsOf: file, encoding: .utf8) else {
            XCTFail("the saved file can't be read")
            return
        }
        XCTAssertTrue(text.hasPrefix(Self.opensshHeader), "the saved file is not an OpenSSH private key")
        let cipher = ExportedKeyFiles.cipher(of: text)
        XCTAssertNotNil(cipher, "the saved file's OpenSSH body can't be read")
        XCTAssertNotEqual(cipher, "none", "the key was saved in the clear")
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
