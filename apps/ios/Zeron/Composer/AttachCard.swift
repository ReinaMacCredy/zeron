import Photos
import UIKit

/// The system keyboard's window. It is internal and sits above every app
/// window, so only a view inside it draws over the keys; Apple does not
/// support this, so it is found by type name as it becomes visible.
enum KeyboardWindow {
    private static weak var window: UIWindow?
    private static var observer: NSObjectProtocol?

    static func observe() {
        guard observer == nil else { return }
        observer = NotificationCenter.default.addObserver(forName: UIWindow.didBecomeVisibleNotification, object: nil, queue: .main) { note in
            guard let w = note.object as? UIWindow, String(describing: type(of: w)).contains("RemoteKeyboardWindow") else { return }
            window = w
        }
    }

    static func over(_ appWindow: UIWindow) -> UIWindow? {
        guard let w = window, !w.isHidden, w.alpha > 0, w.bounds == appWindow.bounds else { return nil }
        return w
    }
}

/// The composer's attach card. With the software keyboard up it is drawn in
/// the keyboard's window, over the live keys. Otherwise the menu sits above
/// the keyboard and Photos swaps the grid in as the text view's input view.
final class AttachCard: UIView {
    private static let menuRadius: CGFloat = 32
    private static let gridRadius: CGFloat = 34

    private weak var anchor: UIView?
    private weak var composer: UIView?
    private weak var input: UITextView?
    private let picker: AttachmentPicker
    private let keyboard: CGRect
    private let onClose: () -> Void
    private let scrim = UIControl()
    private let card = Glass.surface(radius: AttachCard.menuRadius)
    private let menu = UIStackView()
    private var overlay: KeysOverlay?
    private var observers: [NSObjectProtocol] = []
    private var grid: RecentPhotosGrid?
    private var gridHost: GridHost?
    private var inGrid = false
    private var closing = false

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    static func present(in window: UIWindow, input: UITextView, anchor: UIView, composer: UIView, keyboard: CGRect, keysUp: Bool, picker: AttachmentPicker, onClose: @escaping () -> Void) -> AttachCard {
        let card = AttachCard(window: window, input: input, anchor: anchor, composer: composer, keyboard: keyboard, picker: picker, onClose: onClose)
        window.addSubview(card)
        if keysUp, let keys = KeyboardWindow.over(window) { card.host(in: keys, appWindow: window) }
        card.open()
        return card
    }

    private init(window: UIWindow, input: UITextView, anchor: UIView, composer: UIView, keyboard: CGRect, picker: AttachmentPicker, onClose: @escaping () -> Void) {
        self.anchor = anchor
        self.composer = composer
        self.input = input
        self.picker = picker
        self.keyboard = keyboard
        self.onClose = onClose
        super.init(frame: window.bounds)
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        accessibilityViewIsModal = true
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
        menu.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 14, leading: 12, bottom: 14, trailing: 12)
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

    /// Both windows are full-screen at the origin, so the card keeps app-window
    /// coordinates there; `convert` across them returns NaN on iOS 27.
    private func host(in keys: UIWindow, appWindow: UIWindow) {
        let overlay = KeysOverlay(frame: keys.bounds)
        overlay.card = card
        overlay.overrideUserInterfaceStyle = appWindow.traitCollection.userInterfaceStyle
        overlay.tintColor = appWindow.tintColor
        overlay.onEscape = { [weak self] in _ = self?.accessibilityPerformEscape() }
        overlay.onLost = { [weak self] in self?.reattach() }
        overlay.addSubview(card)
        keys.addSubview(overlay)
        self.overlay = overlay
        observers = [UIResponder.keyboardWillHideNotification, UIApplication.didEnterBackgroundNotification].map {
            NotificationCenter.default.addObserver(forName: $0, object: nil, queue: .main) { [weak self] _ in self?.close() }
        }
    }

