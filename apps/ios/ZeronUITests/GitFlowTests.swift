import XCTest

final class GitFlowTests: XCTestCase {
    private let pull = "https://github.com/zeron-sh/zeron/pull/90"

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ route: String? = nil) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast"] + (route.map { ["-route", $0] } ?? [])
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func scrollTo(_ element: XCUIElement, in list: XCUIElement) {
        for _ in 0..<12 where !(element.exists && element.isHittable) {
            list.swipeUp()
        }
        XCTAssertTrue(element.isHittable)
    }

    func testNeedsYouListsPullRequestsNeedingAttention() {
        let app = launch()
        let row = app.cells["needs-you-prs"]
        XCTAssertTrue(row.waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["3 pull requests need you"].exists)
        XCTAssertTrue(app.staticTexts["GitHub, 2 to review · 1 need changes · 1 failing CI"].exists)
        snapshot(app, "git-ios-needs-you-prs")
        row.tap()
        XCTAssertTrue(app.tables["pr-board"].waitForExistence(timeout: 10))
        snapshot(app, "git-ios-needs-you-prs-board")

        let accessory = app.buttons["new-session"]
        XCTAssertTrue(accessory.isHittable)
        app.tables["pr-board"].cells.matching(NSPredicate(format: "identifier BEGINSWITH 'pr-row-'")).firstMatch.tap()
        XCTAssertTrue(app.staticTexts["pr-title"].waitForExistence(timeout: 20))
        XCTAssertFalse(accessory.isHittable, "the PR page hides the New session accessory")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.tables["pr-board"].waitForExistence(timeout: 10))
        expectation(for: NSPredicate(format: "isHittable == true"), evaluatedWith: accessory)
        waitForExpectations(timeout: 5)
    }

    func testTheBoardPageAndGitViewsRender() {
        var app = launch("prs")
        XCTAssertTrue(app.tables["pr-board"].cells.matching(NSPredicate(format: "identifier BEGINSWITH 'pr-row-'")).firstMatch.waitForExistence(timeout: 20))
        snapshot(app, "git-ios-board")

        app = launch("pr:\(pull)")
        XCTAssertTrue(app.staticTexts["pr-title"].waitForExistence(timeout: 20))
        snapshot(app, "git-ios-pr-summary")
        let tabs = app.segmentedControls["pr-tabs"]
        tabs.buttons.element(boundBy: 1).tap()
        XCTAssertTrue(app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'diff-file-'")).firstMatch.waitForExistence(timeout: 10))
        snapshot(app, "git-ios-pr-code")
        scrollTo(app.buttons["Unresolve"].firstMatch, in: app.tables["pr-detail"])
        snapshot(app, "git-ios-pr-code-threads")
        tabs.buttons.element(boundBy: 2).tap()
        XCTAssertTrue(app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'pr-activity-'")).firstMatch.waitForExistence(timeout: 10))
        snapshot(app, "git-ios-pr-activity")

        app = launch("git")
        XCTAssertTrue(app.staticTexts["git-branch"].waitForExistence(timeout: 20))
        snapshot(app, "git-ios-working-tree")

        app = launch("git-history")
        let commit = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'git-commit-'")).firstMatch
        XCTAssertTrue(commit.waitForExistence(timeout: 20))
        snapshot(app, "git-ios-history")
        commit.tap()
        XCTAssertTrue(app.tables["git-commit-detail"].waitForExistence(timeout: 10))
        snapshot(app, "git-ios-commit-detail")
    }

    func testACommentIsPosted() {
        let app = launch("pr:\(pull)")
        let tabs = app.segmentedControls["pr-tabs"]
        XCTAssertTrue(tabs.waitForExistence(timeout: 20))
        tabs.buttons.element(boundBy: 2).tap()
        let input = app.textViews["pr-comment"]
        XCTAssertTrue(input.waitForExistence(timeout: 20))
        input.tap()
        input.typeText("Looks good from the phone.")
        snapshot(app, "git-ios-comment-before")
        app.buttons["pr-send"].tap()
        XCTAssertTrue(app.staticTexts["Comment posted"].waitForExistence(timeout: 10))
        snapshot(app, "git-ios-comment-after")
    }

    func testAFileIsStagedAndCommitted() {
        let app = launch("git")
        let stage = app.buttons["git-stage-status.rs"]
        XCTAssertTrue(stage.waitForExistence(timeout: 20))
        snapshot(app, "git-ios-stage-before")
        stage.tap()
        XCTAssertTrue(app.buttons["git-unstage-status.rs"].waitForExistence(timeout: 10))
        snapshot(app, "git-ios-stage-after")

        app.buttons["git-commit"].tap()
        let message = app.textViews["git-commit-message"]
        XCTAssertTrue(message.waitForExistence(timeout: 10))
        message.tap()
        message.typeText("Show sync status on the session row")
        snapshot(app, "git-ios-commit-before")
        app.buttons["git-commit-go"].tap()
        XCTAssertTrue(message.waitForNonExistence(timeout: 10))
        snapshot(app, "git-ios-commit-after")
    }

    func testPushAsksFirst() {
        let app = launch("git")
        let push = app.buttons["git-push"]
        XCTAssertTrue(push.waitForExistence(timeout: 20))
        snapshot(app, "git-ios-push-before")
        push.tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        XCTAssertTrue(alert.buttons["Cancel"].exists)
        snapshot(app, "git-ios-push-confirm")
        alert.buttons["Push"].tap()
        XCTAssertTrue(alert.waitForNonExistence(timeout: 10))
        snapshot(app, "git-ios-push-after")
    }

    func testMergeAsksFirst() {
        let app = launch("pr:\(pull)")
        let detail = app.tables["pr-detail"]
        XCTAssertTrue(detail.waitForExistence(timeout: 20))
        let merge = app.cells["pr-merge-squash"]
        scrollTo(merge, in: detail)
        snapshot(app, "git-ios-merge-before")
        merge.tap()
        let alert = app.alerts.firstMatch
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        XCTAssertTrue(alert.buttons["Cancel"].exists)
        snapshot(app, "git-ios-merge-confirm")
        alert.buttons["Squash and merge"].tap()
        XCTAssertTrue(app.staticTexts["Pull request merged"].waitForExistence(timeout: 10))
        snapshot(app, "git-ios-merge-after")
    }
}
