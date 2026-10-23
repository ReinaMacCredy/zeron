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

    func testCardButtonsCarryTheCoresAccessibilityIdentifiers() {
        let consent = CardAction.consent(requestId: "r", questionId: "consent", answer: "Deny", identifier: "computer-consent-deny")
        XCTAssertEqual(CardButtonControl.identifier(consent), "computer-consent-deny", "the core names each button")
        XCTAssertEqual(CardButtonControl.identifier(.enableComputer), "computer-denied-enable")
    }

    func testStillFitsInsideThePreviewBox() {
        XCTAssertEqual(ComputerStillPreview.fitted(CGSize(width: 640, height: 400)), CGSize(width: 168, height: 105))
        XCTAssertEqual(ComputerStillPreview.fitted(CGSize(width: 400, height: 800)), CGSize(width: 60, height: 120))
        XCTAssertEqual(ComputerStillPreview.fitted(.zero), ComputerStillPreview.maxSize)
    }
}
