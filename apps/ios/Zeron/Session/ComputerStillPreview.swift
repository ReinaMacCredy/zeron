import UIKit

final class ComputerStillPreview: UIView {
    static let pollInterval: Duration = .seconds(1)
    static let maxSize = CGSize(width: 168, height: 120)
    static let completionFlash: Duration = .milliseconds(1_600)
    static let hideAll = Notification.Name("ComputerStillPreview.hideAll")

    var fetch: (() async -> (sequence: UInt64, image: UIImage)?)?
    var working = false {
        didSet {
            guard working != oldValue else { return }
            let finished = !working && alpha > 0.5 && !dismissed
            if !working { dismissed = false }
            if finished { return complete() }
            restart()
        }
    }
    private(set) var receiving = false

    private let imageView = UIImageView()
    private let hide = UIButton(type: .custom)
    private let live = UILabel()
    private let check = UIImageView(image: UIImage(systemName: "checkmark.circle.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 30, weight: .semibold)))
    private var hideAllObserver: NSObjectProtocol?
    private var width: NSLayoutConstraint!
    private var height: NSLayoutConstraint!
    private var sequence: UInt64?
    private var dismissed = false
    private var poll: Task<Void, Never>?

    override init(frame: CGRect) {
        super.init(frame: frame)
        width = widthAnchor.constraint(equalToConstant: Self.maxSize.width)
        height = heightAnchor.constraint(equalToConstant: Self.maxSize.height)
        NSLayoutConstraint.activate([width, height])
        imageView.frame = bounds
        imageView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        imageView.contentMode = .scaleAspectFit
        imageView.backgroundColor = .black
        imageView.clipsToBounds = true
        imageView.layer.cornerRadius = 10
        imageView.layer.cornerCurve = .continuous
        imageView.layer.borderWidth = 1
        imageView.layer.borderColor = UIColor(white: 1, alpha: 0.12).cgColor
        imageView.isAccessibilityElement = true
        imageView.accessibilityLabel = "Computer preview"
        imageView.accessibilityIdentifier = "computer-still"
        addSubview(imageView)

        let mark = UIImageView(image: UIImage(systemName: "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 10, weight: .semibold)))
        mark.tintColor = .white
        mark.contentMode = .center
        mark.backgroundColor = UIColor.black.withAlphaComponent(0.6)
        mark.layer.cornerRadius = 11
        mark.frame = CGRect(x: 11, y: 11, width: 22, height: 22)
        mark.isUserInteractionEnabled = false
        hide.addSubview(mark)
        hide.accessibilityLabel = "Hide Computer preview"
        hide.accessibilityIdentifier = "computer-still-hide"
        hide.showsMenuAsPrimaryAction = true
        hide.menu = UIMenu(children: [
            UIAction(title: "Hide for this task", image: UIImage(systemName: "eye.slash")) { [weak self] _ in self?.dismiss() },
            UIAction(title: "Hide for all active tasks", image: UIImage(systemName: "rectangle.stack.badge.minus")) { _ in
                NotificationCenter.default.post(name: Self.hideAll, object: nil)
            },
        ])
        addSubview(hide)

        live.text = "Live"
        live.font = Fonts.ui(.sansSemibold, 10)
        live.textColor = .white
        live.textAlignment = .center
        live.backgroundColor = UIColor.black.withAlphaComponent(0.6)
        live.layer.cornerRadius = 8
        live.layer.masksToBounds = true
        live.accessibilityIdentifier = "computer-still-live"
        addSubview(live)

        check.tintColor = .white
        check.layer.shadowOpacity = 0.4
        check.layer.shadowRadius = 4
        check.layer.shadowOffset = .zero
        check.alpha = 0
        check.isAccessibilityElement = true
        check.accessibilityLabel = "Computer use finished"
        check.accessibilityIdentifier = "computer-still-done"
        addSubview(check)
        hideAllObserver = NotificationCenter.default.addObserver(forName: Self.hideAll, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.working else { return }
                self.dismiss()
            }
        }

        alpha = 0
        isHidden = true
        isUserInteractionEnabled = false
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        hide.frame = CGRect(x: bounds.maxX - 33, y: -11, width: 44, height: 44)
        let pill = live.intrinsicContentSize
        live.frame = CGRect(x: 6, y: bounds.maxY - 22, width: pill.width + 12, height: 16)
        check.sizeToFit()
        check.center = CGPoint(x: bounds.midX, y: bounds.midY)
    }

    // The hide button hangs over the corner, outside the image's bounds.
    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        super.point(inside: point, with: event) || hide.frame.contains(point)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        restart()
    }

    private func restart() {
        poll?.cancel()
        poll = nil
        check.alpha = 0
        live.isHidden = false
        guard working, window != nil else { return show(nil) }
        poll = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let fetch = self?.fetch else { return }
                let still = await fetch()
                guard !Task.isCancelled, let self else { return }
                self.show(still)
                try? await Task.sleep(for: Self.pollInterval)
            }
        }
    }

    private func show(_ still: (sequence: UInt64, image: UIImage)?) {
        receiving = still != nil
        guard let still else {
            sequence = nil
            return setVisible(false)
        }
        if still.sequence != sequence {
            sequence = still.sequence
            imageView.image = still.image
            let size = Self.fitted(still.image.size)
            width.constant = size.width
            height.constant = size.height
        }
        setVisible(!dismissed)
    }

    static func fitted(_ image: CGSize) -> CGSize {
        guard image.width > 0, image.height > 0 else { return maxSize }
        let scale = min(maxSize.width / image.width, maxSize.height / image.height)
        return CGSize(width: (image.width * scale).rounded(), height: (image.height * scale).rounded())
    }

    private func complete() {
        poll?.cancel()
        poll = nil
        receiving = false
        live.isHidden = true
        let reveal = { self.check.alpha = 1 }
        if UIAccessibility.isReduceMotionEnabled { reveal() } else { UIView.animate(withDuration: 0.18, animations: reveal) }
        poll = Task { @MainActor [weak self] in
            try? await Task.sleep(for: Self.completionFlash)
            guard !Task.isCancelled, let self else { return }
            self.poll = nil
            self.restart()
        }
    }

    private func dismiss() {
        dismissed = true
        setVisible(false)
    }

    private func setVisible(_ visible: Bool) {
        isUserInteractionEnabled = visible
        guard (alpha > 0.5) != visible else { return }
        if visible { isHidden = false }
        let change = { self.alpha = visible ? 1 : 0 }
        let done = { (_: Bool) in if self.alpha < 0.5 { self.isHidden = true } }
        if UIAccessibility.isReduceMotionEnabled || window == nil {
            change()
            return done(true)
        }
        UIView.animate(withDuration: 0.18, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction], animations: change, completion: done)
    }
}
