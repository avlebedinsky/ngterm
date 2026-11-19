import UIKit
import UniformTypeIdentifiers
import XCTest

/// A program's clipboard reads on a real machine, through the app's real core, of what another app copied
/// (mobile/android's `ClipboardSensitiveReadE2eTest`).
///
/// From a fresh install a person imports their key, adds the machine and opens a named session. A program there reads
/// the clipboard with OSC 52. The test runner, another app than ngterm (as a password manager is), copies each text.
/// A plain copy asks; after Always allow on the machine the next read of a plain copy is sent unasked. A concealed
/// (sensitive) copy asks on that machine all the same: the sheet says the copy is marked sensitive, shows none of it
/// (it may be an exported key), offers no Show and no Always, and the program gets nothing until Allow once. The next
/// read of it asks again, and Deny sends an empty answer. A plain copy after that is sent unasked again.
///
/// The machine is a private `ngterm-headless` behind a private, unprivileged `sshd` on the Mac's 127.0.0.1:
/// `scripts/test-ios-e2e.sh` starts it, installs the app anew and hands the machine to the test through the test
/// runner's environment. The machine's PATH holds `e2e-clip-read <label>`, which sends one OSC 52 read and prints
/// `R:<label>:<length>:<text>`. The script also sets the app's pasteboard permission in the simulator, which otherwise
/// gives the app nothing of another app's copy. Without a machine the test is skipped.
internal final class SensitiveClipOnAMachineTests: XCTestCase {
    private static let wait: TimeInterval = 60
    private static let offerWait: TimeInterval = 5
    // how long a program waits with the question up, to show it gets no answer before one is given
    private static let unanswered: TimeInterval = 3
    private static let beat: TimeInterval = 0.5
    private static let menuPress: TimeInterval = 1
    private static let label = "qa-clip-mac"
    private static let session = "qa-clip-sess"
    private static let reader = "e2e-clip-read"
    private static let plain = "qa-plain-k7r4"
    private static let plainAgain = "qa-plain-k7r4-b"
    private static let secret = "qa-secret-k7r4-hunter2"
    private static let concealedType = "org.nspasteboard.ConcealedType"
    private static let heading = "Let \(session) read your clipboard?"

    @MainActor
    internal func testAPlainCopyIsSentUnaskedAfterAlwaysButASensitiveOneAsksEachTimeAndDenySendsNothing() throws {
        let machine = try E2EMachine.fromEnvironment()
        let app = FakeApp.launchOnTheRealCore(for: self)
        signIn(with: machine, in: app)
        addMachine(machine, in: app)
        openANewSession(in: app)
        expectTerminal("a prompt", in: app) { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

        // a plain copy: the read asks; Always allow on the machine sends it
        copy(Self.plain, concealed: false, back: app)
        read("p1", in: app)
        expectSheet(in: app)
        shot("01-plain-asks")
        XCTAssertFalse(
            FakeApp.element(containing: "holds none", in: app).exists,
            "the app read an empty clipboard: does the simulator let the app read another app's copy?"
        )
        XCTAssertFalse(FakeApp.element(containing: "marked sensitive", in: app).exists, "a plain copy is sensitive")
        XCTAssertTrue(FakeApp.element(containing: Self.plain, in: app).exists, "the sheet shows no plain copy")
        let always = app.buttons["clipboard-always"]
        FakeApp.expect(always, "Always allow on the machine")
        always.tap()
        expectAnswer("p1", Self.plain, in: app)
        shot("02-plain-sent-after-always")

        // the plain copy again: sent unasked
        read("p2", in: app)
        expectAnswer("p2", Self.plain, in: app, unasked: true)
        shot("03-plain-unasked")

        // a concealed copy asks for the one read, and the program gets nothing until Allow once
        copy(Self.secret, concealed: true, back: app)
        read("s1", in: app)
        expectSheet(in: app)
        shot("04-sensitive-asks")
        expectNoAnswerYet("s1", in: app)
        expectSensitiveSheet(in: app)
        app.buttons["clipboard-allow"].tap()
        expectAnswer("s1", Self.secret, in: app)
        shot("05-sensitive-sent-once")

        // the same concealed copy: the next read asks again; Deny sends an empty answer
        read("s2", in: app)
        expectSheet(in: app)
        XCTAssertFalse(app.buttons["clipboard-always"].exists, "Always is offered over a sensitive copy")
        shot("06-sensitive-asks-again")
        app.buttons["clipboard-deny"].tap()
        expectAnswer("s2", "", in: app)
        shot("07-sensitive-denied")

        // a plain copy after that: sent unasked, the machine is still always allowed
        copy(Self.plainAgain, concealed: false, back: app)
        read("p3", in: app)
        expectAnswer("p3", Self.plainAgain, in: app, unasked: true)
        shot("08-plain-unasked-again")
        attach("terminal", terminalText(in: app))
    }

    // MARK: the copies, by the test runner: another app than ngterm

    @MainActor
    private func copy(_ text: String, concealed: Bool, back app: XCUIApplication) {
        var item: [String: Any] = [UTType.utf8PlainText.identifier: text]
        if concealed {
            item[Self.concealedType] = Data()
        }
        UIPasteboard.general.setItems([item])
        XCTAssertEqual(
            UIPasteboard.general.types.contains(Self.concealedType),
            concealed,
            "the copy is not as concealed as asked"
        )
        XCTAssertEqual(app.state, .runningForeground, "the app is not in front")
    }

    // MARK: the reads and their answers

    @MainActor
    private func terminalText(in app: XCUIApplication) -> String {
        (app.textViews.firstMatch.value as? String) ?? ""
    }

    // the reader, typed at the session's shell
    @MainActor
    private func read(_ name: String, in app: XCUIApplication) {
        let terminal = app.textViews.firstMatch
        FakeApp.expect(terminal, "the session's terminal")
        terminal.tap()
        app.typeText("\(Self.reader) \(name)\n")
    }

    @MainActor
    private func expectSheet(in app: XCUIApplication) {
        FakeApp.expect(app.staticTexts[Self.heading], "clipboard question", lasting: Self.wait)
    }

    // the program has no answer while the question is up
    @MainActor
    private func expectNoAnswerYet(_ name: String, in app: XCUIApplication) {
        _ = XCTWaiter.wait(for: [XCTestExpectation(description: "the program waits")], timeout: Self.unanswered)
        let shown = terminalText(in: app)
        XCTAssertFalse(shown.contains("R:\(name):"), "the program got an answer before the question was: \(shown)")
        XCTAssertTrue(app.staticTexts[Self.heading].exists, "the question went with no answer")
    }

    // the sheet over a concealed copy: it says the copy is marked sensitive, shows none of it (it may be an exported
    // key), not even on request, and offers only Allow once and Deny
    @MainActor
    private func expectSensitiveSheet(in app: XCUIApplication) {
        XCTAssertTrue(
            FakeApp.element(containing: "What you copied is marked sensitive", in: app).exists,
            "the sheet does not say the copy is marked sensitive"
        )
        XCTAssertTrue(
            FakeApp.element(containing: "so it isn't shown here", in: app).exists,
            "the sheet does not say the concealed copy isn't shown"
        )
        XCTAssertFalse(FakeApp.element(containing: Self.secret, in: app).exists, "the sheet shows the concealed copy")
        XCTAssertFalse(app.buttons["clipboard-reveal"].exists, "Show is offered over a concealed copy")
        XCTAssertFalse(app.buttons["clipboard-always"].exists, "Always is offered over a sensitive copy")
        XCTAssertFalse(app.switches["clipboard-remember"].exists, "a grant is offered over a sensitive copy")
        XCTAssertEqual(app.buttons["clipboard-allow"].label, "Allow once")
        XCTAssertTrue(app.buttons["clipboard-deny"].exists, "no Deny")
    }

    // the reader's line for `name` in the terminal; with `unasked`, no question came before it
    @MainActor
    private func expectAnswer(_ name: String, _ text: String, in app: XCUIApplication, unasked: Bool = false) {
        let line = "R:\(name):\(text.count):\(text)"
        let end = Date().addingTimeInterval(Self.wait)
        var asked = false
        while Date() < end, !terminalText(in: app).contains(line) {
            if unasked, app.staticTexts[Self.heading].exists {
                asked = true
                shot("unexpected-question-\(name)")
                app.buttons["clipboard-deny"].tap()
            }
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: Self.beat)
        }
        let shown = terminalText(in: app)
        XCTAssertTrue(shown.contains(line), "the terminal never showed \(line): \(shown)")
        XCTAssertFalse(asked, "\(name) asked, though the machine is always allowed and the copy is plain")
    }

