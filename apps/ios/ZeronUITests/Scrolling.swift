import XCTest

extension XCTestCase {
    func scrollTo(_ element: XCUIElement, in list: XCUIElement, back: Bool = false) {
        for _ in 0..<8 where !(element.exists && element.isHittable) {
            if back { list.swipeDown() } else { list.swipeUp() }
        }
    }
}
