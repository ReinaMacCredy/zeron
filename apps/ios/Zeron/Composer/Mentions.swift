import UIKit

/// `@` mentions: the composer shows `@name` tokens; on send they become the
/// canonical links the host resolves (`zeron-file:` files, `zeron-chat:` threads).
struct MentionIndex {
    private(set) var tokens: [String: String] = [:]

    /// Token for a file, disambiguated by parent folder on basename clashes.
    mutating func token(for file: FileMatch) -> String {
        let link = fileMentionLink(path: file.path, isDir: file.isDir)
        let parts = file.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")).split(separator: "/")
        var token = "@" + (parts.last.map(String.init) ?? file.path)
        if let existing = tokens[token], existing != link, parts.count > 1 {
            token = "@" + parts.suffix(2).joined(separator: "/")
        }
        tokens[token] = link
        return token
    }

    mutating func token(for thread: ThreadMention) -> String {
        let token = "@" + thread.title
        tokens[token] = thread.link
        return token
    }

    /// Replace live tokens with canonical links; forget tokens no longer present.
    func encode(_ text: String) -> String {
        var out = text
        for (token, link) in tokens.sorted(by: { $0.key.count > $1.key.count }) where out.contains(token) {
            out = out.replacingOccurrences(of: token, with: link)
        }
        return out
    }

    mutating func token(for entry: InvocationEntry) -> String {
        if let link = entry.link { tokens[entry.insert] = link }
        return entry.insert
    }

    mutating func reset() { tokens.removeAll() }

    /// A second `/` makes it a path, and `$5` or `$HOME` is not a skill.
    static func invocationQuery(in text: String, cursor: Int) -> (range: NSRange, query: String, skill: Bool)? {
        for (prefix, skill) in [(UInt16(36) /* $ */, true), (UInt16(47) /* / */, false)] {
            if let q = activeQuery(in: text, cursor: cursor, prefix: prefix), !q.query.contains("/"),
               !(skill && q.query.first.map { $0.isNumber || $0.isUppercase } == true) {
                return (q.range, q.query, skill)
            }
        }
        return nil
    }

    /// The `@query` being typed at `cursor`, if any (no whitespace inside).
    static func activeQuery(in text: String, cursor: Int, prefix: UInt16 = 64 /* @ */) -> (range: NSRange, query: String)? {
        let ns = text as NSString
        guard cursor <= ns.length else { return nil }
        var i = cursor - 1
        while i >= 0 {
            let c = ns.character(at: i)
            if c == prefix {
                let before = i > 0 ? ns.character(at: i - 1) : 32
                guard before == 32 || before == 10 else { return nil }
                let range = NSRange(location: i, length: cursor - i)
                return (range, ns.substring(with: NSRange(location: i + 1, length: cursor - i - 1)))
            }
            if c == 32 || c == 10 { return nil }
            i -= 1
        }
        return nil
    }
}

enum MentionPick {
    case file(FileMatch)
    case thread(ThreadMention)
    case invocation(InvocationEntry)
}

/// Glass list of matching threads and files above the composer.
final class MentionSuggestions: UIView {
    var onPick: ((MentionPick) -> Void)?
    private let card = Glass.surface(radius: 18)
    private let scroll = UIScrollView()
    private let stack = UIStackView()
    private(set) var threads: [ThreadMention] = []
    private(set) var files: [FileMatch] = []
    private(set) var invocations: [InvocationEntry] = []
    private var note: String?
    private(set) var loading = false

    var isEmpty: Bool { threads.isEmpty && files.isEmpty && invocations.isEmpty && note == nil && !loading }