    // MARK: sign in, the machine, the session

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
        let passphrase = app.secureTextFields["passphrase"]
        passphrase.tap()
        passphrase.typeText(machine.passphrase + "\n")
        app.buttons["import-key-action"].tap()
        FakeApp.expect(app.descendants(matching: .any)["public-key-page"].firstMatch, "the imported key's public half")
        // the system's offer to keep the passphrase in Passwords comes at no fixed time, and takes the next tap
        let page = app.descendants(matching: .any)["public-key-page"].firstMatch
        let done = app.buttons["key-done"]
        FakeApp.expect(done, "the public key page's Done")
        let closed = LogsPage.tap(
            done,
            until: { ScreenRead.isGone(page) },
            in: app,
            "the public key page stayed after Done"
        )
        LogsPage.closeLateOffer(unless: closed, in: app)
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
        FakeApp.expect(connected, "Connected on the machine's row", lasting: Self.wait)
    }

    // New named session on the machine, landing in its terminal; the first attach asks about notifying: not now
    @MainActor
    private func openANewSession(in app: XCUIApplication) {
        app.descendants(matching: .any)["machine-row-\(Self.label)"].firstMatch.tap()
        let new = app.buttons["new-session"]
        FakeApp.expect(new, "New session")
        new.press(forDuration: Self.menuPress)
        let named = app.buttons["new-named-session"]
        FakeApp.expect(named, "New named session in the button's menu")
        named.tap()
        let name = app.textFields["session-name"]
        FakeApp.expect(name, "the New session sheet")
        name.tap()
        name.typeText(Self.session)
        app.buttons["create-session"].tap()
        let later = app.buttons["notifications-later"]
        if later.waitForExistence(timeout: Self.offerWait) {
            later.tap()
        }
        FakeApp.expect(app.descendants(matching: .any)["session-title"].firstMatch, "the new session's view")
    }

    @MainActor
    private func expectTerminal(_ what: String, in app: XCUIApplication, shows: @escaping (String) -> Bool) {
        let terminal = app.textViews.firstMatch
        ScreenRead.expect("the terminal never showed \(what)", lasting: Self.wait) {
            ScreenRead.shows(terminal) { shows(($0.value as? String) ?? "") }
        }
    }

    // the whole screen: the system's "pasted from" banner shows over the app
    @MainActor
    private func shot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func attach(_ name: String, _ text: String) {
        let attachment = XCTAttachment(string: text)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
