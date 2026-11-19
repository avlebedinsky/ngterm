import XCTest

/// A session opened with a hardware keyboard takes the keyboard at once, with no tap on it.
///
/// As Android's `FollowFocus` does, its selected pane's terminal takes it, or its chat's prompt field when the pane
/// shows its chat. The fake core's `split` (`-ngterm-fake-session`): each pane names itself on its first line ("P1
/// ready") and echoes what it was typed on its second ("$ ls"); New session there opens `session 1`, of one pane, `P1`
/// too. The simulator has the Mac's keyboard attached, so each test runs with a hardware keyboard; the session moves'
/// chords chaining is SessionViewTests'.
internal final class SessionKeyboardTests: XCTestCase {
    private static let wait: TimeInterval = 15

    // the terminal whose screen names the pane `pane` and shows `typed` after its prompt (VoiceOver's value of it,
    // which leaves out a line's trailing space)
    @MainActor
    private static func screen(_ pane: String, in app: XCUIApplication, typed: String = "") -> XCUIElement {
        let prompt = typed.isEmpty ? "$" : "$ \(typed)"
        let shows = NSPredicate(format: "value CONTAINS %@ AND value CONTAINS %@", "\(pane) ready", prompt)
        return app.textViews.matching(shows).firstMatch
    }

    // waits until the terminal of the pane `pane` has the keyboard's focus, which it takes as it comes on: typing
    // before that would go nowhere
    @MainActor
    private static func expectFocus(on pane: String, in app: XCUIApplication, _ what: String) {
        let focused = NSPredicate(format: "value CONTAINS %@ AND hasKeyboardFocus == true", "\(pane) ready")
        FakeApp.expect(app.textViews.matching(focused).firstMatch, "the keyboard's focus on \(what), with no tap")
    }

    // as `expectFocus(on:in:_:)`, for a pane that shares its name with another session's (`work`'s first pane and the
    // new session's are both `P1`): the one that was not typed `other`
    @MainActor
    private static func expectFocus(on pane: String, notTyped other: String, in app: XCUIApplication, _ what: String) {
        let focused = NSPredicate(
            format: "value CONTAINS %@ AND NOT (value CONTAINS %@) AND hasKeyboardFocus == true",
            "\(pane) ready",
            other
        )
        FakeApp.expect(app.textViews.matching(focused).firstMatch, "the keyboard's focus on \(what), with no tap")
    }

    // types `text` key by key on the hardware keyboard, to whatever has the focus
    @MainActor
    private static func type(_ text: String, in app: XCUIApplication) {
        for character in text {
            app.typeKey(String(character), modifierFlags: [])
        }
    }

    // no soft keyboard came up: the hardware one types
    @MainActor
    private static func expectNoSoftKeyboard(in app: XCUIApplication, _ when: String) {
        XCTAssertEqual(app.keyboards.count, 0, "a soft keyboard came up \(when)")
    }

    // opens the session named `name` again by its row in the machine's sessions: a phone goes back to them first, a
    // tablet lists them beside the session
    @MainActor
    private static func reopen(_ name: String, in app: XCUIApplication) {
        let row = app.descendants(matching: .any)["session-\(name)"].firstMatch
        if !row.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(row, "\(name)'s row")
        row.tap()
    }

    // waits until the chat's prompt field has the keyboard's focus: typing before that would go nowhere
    @MainActor
    private static func expectPromptFocus(in app: XCUIApplication, _ what: String) {
        let focused = NSPredicate(format: "identifier == 'chat-prompt' AND hasKeyboardFocus == true")
        FakeApp.expect(app.descendants(matching: .any).matching(focused).firstMatch, "the prompt's focus on \(what)")
    }

    // the chat's prompt field takes the focus, then the typing, its first letter a capital as the field starts a
    // sentence
    @MainActor
    private static func expectTyping(in app: XCUIApplication, _ what: String) {
        expectPromptFocus(in: app, what)
        type("hi", in: app)
        let typed = NSPredicate(format: "identifier == 'chat-prompt' AND value ==[c] 'hi' AND hasKeyboardFocus == true")
        FakeApp.expect(
            app.descendants(matching: .any).matching(typed).firstMatch,
            "the typing in the prompt field on \(what)",
            lasting: wait
        )
    }