    override init(frame: CGRect) {
        super.init(frame: frame)
        card.translatesAutoresizingMaskIntoConstraints = false
        addSubview(card)
        stack.axis = .vertical
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.alwaysBounceVertical = false
        scroll.accessibilityIdentifier = "suggestions-list"
        card.contentView.addSubview(scroll)
        scroll.addSubview(stack)
        let fit = scroll.heightAnchor.constraint(equalTo: stack.heightAnchor)
        fit.priority = .defaultHigh - 1
        NSLayoutConstraint.activate([
            card.topAnchor.constraint(equalTo: topAnchor),
            card.leadingAnchor.constraint(equalTo: leadingAnchor),
            card.trailingAnchor.constraint(equalTo: trailingAnchor),
            card.bottomAnchor.constraint(equalTo: bottomAnchor),
            scroll.topAnchor.constraint(equalTo: card.contentView.topAnchor, constant: 4),
            scroll.leadingAnchor.constraint(equalTo: card.contentView.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: card.contentView.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: card.contentView.bottomAnchor, constant: -4),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            stack.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
            stack.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor),
            heightAnchor.constraint(lessThanOrEqualToConstant: 320),
            fit,
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(invocations: [InvocationEntry], error: String?, empty: String?) {
        threads = []
        files = []
        loading = false
        self.invocations = invocations
        note = error ?? empty
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        scroll.contentOffset = .zero
        if let error { stack.addArrangedSubview(message(error, color: Palette.danger, identifier: "invocation-error")) }
        if let empty { stack.addArrangedSubview(message(empty, color: Palette.secondary, identifier: "invocation-empty")) }
        for (i, entry) in invocations.enumerated() {
            let row = InvocationRow(entry: entry) { [weak self] in self?.onPick?(.invocation(entry)) }
            row.accessibilityIdentifier = "invocation-\(i)"
            stack.addArrangedSubview(row)
        }
    }

    func showLoading() {
        threads = []
        files = []
        invocations = []
        note = nil
        loading = true
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for width in [0.42, 0.6, 0.5] {
            stack.addArrangedSubview(SkeletonRow(fraction: width))
        }
    }

    private func message(_ text: String, color: UIColor, identifier: String) -> UIView {
        let label = UILabel()
        label.text = text
        label.font = Fonts.ui(.sans, 13.5)
        label.textColor = color
        label.numberOfLines = 0
        label.accessibilityIdentifier = identifier
        let wrap = UIView()
        label.translatesAutoresizingMaskIntoConstraints = false
        wrap.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 14),
            label.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -14),
            label.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 10),
            label.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -10),
        ])
        return wrap
    }

    func show(threads: [ThreadMention], files: [FileMatch]) {
        invocations = []
        note = nil
        loading = false
        self.threads = threads
        self.files = Array(files.prefix(threads.isEmpty ? 5 : 3))
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        scroll.contentOffset = .zero
        if !threads.isEmpty {
            stack.addArrangedSubview(header("Threads"))
            for (i, thread) in threads.enumerated() {
                let detail = [thread.project, thread.status].compactMap { $0 }.joined(separator: " · ")
                let b = row(symbol: "bubble.left", title: thread.title, subtitle: detail, mono: false) { [weak self] in self?.onPick?(.thread(thread)) }
                b.accessibilityIdentifier = "mention-thread-\(i)"
                stack.addArrangedSubview(b)
            }
            if !self.files.isEmpty { stack.addArrangedSubview(header("Files")) }
        }
        for (i, file) in self.files.enumerated() {
            let name = file.path.split(separator: "/").last.map(String.init) ?? file.path
            let dir = file.path.components(separatedBy: "/").dropLast().joined(separator: "/")
            let b = row(symbol: file.isDir ? "folder" : "doc.text", title: name, subtitle: dir, mono: true) { [weak self] in self?.onPick?(.file(file)) }
            b.accessibilityIdentifier = "mention-\(i)"
            stack.addArrangedSubview(b)
        }
    }

    private func header(_ text: String) -> UIView {
        let label = UILabel()
        label.text = text
        label.font = Fonts.ui(.sansMedium, 12)
        label.textColor = Palette.tertiary
        label.numberOfLines = 0
        let wrap = UIView()
        label.translatesAutoresizingMaskIntoConstraints = false
        wrap.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 14),
            label.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -14),
            label.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 6),
            label.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -2),
        ])
        return wrap
    }

    private func row(symbol: String, title: String, subtitle: String, mono: Bool, pick: @escaping () -> Void) -> UIButton {
        var c = UIButton.Configuration.plain()
        c.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .medium))
        c.imagePadding = 10
        c.baseForegroundColor = Palette.text
        var t = AttributedString(title)
        t.font = Fonts.ui(.sansMedium, 15)
        c.attributedTitle = t
        if !subtitle.isEmpty {
            var sub = AttributedString(subtitle)
            sub.font = mono ? Fonts.ui(.mono, 11.5) : Fonts.ui(.sans, 12)
            sub.foregroundColor = Palette.tertiary
            c.attributedSubtitle = sub
        }
        c.titleAlignment = .leading
        c.titleLineBreakMode = .byTruncatingTail
        c.contentInsets = NSDirectionalEdgeInsets(top: 8, leading: 14, bottom: 8, trailing: 14)
        let b = UIButton(configuration: c, primaryAction: UIAction { _ in pick() })
        b.contentHorizontalAlignment = .leading
        return b
    }
}