    /// UIKit rebuilds the keyboard's views without notice.
    private func reattach() {
        DispatchQueue.main.async { [weak self] in
            guard let self, let overlay = self.overlay, !overlay.ending, overlay.window == nil else { return }
            guard let window = self.window, let keys = KeyboardWindow.over(window) else { return self.close() }
            overlay.frame = keys.bounds
            keys.addSubview(overlay)
        }
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

    // MARK: Geometry (window coordinates)

    private var anchorRect: CGRect {
        let size = CGSize(width: 34, height: 34)
        guard let anchor, anchor.window != nil else {
            return CGRect(origin: CGPoint(x: 13, y: keyboard.minY - 50), size: size)
        }
        let c = anchor.convert(CGPoint(x: anchor.bounds.midX, y: anchor.bounds.midY), to: nil)
        return CGRect(x: c.x - size.width / 2, y: c.y - size.height / 2, width: size.width, height: size.height)
    }

    private var menuRect: CGRect {
        let left = (composer.map { $0.convert($0.bounds, to: nil).minX } ?? 10) + 3
        let w = min(280, bounds.width - left - 8)
        let h = CGFloat(menu.arrangedSubviews.count) * MenuRow.height + 28
        let bottom = overlay == nil ? keyboard.minY : keyboard.minY + 132
        return CGRect(x: left, y: bottom - h, width: w, height: h)
    }

    private var gridRect: CGRect {
        CGRect(x: 2, y: keyboard.minY + 6, width: bounds.width - 4, height: keyboard.height - 46)
    }

    /// Without the keyboard's window, the grid's card is drawn twice with the
    /// same frame: this view shows the part above the keyboard top, the grid
    /// host's twin the part below.
    private func place(_ rect: CGRect, radius: CGFloat?) {
        let corners: UICornerConfiguration = radius.map { .uniformCorners(radius: .fixed($0)) } ?? .capsule()
        card.frame = rect
        card.cornerConfiguration = corners
        gridHost?.card.frame = rect.offsetBy(dx: 0, dy: -keyboard.minY)
        gridHost?.card.cornerConfiguration = corners
    }

    // MARK: Transitions

    private func open() {
        let target = menuRect
        menu.frame = CGRect(origin: .zero, size: target.size)
        if reduceMotion {
            place(target, radius: Self.menuRadius)
            UIView.animate(withDuration: 0.12) {
                self.card.effect = Glass.effect()
                self.menu.alpha = 1
            }
        } else {
            place(anchorRect, radius: nil)
            UIView.animate(withDuration: 0.5, delay: 0, usingSpringWithDamping: 0.78, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
                self.place(target, radius: Self.menuRadius)
                self.card.effect = Glass.effect()
            }
            UIView.animate(withDuration: 0.24, delay: 0.1, options: [.allowUserInteraction, .beginFromCurrentState]) { self.menu.alpha = 1 }
        }
        UIAccessibility.post(notification: .screenChanged, argument: menu.arrangedSubviews.first)
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
        let grid = RecentPhotosGrid(limit: picker.gridLimit, showsCamera: picker.canTakePhoto, attached: picker.attached)
        grid.alpha = 0
        grid.frame = CGRect(origin: .zero, size: gridRect.size)
        grid.onBack = { [weak self] in self?.morph(toGrid: false) }
        grid.onCamera = { [weak self] in self?.close { $0.presentCamera() } }
        grid.onAllPhotos = { [weak self] in self?.close { $0.presentPhotos() } }
        grid.onAdd = { [weak self] ids in
            guard let self else { return }
            let picker = self.picker
            self.close()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) { picker.attach(ids) }
        }
        self.grid = grid
        return grid
    }

    private func morph(toGrid: Bool) {
        guard !closing, toGrid != inGrid else { return }
        inGrid = toGrid
        guard toGrid else { return runMorph(toGrid: false) }
        let grid = makeGrid()
        if overlay != nil {
            card.contentView.addSubview(grid)
            return runMorph(toGrid: true)
        }
        let host = GridHost(grid: grid, keyboard: keyboard)
        host.card.frame = card.frame.offsetBy(dx: 0, dy: -keyboard.minY)
        host.onBackground = { [weak self] in self?.close() }
        host.onEscape = { [weak self] in self?.morph(toGrid: false) }
        host.onMounted = { [weak self] in self?.runMorph(toGrid: true) }
        gridHost = host
        input?.inputView = host
        input?.reloadInputViews()
    }

    private func runMorph(toGrid: Bool) {
        guard let grid, !closing else { return }
        let target = toGrid ? gridRect : menuRect
        let radius = toGrid ? Self.gridRadius : Self.menuRadius
        let shown: UIView = toGrid ? grid : menu
        let hidden: UIView = toGrid ? menu : grid
        let done = { [weak self] in
            guard let self, !toGrid, !self.inGrid, !self.closing else { return }
            self.dropGrid()
        }
        if reduceMotion {
            place(target, radius: radius)
            UIView.animate(withDuration: 0.12) {
                hidden.alpha = 0
                shown.alpha = 1
            } completion: { _ in done() }
        } else {
            UIView.animate(withDuration: 0.52, delay: 0, usingSpringWithDamping: 0.84, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
                self.place(target, radius: radius)
            }
            UIView.animate(withDuration: 0.16, delay: 0, options: [.beginFromCurrentState]) { hidden.alpha = 0 } completion: { _ in done() }
            UIView.animate(withDuration: 0.26, delay: 0.12, options: [.beginFromCurrentState, .allowUserInteraction]) { shown.alpha = 1 }
        }
        UIAccessibility.post(notification: .screenChanged, argument: shown)
    }

    private func dropGrid() {
        grid?.removeFromSuperview()
        grid = nil
        guard let host = gridHost else { return }
        gridHost = nil
        if input?.inputView === host {
            input?.inputView = nil
            input?.reloadInputViews()
        }
    }

