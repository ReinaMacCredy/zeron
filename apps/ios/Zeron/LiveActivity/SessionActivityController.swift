import ActivityKit
import Foundation
import UIKit

@MainActor
final class SessionActivityController {
    typealias State = SessionActivityAttributes.ContentState

    struct Item: Equatable {
        var chatId: String?
        var state: State
        var live: Bool { state.running + state.asking > 0 }
    }

    private let app: AppModel
    private var token: AnyObject?
    private var prefsObserver: NSObjectProtocol?
    private var foregroundObserver: NSObjectProtocol?
    private var activities: [String: Activity<SessionActivityAttributes>] = [:]
    private var sent: [String: State] = [:]
    private var tokenTasks: [String: Task<Void, Never>] = [:]
    private var finished: [String: (state: SessionActivityState, at: Date, row: SessionActivityRow)] = [:]
    private var lastLive: [String: SessionActivityRow] = [:]
    private var pending: Task<Void, Never>?
    private var sentAt = Date.distantPast
    private var items: [Item] = []

    private static let interval: TimeInterval = 1
    private static let maxAge: TimeInterval = 7.5 * 60 * 60
    private static let startsKey = "sessionActivityStarts"

    init(app: AppModel) {
        self.app = app
    }

    func attach() {
        SessionActivityAction.stop = { [weak app] chatId in app?.interrupt(chatId) }
        for activity in Activity<SessionActivityAttributes>.activities {
            let key = activity.attributes.chatId ?? ""
            if activities[key] == nil, activity.attributes == LiveActivityPrefs.current.attributes(chatId: activity.attributes.chatId) {
                activities[key] = activity
                sent[key] = activity.content.state
                watchToken(activity)
            } else {
                Task { await activity.end(nil, dismissalPolicy: .immediate) }
            }
        }
        token = app.observe { [weak self] in self?.sync() }
        prefsObserver = NotificationCenter.default.addObserver(forName: LiveActivityPrefs.didChange, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.restart() }
        }
        foregroundObserver = NotificationCenter.default.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.renewOld() }
        }
        sync()
    }

    private func sync() {
        let prefs = LiveActivityPrefs.current
        let next = Self.items(rows: rows(prefs), prefs: prefs)
        let newlyAsking = Set(next.flatMap { $0.state.rows.filter { $0.state == .asking }.map(\.id) })
            .subtracting(items.flatMap { $0.state.rows.filter { $0.state == .asking }.map(\.id) })
        guard next != items else { return }
        items = next
        pending?.cancel()
        let delay = Self.interval - Date().timeIntervalSince(sentAt)
        let urgent = !newlyAsking.isEmpty || next.count != activities.count
        pending = Task { [weak self] in
            if !urgent, delay > 0 {
                try? await Task.sleep(for: .seconds(delay))
                if Task.isCancelled { return }
            }
            self?.apply(alerting: newlyAsking)
        }
    }

    private func rows(_ prefs: LiveActivityPrefs) -> [SessionActivityRow] {
        guard app.isSignedIn else {
            lastLive = [:]
            finished = [:]
            return []
        }
        let now = Date()
        var live: [String: SessionActivityRow] = [:]
        for r in app.sessionRows where !r.archived && r.parentChatId == nil && r.sidechatOf == nil {
            let state: SessionActivityState
            switch r.indicator {
            case .working: state = .working
            case .awaitingInput: state = .asking
            default: continue
            }
            if prefs.which == .pinned, !r.pinned { continue }
            live[r.id] = Self.row(r, state: state)
        }
        for (id, row) in lastLive where live[id] == nil {
            let errored = app.row(id)?.indicator == .errored
            finished[id] = (errored ? .failed : .done, now, row)
        }
        for id in live.keys { finished[id] = nil }
        finished = finished.filter { now.timeIntervalSince($0.value.at) < max(prefs.endAfter.seconds, 1) }
        lastLive = live
        var out = Array(live.values)
        if prefs.which == .asking { out = out.filter { $0.state == .asking } }
        out.sort { a, b in
            a.state == b.state ? (a.since ?? 0, a.id) < (b.since ?? 0, b.id) : a.state == .asking
        }
        let done = finished.sorted { $0.value.at > $1.value.at }.map { id, f in
            var row = f.row
            row.state = f.state
            row.since = nil
            return row
        }
        return out + (prefs.which == .asking ? [] : done)
    }

    static func row(_ r: SessionRow, state: SessionActivityState) -> SessionActivityRow {
        SessionActivityRow(
            id: r.id,
            title: r.title,
            project: r.project?.name ?? r.deviceName ?? "",
            branch: r.branch,
            state: state,
            since: r.workingSinceMs.map { Double($0) / 1000 },
            step: step(r.preview)
        )
    }

    static func step(_ preview: String?) -> String? {
        let line = preview?.split(whereSeparator: \.isNewline).lazy.map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty }
        return line.map { String($0.prefix(80)) }
    }

    static func items(rows: [SessionActivityRow], prefs: LiveActivityPrefs) -> [Item] {
        guard prefs.enabled else { return [] }
        let running = rows.filter { $0.state == .working }.count
        let asking = rows.filter { $0.state == .asking }.count
        switch prefs.mode {
        case .grouped:
            guard !rows.isEmpty else { return [] }
            return [Item(chatId: nil, state: State(rows: Array(rows.prefix(3)), running: running, asking: asking))]
        case .session:
            return rows.prefix(3).map { row in
                Item(chatId: row.id, state: State(rows: [row], running: row.state == .working ? 1 : 0, asking: row.state == .asking ? 1 : 0))
            }
        }
    }

    private func apply(alerting: Set<String>) {
        sentAt = Date()
        let prefs = LiveActivityPrefs.current
        let wanted = Dictionary(items.map { ($0.chatId ?? "", $0) }, uniquingKeysWith: { a, _ in a })
        for (key, activity) in activities where wanted[key]?.live != true {
            end(key, activity, final: wanted[key]?.state ?? sent[key], prefs: prefs)
        }
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return }
        for (key, item) in wanted where item.live {
            if let activity = activities[key] {
                guard sent[key] != item.state else { continue }
                sent[key] = item.state
                let alert = prefs.alertAsk ? item.state.rows.first { $0.state == .asking && alerting.contains($0.id) } : nil
                let content = ActivityContent(state: item.state, staleDate: nil)
                Task {
                    if let alert {
                        await activity.update(content, alertConfiguration: AlertConfiguration(title: "Needs you", body: "\(alert.title)", sound: .default))
                    } else {
                        await activity.update(content)
                    }
                }
            } else if UIApplication.shared.applicationState != .background {
                start(key, item, prefs: prefs)
            }
        }
    }

    private func start(_ key: String, _ item: Item, prefs: LiveActivityPrefs) {
        let attributes = prefs.attributes(chatId: item.chatId)
        let content = ActivityContent(state: item.state, staleDate: nil)
        let activity = (try? Activity.request(attributes: attributes, content: content, pushType: .token))
            ?? (try? Activity.request(attributes: attributes, content: content, pushType: nil))
        guard let activity else { return }
        activities[key] = activity
        sent[key] = item.state
        var starts = UserDefaults.standard.dictionary(forKey: Self.startsKey) as? [String: Double] ?? [:]
        starts = starts.filter { id, _ in activities.values.contains { $0.id == id } }
        starts[activity.id] = Date().timeIntervalSince1970
        UserDefaults.standard.set(starts, forKey: Self.startsKey)
        watchToken(activity)
    }

    private func end(_ key: String, _ activity: Activity<SessionActivityAttributes>, final: State?, prefs: LiveActivityPrefs, immediate: Bool = false) {
        activities[key] = nil
        sent[key] = nil
        tokenTasks[activity.id]?.cancel()
        tokenTasks[activity.id] = nil
        EdgeLiveActivity.unregister(app: app, activityId: activity.id)
        let seconds = prefs.endAfter.seconds
        let policy: ActivityUIDismissalPolicy = immediate || seconds == 0 ? .immediate : .after(Date().addingTimeInterval(seconds))
        let content = final.map { state in
            var done = state
            done.rows = done.rows.map { row in
                var row = row
                if row.state == .working || row.state == .asking { row.state = .done }
                row.since = nil
                return row
            }
            done.running = 0
            done.asking = 0
            return ActivityContent(state: done, staleDate: nil)
        }
        Task { await activity.end(content, dismissalPolicy: policy) }
    }

    private func restart() {
        let prefs = LiveActivityPrefs.current
        for (key, activity) in activities { end(key, activity, final: nil, prefs: prefs, immediate: true) }
        items = []
        sync()
    }

    private func renewOld() {
        let starts = UserDefaults.standard.dictionary(forKey: Self.startsKey) as? [String: Double] ?? [:]
        let now = Date().timeIntervalSince1970
        let prefs = LiveActivityPrefs.current
        for (key, activity) in activities where now - (starts[activity.id] ?? now) > Self.maxAge {
            end(key, activity, final: nil, prefs: prefs, immediate: true)
        }
        apply(alerting: [])
    }

    private func watchToken(_ activity: Activity<SessionActivityAttributes>) {
        tokenTasks[activity.id]?.cancel()
        let prefs = LiveActivityPrefs.current
        let pinned = prefs.which == .pinned ? app.sessionRows.filter(\.pinned).map(\.id) : []
        tokenTasks[activity.id] = Task { [weak self] in
            for await data in activity.pushTokenUpdates {
                guard let self else { return }
                let token = data.map { String(format: "%02x", $0) }.joined()
                EdgeLiveActivity.register(app: self.app, activityId: activity.id, token: token, chatId: activity.attributes.chatId, prefs: prefs, pinned: pinned)
            }
        }
    }
}

