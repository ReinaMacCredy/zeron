import ActivityKit
import SwiftUI
import WidgetKit

struct SessionLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: SessionActivityAttributes.self) { context in
            SessionLockScreen(state: context.state, attributes: context.attributes)
                .activitySystemActionForegroundColor(.primary)
                .widgetURL(SessionLink.url(context.attributes.chatId))
        } dynamicIsland: { context in
            let state = context.state
            let attributes = context.attributes
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    SessionExpandedLeading(state: state, attributes: attributes)
                        .padding(.leading, 6)
                        .frame(maxHeight: .infinity, alignment: .center)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    SessionExpandedTrailing(state: state, attributes: attributes)
                        .padding(.trailing, 6)
                        .frame(maxHeight: .infinity, alignment: .center)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    SessionExpandedBody(state: state, attributes: attributes)
                        .padding(.horizontal, 6)
                        .padding(.top, 4)
                }
            } compactLeading: {
                SessionCompactLeading(state: state)
                    .padding(.leading, 2)
            } compactTrailing: {
                SessionCompactTrailing(state: state, attributes: attributes)
                    .padding(.trailing, 2)
            } minimal: {
                SessionMinimal(state: state)
            }
            .keylineTint(state.asking > 0 ? SessionTone.ask : SessionTone.accent)
            .widgetURL(SessionLink.url(attributes.chatId ?? (state.asking > 0 ? state.lead?.id : nil)))
        }
    }
}

#if DEBUG
private let grouped = SessionActivityAttributes(chatId: nil, trailing: .count, showStep: true, lockHide: false)
private let sample = SessionActivityAttributes.ContentState(
    rows: [
        SessionActivityRow(id: "a", title: "Fix sync retry", project: "zeron", branch: "main", state: .asking, since: Date.now.addingTimeInterval(-300).timeIntervalSince1970, step: "Allow edit to edge/src?"),
        SessionActivityRow(id: "b", title: "Live Activity", project: "zeron", branch: "ios-la", state: .working, since: Date.now.addingTimeInterval(-83).timeIntervalSince1970, step: "Running tests"),
    ],
    running: 1,
    asking: 1
)

#Preview("Island", as: .dynamicIsland(.expanded), using: grouped) {
    SessionLiveActivity()
} contentStates: {
    sample
}

#Preview("Lock Screen", as: .content, using: grouped) {
    SessionLiveActivity()
} contentStates: {
    sample
}
#endif