    func close(then: ((AttachmentPicker) -> Void)? = nil) {
        guard !closing else { return }
        closing = true
        isUserInteractionEnabled = false
        card.isUserInteractionEnabled = false
        gridHost?.isUserInteractionEnabled = false
        onClose()
        let finish = { [self] in
            dropGrid()
            overlay?.ending = true
            overlay?.removeFromSuperview()
            observers.forEach(NotificationCenter.default.removeObserver)
            removeFromSuperview()
            then?(picker)
        }
        let contents = [menu, grid].compactMap { $0 }
        if reduceMotion {
            UIView.animate(withDuration: 0.12) {
                self.alpha = 0
                self.card.alpha = 0
                self.gridHost?.card.alpha = 0
            } completion: { _ in finish() }
            return
        }
        UIView.animate(withDuration: 0.14, delay: 0, options: [.beginFromCurrentState]) { contents.forEach { $0.alpha = 0 } } completion: { _ in self.dropGrid() }
        UIView.animate(withDuration: 0.4, delay: 0, usingSpringWithDamping: 0.9, initialSpringVelocity: 0, options: [.beginFromCurrentState]) {
            self.place(self.anchorRect, radius: nil)
            self.card.effect = nil
            self.gridHost?.card.effect = nil
        } completion: { _ in finish() }
    }
}

/// The card's layer in the keyboard's window: touches outside the card go on
/// to the keys under it.
private final class KeysOverlay: UIView {
    weak var card: UIView?
    var ending = false
    var onLost: () -> Void = {}
    var onEscape: () -> Void = {}

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard let card, card.frame.contains(point) else { return nil }
        return super.hitTest(point, with: event)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil, !ending { onLost() }
    }

    override func accessibilityPerformEscape() -> Bool {
        onEscape()
        return true
    }
}

/// The grid's half of the card, mounted as the text view's input view when
/// the keyboard's window can't be reached.
private final class GridHost: UIInputView {
    let card = Glass.surface(radius: 34)
    var onMounted: (() -> Void)?
    var onBackground: (() -> Void)?
    var onEscape: (() -> Void)?

    init(grid: RecentPhotosGrid, keyboard: CGRect) {
        super.init(frame: CGRect(origin: .zero, size: keyboard.size), inputViewStyle: .keyboard)
        allowsSelfSizing = false
        let background = UIControl(frame: bounds)
        background.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        background.addAction(UIAction { [weak self] _ in self?.onBackground?() }, for: .touchUpInside)
        addSubview(background)
        card.clipsToBounds = true
        card.effect = Glass.effect()
        addSubview(card)
        card.contentView.addSubview(grid)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func accessibilityPerformEscape() -> Bool {
        onEscape?()
        return true
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, bounds.width > 0, let mounted = onMounted else { return }
        onMounted = nil
        // Mounting lays out without animation; start the spring a turn later.
        DispatchQueue.main.async(execute: mounted)
    }
}

private final class MenuRow: UIControl {
    static let height: CGFloat = 64

    private let highlight = UIView()

    init(title: String, symbol: String) {
        super.init(frame: .zero)
        highlight.backgroundColor = Palette.controlFill
        highlight.layer.cornerRadius = 22
        highlight.layer.cornerCurve = .continuous
        highlight.alpha = 0
        highlight.isUserInteractionEnabled = false
        highlight.translatesAutoresizingMaskIntoConstraints = false
        addSubview(highlight)

        let well = UIView()
        well.backgroundColor = Palette.controlFill
        well.layer.cornerRadius = 22
        well.isUserInteractionEnabled = false
        well.translatesAutoresizingMaskIntoConstraints = false
        addSubview(well)
        let icon = UIImageView(image: UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 19, weight: .regular)))
        icon.tintColor = Palette.text
        icon.contentMode = .center
        icon.translatesAutoresizingMaskIntoConstraints = false
        well.addSubview(icon)

        let label = UILabel()
        label.text = title
        label.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: .systemFont(ofSize: 18), maximumPointSize: 26)
        label.adjustsFontForContentSizeCategory = true
        label.adjustsFontSizeToFitWidth = true
        label.minimumScaleFactor = 0.7
        label.textColor = Palette.text
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.height),
            highlight.topAnchor.constraint(equalTo: topAnchor),
            highlight.bottomAnchor.constraint(equalTo: bottomAnchor),
            highlight.leadingAnchor.constraint(equalTo: leadingAnchor),
            highlight.trailingAnchor.constraint(equalTo: trailingAnchor),
            well.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            well.centerYAnchor.constraint(equalTo: centerYAnchor),
            well.widthAnchor.constraint(equalToConstant: 44),
            well.heightAnchor.constraint(equalToConstant: 44),
            icon.centerXAnchor.constraint(equalTo: well.centerXAnchor),
            icon.centerYAnchor.constraint(equalTo: well.centerYAnchor),
            label.leadingAnchor.constraint(equalTo: well.trailingAnchor, constant: 16),
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
