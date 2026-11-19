import XCTest

/// The launch smoke test and the frame's accessibility audit, on the fake core.
internal final class LaunchTests: XCTestCase {
    // how many swipes the form may take to bring its last row into view
    private static let swipes = 12

    @MainActor
    private func launch() -> XCUIApplication {
        // with a key, so the first run is over (FirstRunTests walk through it)
        let app = FakeApp.launch(withKey: true, for: self)
        let firstScreen = app.buttons
            .matching(NSPredicate(format: "identifier == 'settings' OR label == 'Show machines'"))
            .firstMatch
        FakeApp.expect(firstScreen, "first screen", lasting: 30)
        return app
    }

    // an iPad held upright starts on the sessions column, its machines hidden: Show machines brings them in
    @MainActor
    private func openSettings(in app: XCUIApplication) {
        let settings = app.buttons["settings"]
        if !settings.exists {
            app.buttons["Show machines"].tap()
            FakeApp.expect(settings, "Settings in the machines' column", lasting: 10)
        }
        settings.tap()
        FakeApp.expect(app.navigationBars["Settings"], "Settings", lasting: 10)
    }

    @MainActor
    internal func testLaunchesOnTheFakeCore() {
        let app = launch()
        openSettings(in: app)
        // the version is the form's last row: a list makes its rows as they come into view
        let version = app.descendants(matching: .any)["core-version"]
        let form = app.collectionViews["settings-form"]
        for _ in 0..<Self.swipes where !version.exists {
            form.swipeUp()
        }
        FakeApp.expect(version, "the core's version", lasting: 10)
        XCTAssertTrue(version.label.contains("0.0.0-fake"), version.label)
    }

    @MainActor
    internal func testTheFramePassesTheAccessibilityAudit() throws {
        let app = launch()
        try FakeApp.audit(app)
        openSettings(in: app)
        // Settings is a form, audited as the machine form is; on an iPad on its side it runs past the sheet's foot
        try FakeApp.auditForm(app, page: app.collectionViews["settings-form"])
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
