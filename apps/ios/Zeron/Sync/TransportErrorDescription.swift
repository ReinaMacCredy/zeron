import Foundation

func describeTransportError(_ error: Error) -> String {
    let ns = error as NSError
    var description = "\(ns.domain)#\(ns.code)"
    if ns.domain == NSURLErrorDomain {
        let name = ns.code == NSURLErrorTimedOut
            ? "timedOut"
            : String(describing: URLError.Code(rawValue: ns.code))
        description += " \(name)"
    }
    return description
}