@MainActor
enum EdgeLiveActivity {
    static func register(app: AppModel, activityId: String, token: String, chatId: String?, prefs: LiveActivityPrefs, pinned: [String]) {
        var body: [String: Any] = [
            "id": activityId,
            "token": token,
            "environment": PushNotifications.apnsEnvironment,
            "which": prefs.which.rawValue,
            "alert": prefs.alertAsk,
            "endAfter": Int(prefs.endAfter.seconds),
            "pinned": pinned,
        ]
        if let chatId { body["chatId"] = chatId }
        send(app: app, method: "POST", activityId: activityId, body: body)
    }

    static func unregister(app: AppModel, activityId: String) {
        send(app: app, method: "DELETE", activityId: activityId, body: nil)
    }

    private static func send(app: AppModel, method: String, activityId: String, body: [String: Any]?) {
        guard !app.isDemo, let edge = app.edgeAccess,
              var components = URLComponents(string: edge.url) else { return }
        components.path = (components.path.hasSuffix("/") ? String(components.path.dropLast()) : components.path) + "/registry/\(edge.org)/live-activity"
        components.queryItems = [URLQueryItem(name: "device", value: AppModel.deviceId), URLQueryItem(name: "id", value: activityId)]
        guard let url = components.url else { return }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("Bearer \(edge.bearer)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        }
        Task.detached {
            do {
                let (_, response) = try await URLSession.shared.data(for: request)
                if let http = response as? HTTPURLResponse, http.statusCode >= 300 {
                    NSLog("live activity target \(method) failed: \(http.statusCode)")
                }
            } catch {
                NSLog("live activity target \(method) failed: \(error)")
            }
        }
    }
}
