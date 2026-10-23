import PhotosUI
import UIKit

/// An image, or a long paste as `Pasted text N.txt`, staged in the composer before sending.
struct StagedImage: Identifiable, Equatable {
    let id: String
    let name: String
    let data: Data
    let thumbnail: UIImage
    var mimeType = "image/jpeg"

    var isImage: Bool { mimeType.hasPrefix("image/") }

    static func pastedText(_ text: String, name: String) -> StagedImage {
        StagedImage(id: UUID().uuidString, name: name, data: Data(text.utf8), thumbnail: UIImage(systemName: "doc.text") ?? UIImage(), mimeType: "text/plain")
    }
}

/// How a message sent during a running turn is delivered.
enum DeliveryMode: String {
    /// Waits in the shared queue for the turn to end (default).
    case queue
    /// Injected mid-turn when the harness supports it.
    case steer
    /// Stops the turn, then sends.
    case interrupt
}

/// A context chip in the composer toolbar (model, effort, branch…).
struct ComposerChip: Equatable {
    let id: String
    let title: String
    let symbol: String?
    var tint: UIColor? = nil
    /// Brand mark / custom glyph (takes precedence over `symbol`).
    var icon: UIImage? = nil
}

/// The composer. One glass surface with two states that morph into each
/// other (the same views re-anchor inside a spring animation):
///
/// - resting: a single-line capsule — [+]  Message…  [↑]
/// - active (focused, or holding a draft/photos): a card — photos, full-width
///   text, and a toolbar row inside the glass: [+] [model] [effort] [branch] … [Send]
///
/// The action button is one control that becomes Send, Queue/Steer (a labelled
/// pill while an agent works) or Stop.
final class ComposerBar: UIView, UITextViewDelegate, UIGestureRecognizerDelegate {
    enum Action: Equatable {
        case send
        case queue
        case stop
        case resume
    }

    // Callbacks
    /// What happened to a send, so the composer knows whether to clear.
    enum SendResult {
        /// Taken: the composer clears.
        case sent
        /// Not taken (e.g. couldn't start the session): text and images stay.
        case kept
        /// Taken, and the handler already put the composer's next contents in
        /// place (committing a queued edit restores the stashed draft).
        case replaced
    }

    var onSend: ((String, [StagedImage], DeliveryMode) -> SendResult)?
    var onStop: (() -> Void)?
    var onChipTap: ((String, UIView) -> Void)?
    var onCommentRemove: ((String) -> Void)?
    var onCommentsClear: (() -> Void)?
    var onHeightChange: (() -> Void)?
    var onFocusChange: ((Bool) -> Void)?
    var onActionChange: (() -> Void)?
    /// `@` file search (nil disables mentions).
    var mentionSearch: ((String) async -> [FileMatch])?
    var threadSearch: ((String) -> [ThreadMention])?
    var invocationCatalog: ((Bool) async -> InvocationCatalog?)?

    // State
    var running = false { didSet { if running != oldValue { refreshAction(animated: true) } } }
    var resumable = false { didSet { if resumable != oldValue { refreshAction(animated: true) } } }
    var canSteer = false { didSet { refreshAction(animated: false) } }
    var preferredDelivery: DeliveryMode = .queue { didSet { refreshAction(animated: false) } }
    var placeholder = "Message" { didSet { refreshPlaceholder() } }
    var prediction: String? { didSet { if prediction != oldValue { refreshPlaceholder() } } }
    var chips: [ComposerChip] = [] { didSet { if chips != oldValue { rebuildChips() } } }
    /// Chips with a menu open it on tap (native, glassy, lazily loaded).
    var chipMenus: [String: () -> UIMenu?] = [:] { didSet { applyChipMenus() } }
    /// Stay in the card state even when idle (new-session canvas).
    var chipsAlwaysVisible = false { didSet { updateMode(animated: false) } }
    var images: [StagedImage] = [] { didSet { syncThumbs(); refreshAction(animated: true); updateMode(animated: true) } }
    var quotes: [String] = [] { didSet { syncThumbs(); refreshAction(animated: true); updateMode(animated: true) } }
    var comments: SessionChrome.Comments? { didSet { if comments != oldValue { syncThumbs(); refreshAction(animated: true); updateMode(animated: true) } } }
    private var hasComments: Bool { comments?.items.isEmpty == false }

    var text: String {
        get { textView.text }
        set {
            textView.text = newValue
            textChanged()
            updateMode(animated: false)
        }
    }

    let textView = UITextView()
    private let glass = Glass.surface(interactive: false, radius: 25)
    private let placeholderLabel = UILabel()
    private let acceptBadge = UIButton(type: .system)
    private let attachButton = UIButton(type: .system)
    private let actionButton = UIButton(type: .system)
    private let thumbs = UIStackView()
    private let thumbsScroll = UIScrollView()
    private var thumbCells: [String: ThumbCell] = [:]
    private var quoteChip: UIButton?
    private var commentsChipView: UIButton?
    private var collapseDelay: TimeInterval = 0
    private let toolbar = UIView()
    private let chipStrip = UIStackView()
    private let chipScroll = FadingScrollView()
    private var textHeight: NSLayoutConstraint!
    private var thumbsHeight: NSLayoutConstraint!
    private var toolbarHeight: NSLayoutConstraint!
    private var compactConstraints: [NSLayoutConstraint] = []
    private var cardConstraints: [NSLayoutConstraint] = []
    private(set) var isCard = false
    private var currentAction: Action = .send
    var isQueueing: Bool { currentAction == .queue }
    private var mentions = MentionIndex()
    private let suggestions = MentionSuggestions()
    private var suggestionsShown = false
    private var mentionQuery: (range: NSRange, query: String)?
    private var mentionTask: Task<Void, Never>?
    private var attachCard: AttachCard?
    private var keyboardFrame: CGRect?

