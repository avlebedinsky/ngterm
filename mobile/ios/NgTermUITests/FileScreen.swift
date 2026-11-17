import XCTest

/// The system's file screen: the one a key's export saves through, and the one Import a key's Choose file opens.
///
/// Each wait reads the screen again and again, a beat apart, and gives up only after a number of reads, never after a
/// time alone. On a loaded runner one read can take as long as a whole timed wait: the first read after the screen
/// closed waited 29 s on the closing screen's process (ExportKeyTests on CI, "the file screen stayed"), though the
/// screen was gone in 6 s. A step that did not take is done again after some reads. A wait that fails attaches the
/// app's hierarchy and a screenshot.
///
/// The import's screen opens where it was last, often on Recents. Recents shows a new file only after the system
/// indexes it, so a file saved some seconds before can be missing there (ExportKeyTests on CI, phone and tablet). The
/// export's save puts its file in the device's own storage (On My iPhone, On My iPad), so a test opens that place.
internal enum FileScreen {
    // how often the screen is read, the fewest reads a wait gives up after, and the reads after which a step that did
    // not take is done again
    private static let beat: TimeInterval = 0.5
    private static let reads = 60
    private static let retry = 15
    // the device's own storage among the places: its sidebar row on an iPad, its row in a phone's Browse list
    private static let ownStorage = "identifier BEGINSWITH 'DOC.sidebar.item.On My ' OR label BEGINSWITH 'On My '"
    // where a drag of the screen's bar starts, and how long it is held first
    private static let middle: CGFloat = 0.5
    private static let dragPress: TimeInterval = 0.1

    /// Taps `opener`, then waits for the export's file screen to take taps.
    ///
    /// The screen takes taps once its Save is enabled and still. It can take seconds to lay out, and a tap that lands
    /// while its toolbar still moves opens a neighbouring button's menu instead. `opener` is tapped again while the
    /// screen hasn't come and `opener` takes taps.
    @MainActor
    @discardableResult
    internal static func open(with opener: XCUIElement, in app: XCUIApplication) -> Bool {
        opener.tap()
        let place = saveButton(in: app)
        var before = CGRect.null
        return read("the system's file screen", in: app) {
            guard place.exists else {
                return false
            }
            let now = place.frame
            defer { before = now }
            return place.isEnabled && place.isHittable && now == before
        } again: {
            if !place.exists, opener.isHittable {
                opener.tap()
            }
        }
    }

    /// Taps the export's file screen's Save, then waits for the screen to go.
    ///
    /// The screen is gone once `back`, on the screen under it, takes taps again. Save is tapped again while it stays
    /// enabled and takes taps.
    @MainActor
    internal static func save(in app: XCUIApplication, back: XCUIElement) {
        let place = saveButton(in: app)
        place.tap()
        read("the system's file screen gone after its Save", in: app) {
            back.isHittable
        } again: {
            if place.exists, place.isEnabled, place.isHittable {
                place.tap()
            }
        }
    }

    /// Leaves the export's file screen without a place for the file, then waits for the screen to go.
    ///
    /// The screen is gone once `back`, on the screen under it, takes taps again. It is left again while it stays.
    @MainActor
    internal static func cancel(in app: XCUIApplication, back: XCUIElement) {
        leave(app)
        read("the system's file screen gone after its Cancel", in: app) {
            back.isHittable
        } again: {
            if saveButton(in: app).exists {
                leave(app)
            }
        }
    }

    /// Taps `opener`, then the file whose name starts with `prefix` in the device's own storage, and waits for `back`.
    ///
    /// `back` is what the app shows once it has the file.
    /// An iPad shows the places in its sidebar, where the open place is selected. A phone shows them under its
    /// Browse tab: once that tab is selected, its list shows the places, or the place it showed last.
    @MainActor
    internal static func choose(
        fileStartingWith prefix: String,
        opening opener: XCUIElement,
        then back: XCUIElement,
        in app: XCUIApplication
    ) {
        opener.tap()
        let file = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", prefix))
            .firstMatch
        let found = read("the file \(prefix) in the device's own storage", in: app) {
            ownStorageIsOpen(in: app) && file.exists && file.isHittable
        } again: {
            if !ownStorageIsShown(in: app), opener.isHittable {
                opener.tap()
            }
        }
        guard found else {
            return
        }
        file.tap()
        read("what the app shows once it has the file", in: app) {
            back.exists
        } again: {
            if file.exists, file.isHittable {
                file.tap()
            }
        }
    }

    // the export's file screen's Save, which places the file
    @MainActor
    private static func saveButton(in app: XCUIApplication) -> XCUIElement {
        app.buttons["DOCPicker.actionButton"]
    }

    // the screen's Cancel where it has one, else the sheet swiped down by its bar, as a user lets it go
    @MainActor
    private static func leave(_ app: XCUIApplication) {
        let cancel = app.buttons["Cancel"].firstMatch
        if cancel.exists {
            cancel.tap()
            return
        }
        let bar = app.navigationBars["FullDocumentManagerViewControllerNavigationBar"]
        guard bar.exists else {
            return
        }
        let start = bar.coordinate(withNormalizedOffset: CGVector(dx: middle, dy: middle))
        let foot = app.coordinate(withNormalizedOffset: CGVector(dx: middle, dy: 1))
        start.press(forDuration: dragPress, thenDragTo: foot)
    }

    // whether the place list shows at all: the iPad's sidebar row, or the phone's Browse tab
    @MainActor
    private static func ownStorageIsShown(in app: XCUIApplication) -> Bool {
        ownStorageRow(in: app).exists || app.tabBars.buttons["Browse"].exists
    }

    // opens the device's own storage, a tap at most per read; true once it is open
    @MainActor
    private static func ownStorageIsOpen(in app: XCUIApplication) -> Bool {
        let own = ownStorageRow(in: app)
        let tab = own.exists ? own : app.tabBars.buttons["Browse"]
        guard tab.exists, tab.isHittable else {
            return false
        }
        if tab.isSelected {
            return true
        }
        tab.tap()
        return false
    }

    @MainActor
    private static func ownStorageRow(in app: XCUIApplication) -> XCUIElement {
        app.cells.matching(NSPredicate(format: ownStorage)).firstMatch
    }

    // reads `done` until it holds, a beat apart, and runs `again` after each `retry` reads; after `reads` reads it
    // fails the test with the app's hierarchy and a screenshot
    @MainActor
    @discardableResult
    private static func read(
        _ what: String,
        in app: XCUIApplication,
        until done: () -> Bool,
        again: () -> Void
    ) -> Bool {
        for count in 1...reads {
            if done() {
                return true
            }
            if count.isMultiple(of: retry) {
                again()
            }
            _ = XCTWaiter.wait(for: [XCTestExpectation(description: "a beat")], timeout: beat)
        }
        XCTContext.runActivity(named: "no \(what)") { activity in
            let tree = XCTAttachment(string: app.debugDescription)
            tree.name = "hierarchy"
            tree.lifetime = .keepAlways
            activity.add(tree)
            let shot = XCTAttachment(screenshot: app.screenshot())
            shot.name = "screen"
            shot.lifetime = .keepAlways
            activity.add(shot)
        }
        XCTFail("no \(what) after \(reads) reads")
        return false
    }
}
