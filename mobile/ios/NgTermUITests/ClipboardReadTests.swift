import XCTest

/// A program's clipboard read in the app on the fake core (mobile/android's `ClipboardReadPromptTest`): the sheet asks
/// over whatever shows, passes the accessibility audit, and goes once answered, or swiped away (which denies).
///
/// A second read waits behind the first, so its question showing proves the first was answered.
internal final class ClipboardReadTests: XCTestCase {
    @MainActor
    private func launch() -> XCUIApplication {
        let app = FakeApp.launch(withKey: true, arguments: ["-ngterm-fake-clipboard-read"], for: self)
        FakeApp.expect(app.staticTexts["Let docs-site read your clipboard?"], "clipboard question")
        return app
    }

    @MainActor
    internal func testDenyAnswersAndTheSheetGoes() throws {
        let app = launch()
        XCTAssertTrue(FakeApp.element(containing: "A program in docs-site on build-01", in: app).exists)
        XCTAssertTrue(app.buttons["clipboard-always"].label.contains("build-01"))
        FakeApp.expect(app.staticTexts["npm run deploy -- --prod"], "what the read would hand over")
        XCTAssertTrue(app.switches["clipboard-remember"].exists)
        try FakeApp.auditSheet(app)
        app.buttons["clipboard-deny"].tap()
        FakeApp.expectGone(
            app.staticTexts["Let docs-site read your clipboard?"],
            "the first read's question",
            lasting: 10
        )
        FakeApp.expect(app.staticTexts["Let notes read your clipboard?"], "the next read's question")
        XCTAssertFalse(app.switches["clipboard-remember"].exists, "a read with no password keeps no grant")
    }

    @MainActor
    internal func testASensitiveClipIsHiddenAndAskedForTheOneRead() throws {
        let arguments = ["-ngterm-fake-clipboard-read", "-ngterm-fake-clipboard-sensitive"]
        let app = FakeApp.launch(withKey: true, arguments: arguments, for: self)
        FakeApp.expect(app.staticTexts["Let docs-site read your clipboard?"], "clipboard question")
        XCTAssertTrue(FakeApp.element(containing: "What you copied is marked sensitive", in: app).exists)
        XCTAssertEqual(app.buttons["clipboard-allow"].label, "Allow once")
        XCTAssertFalse(app.buttons["clipboard-always"].exists, "no standing permission over a sensitive clip")
        XCTAssertFalse(app.switches["clipboard-remember"].exists)
        XCTAssertTrue(app.staticTexts["Hidden"].exists)
        XCTAssertFalse(app.staticTexts["npm run deploy -- --prod"].exists)
        try FakeApp.auditSheet(app)
        app.buttons["clipboard-reveal"].tap()
        FakeApp.expect(app.staticTexts["npm run deploy -- --prod"], "what the read would hand over, once asked for")
        app.buttons["clipboard-allow"].tap()
        FakeApp.expectGone(
            app.staticTexts["Let docs-site read your clipboard?"],
            "the first read's question",
            lasting: 10
        )
        FakeApp.expect(app.staticTexts["Let notes read your clipboard?"], "the next read's question")
        XCTAssertEqual(app.buttons["clipboard-allow"].label, "Allow once", "the next read asks again")
    }

    @MainActor
    internal func testSwipingTheSheetAwayDenies() {
        let app = launch()
        // the sheet pulled down from its heading to the screen's foot
        let heading = app.staticTexts["Let docs-site read your clipboard?"]
        let foot = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.98))
        heading.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.05, thenDragTo: foot, withVelocity: .fast, thenHoldForDuration: 0)
        FakeApp.expectGone(
            app.staticTexts["Let docs-site read your clipboard?"],
            "the first read's question",
            lasting: 10
        )
        // the read was answered: the next one's question shows, and the first's doesn't come back
        FakeApp.expect(app.staticTexts["Let notes read your clipboard?"], "the next read's question")
        XCTAssertFalse(app.staticTexts["Let docs-site read your clipboard?"].exists)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