    private let font = Fonts.ui(.sans, UIFontMetrics(forTextStyle: .body).scaledValue(for: 16.5))
    private var maxLines: Int {
        // Resting capsule previews at most two lines of a draft.
        if !isCard { return 2 }
        return traitCollection.verticalSizeClass == .compact ? 3 : 8
    }
    private static let control: CGFloat = 34
    private static let compactHeight: CGFloat = 50

    override init(frame: CGRect) {
        super.init(frame: frame)
        build()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func build() {
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        let content = glass.contentView
        // Tapping anywhere on the surface focuses the input.
        let tap = UITapGestureRecognizer(target: self, action: #selector(focusFromTap))
        tap.cancelsTouchesInView = false
        tap.delegate = self
        glass.addGestureRecognizer(tap)

        thumbsScroll.showsHorizontalScrollIndicator = false
        thumbsScroll.translatesAutoresizingMaskIntoConstraints = false
        thumbs.axis = .horizontal
        thumbs.spacing = 6
        thumbs.alignment = .center
        thumbs.isLayoutMarginsRelativeArrangement = true
        thumbs.translatesAutoresizingMaskIntoConstraints = false
        thumbsScroll.addSubview(thumbs)
        content.addSubview(thumbsScroll)

        var attach = UIButton.Configuration.filled()
        attach.image = UIImage(systemName: "plus", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold))
        attach.baseForegroundColor = Palette.text
        attach.baseBackgroundColor = Palette.controlFill
        attach.cornerStyle = .capsule
        attach.contentInsets = .zero
        attachButton.configuration = attach
        attachButton.accessibilityLabel = "Attach"
        attachButton.accessibilityIdentifier = "composer-attach"
        attachButton.translatesAutoresizingMaskIntoConstraints = false
        attachButton.addAction(UIAction { [weak self] _ in self?.toggleAttachCard() }, for: .touchUpInside)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboardWillChangeFrame), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        content.addSubview(attachButton)

        textView.font = font
        textView.textColor = Palette.text
        textView.tintColor = Palette.accent
        textView.backgroundColor = .clear
        textView.delegate = self
        textView.isScrollEnabled = false
        textView.textContainerInset = UIEdgeInsets(top: 14, left: 0, bottom: 14, right: 0)
        textView.textContainer.lineFragmentPadding = 0
        textView.accessibilityIdentifier = "composer-input"
        textView.translatesAutoresizingMaskIntoConstraints = false
        textView.keyboardDismissMode = .interactive
        content.addSubview(textView)

        placeholderLabel.font = font
        placeholderLabel.textColor = Palette.tertiary
        placeholderLabel.text = placeholder
        placeholderLabel.isUserInteractionEnabled = false
        placeholderLabel.translatesAutoresizingMaskIntoConstraints = false
        placeholderLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        placeholderLabel.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(acceptPrediction)))
        placeholderLabel.accessibilityIdentifier = "composer-placeholder"
        content.addSubview(placeholderLabel)

        var badge = UIButton.Configuration.filled()
        badge.title = "Accept"
        badge.baseBackgroundColor = Palette.chip
        badge.baseForegroundColor = Palette.secondary
        badge.cornerStyle = .capsule
        badge.contentInsets = NSDirectionalEdgeInsets(top: 1, leading: 6, bottom: 1, trailing: 6)
        badge.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var attrs = attrs
            attrs.font = .systemFont(ofSize: 11, weight: .semibold)
            return attrs
        }
        acceptBadge.configuration = badge
        acceptBadge.isHidden = true
        acceptBadge.accessibilityIdentifier = "composer-accept-prediction"
        acceptBadge.translatesAutoresizingMaskIntoConstraints = false
        acceptBadge.setContentCompressionResistancePriority(.required, for: .horizontal)
        acceptBadge.addTarget(self, action: #selector(acceptPrediction), for: .touchUpInside)
        content.addSubview(acceptBadge)

        toolbar.translatesAutoresizingMaskIntoConstraints = false
        toolbar.clipsToBounds = true
        // Beneath the controls: the row spans the card, and above them it
        // would swallow taps on "+" (the action button is added later).
        content.insertSubview(toolbar, at: 0)
        chipScroll.showsHorizontalScrollIndicator = false
        chipScroll.translatesAutoresizingMaskIntoConstraints = false
        chipStrip.axis = .horizontal
        chipStrip.spacing = 6
        chipStrip.alignment = .center
        chipStrip.translatesAutoresizingMaskIntoConstraints = false
        chipScroll.addSubview(chipStrip)
        toolbar.addSubview(chipScroll)

        actionButton.translatesAutoresizingMaskIntoConstraints = false
        actionButton.accessibilityIdentifier = "composer-send"
        actionButton.addAction(UIAction { [weak self] _ in self?.primaryAction() }, for: .touchUpInside)
        actionButton.setContentHuggingPriority(.required, for: .horizontal)
        content.addSubview(actionButton)

        let c = Self.control
        textHeight = textView.heightAnchor.constraint(equalToConstant: Self.compactHeight)
        thumbsHeight = thumbsScroll.heightAnchor.constraint(equalToConstant: 0)
        toolbarHeight = toolbar.heightAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),

            thumbsScroll.topAnchor.constraint(equalTo: content.topAnchor),
            thumbsScroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            thumbsScroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            thumbsHeight,
            thumbs.topAnchor.constraint(equalTo: thumbsScroll.contentLayoutGuide.topAnchor),
            thumbs.bottomAnchor.constraint(equalTo: thumbsScroll.contentLayoutGuide.bottomAnchor),
            thumbs.leadingAnchor.constraint(equalTo: thumbsScroll.contentLayoutGuide.leadingAnchor),
            thumbs.trailingAnchor.constraint(equalTo: thumbsScroll.contentLayoutGuide.trailingAnchor),
            thumbs.heightAnchor.constraint(equalTo: thumbsScroll.frameLayoutGuide.heightAnchor),

            textView.topAnchor.constraint(equalTo: thumbsScroll.bottomAnchor),
            textHeight,
            toolbar.topAnchor.constraint(equalTo: textView.bottomAnchor),
            toolbar.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            toolbar.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            toolbarHeight,

            placeholderLabel.leadingAnchor.constraint(equalTo: textView.leadingAnchor),
            placeholderLabel.trailingAnchor.constraint(lessThanOrEqualTo: textView.trailingAnchor),
            placeholderLabel.topAnchor.constraint(equalTo: textView.topAnchor, constant: 14),
            acceptBadge.leadingAnchor.constraint(equalTo: placeholderLabel.trailingAnchor, constant: 6),
            acceptBadge.trailingAnchor.constraint(lessThanOrEqualTo: textView.trailingAnchor),
            acceptBadge.centerYAnchor.constraint(equalTo: placeholderLabel.centerYAnchor),

            attachButton.widthAnchor.constraint(equalToConstant: c),
            attachButton.heightAnchor.constraint(equalToConstant: c),
            actionButton.heightAnchor.constraint(equalToConstant: c),
            actionButton.widthAnchor.constraint(greaterThanOrEqualToConstant: c),

            chipScroll.leadingAnchor.constraint(equalTo: attachButton.trailingAnchor, constant: 8),
            chipScroll.trailingAnchor.constraint(equalTo: actionButton.leadingAnchor, constant: -8),
            chipScroll.centerYAnchor.constraint(equalTo: toolbar.centerYAnchor),
            chipScroll.heightAnchor.constraint(equalToConstant: c),
            chipStrip.topAnchor.constraint(equalTo: chipScroll.contentLayoutGuide.topAnchor),
            chipStrip.bottomAnchor.constraint(equalTo: chipScroll.contentLayoutGuide.bottomAnchor),
            chipStrip.leadingAnchor.constraint(equalTo: chipScroll.contentLayoutGuide.leadingAnchor),
            chipStrip.trailingAnchor.constraint(equalTo: chipScroll.contentLayoutGuide.trailingAnchor),
            chipStrip.heightAnchor.constraint(equalTo: chipScroll.frameLayoutGuide.heightAnchor),
        ])
        // Resting capsule: controls at the capsule's ends, text between them.
        compactConstraints = [
            attachButton.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 8),
            attachButton.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -8),
            actionButton.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -8),
            actionButton.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -8),
            textView.leadingAnchor.constraint(equalTo: attachButton.trailingAnchor, constant: 10),
            textView.trailingAnchor.constraint(equalTo: actionButton.leadingAnchor, constant: -8),
        ]
        // Card: full-width text, controls in the toolbar row.
        cardConstraints = [
            attachButton.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 10),
            attachButton.centerYAnchor.constraint(equalTo: toolbar.centerYAnchor),
            actionButton.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -10),
            actionButton.centerYAnchor.constraint(equalTo: toolbar.centerYAnchor),
            textView.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            textView.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -16),
        ]
        NSLayoutConstraint.activate(compactConstraints)
        chipScroll.alpha = 0
        suggestions.isHidden = true
        suggestions.translatesAutoresizingMaskIntoConstraints = false
        suggestions.onPick = { [weak self] pick in self?.insertMention(pick) }
        refreshAction(animated: false)
    }

    /// Taps on controls (attach, chips, send) are theirs: focusing would
    /// morph the capsule under the finger and cancel the control's menu.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        var v = touch.view
        while let view = v, view !== glass {
            if view is UIControl { return false }
            v = view.superview
        }
        return true
    }

    /// The text view's own taps must not starve the one that swaps the attach card back to the keyboard.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }

    @objc private func focusFromTap() {
        if let attachCard { return attachCard.close() }
        if !textView.isFirstResponder { textView.becomeFirstResponder() }
    }

    /// Suggestions float above the bar, outside its bounds — so they live in
    /// the screen's root view (ancestors would otherwise swallow their taps).
    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard let root = findViewController()?.view, suggestions.superview !== root else { return }
        root.addSubview(suggestions)
        NSLayoutConstraint.activate([
            suggestions.leadingAnchor.constraint(equalTo: leadingAnchor),
            suggestions.trailingAnchor.constraint(equalTo: trailingAnchor),
            suggestions.bottomAnchor.constraint(equalTo: topAnchor, constant: -8),
            suggestions.topAnchor.constraint(greaterThanOrEqualTo: root.safeAreaLayoutGuide.topAnchor, constant: 8),
        ])
    }

    // MARK: Resting ↔ card

    /// The card is for composing: focused, holding photos, or pinned open
    /// (new-session canvas). An unfocused draft rests as the capsule.
    private var wantsCard: Bool {
        chipsAlwaysVisible || textView.isFirstResponder || !images.isEmpty || !quotes.isEmpty || hasComments
    }

    private func updateMode(animated: Bool) {
        let card = wantsCard
        guard card != isCard else { return }
        isCard = card
        let change = {
            if card {
                NSLayoutConstraint.deactivate(self.compactConstraints)
                NSLayoutConstraint.activate(self.cardConstraints)
            } else {
                NSLayoutConstraint.deactivate(self.cardConstraints)
                NSLayoutConstraint.activate(self.compactConstraints)
            }
            self.toolbarHeight.constant = card ? 50 : 0
            self.chipScroll.alpha = card ? 1 : 0
            self.textView.textContainerInset = card ? UIEdgeInsets(top: 14, left: 0, bottom: 6, right: 0) : UIEdgeInsets(top: 14, left: 0, bottom: 14, right: 0)
            self.glass.cornerConfiguration = .uniformCorners(radius: .fixed(card ? 26 : 25))
            self.recomputeTextHeight()
            self.onHeightChange?()
            self.superview?.layoutIfNeeded()
        }
        guard animated, window != nil, !UIAccessibility.isReduceMotionEnabled else { return change() }
        UIView.animate(withDuration: 0.42, delay: 0, usingSpringWithDamping: 0.86, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: change)
    }

    // MARK: Mentions

    private func updateMentionQuery() {
        guard mentionSearch != nil || threadSearch != nil || invocationCatalog != nil else { return }
        mentionTask?.cancel()
        if let catalog = invocationCatalog, let q = MentionIndex.invocationQuery(in: textView.text, cursor: textView.selectedRange.location) {
            mentionQuery = (q.range, q.query)
            mentionTask = Task { [weak self] in
                guard let found = await catalog(q.skill), !Task.isCancelled, let self, self.mentionQuery?.query == q.query else { return }
                let matches = filterInvocations(catalog: found, query: q.query)
                self.suggestions.show(invocations: matches.entries, note: matches.note)
                self.setSuggestionsVisible(!self.suggestions.isEmpty)
            }
            return
        }
        guard mentionSearch != nil || threadSearch != nil else {
            mentionQuery = nil
            setSuggestionsVisible(false)
            return
        }
        mentionQuery = MentionIndex.activeQuery(in: textView.text, cursor: textView.selectedRange.location)
        guard let q = mentionQuery else {
            setSuggestionsVisible(false)
            return
        }
        let threads = threadSearch?(q.query) ?? []
        suggestions.show(threads: threads, files: suggestions.files)
        setSuggestionsVisible(!suggestions.isEmpty)
        mentionTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled, let self, let search = self.mentionSearch else { return }
            let files = await search(q.query)
            guard !Task.isCancelled, self.mentionQuery?.query == q.query else { return }
            self.suggestions.show(threads: threads, files: files)
            self.setSuggestionsVisible(!self.suggestions.isEmpty)
        }
    }

    private func setSuggestionsVisible(_ visible: Bool) {
        guard suggestionsShown != visible else { return }
        suggestionsShown = visible
        if visible && suggestions.isHidden {
            suggestions.alpha = 0
            suggestions.transform = CGAffineTransform(translationX: 0, y: 8)
        }
        suggestions.isHidden = false
        UIView.animate(withDuration: 0.22, delay: 0, options: [.curveEaseOut, .beginFromCurrentState]) {
            self.suggestions.alpha = visible ? 1 : 0
            self.suggestions.transform = visible ? .identity : CGAffineTransform(translationX: 0, y: 8)
        } completion: { _ in
            if !self.suggestionsShown { self.suggestions.isHidden = true }
        }
    }

    private func insertMention(_ pick: MentionPick) {
        guard let q = mentionQuery else { return }
        UISelectionFeedbackGenerator().selectionChanged()
        let token: String
        switch pick {
        case .file(let file): token = mentions.token(for: file) + " "
        case .thread(let thread): token = mentions.token(for: thread) + " "
        case .invocation(let entry): token = mentions.token(for: entry) + " "
        }
        let ns = textView.text as NSString
        textView.text = ns.replacingCharacters(in: q.range, with: token)
        textView.selectedRange = NSRange(location: q.range.location + (token as NSString).length, length: 0)
        mentionQuery = nil
        setSuggestionsVisible(false)
        textChanged()
    }

    /// Accent the live `@tokens`; everything else is plain body text.
    private func styleMentions() {
        guard !mentions.tokens.isEmpty, textView.markedTextRange == nil else { return }
        let selected = textView.selectedRange
        let base: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: Palette.text]
        let styled = NSMutableAttributedString(string: textView.text, attributes: base)
        let ns = textView.text as NSString
        for token in mentions.tokens.keys {
            var search = NSRange(location: 0, length: ns.length)
            while true {
                let r = ns.range(of: token, options: [], range: search)
                if r.location == NSNotFound { break }
                styled.addAttributes([.foregroundColor: Palette.accent, .font: Fonts.ui(.sansMedium, font.pointSize)], range: r)
                search = NSRange(location: r.upperBound, length: ns.length - r.upperBound)
            }
        }
        textView.attributedText = styled
        textView.selectedRange = selected
        textView.typingAttributes = base
    }

    // MARK: Focus & chips

    @discardableResult
    override func becomeFirstResponder() -> Bool { textView.becomeFirstResponder() }

    @discardableResult
    override func resignFirstResponder() -> Bool { textView.resignFirstResponder() }

    func textViewDidBeginEditing(_ textView: UITextView) {
        updateMode(animated: true)
        onFocusChange?(true)
    }

    func textViewDidEndEditing(_ textView: UITextView) {
        attachCard?.close()
        setSuggestionsVisible(false)
        updateMode(animated: true)
        textView.setContentOffset(.zero, animated: false)
        onFocusChange?(false)
    }

    private func rebuildChips() {
        chipStrip.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for chip in chips {
            var config = UIButton.Configuration.filled()
            config.title = chip.title
            if let icon = chip.icon {
                config.image = icon
            } else if let symbol = chip.symbol {
                config.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 11.5, weight: .semibold))
            }
            config.imagePadding = chip.id == "pr" ? 5 : 6
            // Tinted chips (the PR badge) use the desktop's tone wash: fill
            // @ 0.08, ink @ 0.85.
            config.baseForegroundColor = chip.tint.map { $0.withAlphaComponent(0.85) } ?? Palette.text
            config.baseBackgroundColor = chip.tint.map { $0.withAlphaComponent(0.1) } ?? Palette.controlFill
            config.cornerStyle = .capsule
            config.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 11, bottom: 7, trailing: 11)
            config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
                var a = attrs
                // PR numbers stay mono like the list's PR badges; the checkout
                // chip (branch / New worktree) reads as ordinary UI text.
                let mono = chip.id == "pr"
                a.font = Fonts.ui(mono ? .monoMedium : .sansMedium, mono ? 12.5 : 13.5)
                return a
            }
            let b = UIButton(configuration: config)
            b.accessibilityIdentifier = "composer-chip-\(chip.id)"
            b.setContentHuggingPriority(.required, for: .horizontal)
            b.setContentCompressionResistancePriority(.required, for: .horizontal)
            b.addAction(UIAction { [weak self, weak b] _ in
                guard let self, let b else { return }
                self.onChipTap?(chip.id, b)
            }, for: .touchUpInside)
            chipStrip.addArrangedSubview(b)
        }
        applyChipMenus()
    }

    private func applyChipMenus() {
        for case let b as UIButton in chipStrip.arrangedSubviews {
            guard let id = b.accessibilityIdentifier?.replacingOccurrences(of: "composer-chip-", with: ""),
                  let provider = chipMenus[id]
            else { continue }
            b.menu = provider()
            b.showsMenuAsPrimaryAction = b.menu != nil
        }
    }

    // MARK: Attachments

    private func toggleAttachCard() {
        if let attachCard { return attachCard.close() }
        guard let host = findViewController() else { return }
        let picker = AttachmentPicker(host: host, limit: 8 - images.count) { [weak self] in self?.addImages($0) }
        let present = { [weak self] in
            guard let self, let window = self.window, self.attachCard == nil else { return }
            let fallback = CGRect(x: 0, y: window.bounds.height - 336, width: window.bounds.width, height: 336)
            var keyboard = self.keyboardFrame.flatMap { $0.height >= 200 ? $0 : nil } ?? fallback
            if let frame = self.keyboardFrame, frame.height < 200 { keyboard.origin.y = frame.minY - keyboard.height }
            self.attachCard = AttachCard.present(in: window, input: self.textView, anchor: self.attachButton, composer: self, keyboard: keyboard, picker: picker) { [weak self] in
                self?.attachCard = nil
                self?.rotateAttach(open: false)
            }
            self.rotateAttach(open: true)
        }
        if textView.isFirstResponder { return present() }
        textView.becomeFirstResponder()
        DispatchQueue.main.async(execute: present)
    }

    @objc private func keyboardWillChangeFrame(_ note: Notification) {
        guard attachCard == nil, let window, let frame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
        let local = window.convert(frame, from: window.screen.coordinateSpace)
        keyboardFrame = local.minY < window.bounds.height ? local : nil
    }

    private func rotateAttach(open: Bool) {
        let apply = { self.attachButton.transform = open ? CGAffineTransform(rotationAngle: .pi / 4) : .identity }
        guard !UIAccessibility.isReduceMotionEnabled else { return apply() }
        UIView.animate(withDuration: 0.42, delay: 0, usingSpringWithDamping: 0.72, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: apply)
    }

    func addImages(_ new: [StagedImage]) {
        images.append(contentsOf: new.prefix(max(0, 8 - images.count)))
    }

    func clearImages() { images.removeAll() }

    private func commentsChip(_ comments: SessionChrome.Comments) -> UIButton {
        var c = UIButton.Configuration.filled()
        c.title = comments.title
        c.image = UIImage(systemName: "text.bubble", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11.5, weight: .semibold))
        c.imagePadding = 6
        c.baseForegroundColor = Palette.text
        c.baseBackgroundColor = Palette.controlFill
        c.cornerStyle = .capsule
        c.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 11, bottom: 7, trailing: 11)
        c.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var a = attrs
            a.font = Fonts.ui(.sansMedium, 13.5)
            return a
        }
        let chip = UIButton(configuration: c)
        chip.accessibilityIdentifier = "composer-comments"
        let rows = comments.items.map { item in
            let remove = UIImage(systemName: "xmark.circle.fill")?
                .withTintColor(item.severity?.ink ?? Palette.secondary, renderingMode: .alwaysOriginal)
            return UIAction(title: item.text, subtitle: item.location, image: remove) { [weak self] _ in self?.onCommentRemove?(item.id) }
        }
        let clear = UIAction(title: comments.clear, image: UIImage(systemName: "xmark"), attributes: .destructive) { [weak self] _ in
            self?.onCommentsClear?()
        }
        chip.menu = UIMenu(children: [UIMenu(options: .displayInline, children: rows), clear])
        chip.showsMenuAsPrimaryAction = true
        return chip
    }

    private func removeImage(id: String) {
        guard let cell = thumbCells.removeValue(forKey: id) else { return }
        let remaining = images.filter { $0.id != id }
        if remaining.isEmpty { collapseDelay = 0.12 }
        if UIAccessibility.isReduceMotionEnabled {
            UIView.animate(withDuration: 0.12) { cell.alpha = 0 } completion: { _ in cell.removeFromSuperview() }
        } else {
            UIView.animate(withDuration: 0.22, delay: 0, options: [.beginFromCurrentState]) {
                cell.isHidden = true
                cell.transform = CGAffineTransform(scaleX: 0.6, y: 0.6)
                self.thumbs.layoutIfNeeded()
            } completion: { _ in cell.removeFromSuperview() }
            UIView.animate(withDuration: 0.15, delay: 0, options: [.beginFromCurrentState]) { cell.alpha = 0 }
        }
        images = remaining
    }

    private func syncThumbs() {
        quoteChip?.removeFromSuperview()
        quoteChip = quotes.isEmpty ? nil : makeQuoteChip()
        if let quoteChip { thumbs.insertArrangedSubview(quoteChip, at: 0) }
        commentsChipView?.removeFromSuperview()
        commentsChipView = comments.flatMap { $0.items.isEmpty ? nil : commentsChip($0) }
        if let commentsChipView { thumbs.insertArrangedSubview(commentsChipView, at: quoteChip == nil ? 0 : 1) }

        let ids = Set(images.map(\.id))
        for (id, cell) in thumbCells where !ids.contains(id) {
            cell.removeFromSuperview()
            thumbCells[id] = nil
        }
        let wasEmpty = thumbCells.isEmpty
        var added: [ThumbCell] = []
        for img in images where thumbCells[img.id] == nil {
            let cell = ThumbCell(item: img, tile: img.isImage ? nil : pastedTextTile(img))
            cell.onRemove = { [weak self] in self?.removeImage(id: img.id) }
            cell.onOpen = { [weak self, weak cell] in
                guard let self, let cell else { return }
                if img.isImage { self.preview(img, from: cell) } else { self.previewPastedText(img) }
            }
            thumbCells[img.id] = cell
            thumbs.addArrangedSubview(cell)
            added.append(cell)
        }
        let pictures = images.filter(\.isImage)
        for (i, img) in pictures.enumerated() {
            thumbCells[img.id]?.accessibilityLabel = "Image \(i + 1) of \(pictures.count)"
        }

        let height: CGFloat = !images.isEmpty ? 118 : quotes.isEmpty && !hasComments ? 0 : 46
        thumbs.layoutMargins = images.isEmpty
            ? UIEdgeInsets(top: 12, left: 8, bottom: 0, right: 8)
            : UIEdgeInsets(top: 8, left: 8, bottom: 6, right: 8)
        let animated = window != nil && !UIAccessibility.isReduceMotionEnabled
        if animated, !added.isEmpty {
            UIView.performWithoutAnimation {
                for cell in added {
                    cell.alpha = 0
                    cell.transform = CGAffineTransform(scaleX: 0.4, y: 0.4)
                }
                thumbs.layoutIfNeeded()
            }
            for (n, cell) in added.enumerated() {
                let delay = (wasEmpty ? 0.12 : 0) + Double(n) * 0.04
                UIView.animate(withDuration: 0.42, delay: delay, usingSpringWithDamping: 0.86, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) { cell.transform = .identity }
                UIView.animate(withDuration: 0.16, delay: delay, options: [.allowUserInteraction, .beginFromCurrentState]) { cell.alpha = 1 }
            }
        } else if !added.isEmpty, window != nil {
            added.forEach { $0.alpha = 0 }
            UIView.animate(withDuration: 0.12) { added.forEach { $0.alpha = 1 } }
        }

        guard height != thumbsHeight.constant else { return }
        let resize = {
            self.thumbsHeight.constant = height
            self.onHeightChange?()
            self.superview?.layoutIfNeeded()
        }
        let delay = collapseDelay
        collapseDelay = 0
        guard animated else { return resize() }
        let spring = {
            UIView.animate(withDuration: 0.42, delay: 0, usingSpringWithDamping: 0.86, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: resize)
        }
        if delay > 0 { DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: spring) } else { spring() }
    }

    private func makeQuoteChip() -> UIButton {
        var q = UIButton.Configuration.filled()
        q.title = quoteChipLabel(count: UInt32(quotes.count))
        q.image = UIImage(systemName: "text.quote", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11.5, weight: .semibold))
        q.imagePadding = 6
        q.baseForegroundColor = Palette.text
        q.baseBackgroundColor = Palette.controlFill
        q.cornerStyle = .capsule
        q.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 11, bottom: 7, trailing: 11)
        q.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var a = attrs
            a.font = Fonts.ui(.sansMedium, 13.5)
            return a
        }
        let chip = UIButton(configuration: q)
        chip.accessibilityIdentifier = "composer-quotes"
        let previews = quotes.map { UIAction(title: $0, attributes: .disabled) { _ in } }
        let remove = UIAction(title: "Remove", image: UIImage(systemName: "xmark"), attributes: .destructive) { [weak self] _ in
            self?.quotes = []
        }
        chip.menu = UIMenu(children: [UIMenu(options: .displayInline, children: previews), remove])
        chip.showsMenuAsPrimaryAction = true
        return chip
    }

    private func pastedTextTile(_ item: StagedImage) -> UIView {
        let text = String(decoding: item.data, as: UTF8.self)
        let info = pastedTextPreview(reference: item.name, text: text)
        let tile = UIView()
        tile.backgroundColor = Palette.controlFill
        let icon = UIImageView(image: UIImage(systemName: "doc.text", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .medium)))
        icon.tintColor = Palette.secondary
        let title = UILabel()
        title.text = info.title
        title.font = Fonts.ui(.sansMedium, 13.5)
        title.textColor = Palette.text
        let subtitle = UILabel()
        subtitle.text = info.subtitle
        subtitle.font = Fonts.ui(.sans, 12)
        subtitle.textColor = Palette.tertiary
        let labels = UIStackView(arrangedSubviews: [title, subtitle])
        labels.axis = .vertical
        labels.spacing = 1
        let row = UIStackView(arrangedSubviews: [icon, labels])
        row.alignment = .center
        row.spacing = 9
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        tile.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: tile.leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(lessThanOrEqualTo: tile.trailingAnchor, constant: -12),
            row.centerYAnchor.constraint(equalTo: tile.centerYAnchor),
        ])
        return tile
    }

    private func previewPastedText(_ item: StagedImage) {
        guard let vc = findViewController() else { return }
        PastedTextPreviewer.present(text: String(decoding: item.data, as: UTF8.self), reference: item.name, from: vc)
    }

    private func preview(_ item: StagedImage, from cell: ThumbCell) {
        guard let vc = findViewController() else { return }
        let pictures = images.filter(\.isImage)
        let index = pictures.firstIndex { $0.id == item.id } ?? 0
        let preview = AttachmentPreview(item: item, index: index, count: pictures.count, source: cell.imageArea) { [weak self] in
            self?.removeImage(id: item.id)
        }
        vc.present(preview, animated: false)
    }

    // MARK: Text

    func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
        guard images.count < 8, pastesAsFile(pasted: text, draft: textView.text) else { return true }
        images.append(.pastedText(text, name: nextPastedTextName(staged: images.map(\.name))))
        return false
    }

    func textViewDidChange(_ textView: UITextView) {
        textChanged()
        styleMentions()
        updateMentionQuery()
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        if mentionQuery != nil { updateMentionQuery() }
    }

    private func recomputeTextHeight() {
        // At rest a draft is a preview: whole lines ending in "…", never a
        // scrolled view whose next line bleeds into the bottom inset.
        let preview = !isCard
        let lines = preview ? maxLines : 0
        let breakMode: NSLineBreakMode = preview ? .byTruncatingTail : .byWordWrapping
        if textView.textContainer.maximumNumberOfLines != lines || textView.textContainer.lineBreakMode != breakMode {
            textView.textContainer.maximumNumberOfLines = lines
            textView.textContainer.lineBreakMode = breakMode
        }
        let width = max(40, textView.bounds.width)
        let fitting = textView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
        let insets = textView.textContainerInset.top + textView.textContainerInset.bottom
        let maxHeight = font.lineHeight * CGFloat(maxLines) + insets
        let minHeight: CGFloat = isCard ? 46 : Self.compactHeight
        textView.isScrollEnabled = !preview && fitting > maxHeight
        if preview { textView.contentOffset = .zero }
        textHeight.constant = preview ? max(minHeight, fitting) : min(max(minHeight, fitting), maxHeight)
    }

    private func refreshPlaceholder() {
        let ghost = predictionPlaceholder(prediction: prediction, text: textView.text)
        placeholderLabel.text = ghost ?? placeholder
        placeholderLabel.isHidden = !textView.text.isEmpty
        placeholderLabel.isUserInteractionEnabled = ghost != nil
        acceptBadge.isHidden = ghost == nil
    }

    @objc private func acceptPrediction() {
        guard let prediction else { return }
        text = prediction
        textView.becomeFirstResponder()
    }

    private func textChanged() {
        refreshPlaceholder()
        let before = textHeight.constant
        updateMode(animated: true)
        recomputeTextHeight()
        if abs(textHeight.constant - before) > 0.5 {
            UIView.animate(withDuration: 0.18, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
                self.onHeightChange?()
                self.superview?.layoutIfNeeded()
            }
        }
        refreshAction(animated: true)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if textView.bounds.width > 0, !textView.text.isEmpty, abs(textView.contentSize.height - textHeight.constant) > 1 {
            recomputeTextHeight()
        }
    }

    // MARK: Action button

    private var hasContent: Bool {
        !textView.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !images.isEmpty || !quotes.isEmpty || hasComments
    }

    private func refreshAction(animated: Bool) {
        let action: Action = running ? (hasContent ? .queue : .stop) : (resumable && !hasContent ? .resume : .send)
        let enabled = action == .stop || action == .resume || hasContent
        let steering = action == .queue && preferredDelivery == .steer && canSteer
        var config = UIButton.Configuration.filled()
        config.cornerStyle = .capsule
        let symbol: String
        switch action {
        case .send:
            symbol = "arrow.up"
            config.baseBackgroundColor = enabled ? Palette.accent : Palette.controlFill
            config.baseForegroundColor = enabled ? .white : Palette.tertiary
            config.contentInsets = .zero
        case .queue:
            // Same look as Send (the long-press menu still offers steer /
            // stop-and-send); only the accessibility label says "Queue".
            symbol = "arrow.up"
            config.baseBackgroundColor = Palette.accent
            config.baseForegroundColor = .white
            config.contentInsets = .zero
        case .stop:
            symbol = "stop.fill"
            config.baseBackgroundColor = Palette.text
            config.baseForegroundColor = Palette.background
            config.contentInsets = .zero
        case .resume:
            symbol = "play.fill"
            config.baseBackgroundColor = Palette.accent
            config.baseForegroundColor = .white
            config.contentInsets = .zero
        }
        config.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: action == .stop ? 11 : 15, weight: .bold))
        let changed = currentAction != action
        currentAction = action
        let apply = {
            self.actionButton.configuration = config
            self.actionButton.isEnabled = enabled
            self.superview?.layoutIfNeeded()
        }
        if animated, changed, window != nil {
            UIView.animate(withDuration: 0.32, delay: 0, usingSpringWithDamping: 0.8, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: apply)
            actionButton.imageView?.addSymbolEffect(.bounce.byLayer, options: .nonRepeating)
        } else {
            apply()
        }
        switch action {
        case .send: actionButton.accessibilityLabel = "Send message"
        case .queue: actionButton.accessibilityLabel = steering ? "Steer" : "Queue message"
        case .stop: actionButton.accessibilityLabel = "Stop response"
        case .resume: actionButton.accessibilityLabel = "Resume"
        }
        if changed { onActionChange?() }
        // Long-press offers the other delivery modes mid-turn.
        if action == .queue {
            var items: [UIMenuElement] = [
                UIAction(title: "Queue for next turn", image: UIImage(systemName: "text.line.last.and.arrowtriangle.forward")) { [weak self] _ in self?.send(.queue) },
            ]
            if canSteer {
                items.append(UIAction(title: "Steer now", subtitle: "Deliver into the running turn", image: UIImage(systemName: "arrow.turn.down.right")) { [weak self] _ in self?.send(.steer) })
            }
            items.append(UIAction(title: "Stop and send", image: UIImage(systemName: "stop.circle"), attributes: .destructive) { [weak self] _ in self?.send(.interrupt) })
            actionButton.menu = UIMenu(title: "While the agent works", children: items)
            actionButton.showsMenuAsPrimaryAction = false
        } else {
            actionButton.menu = nil
        }
    }

    private func primaryAction() {
        switch currentAction {
        case .stop:
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
            onStop?()
        case .send:
            send(.queue)
        case .queue:
            send(canSteer ? preferredDelivery : (preferredDelivery == .steer ? .queue : preferredDelivery))
        case .resume:
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            _ = onSend?(resumePrompt(), [], .queue)
        }
    }

    private func send(_ mode: DeliveryMode) {
        guard hasContent else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        let body = withQuotes(text: mentions.encode(textView.text.trimmingCharacters(in: .whitespacesAndNewlines)), quotes: quotes)
        let staged = images
        let result = onSend?(body, staged, mode) ?? .sent
        guard result != .kept else { return }
        setSuggestionsVisible(false)
        // `.replaced` put a draft back: its @file mentions must still resolve.
        guard result == .sent else { return }
        mentions.reset()
        textView.text = ""
        images = []
        quotes = []
        textChanged()
    }

    // External keyboards: ⌘↩ sends.
    override var keyCommands: [UIKeyCommand]? {
        [UIKeyCommand(title: "Send", action: #selector(commandReturn), input: "\r", modifierFlags: .command)]
    }

    @objc private func commandReturn() {
        if hasContent { primaryAction() }
    }
}

/// A staged attachment in the composer's row: a 104 pt image (or a pasted
/// text tile) with a remove button, keyed by `StagedImage.id`.
private final class ThumbCell: UIView {
    var onRemove: (() -> Void)?
    var onOpen: (() -> Void)?
    let imageArea: UIView

    init(item: StagedImage, tile: UIView?) {
        if let tile {
            imageArea = tile
        } else {
            let iv = UIImageView(image: item.thumbnail)
            iv.contentMode = .scaleAspectFill
            imageArea = iv
        }
        super.init(frame: .zero)
        imageArea.clipsToBounds = true
        imageArea.layer.cornerRadius = 22
        imageArea.layer.cornerCurve = .continuous
        imageArea.isUserInteractionEnabled = true
        imageArea.translatesAutoresizingMaskIntoConstraints = false
        imageArea.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(open)))
        addSubview(imageArea)

        var x = UIButton.Configuration.filled()
        x.image = UIImage(systemName: "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 9, weight: .bold))
        x.baseBackgroundColor = UIColor.black.withAlphaComponent(0.5)
        x.baseForegroundColor = .white
        x.cornerStyle = .capsule
        x.contentInsets = .zero
        let remove = UIButton(configuration: x)
        remove.accessibilityLabel = item.isImage ? "Remove image" : "Remove pasted text"
        remove.translatesAutoresizingMaskIntoConstraints = false
        remove.addAction(UIAction { [weak self] _ in self?.onRemove?() }, for: .touchUpInside)
        addSubview(remove)

        NSLayoutConstraint.activate([
            imageArea.topAnchor.constraint(equalTo: topAnchor),
            imageArea.bottomAnchor.constraint(equalTo: bottomAnchor),
            imageArea.leadingAnchor.constraint(equalTo: leadingAnchor),
            imageArea.trailingAnchor.constraint(equalTo: trailingAnchor),
            widthAnchor.constraint(equalToConstant: item.isImage ? 104 : 168),
            heightAnchor.constraint(equalToConstant: 104),
            remove.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            remove.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            remove.widthAnchor.constraint(equalToConstant: 26),
            remove.heightAnchor.constraint(equalToConstant: 26),
        ])
        imageArea.isAccessibilityElement = true
        imageArea.accessibilityTraits = .button
        imageArea.accessibilityIdentifier = item.isImage ? "composer-image" : "composer-pasted-text"
        if !item.isImage { imageArea.accessibilityLabel = pastedTextPreview(reference: item.name, text: String(decoding: item.data, as: UTF8.self)).title }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var accessibilityLabel: String? {
        get { imageArea.accessibilityLabel }
        set { imageArea.accessibilityLabel = newValue }
    }

    @objc private func open() { onOpen?() }
}
