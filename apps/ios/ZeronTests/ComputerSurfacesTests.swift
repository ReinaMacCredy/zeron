import XCTest
@testable import Zeron

final class ComputerSurfacesTests: XCTestCase {
    func testEnablePrefixesTheDraftOnceLikeTheMac() {
        let cases: [(String, String)] = [
            ("", "/computer-use "),
            ("  \n", "/computer-use "),
            ("open Calculator", "/computer-use open Calculator"),
            ("/computer-use open Calculator", "/computer-use open Calculator"),
            ("/computer-use ", "/computer-use "),
            ("/COMPUTER-USE x", "/COMPUTER-USE x"),
            ("   /computer-use", "   /computer-use"),
            ("/computer-use\nline two", "/computer-use\nline two"),
            ("    /computer-use", "/computer-use     /computer-use"),
            ("/computer-used", "/computer-use /computer-used"),
        ]
        for (draft, expected) in cases {
            XCTAssertEqual(computerUseEnablingDraft(draft: draft), expected, draft)
        }
    }

    func testCardButtonsCarryTheOldAccessibilityIdentifiers() {
        let consent = { (answer: String) in CardAction.consent(requestId: "r", questionId: "consent", answer: answer) }
        XCTAssertEqual(CardButtonControl.identifier(consent("Allow")), "computer-consent-allow")
        XCTAssertEqual(CardButtonControl.identifier(consent("Deny")), "computer-consent-decline")
        XCTAssertEqual(CardButtonControl.identifier(consent("Always allow Safari")), "computer-consent-always")
        XCTAssertEqual(CardButtonControl.identifier(.enableComputer), "computer-denied-enable")
    }

    func testStillFitsInsideThePreviewBox() {
        XCTAssertEqual(ComputerStillPreview.fitted(CGSize(width: 640, height: 400)), CGSize(width: 168, height: 105))
        XCTAssertEqual(ComputerStillPreview.fitted(CGSize(width: 400, height: 800)), CGSize(width: 60, height: 120))
        XCTAssertEqual(ComputerStillPreview.fitted(.zero), ComputerStillPreview.maxSize)
    }
}
