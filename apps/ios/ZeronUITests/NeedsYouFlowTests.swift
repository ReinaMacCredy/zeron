import XCTest

final class NeedsYouFlowTests: XCTestCase {
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

    private func row(_ app: XCUIApplication, _ fragment: String) -> XCUIElement {
        expand(app)
        return app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'needs-you-' AND identifier CONTAINS %@", fragment)).firstMatch
    }

    private func expand(_ app: XCUIApplication) {
        let grabber = app.buttons["Sheet Grabber"]
        if grabber.exists, grabber.value as? String != "Full screen" { grabber.tap() }
    }

    private func scrollTo(_ element: XCUIElement, in list: XCUIElement) {
        for _ in 0..<8 where !(element.exists && element.isHittable) {
            list.swipeUp()
        }
    }

    private func openSheet(_ app: XCUIApplication) -> XCUIElement {
        let chip = app.buttons["needs-you-chip"]
        XCTAssertTrue(chip.waitForExistence(timeout: 20))
        chip.tap()
        let list = app.tables["needs-you-list"]
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        return list
    }

    func testAQuestionIsAnsweredFromTheChipSheet() {
        let app = launch()
        XCTAssertTrue(app.buttons["needs-you-chip"].waitForExistence(timeout: 20))
        snapshot(app, "sessions-needs-you")
        let list = openSheet(app)
        snapshot(app, "sheet")
        let picker = row(app, "chat-picker")
        scrollTo(picker, in: list)
        picker.tap()
        let first = app.buttons["question-option-0"]
        XCTAssertTrue(first.waitForExistence(timeout: 5))
        snapshot(app, "question")
        first.tap()
        let harness = app.buttons["question-option-1"]
        XCTAssertTrue(harness.waitForExistence(timeout: 5))
        harness.tap()
        app.buttons["question-next"].tap()
        XCTAssertTrue(list.waitForExistence(timeout: 5))
        XCTAssertTrue(picker.waitForNonExistence(timeout: 10))
        snapshot(app, "question-answered")
    }

    func testAnApprovalIsApprovedFromTheSheet() {
        let app = launch()
        let list = openSheet(app)
        let approval = row(app, "chat-approve-test")
        scrollTo(approval, in: list)
        approval.tap()
        let approve = app.buttons["question-option-0"]
        XCTAssertTrue(approve.waitForExistence(timeout: 5))
        snapshot(app, "approval")
        approve.tap()
        XCTAssertTrue(approval.waitForNonExistence(timeout: 10))
        snapshot(app, "approval-answered")
    }

    func testADestructiveApprovalIsAnsweredInItsThread() {
        let app = launch()
        let list = openSheet(app)
        let approval = row(app, "chat-approve-push")
        scrollTo(approval, in: list)
        approval.tap()
        let open = app.buttons["question-open-thread"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["question-option-0"].exists)
        snapshot(app, "destructive-approval")
        open.tap()
        let approve = app.buttons["question-option-0"]
        XCTAssertTrue(approve.waitForExistence(timeout: 10))
        snapshot(app, "destructive-approval-thread")
        approve.tap()
        XCTAssertTrue(approve.waitForNonExistence(timeout: 10))
        snapshot(app, "destructive-approval-answered")
    }

    func testAStepQuestionIsAnsweredFromTheSheet() {
        let app = launch()
        let list = openSheet(app)
        let step = row(app, "chat-shelf-4")
        scrollTo(step, in: list)
        step.tap()
        let first = app.buttons["question-option-0"]
        XCTAssertTrue(first.waitForExistence(timeout: 5))
        snapshot(app, "step-question")
        first.tap()
        XCTAssertTrue(step.waitForNonExistence(timeout: 10))
        snapshot(app, "step-question-answered")
    }

