import Foundation

struct LiveActivityPrefs: Codable, Equatable {
    enum Mode: String, Codable, CaseIterable {
        case grouped, session
    }

    enum Which: String, Codable, CaseIterable {
        case all, pinned, asking
    }

    enum EndAfter: String, Codable, CaseIterable {
        case now, quarter, hour

        var seconds: TimeInterval {
            switch self {
            case .now: 0
            case .quarter: 15 * 60
            case .hour: 60 * 60
            }
        }
    }

    var enabled = true
    var mode = Mode.grouped
    var which = Which.all
    var trailing = SessionActivityTrailing.count
    var showStep = true
    var lockHide = false
    var alertAsk = true
    var endAfter = EndAfter.quarter

    static let didChange = Notification.Name("LiveActivityPrefs.didChange")
    private static let key = "liveActivityPrefs"

    static var current: LiveActivityPrefs {
        get { UserDefaults.standard.data(forKey: key).flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? Self() }
        set {
            guard newValue != current else { return }
            UserDefaults.standard.set(try? JSONEncoder().encode(newValue), forKey: key)
            NotificationCenter.default.post(name: didChange, object: nil)
        }
    }

    func attributes(chatId: String?) -> SessionActivityAttributes {
        SessionActivityAttributes(chatId: chatId, trailing: trailing, showStep: showStep, lockHide: lockHide)
    }
}
