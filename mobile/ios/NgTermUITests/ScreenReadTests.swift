import XCTest

/// The UI tests' waits (``ScreenRead``): what counts as one of their reads, and what tells them an element has gone.
///
/// On a loaded runner a read of the screen can take many seconds and then fail. A wait must count only the reads that
/// completed, and an element has gone only when a read completed and found no match.
internal final class ScreenReadTests: XCTestCase {
    // the error XCTest gives a read that matched nothing, and the one it gives a read of an app that does not run
    private static let uiTestingErrors = "com.apple.dt.xctest.ui-testing.error"
    private static let noMatches = 10_008
    private static let notRunning = 10_001
    // an app that is not installed, so no read of it completes
    private static let noApp = "com.nickalie.ngterm.no-such-app"

    @MainActor
    internal func testAReadThatFailedIsNotOneOfTheWaitsReads() {
        var calls = 0
        let polled = ScreenRead.Wait(reads: 3, beat: 0).poll {
            calls += 1
            return calls <= 2 ? .failed : .doesNotHold
        }
        XCTAssertEqual(polled, ScreenRead.Polled(held: false, reads: 3, failed: 2, seconds: polled.seconds))
        XCTAssertEqual(calls, 5)
    }

    @MainActor
    internal func testReadsThatAllFailEndTheWaitOnlyAtItsTimeLimit() {
        let polled = ScreenRead.Wait(reads: 3, beat: 0, limit: 1).poll { ScreenRead.Answer.failed }
        XCTAssertFalse(polled.held)
        XCTAssertEqual(polled.reads, 0)
        XCTAssertGreaterThanOrEqual(polled.seconds, 1)
    }

    @MainActor
    internal func testAWaitReadsForItsTimeAtLeast() {
        let polled = ScreenRead.Wait(reads: 1, lasting: 1, beat: 0.1).poll { ScreenRead.Answer.doesNotHold }
        XCTAssertFalse(polled.held)
        XCTAssertGreaterThanOrEqual(polled.seconds, 1)
        XCTAssertGreaterThan(polled.reads, 1)
    }

    @MainActor
    internal func testAWaitEndsAtTheReadThatHolds() {
        var calls = 0
        let polled = ScreenRead.Wait(beat: 0).poll {
            calls += 1
            return ScreenRead.Answer(calls == 2)
        }
        XCTAssertTrue(polled.held)
        XCTAssertEqual(calls, 2)
    }

    internal func testOnlyAReadThatMatchedNothingReadsAbsent() {
        let noMatch = NSError(domain: Self.uiTestingErrors, code: Self.noMatches)
        XCTAssertEqual(ScreenRead.reading(after: noMatch), .absent)
        let appGone = NSError(domain: Self.uiTestingErrors, code: Self.notRunning)
        XCTAssertEqual(ScreenRead.reading(after: appGone), .failed)
        XCTAssertEqual(ScreenRead.reading(after: NSError(domain: NSCocoaErrorDomain, code: Self.noMatches)), .failed)
    }

    // on the app, a read finds what shows and reads absent what does not: the error XCTest gives is the one
    // `reading(after:)` takes for no match
    @MainActor
    internal func testAReadOfTheAppFindsWhatShowsAndWhatDoesNot() {
        let app = FakeApp.launch(withKey: true, for: self)
        FakeApp.showMachines(in: app)
        let add = app.buttons["add-machine"]
        XCTAssertEqual(ScreenRead.reading(add), .shown)
        XCTAssertEqual(ScreenRead.isGone(add), .doesNotHold)
        XCTAssertEqual(ScreenRead.shows(add, matching: NSPredicate(format: "identifier == 'add-machine'")), .holds)
        let missing = app.descendants(matching: .any)["no-such-element"].firstMatch
        XCTAssertEqual(ScreenRead.reading(missing), .absent)
        XCTAssertEqual(ScreenRead.isGone(missing), .holds)
        XCTAssertEqual(ScreenRead.shows(missing) { _ in true }, .doesNotHold)
        XCTAssertEqual(ScreenRead.reading(app.windows.element(boundBy: 99)), .absent)
        FakeApp.expectGone(missing, "an element that never showed")
    }

    // a read that fails says that the element does not exist, as a read that found none does: only the read of the
    // snapshot tells them apart
    @MainActor
    internal func testAReadThatFailedTellsNothing() {
        let elsewhere = XCUIApplication(bundleIdentifier: Self.noApp).buttons["add-machine"]
        XCTAssertFalse(elsewhere.exists)
        XCTAssertEqual(ScreenRead.reading(elsewhere), .failed)
        XCTAssertEqual(ScreenRead.isGone(elsewhere), .failed)
        XCTAssertEqual(ScreenRead.shows(elsewhere) { _ in true }, .failed)
        let polled = ScreenRead.Wait(reads: 1, beat: 0, limit: 1).poll { ScreenRead.isGone(elsewhere) }
        XCTAssertEqual(polled.reads, 0)
        XCTAssertGreaterThan(polled.failed, 0)
    }

    deinit {
        // nothing to release: XCTest owns the app it launched
    }
}