    func testADecisionThatFailsKeepsItsPickAndSendsAgain() {
        let app = launch()
        let list = openSheet(app)
        let settle = row(app, "item:item-settle")
        scrollTo(settle, in: list)
        settle.tap()
        let group = app.tables["needs-you-group"]
        XCTAssertTrue(group.waitForExistence(timeout: 5))
        let keep = app.cells["needs-you-option-dev-mini/item:item-settle-0"]
        scrollTo(keep, in: group)
        keep.tap()
        snapshot(app, "decision-picked")
        let send = app.buttons["needs-you-send"]
        scrollTo(send, in: group)
        send.tap()
        let failed = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Could not send'")).firstMatch
        XCTAssertTrue(failed.waitForExistence(timeout: 10))
        snapshot(app, "decision-failed")
        scrollTo(send, in: group)
        XCTAssertTrue(send.isEnabled)
        send.tap()
        XCTAssertTrue(keep.waitForNonExistence(timeout: 10))
        snapshot(app, "decision-sent")
    }

    func testAPlanIsApprovedFromTheSheet() {
        let app = launch()
        _ = openSheet(app)
        let plans = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Plans'")).firstMatch
        XCTAssertTrue(plans.waitForExistence(timeout: 5))
        plans.tap()
        let plan = row(app, "plan:")
        XCTAssertTrue(plan.waitForExistence(timeout: 5))
        snapshot(app, "sheet-plans")
        plan.tap()
        let approve = app.buttons["needs-you-approve-plan"]
        XCTAssertTrue(approve.waitForExistence(timeout: 5))
        snapshot(app, "plan")
        approve.tap()
        XCTAssertTrue(plan.waitForNonExistence(timeout: 10))
        snapshot(app, "plan-approved")
    }

    func testThePillSitsAboveTheKeyboardAndOpensTheSheet() {
        let app = launch("chat:chat-blog")
        let pill = app.buttons["needs-you-pill"]
        XCTAssertTrue(pill.waitForExistence(timeout: 20))
        snapshot(app, "pill")
        let composer = app.textViews["composer-input"]
        XCTAssertTrue(composer.waitForExistence(timeout: 5))
        composer.tap()
        _ = app.keyboards.firstMatch.waitForExistence(timeout: 5)
        XCTAssertTrue(pill.isHittable)
        snapshot(app, "pill-keyboard")
        pill.tap()
        XCTAssertTrue(app.tables["needs-you-list"].waitForExistence(timeout: 5))
        snapshot(app, "pill-sheet")
    }

    func testTheSupervisorOpensTheSharedSheet() {
        let app = launch("tasks:supervisor")
        let page = app.collectionViews["supervisor"]
        XCTAssertTrue(page.waitForExistence(timeout: 15))
        XCTAssertTrue(app.cells["sv-host:dev-mac"].waitForExistence(timeout: 15))
        snapshot(app, "supervisor")
        let entry = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'sv-needs:' AND identifier CONTAINS 'item-worktrees'")).firstMatch
        scrollTo(entry, in: page)
        entry.tap()
        XCTAssertTrue(app.tables["needs-you-group"].waitForExistence(timeout: 5))
        snapshot(app, "supervisor-decisions")
    }

    func testTheInlineHtmlPage() {
        let app = launch("chat:chat-visuals")
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 20))
        let page = app.webViews["html-render-page"]
        for _ in 0..<6 where !(page.exists && page.isHittable) {
            app.swipeDown()
        }
        snapshot(app, "html-page")
        app.buttons["html-render-expand"].firstMatch.tap()
        let views = app.segmentedControls["html-sheet-view"]
        XCTAssertTrue(views.waitForExistence(timeout: 10))
        snapshot(app, "html-sheet-live")
        views.buttons["Source"].tap()
        let source = app.textViews["html-sheet-source"]
        XCTAssertTrue(source.waitForExistence(timeout: 5))
        XCTAssertTrue((source.value as? String ?? "").contains("<"))
        snapshot(app, "html-sheet-source")
        app.buttons["html-sheet-save"].tap()
        _ = app.buttons["Cancel"].waitForExistence(timeout: 10)
        snapshot(app, "html-sheet-save")
    }
}
