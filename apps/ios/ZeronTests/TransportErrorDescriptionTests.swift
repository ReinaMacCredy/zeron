import Foundation
import XCTest
@testable import Zeron

final class TransportErrorDescriptionTests: XCTestCase {
    func testTransportErrorDescriptionDoesNotExposeUserInfo() {
        let nested = NSError(domain: "Nested", code: 7,
                             userInfo: [NSLocalizedDescriptionKey: "SENTINEL"])
        let error = NSError(
            domain: NSURLErrorDomain,
            code: NSURLErrorTimedOut,
            userInfo: [
                NSURLErrorFailingURLStringErrorKey: "wss://x/ws?token=SENTINEL",
                NSUnderlyingErrorKey: nested,
            ]
        )
        let description = describeTransportError(error)
        XCTAssertFalse(description.contains("SENTINEL"))
        XCTAssertTrue(description.contains("-1001"))
        XCTAssertTrue(description.contains("timedOut"))
    }
}
