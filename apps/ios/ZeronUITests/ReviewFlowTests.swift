import XCTest

final class ReviewFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "chat:chat-tabs"]
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func chipShot(_ app: XCUIApplication, _ name: String) {
        let chips = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'composer-chip-'"))
        if !app.buttons["composer-chip-review"].isHittable,
           let visible = chips.allElementsBoundByIndex.first(where: { $0.isHittable }) {
            visible.swipeLeft()
        }
        snapshot(app, name)
    }

    private func reviewChip(_ app: XCUIApplication) -> XCUIElement {
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 15))
        input.tap()
        let chip = app.buttons["composer-chip-review"]
        XCTAssertTrue(chip.waitForExistence(timeout: 15))
        return chip
    }

    private func matching(_ query: XCUIElementQuery, prefix: String) -> XCUIElementQuery {
        query.matching(NSPredicate(format: "identifier BEGINSWITH %@", prefix))
    }

    func testFindingsSheetDismissAndDiffCardFix() {
        let app = launch()
        let chip = reviewChip(app)
        XCTAssertTrue(chip.label.contains("3 findings"), chip.label)
        chipShot(app, "review-ios-chip-findings")

        chip.tap()
        let list = app.tables["review-findings"]
        XCTAssertTrue(list.waitForExistence(timeout: 10))
        XCTAssertTrue(app.navigationBars["Findings"].exists)
        XCTAssertTrue(list.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "1 fixed")).firstMatch.exists)
        XCTAssertEqual(matching(list.cells, prefix: "review-finding-").count, 6)
        snapshot(app, "review-ios-findings")

        let dismissButtons = matching(list.buttons, prefix: "review-dismiss-")
        XCTAssertEqual(dismissButtons.count, 3)
        list.buttons["review-dismiss-F6"].tap()
        expectation(for: NSPredicate(format: "count == 2"), evaluatedWith: dismissButtons)
        waitForExpectations(timeout: 5)

        list.cells["review-finding-F1"].tap()
        let card = app.cells["diff-finding-F1"]
        XCTAssertTrue(card.waitForExistence(timeout: 15))
        XCTAssertTrue(card.isHittable)
        snapshot(app, "review-ios-diff-card")
        app.buttons["finding-fix-F1"].tap()
        app.navigationBars.buttons.matching(NSPredicate(format: "label ==[c] 'close'")).firstMatch.tap()

        let fixAll = app.buttons["review-fix-all"]
        XCTAssertTrue(fixAll.waitForExistence(timeout: 5))
        fixAll.tap()
        let staged = app.buttons["composer-comments"]
        XCTAssertTrue(staged.waitForExistence(timeout: 10))
        XCTAssertTrue(staged.label.contains("2 comments"), staged.label)
        snapshot(app, "review-ios-staged-comments")
        staged.tap()
        let row = app.collectionViews.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Status cache")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        snapshot(app, "review-ios-staged-menu")
        row.tap()
        expectation(for: NSPredicate(format: "label CONTAINS %@", "1 comment"), evaluatedWith: staged)
        waitForExpectations(timeout: 5)
    }

    func testReviewOptionsStartAReview() {
        let app = launch()
        _ = reviewChip(app)
        let menu = app.buttons["session-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10))
        menu.tap()
        let entry = app.buttons["Review changes"]
        XCTAssertTrue(entry.waitForExistence(timeout: 5))
        entry.tap()
        let options = app.tables["review-options"]
        XCTAssertTrue(options.waitForExistence(timeout: 10))
        XCTAssertTrue(options.cells["review-scope-uncommitted"].waitForExistence(timeout: 10))
        snapshot(app, "review-ios-options")

        options.cells["review-scope-commit"].tap()
        let commits = app.tables["review-commits"]
        XCTAssertTrue(commits.cells["review-commit-0"].waitForExistence(timeout: 10))
        snapshot(app, "review-ios-commits")
        commits.cells["review-commit-0"].tap()
        XCTAssertTrue(options.waitForExistence(timeout: 5))
        let start = app.buttons["review-start"]
        expectation(for: NSPredicate(format: "label BEGINSWITH 'Review '"), evaluatedWith: start)
        waitForExpectations(timeout: 10)

        options.cells["review-scope-uncommitted"].tap()
        let models = matching(options.buttons, prefix: "review-reviewer-models-").firstMatch
        XCTAssertTrue(models.waitForExistence(timeout: 10))
        models.tap()
        let page = app.tables["review-models"]
        XCTAssertTrue(page.cells["review-model-0-1"].waitForExistence(timeout: 10))
        snapshot(app, "review-ios-models")
        page.cells["review-model-0-1"].tap()
        app.navigationBars.buttons.element(boundBy: 0).tap()

        start.tap()
        let chip = app.buttons["composer-chip-review"]
        expectation(for: NSPredicate(format: "label BEGINSWITH 'Reviewing'"), evaluatedWith: chip)
        waitForExpectations(timeout: 10)
        chipShot(app, "review-ios-chip-reviewing")
        expectation(for: NSPredicate(format: "label CONTAINS 'finding'"), evaluatedWith: chip)
        waitForExpectations(timeout: 20)
        chipShot(app, "review-ios-chip-done")
    }

    func testSessionInfoChangesRowOpensTheDiffWithCards() {
        let app = launch()
        _ = reviewChip(app)
        let menu = app.buttons["session-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10))
        menu.tap()
        let info = app.buttons["Session Info"]
        XCTAssertTrue(info.waitForExistence(timeout: 5))
        info.tap()
        let row = app.cells["info-changes"]
        XCTAssertTrue(row.waitForExistence(timeout: 15))
        XCTAssertTrue(row.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "file")).firstMatch.exists)
        snapshot(app, "review-ios-info-changes")
        row.tap()
        XCTAssertTrue(app.tables["diff-sheet"].waitForExistence(timeout: 10))
        XCTAssertTrue(matching(app.cells, prefix: "diff-finding-").firstMatch.waitForExistence(timeout: 15))
        snapshot(app, "review-ios-info-changes-diff")
    }

    func testCodeReviewSettingsPageFromTheMacPage() {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "page:mac"]
        app.launch()
        let link = app.cells["host-review"]
        XCTAssertTrue(link.waitForExistence(timeout: 15))
        link.tap()
        let reviewer = app.cells["host-review:reviewer"]
        XCTAssertTrue(reviewer.waitForExistence(timeout: 10))
        XCTAssertTrue(app.cells["host-instructions"].exists)
        snapshot(app, "review-ios-settings")
        reviewer.tap()
        XCTAssertTrue(app.cells["host-reviewer:auto"].waitForExistence(timeout: 10))
        snapshot(app, "review-ios-settings-reviewer")
    }

    func testAgentsSheetOpensTheReviewerFindings() {
        let app = launch()
        _ = reviewChip(app)
        let menu = app.buttons["session-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10))
        menu.tap()
        let agents = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Agents'")).firstMatch
        XCTAssertTrue(agents.waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "1 fixed")).firstMatch.exists)
        snapshot(app, "review-ios-agents-pill")
        agents.tap()
        let row = app.cells["agents-row-0"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        snapshot(app, "review-ios-agents-sheet")
        row.tap()
        XCTAssertTrue(app.tables["review-findings"].waitForExistence(timeout: 10))
    }
}
