import UIKit

struct NeedsYouAsk: Equatable {
    let chatId: String
    let requestId: String?
    let questions: [UserInputQuestion]
    let destructive: Bool
    let hostName: String
    let atMs: Int64

    var symbol: String { questions.isEmpty ? "exclamationmark.circle" : "questionmark.circle" }

    var kindLabel: String { questions.isEmpty ? "Waiting for you" : "Question" }

    var actionLabel: String { questions.isEmpty ? "Open" : "Answer" }

    func title(_ app: AppModel) -> String { app.row(chatId)?.title ?? "Thread" }

    var panelQuestions: [SessionChrome.Question] {
        questions.map {
            SessionChrome.Question(id: $0.id, header: $0.header, text: $0.question, options: $0.options, multiSelect: $0.multiSelect, prefill: $0.prefill, multiline: $0.multiline, secret: $0.secret)
        }
    }

    func meta(_ app: AppModel) -> String {
        var parts = [kindLabel]
        if let project = app.row(chatId)?.project?.name { parts.append(project) }
        parts.append(relativeTimeLabel(atMs: atMs, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
        return parts.joined(separator: " · ")
    }
}

enum NeedsYou {
    static func present(app: AppModel, from host: UIViewController, excluding chatId: String?, onClose: ((NeedsYouAsk?) -> Void)? = nil) {
        let list = NeedsYouListController(app: app, excluding: chatId)
        let nav = NeedsYouNavigationController(rootViewController: list)
        nav.onClose = onClose
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
            sheet.prefersScrollingExpandsWhenScrolledToEdge = false
        }
        host.present(nav, animated: true)
    }

    static func waiting(_ app: AppModel, excluding chatId: String?) -> [NeedsYouAsk] {
        app.needsYou.filter { $0.chatId != chatId }
    }

    static func threads(_ app: AppModel) -> [SessionRowVM] {
        app.needsYou.compactMap { ask in app.row(ask.chatId).map(AppModel.vm) }
    }

    static func openThread(_ chatId: String, from vc: UIViewController) {
        let window = vc.view.window
        vc.presentingViewController?.dismiss(animated: true) {
            (window?.rootViewController as? AppRouter)?.openSession(chatId)
        }
    }
}

final class NeedsYouNavigationController: UINavigationController {
    var onClose: ((NeedsYouAsk?) -> Void)?
    var sent: NeedsYouAsk?

    override func viewDidLoad() {
        super.viewDidLoad()
        navigationBar.titleTextAttributes = [.font: Fonts.ui(.sansSemibold, 16), .foregroundColor: Palette.text]
        navigationBar.tintColor = Palette.text
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        guard isBeingDismissed || presentingViewController == nil else { return }
        let close = onClose
        onClose = nil
        close?(sent)
    }

    func expand() {
        guard let sheet = sheetPresentationController else { return }
        sheet.animateChanges { sheet.selectedDetentIdentifier = .large }
    }
}

final class NeedsYouListController: UITableViewController {
    private let app: AppModel
    private let excluding: String?
    private var asks: [NeedsYouAsk] = []
    private var token: AnyObject?

    init(app: AppModel, excluding: String?) {
        self.app = app
        self.excluding = excluding
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        tableView.accessibilityIdentifier = "needs-you-list"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        navigationItem.backButtonTitle = "Needs you"
        token = app.observeNeedsYou { [weak self] in self?.reload() }
        reload()
    }

