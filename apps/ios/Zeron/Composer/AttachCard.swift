import Photos
import UIKit

/// The composer's attach card: it takes the keyboard's place as the text
/// view's input view, springs out of the "+" as a Liquid Glass menu, and
/// morphs in place into the recent photos grid. The keyboard window draws
/// above every app window on iOS 27, so the input view is the only public
/// way to put the card where the keyboard is.
final class AttachCard: UIInputView {
    private static let menuRadius: CGFloat = 30

    private weak var anchor: UIView?
    private weak var input: UITextView?
    private let picker: AttachmentPicker
    private let onClose: () -> Void
    private let scrim = UIControl()
    private let card = Glass.surface(radius: AttachCard.menuRadius)
    private let menu = UIStackView()
    private var grid: RecentPhotosGrid?
    private var inGrid = false
    private var closing = false
    private var laidOutSize = CGSize.zero

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    static func present(in input: UITextView, from anchor: UIView, picker: AttachmentPicker, height: CGFloat, onClose: @escaping () -> Void) -> AttachCard {
        let card = AttachCard(anchor: anchor, input: input, picker: picker, height: height, onClose: onClose)
        input.inputView = card
        if input.isFirstResponder { input.reloadInputViews() } else { input.becomeFirstResponder() }
        return card
    }

    private init(anchor: UIView, input: UITextView, picker: AttachmentPicker, height: CGFloat, onClose: @escaping () -> Void) {
        self.anchor = anchor
        self.input = input
        self.picker = picker
        self.onClose = onClose
        super.init(frame: CGRect(x: 0, y: 0, width: anchor.window?.bounds.width ?? 0, height: height), inputViewStyle: .keyboard)
        build()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func build() {
        scrim.frame = bounds
        scrim.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        scrim.isAccessibilityElement = true
        scrim.accessibilityLabel = "Close"
        scrim.accessibilityTraits = .button
        scrim.addAction(UIAction { [weak self] _ in self?.close() }, for: .touchUpInside)
        addSubview(scrim)

        card.effect = nil
        card.clipsToBounds = true
        card.accessibilityIdentifier = "attach-card"
        addSubview(card)

        menu.axis = .vertical
        menu.isLayoutMarginsRelativeArrangement = true
        menu.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 10, leading: 10, bottom: 10, trailing: 10)
        menu.alpha = 0
        if picker.canTakePhoto {
            menu.addArrangedSubview(row("camera", "Camera", symbol: "camera") { [weak self] in
                self?.close { $0.presentCamera() }
            })
        }
        menu.addArrangedSubview(row("photos", "Photos", symbol: "photo.on.rectangle") { [weak self] in self?.showPhotos() })
        menu.addArrangedSubview(row("files", "Files", symbol: "paperclip") { [weak self] in
            self?.close { $0.presentFiles() }
        })
        if picker.canPaste {
            menu.addArrangedSubview(row("paste", "Paste image", symbol: "doc.on.clipboard") { [weak self] in
                self?.close { $0.pasteImages() }
            })
        }
        card.contentView.addSubview(menu)
    }

    private func row(_ id: String, _ title: String, symbol: String, action: @escaping () -> Void) -> MenuRow {
        let r = MenuRow(title: title, symbol: symbol)
        r.accessibilityIdentifier = "attach-\(id)"
        r.addAction(UIAction { _ in action() }, for: .touchUpInside)
        return r
    }

    override func accessibilityPerformEscape() -> Bool {
        if inGrid { morph(toGrid: false) } else { close() }
        return true
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, bounds.width > 0, bounds.size != laidOutSize else { return }
        let first = laidOutSize == .zero
        laidOutSize = bounds.size
        if first { return open() }
        guard !closing else { return }
        card.frame = inGrid ? gridRect : menuRect
        menu.frame = CGRect(origin: .zero, size: menuRect.size)
        grid?.frame = CGRect(origin: .zero, size: gridRect.size)
    }