private final class InvocationRow: UIControl {
    private let pick: () -> Void

    init(entry: InvocationEntry, pick: @escaping () -> Void) {
        self.pick = pick
        super.init(frame: .zero)
        let icon = UIImageView(image: UIImage(systemName: entry.skill ? "sparkles" : "command", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .medium)))
        icon.tintColor = Palette.secondary
        icon.contentMode = .center
        let title = UILabel()
        title.text = entry.title
        title.font = Fonts.ui(.sansMedium, 15)
        title.textColor = Palette.text
        title.lineBreakMode = .byTruncatingTail
        let detail = UILabel()
        detail.text = entry.detail
        detail.font = Fonts.ui(.sans, 13.5)
        detail.textColor = Palette.secondary
        detail.lineBreakMode = .byTruncatingTail
        detail.isHidden = entry.detail.isEmpty
        title.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        detail.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        title.setContentHuggingPriority(.required, for: .horizontal)
        let line = UIStackView(arrangedSubviews: [icon, title, detail])
        line.spacing = 8
        line.alignment = .center
        line.isUserInteractionEnabled = false
        line.translatesAutoresizingMaskIntoConstraints = false
        addSubview(line)
        var constraints = [
            icon.widthAnchor.constraint(equalToConstant: 18),
            line.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            line.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            line.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            line.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ]
        if !entry.detail.isEmpty {
            constraints.append(title.widthAnchor.constraint(lessThanOrEqualTo: line.widthAnchor, multiplier: 0.55))
        }
        NSLayoutConstraint.activate(constraints)
        alpha = entry.blocked ? 0.45 : 1
        isAccessibilityElement = true
        accessibilityTraits = entry.blocked ? [.button, .notEnabled] : .button
        accessibilityLabel = entry.title
        accessibilityValue = entry.detail
        addAction(UIAction { [weak self] _ in self?.pick() }, for: .touchUpInside)
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet { backgroundColor = isHighlighted ? Palette.controlFill : .clear }
    }
}

private final class SkeletonRow: UIView {
    init(fraction: CGFloat) {
        super.init(frame: .zero)
        let bar = UIView()
        bar.backgroundColor = Palette.controlFill
        bar.layer.cornerRadius = 5
        bar.translatesAutoresizingMaskIntoConstraints = false
        addSubview(bar)
        NSLayoutConstraint.activate([
            bar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            bar.widthAnchor.constraint(equalTo: widthAnchor, multiplier: fraction),
            bar.heightAnchor.constraint(equalToConstant: 12),
            bar.topAnchor.constraint(equalTo: topAnchor, constant: 13),
            bar.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -13),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}