    private func reload() {
        asks = NeedsYou.waiting(app, excluding: excluding)
        title = "Needs you · \(asks.count)"
        tableView.reloadData()
        if asks.isEmpty, navigationController?.topViewController === self {
            presentingViewController?.dismiss(animated: true)
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { asks.count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let ask = asks[indexPath.row]
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = UIListContentConfiguration.subtitleCell()
        c.image = UIImage(systemName: ask.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
        c.imageProperties.tintColor = Palette.ask
        c.text = ask.title(app)
        c.textProperties.font = Fonts.ui(.sansSemibold, 16)
        c.textProperties.numberOfLines = 2
        c.secondaryAttributedText = NSAttributedString(string: [ask.hostName, ask.meta(app)].filter { !$0.isEmpty }.joined(separator: " · "), attributes: [.font: Fonts.ui(.sans, 13), .foregroundColor: Palette.secondary])
        c.secondaryTextProperties.numberOfLines = 0
        c.textToSecondaryTextVerticalPadding = 3
        cell.contentConfiguration = c
        let action = UILabel()
        action.text = ask.actionLabel
        action.font = Fonts.ui(.sansSemibold, 14)
        action.textColor = Palette.accent
        action.sizeToFit()
        cell.accessoryView = action
        cell.accessibilityIdentifier = "needs-you-\(ask.chatId)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let ask = asks[indexPath.row]
        guard !ask.questions.isEmpty else { return NeedsYou.openThread(ask.chatId, from: self) }
        navigationController?.pushViewController(NeedsYouQuestionController(app: app, ask: ask), animated: true)
        (navigationController as? NeedsYouNavigationController)?.expand()
    }
}

final class NeedsYouQuestionController: UIViewController {
    private let app: AppModel
    private let ask: NeedsYouAsk
    private let panel = QuestionPanel()
    private var token: AnyObject?

    init(app: AppModel, ask: NeedsYouAsk) {
        self.app = app
        self.ask = ask
        super.init(nibName: nil, bundle: nil)
        title = ask.title(app)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = true
        scroll.keyboardDismissMode = .interactive
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        let kind = UILabel()
        kind.text = [ask.kindLabel, ask.hostName].filter { !$0.isEmpty }.joined(separator: " · ")
        kind.font = Fonts.ui(.sansMedium, 13)
        kind.textColor = Palette.ask
        kind.numberOfLines = 0
        let note = UILabel()
        note.text = "This changes files or the remote. Review it in the thread."
        note.font = Fonts.ui(.sans, 14)
        note.textColor = Palette.secondary
        note.numberOfLines = 0
        note.isHidden = !ask.destructive
        let stack = UIStackView(arrangedSubviews: [kind, panel, note])
        stack.axis = .vertical
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 8),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -16),
            stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -16),
        ])
        let chatId = ask.chatId
        panel.onOpenThread = { [weak self] in
            guard let self else { return }
            NeedsYou.openThread(chatId, from: self)
        }
        panel.quickAnswer = !ask.destructive
        panel.configure(ask.panelQuestions)
        panel.onSubmit = { [weak self] answers in self?.send(answers) }
        token = app.observeNeedsYou { [weak self] in
            guard let self, !self.app.needsYou.contains(where: { $0.requestId == self.ask.requestId }) else { return }
            if (self.navigationController as? NeedsYouNavigationController)?.sent == nil {
                self.navigationController?.popViewController(animated: true)
            }
        }
    }

    private func send(_ answers: [(questionId: String, labels: [String])]) {
        guard let nav = navigationController as? NeedsYouNavigationController else { return }
        nav.sent = ask
        guard app.answer(ask, answers) else {
            nav.sent = nil
            Toast.show("Couldn't send. Try again.", in: view.window, atTop: true)
            return
        }
        let window = view.window
        let dest = ask.title(app)
        if nav.viewControllers.count > 1, !app.needsYou.isEmpty {
            nav.popToRootViewController(animated: true)
            Toast.show("Sent to \(dest)", in: window, atTop: true)
        } else {
            nav.presentingViewController?.dismiss(animated: true) {
                Toast.show("Sent to \(dest)", in: window, atTop: true)
            }
        }
    }
}