    // MARK: Geometry

    /// The "+" sits just above this view, in another window.
    private var anchorRect: CGRect {
        let size = CGSize(width: 34, height: 34)
        guard let anchor, window != nil, anchor.window != nil else {
            return CGRect(origin: CGPoint(x: 10, y: 8), size: size)
        }
        let x = anchor.convert(CGPoint(x: anchor.bounds.midX, y: 0), to: nil).x - convert(CGPoint.zero, to: nil).x
        return CGRect(x: x - size.width / 2, y: 8, width: size.width, height: size.height)
    }

    private var menuRect: CGRect {
        let w = min(288, bounds.width - 20)
        let h = min(CGFloat(menu.arrangedSubviews.count) * MenuRow.height + 20, bounds.height - safeAreaInsets.bottom - 20)
        let x = min(max(10, anchorRect.minX - 6), bounds.width - w - 10)
        return CGRect(x: x, y: 10, width: w, height: h)
    }

    private var gridRect: CGRect {
        CGRect(x: 6, y: 6, width: bounds.width - 12, height: bounds.height - safeAreaInsets.bottom - 12)
    }

    // MARK: Transitions

    private func open() {
        let target = menuRect
        menu.frame = CGRect(origin: .zero, size: target.size)
        if reduceMotion {
            card.frame = target
            UIView.animate(withDuration: 0.22) { self.reveal(self.menu) }
        } else {
            card.frame = anchorRect
            card.cornerConfiguration = .capsule()
            UIView.animate(withDuration: 0.5, delay: 0, usingSpringWithDamping: 0.78, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
                self.card.frame = target
                self.card.cornerConfiguration = .uniformCorners(radius: .fixed(Self.menuRadius))
                self.card.effect = Glass.effect()
            }
            UIView.animate(withDuration: 0.24, delay: 0.1, options: [.allowUserInteraction, .beginFromCurrentState]) { self.menu.alpha = 1 }
        }
        UIAccessibility.post(notification: .screenChanged, argument: menu.arrangedSubviews.first)
    }

    private func reveal(_ content: UIView) {
        card.effect = Glass.effect()
        content.alpha = 1
    }

