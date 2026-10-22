import UIKit

extension PendingInput {
    var symbol: String { approval ? "exclamationmark.circle" : "questionmark.circle" }
    var kindLabel: String { approval ? "Approve" : "Input" }
    var actionLabel: String { approval ? "Review" : "Answer" }
    var projectLabel: String { session.project?.name ?? session.deviceName ?? "No project" }
    var questions: [SessionChrome.Question] {
        request.questions.map {
            SessionChrome.Question(id: $0.id, header: $0.header, text: $0.question, options: $0.options, multiSelect: $0.multiSelect, prefill: $0.prefill, multiline: $0.multiline)
        }
    }
}

enum NeedsYou {
    static func present(app: AppModel, from host: UIViewController, excluding chatId: String?, onClose: ((PendingInput?) -> Void)? = nil) {
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

    static func waiting(_ app: AppModel, excluding chatId: String?) -> [PendingInput] {
        app.pending.filter { $0.session.id != chatId }
    }

    static func openThread(_ chatId: String, from vc: UIViewController) {
        let window = vc.view.window
        vc.presentingViewController?.dismiss(animated: true) {
            (window?.rootViewController as? AppRouter)?.openSession(chatId)
        }
    }
}

final class NeedsYouNavigationController: UINavigationController {
    var onClose: ((PendingInput?) -> Void)?
    var sent: PendingInput?

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
}

final class NeedsYouListController: UITableViewController {
    private let app: AppModel
    private let excluding: String?
    private var items: [PendingInput] = []
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
        token = app.observePending { [weak self] in self?.reload() }
        reload()
    }

    private func reload() {
        items = NeedsYou.waiting(app, excluding: excluding)
        title = "Needs you · \(items.count)"
        tableView.reloadData()
        if items.isEmpty, navigationController?.topViewController === self {
            presentingViewController?.dismiss(animated: true)
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { items.count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let item = items[indexPath.row]
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = UIListContentConfiguration.subtitleCell()
        c.image = UIImage(systemName: item.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
        c.imageProperties.tintColor = Palette.ask
        c.text = item.session.title
        c.textProperties.font = Fonts.ui(.sansSemibold, 16)
        c.textProperties.numberOfLines = 1
        c.secondaryText = [item.approval ? "Tool approval" : "Question", item.projectLabel, AppModel.vm(item.session).timeLabel].filter { !$0.isEmpty }.joined(separator: " · ")
        c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        c.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = c
        let action = UILabel()
        action.text = item.actionLabel
        action.font = Fonts.ui(.sansSemibold, 14)
        action.textColor = Palette.accent
        action.sizeToFit()
        cell.accessoryView = action
        cell.accessibilityIdentifier = "needs-you-\(item.session.id)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let item = items[indexPath.row]
        navigationController?.pushViewController(NeedsYouQuestionController(app: app, input: item), animated: true)
        if let sheet = navigationController?.sheetPresentationController {
            sheet.animateChanges { sheet.selectedDetentIdentifier = .large }
        }
    }
}

final class NeedsYouQuestionController: UIViewController {
    private let app: AppModel
    private let input: PendingInput
    private let panel = QuestionPanel()
    private var token: AnyObject?

    init(app: AppModel, input: PendingInput) {
        self.app = app
        self.input = input
        super.init(nibName: nil, bundle: nil)
        title = input.session.title
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
        kind.text = "\(input.approval ? "Tool approval" : "Question") · \(input.projectLabel)"
        kind.font = Fonts.ui(.sansMedium, 13)
        kind.textColor = Palette.ask
        let note = UILabel()
        note.text = "This changes files or the remote. Review it in the thread."
        note.font = Fonts.ui(.sans, 14)
        note.textColor = Palette.secondary
        note.numberOfLines = 0
        note.isHidden = !input.destructive
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
        let chatId = input.session.id
        panel.onOpenThread = { [weak self] in
            guard let self else { return }
            NeedsYou.openThread(chatId, from: self)
        }
        panel.quickAnswer = !input.destructive
        panel.configure(input.questions)
        panel.onSubmit = { [weak self] answers in self?.send(answers) }
        token = app.observePending { [weak self] in
            guard let self, !self.app.pending.contains(where: { $0.request.requestId == self.input.request.requestId }) else { return }
            if (self.navigationController as? NeedsYouNavigationController)?.sent == nil {
                self.navigationController?.popViewController(animated: true)
            }
        }
    }

    private func send(_ answers: [(questionId: String, labels: [String])]) {
        guard let nav = navigationController as? NeedsYouNavigationController else { return }
        guard app.answer(input, answers) else {
            Toast.show("Couldn't send. Try again.", in: view.window, atTop: true)
            return
        }
        nav.sent = input
        let window = view.window
        let title = input.session.title
        nav.presentingViewController?.dismiss(animated: true) {
            Toast.show("Sent to \(title)", in: window, atTop: true)
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

    func configure(_ waiting: [PendingInput]) {
        guard let first = waiting.first else { return }
        glyph.image = UIImage(systemName: first.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .semibold))
        name.text = first.session.title
        kind.text = "· \(first.kindLabel)"
        more.text = "+\(waiting.count - 1)"
        moreWell.isHidden = waiting.count < 2
        accessibilityLabel = "\(first.session.title), \(first.kindLabel)" + (waiting.count > 1 ? ", \(waiting.count - 1) more waiting" : "")
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