final class NeedsYouPill: UIControl {
    private let glyph = UIImageView()
    private let name = UILabel()
    private let kind = UILabel()
    private let more = UILabel()
    private let moreWell = UIView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Palette.elevated.withAlphaComponent(0.96)
        layer.cornerRadius = 16
        layer.cornerCurve = .continuous
        layer.borderWidth = 1
        layer.borderColor = Palette.ask.cgColor
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.12
        layer.shadowRadius = 8
        layer.shadowOffset = CGSize(width: 0, height: 3)
        glyph.tintColor = Palette.ask
        glyph.contentMode = .center
        name.font = Fonts.ui(.sansSemibold, 13)
        name.textColor = Palette.text
        name.lineBreakMode = .byTruncatingTail
        kind.font = Fonts.ui(.sansMedium, 13)
        kind.textColor = Palette.secondary
        more.font = Fonts.ui(.sansSemibold, 11)
        more.textColor = Palette.ask
        moreWell.backgroundColor = Palette.ask.withAlphaComponent(0.14)
        moreWell.layer.cornerRadius = 9
        moreWell.addSubview(more)
        let stack = UIStackView(arrangedSubviews: [glyph, name, kind, moreWell])
        stack.spacing = 6
        stack.alignment = .center
        stack.isUserInteractionEnabled = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        more.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for v in [kind, moreWell] as [UIView] { v.setContentCompressionResistancePriority(.required, for: .horizontal) }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 32),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            glyph.widthAnchor.constraint(equalToConstant: 17),
            more.leadingAnchor.constraint(equalTo: moreWell.leadingAnchor, constant: 6),
            more.trailingAnchor.constraint(equalTo: moreWell.trailingAnchor, constant: -6),
            more.centerYAnchor.constraint(equalTo: moreWell.centerYAnchor),
            moreWell.heightAnchor.constraint(equalToConstant: 18),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityIdentifier = "needs-you-pill"
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: NeedsYouPill, _) in
            self.layer.borderColor = Palette.ask.resolvedColor(with: self.traitCollection).cgColor
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet { UIView.animate(withDuration: 0.15) { self.alpha = self.isHighlighted ? 0.7 : 1 } }
    }

    func configure(_ waiting: [NeedsYouAsk], app: AppModel) {
        guard let first = waiting.first else { return }
        let title = first.title(app)
        glyph.image = UIImage(systemName: first.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .semibold))
        name.text = title
        kind.text = "· \(first.kindLabel)"
        more.text = "+\(waiting.count - 1)"
        moreWell.isHidden = waiting.count < 2
        accessibilityLabel = "\(title), \(first.kindLabel)" + (waiting.count > 1 ? ", \(waiting.count - 1) more waiting" : "")
    }
}

final class NeedsYouChip: UIControl {
    var count = 0 {
        didSet {
            guard count != oldValue else { return }
            label.text = "\(count) need\(count == 1 ? "s" : "") you"
            accessibilityLabel = label.text
            invalidateIntrinsicContentSize()
        }
    }
    private let label = UILabel()
    private let dot = UIView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Palette.ask.withAlphaComponent(0.14)
        layer.cornerRadius = 13
        layer.cornerCurve = .continuous
        label.font = Fonts.ui(.sansSemibold, 12.5)
        label.textColor = Palette.ask
        dot.backgroundColor = Palette.ask
        dot.layer.cornerRadius = 4
        for v in [dot, label] {
            v.translatesAutoresizingMaskIntoConstraints = false
            v.isUserInteractionEnabled = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 26),
            dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9),
            dot.centerYAnchor.constraint(equalTo: centerYAnchor),
            dot.widthAnchor.constraint(equalToConstant: 8),
            dot.heightAnchor.constraint(equalToConstant: 8),
            label.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 6),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityIdentifier = "needs-you-chip"
    }

    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: CGSize {
        CGSize(width: 9 + 8 + 6 + ceil(label.intrinsicContentSize.width) + 10, height: 26)
    }

    override var isHighlighted: Bool {
        didSet { UIView.animate(withDuration: 0.15) { self.alpha = self.isHighlighted ? 0.6 : 1 } }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        dot.layer.removeAnimation(forKey: "pulse")
        guard window != nil, !UIAccessibility.isReduceMotionEnabled else { return }
        let pulse = CABasicAnimation(keyPath: "opacity")
        pulse.fromValue = 1
        pulse.toValue = 0.35
        pulse.duration = 0.6
        pulse.autoreverses = true
        pulse.repeatCount = .infinity
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        dot.layer.add(pulse, forKey: "pulse")
    }
}