    private func showPhotos() {
        switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
        case .authorized, .limited:
            morph(toGrid: true)
        case .notDetermined:
            PHPhotoLibrary.requestAuthorization(for: .readWrite) { status in
                DispatchQueue.main.async { [weak self] in
                    guard let self, !self.closing else { return }
                    if status == .authorized || status == .limited {
                        self.morph(toGrid: true)
                    } else {
                        self.close { $0.presentPhotos() }
                    }
                }
            }
        default:
            close { $0.presentPhotos() }
        }
    }

    private func makeGrid() -> RecentPhotosGrid {
        let g = RecentPhotosGrid(limit: picker.limit, showsCamera: picker.canTakePhoto)
        g.alpha = 0
        g.onBack = { [weak self] in self?.morph(toGrid: false) }
        g.onCamera = { [weak self] in self?.close { $0.presentCamera() } }
        g.onAllPhotos = { [weak self] in self?.close { $0.presentPhotos() } }
        g.onAdd = { [weak self] assets in self?.close { $0.attach(assets) } }
        card.contentView.addSubview(g)
        grid = g
        return g
    }

    private func morph(toGrid: Bool) {
        guard !closing else { return }
        inGrid = toGrid
        let g = grid ?? makeGrid()
        let target = toGrid ? gridRect : menuRect
        let corners: UICornerConfiguration = .uniformCorners(radius: toGrid ? .containerConcentric(minimum: 34) : .fixed(Self.menuRadius))
        if toGrid { g.frame = CGRect(origin: .zero, size: target.size) }
        let shown: UIView = toGrid ? g : menu
        let hidden: UIView = toGrid ? menu : g
        if reduceMotion {
            UIView.animate(withDuration: 0.15) {
                self.card.effect = nil
                hidden.alpha = 0
            } completion: { _ in
                self.card.frame = target
                self.card.cornerConfiguration = corners
                UIView.animate(withDuration: 0.22) { self.reveal(shown) }
            }
        } else {
            UIView.animate(withDuration: 0.52, delay: 0, usingSpringWithDamping: 0.84, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
                self.card.frame = target
                self.card.cornerConfiguration = corners
            }
            UIView.animate(withDuration: 0.16, delay: 0, options: [.beginFromCurrentState]) { hidden.alpha = 0 }
            UIView.animate(withDuration: 0.26, delay: 0.12, options: [.beginFromCurrentState, .allowUserInteraction]) { shown.alpha = 1 }
        }
        UIAccessibility.post(notification: .screenChanged, argument: shown)
    }

    func close(then: ((AttachmentPicker) -> Void)? = nil) {
        guard !closing else { return }
        closing = true
        onClose()
        let finish = { [self] in
            if input?.inputView === self {
                input?.inputView = nil
                input?.reloadInputViews()
            }
            then?(picker)
        }
        let contents = [menu, grid].compactMap { $0 }
        if reduceMotion {
            UIView.animate(withDuration: 0.2) {
                self.card.effect = nil
                contents.forEach { $0.alpha = 0 }
            } completion: { _ in finish() }
            return
        }
        UIView.animate(withDuration: 0.14, delay: 0, options: [.beginFromCurrentState]) { contents.forEach { $0.alpha = 0 } }
        UIView.animate(withDuration: 0.4, delay: 0, usingSpringWithDamping: 0.9, initialSpringVelocity: 0, options: [.beginFromCurrentState]) {
            self.card.frame = self.anchorRect
            self.card.cornerConfiguration = .capsule()
            self.card.effect = nil
        } completion: { _ in finish() }
    }
}

private final class MenuRow: UIControl {
    static let height: CGFloat = 58

    private let highlight = UIView()

    init(title: String, symbol: String) {
        super.init(frame: .zero)
        highlight.backgroundColor = Palette.controlFill
        highlight.layer.cornerRadius = 20
        highlight.layer.cornerCurve = .continuous
        highlight.alpha = 0
        highlight.isUserInteractionEnabled = false
        highlight.translatesAutoresizingMaskIntoConstraints = false
        addSubview(highlight)

        let well = UIView()
        well.backgroundColor = Palette.controlFill
        well.layer.cornerRadius = 20
        well.isUserInteractionEnabled = false
        well.translatesAutoresizingMaskIntoConstraints = false
        addSubview(well)
        let icon = UIImageView(image: UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .medium)))
        icon.tintColor = Palette.text
        icon.contentMode = .center
        icon.translatesAutoresizingMaskIntoConstraints = false
        well.addSubview(icon)

        let label = UILabel()
        label.text = title
        label.font = UIFont.systemFont(ofSize: 17)
        label.textColor = Palette.text
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.height),
            highlight.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            highlight.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
            highlight.leadingAnchor.constraint(equalTo: leadingAnchor),
            highlight.trailingAnchor.constraint(equalTo: trailingAnchor),
            well.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            well.centerYAnchor.constraint(equalTo: centerYAnchor),
            well.widthAnchor.constraint(equalToConstant: 40),
            well.heightAnchor.constraint(equalToConstant: 40),
            icon.centerXAnchor.constraint(equalTo: well.centerXAnchor),
            icon.centerYAnchor.constraint(equalTo: well.centerYAnchor),
            label.leadingAnchor.constraint(equalTo: well.trailingAnchor, constant: 14),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityLabel = title
        accessibilityTraits = .button
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet {
            UIView.animate(withDuration: isHighlighted ? 0.08 : 0.25, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
                self.highlight.alpha = self.isHighlighted ? 1 : 0
            }
        }
    }
}
