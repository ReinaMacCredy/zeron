import Foundation

enum ComputerNotice: Hashable {
    case disclosure, escapeStop, setupRequired, unavailable, controlDenied
}

enum ConsentOutcome: Hashable {
    case allowed, declined, cancelled
}

enum ComputerCopy {
    static let consentAllow = "Allow"
    static let consentDecline = "Decline"

    static let controlOnDisclosure = "Computer control ON for this turn: the agent is driving the desktop and the user can switch it off in Settings."
    static let escapeStopNotice = "Computer use stopped: Escape was pressed"
    static let setupNote = "Computer control needs setup"
    static let unavailableNote = "Computer is unavailable for this turn"
    static let deniedTitle = "Computer control is off"
    static let deniedHint = "Turn it on in Settings to let the agent use the desktop."
    private static let capabilityDeniedCode = "capability_denied"

    static func notice(_ note: String) -> ComputerNotice? {
        switch note {
        case controlOnDisclosure: return .disclosure
        case escapeStopNotice: return .escapeStop
        case setupNote: return .setupRequired
        case unavailableNote: return .unavailable
        default: return nil
        }
    }

    static func isControlDenied(_ output: String?) -> Bool {
        output?.contains(capabilityDeniedCode) ?? false
    }

    static func consentAnswers(questionId: String, allow: Bool) -> [UserInputAnswer] {
        [UserInputAnswer(questionId: questionId, labels: [allow ? consentAllow : consentDecline])]
    }

    static func consentOutcome(resolved: Bool, answers: [UserInputAnswer]?) -> ConsentOutcome? {
        let labels = (answers ?? []).flatMap(\.labels)
        if labels.contains(consentAllow) { return .allowed }
        if !labels.isEmpty { return .declined }
        return resolved ? .cancelled : nil
    }

    static func consentTitle(_ scope: ComputerConsentScope) -> String {
        switch scope {
        case .task: return "Allow Computer for this task?"
        case .call: return "Approve this tool call?"
        }
    }

    static func consentAllowLabel(_ scope: ComputerConsentScope) -> String {
        switch scope {
        case .task: return "Allow Computer for this task"
        case .call: return "Approve once"
        }
    }

    static func consentAllowDescription(_ scope: ComputerConsentScope) -> String {
        switch scope {
        case .task:
            return "Continue routine desktop actions until this response ends. Stop cancels access. Clipboard reads still ask separately."
        case .call:
            return "Allow just this request"
        }
    }

    static func consentDetail(_ scope: ComputerConsentScope, question: String) -> String {
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard scope == .task, trimmed.hasPrefix(consentTitle(.task)) else { return trimmed }
        return String(trimmed.dropFirst(consentTitle(.task).count))
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func outcomeLabel(_ outcome: ConsentOutcome) -> String {
        switch outcome {
        case .allowed: return "Allowed"
        case .declined: return "Declined"
        case .cancelled: return "Cancelled"
        }
    }

    static let toolTitles: [(String, String)] = [
        ("computer_screenshot", "Take a screenshot"),
        ("computer_get_state", "Read the screen"),
        ("computer_get_screen_size", "Measure the screen"),
        ("computer_list_windows", "Find open windows"),
        ("computer_list_apps", "List apps"),
        ("computer_verify_state", "Verify state"),
        ("computer_zoom", "Zoom into a window"),
        ("computer_get_accessibility_tree", "List apps and windows"),
        ("computer_get_cursor_position", "Read the cursor position"),
        ("computer_help", "Read the Computer playbook"),
        ("computer_click", "Click"),
        ("computer_move_cursor", "Move the agent cursor"),
        ("computer_drag", "Drag"),
        ("computer_scroll", "Scroll"),
        ("computer_type_text", "Type"),
        ("computer_press_key", "Press a key"),
        ("computer_set_value", "Set a field"),
        ("computer_select_text", "Select text"),
        ("computer_perform_action", "Activate a control"),
        ("computer_launch_app", "Open an app"),
        ("computer_activate_window", "Activate a window"),
        ("computer_set_window_frame", "Move or resize a window"),
        ("computer_invoke_menu", "Invoke a menu item"),
        ("computer_kill_app", "Force-quit an app"),
        ("computer_set_window_minimized", "Minimize or restore a window"),
        ("computer_set_app_visibility", "Hide or unhide an app"),
        ("computer_wait", "Wait"),
        ("computer_read_clipboard", "Read the clipboard"),
        ("computer_write_clipboard", "Write to the clipboard"),
        ("computer_paste", "Paste text"),
        ("computer_run", "Run a sequence"),
        ("computer_inspect", "Inspect the computer"),
        ("computer_spaces", "Inspect desktop Spaces"),
        ("computer_browser_state", "Read the browser page"),
        ("computer_browser_prepare", "Prepare a browser"),
        ("computer_browser_navigate", "Open a browser page"),
        ("computer_browser_click", "Click in the browser"),
        ("computer_browser_type", "Type in a browser field"),
        ("computer_browser_dialog", "Handle a browser dialog"),
        ("computer_browser_upload", "Attach files in the browser"),
        ("computer_browser_download", "Download a file"),
        ("computer_browser_pointer", "Use the pointer in the browser"),
        ("computer_browser_press", "Press Enter in the browser"),
    ]

    static func toolName(_ candidate: String) -> String? {
        var normalized = ""
        for ch in candidate.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            if ch.isASCII, ch.isLetter || ch.isNumber {
                normalized.append(ch)
            } else if !normalized.hasSuffix("_") {
                normalized.append("_")
            }
        }
        return toolTitles.map(\.0).first { name in
            normalized == name
                || (normalized.hasSuffix(name) && normalized.dropLast(name.count).hasSuffix("_"))
        }
    }

    static func toolTitle(_ tool: String) -> String {
        toolTitles.first { $0.0 == tool }?.1 ?? "Computer"
    }
}

extension RenderToolCall {
    var computerTool: String? {
        switch tag {
        case "mcp": return string("tool").flatMap(ComputerCopy.toolName)
        case "unknown": return string("name").flatMap(ComputerCopy.toolName)
        default: return nil
        }
    }
}