    // the app on `split`'s sessions, past the machines
    @MainActor
    private func openSplit(arguments: [String]) -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-session"] + arguments, for: self)
        FakeApp.showMachines(in: app)
        FakeApp.expectRow("split", reads: "Connected", in: app)
        app.descendants(matching: .any)["machine-row-split"].firstMatch.tap()
        return app
    }

    // a tap on `work`'s row opens it on its first pane, which takes the typing; a tablet's split shows the second pane
    // beside it, which takes none; then New session opens a session whose pane takes the typing in its turn, and
    // `work` opened again by its row takes it back
    @MainActor
    internal func testASessionOpenedByItsRowOrNewSessionTakesTheTypingAtOnce() {
        let app = openSplit(arguments: [])
        let row = app.descendants(matching: .any)["session-work"].firstMatch
        FakeApp.expect(row, "work's row")
        row.tap()
        FakeApp.expect(Self.screen("P1", in: app), "work's first pane")
        Self.expectFocus(on: "P1", in: app, "work's first pane")
        Self.type("ls", in: app)
        FakeApp.expect(Self.screen("P1", in: app, typed: "ls"), "the typing on work's first pane, with no tap")
        XCTAssertFalse(Self.screen("P2", in: app, typed: "l").exists, "the pane not selected took the typing")
        Self.expectNoSoftKeyboard(in: app, "on work")

        // a phone goes back from the session to the machine's sessions; a tablet shows them beside it
        let newSession = app.buttons["new-session"]
        if !newSession.exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        FakeApp.expect(newSession, "New session")
        newSession.tap()
        FakeApp.expect(Self.screen("P1", in: app), "the new session's pane")
        Self.expectFocus(on: "P1", notTyped: "ls", in: app, "the new session's pane")
        Self.type("pwd", in: app)
        FakeApp.expect(Self.screen("P1", in: app, typed: "pwd"), "the typing on the new session's pane, with no tap")
        Self.expectNoSoftKeyboard(in: app, "on the new session")

        // each opened again by its row takes the typing again: work, then the new session (`session 1`)
        Self.reopen("work", in: app)
        FakeApp.expect(Self.screen("P1", in: app), "work's first pane again")
        Self.expectFocus(on: "P1", notTyped: "pwd", in: app, "work opened again")
        Self.type("cd", in: app)
        let again = NSPredicate(
            format: "value CONTAINS 'P1 ready' AND value CONTAINS 'cd' AND NOT (value CONTAINS 'pwd')"
        )
        FakeApp.expect(app.textViews.matching(again).firstMatch, "the typing on work opened again, with no tap")
        Self.reopen("session 1", in: app)
        FakeApp.expect(Self.screen("P1", in: app, typed: "pwd"), "the new session's pane again")
        Self.expectFocus(on: "P1", notTyped: "cd", in: app, "the new session opened again")
        Self.type("ls", in: app)
        FakeApp.expect(Self.screen("P1", in: app, typed: "pwdls"), "the typing on the new session opened again")
    }

    // `work`'s first pane runs Claude Code, so the session opens on its chat: the prompt field takes the typing, its
    // first letter a capital as the field starts a sentence. Whether the system keyboard comes up is the system's
    // call for a text field, and a simulator run without Simulator.app has no hardware keyboard by UIKit's account
    // (the app's terminals go by GameController's), so the chat has no soft-keyboard check
    @MainActor
    internal func testASessionOpenedOnItsChatTakesTheTypingInItsPrompt() {
        let app = openSplit(arguments: ["-ngterm-fake-chat"])
        app.descendants(matching: .any)["session-work"].firstMatch.tap()
        Self.expectTyping(in: app, "the chat opened first")
    }

    // `work` opens on its chat; Show terminal hands the prompt field's keyboard to the pane's terminal, which takes
    // the keys typed at once, and Show chat hands it back to the field, the pane keeping one terminal under its chat
    @MainActor
    internal func testAPaneTurningToItsTerminalAndBackKeepsTheTyping() {
        let app = openSplit(arguments: ["-ngterm-fake-chat"])
        app.descendants(matching: .any)["session-work"].firstMatch.tap()
        Self.expectPromptFocus(in: app, "the chat opened first")
        let viewSwitch = app.descendants(matching: .any)["session-view-switch"].firstMatch
        FakeApp.expect(viewSwitch, "Show terminal")
        viewSwitch.tap()
        Self.type("ls", in: app)
        FakeApp.expect(Self.screen("P1", in: app, typed: "ls"), "the typing on the terminal, with no tap")
        Self.expectFocus(on: "P1", in: app, "the terminal shown in the chat's place")
        viewSwitch.tap()
        Self.expectTyping(in: app, "the chat shown again")
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
