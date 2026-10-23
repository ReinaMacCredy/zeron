import UIKit

/// Full-screen preview of a staged image: it zooms out of its composer
/// thumbnail, and back into it on close; Remove drops the image.
final class AttachmentPreview: UIViewController {
    private let item: StagedImage
    private let index: Int
    private let count: Int
    private weak var source: UIView?
    private let onRemove: () -> Void
    private let backdrop = UIView()
    private let imageView = UIImageView()
    private let countLabel = UILabel()
    private let removeControl = UIControl()
    private var controls: [UIView] = []
    private var leaving = false

    private var reduceMotion: Bool { UIAccessibility.isReduceMotionEnabled }

    init(item: StagedImage, index: Int, count: Int, source: UIView, onRemove: @escaping () -> Void) {
        self.item = item
        self.index = index
        self.count = count
        self.source = source
        self.onRemove = onRemove
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .overFullScreen
        modalPresentationCapturesStatusBarAppearance = true
        overrideUserInterfaceStyle = .dark
    }

    required init?(coder: NSCoder) { fatalError() }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear
        backdrop.backgroundColor = .black
        backdrop.alpha = 0
        backdrop.frame = view.bounds
        backdrop.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(backdrop)

        imageView.image = item.thumbnail
        imageView.contentMode = .scaleAspectFill
        imageView.clipsToBounds = true
        imageView.layer.cornerCurve = .continuous
        imageView.isAccessibilityElement = true
        imageView.accessibilityLabel = "Image \(index + 1) of \(count)"
        imageView.accessibilityTraits = .image
        view.addSubview(imageView)
        let data = item.data
        Task.detached(priority: .userInitiated) {
            guard let full = UIImage(data: data)?.preparingForDisplay() else { return }
            await MainActor.run { [weak self] in self?.imageView.image = full }
        }

        let close = Glass.circleButton(symbol: "xmark", size: 48, pointSize: 17, action: UIAction { [weak self] _ in self?.dismissToThumbnail() })
        close.accessibilityLabel = "Close preview"
        close.accessibilityIdentifier = "attachment-preview-close"
        view.addSubview(close)

        countLabel.text = "\(index + 1) / \(count)"
        countLabel.font = UIFont.preferredFont(forTextStyle: .subheadline)
        countLabel.adjustsFontForContentSizeCategory = true
        countLabel.textColor = UIColor.white.withAlphaComponent(0.7)
        countLabel.accessibilityLabel = "Image \(index + 1) of \(count)"
        countLabel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(countLabel)

        let circle = UIView()
        circle.backgroundColor = UIColor(red: 60 / 255, green: 60 / 255, blue: 64 / 255, alpha: 0.8)
        circle.layer.cornerRadius = 28
        circle.isUserInteractionEnabled = false
        circle.translatesAutoresizingMaskIntoConstraints = false
        let trash = UIImageView(image: UIImage(systemName: "trash", withConfiguration: UIImage.SymbolConfiguration(pointSize: 21, weight: .regular)))
        trash.tintColor = .white
        trash.translatesAutoresizingMaskIntoConstraints = false
        circle.addSubview(trash)
        let label = UILabel()
        label.text = "Remove"
        label.font = UIFont.preferredFont(forTextStyle: .subheadline)
        label.adjustsFontForContentSizeCategory = true
        label.textColor = .white
        label.translatesAutoresizingMaskIntoConstraints = false
        removeControl.addSubview(circle)
        removeControl.addSubview(label)
        removeControl.isAccessibilityElement = true
        removeControl.accessibilityLabel = "Remove image"
        removeControl.accessibilityTraits = .button
        removeControl.accessibilityIdentifier = "attachment-preview-remove"
        removeControl.addAction(UIAction { [weak self] _ in self?.removeImage() }, for: .touchUpInside)
        removeControl.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(removeControl)

        NSLayoutConstraint.activate([
            close.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            close.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 4),
            countLabel.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            countLabel.centerYAnchor.constraint(equalTo: close.centerYAnchor),
            circle.topAnchor.constraint(equalTo: removeControl.topAnchor),
            circle.centerXAnchor.constraint(equalTo: removeControl.centerXAnchor),
            circle.widthAnchor.constraint(equalToConstant: 56),
            circle.heightAnchor.constraint(equalToConstant: 56),
            trash.centerXAnchor.constraint(equalTo: circle.centerXAnchor),
            trash.centerYAnchor.constraint(equalTo: circle.centerYAnchor),
            label.topAnchor.constraint(equalTo: circle.bottomAnchor, constant: 8),
            label.centerXAnchor.constraint(equalTo: removeControl.centerXAnchor),
            label.bottomAnchor.constraint(equalTo: removeControl.bottomAnchor),
            removeControl.widthAnchor.constraint(greaterThanOrEqualTo: circle.widthAnchor),
            removeControl.widthAnchor.constraint(greaterThanOrEqualTo: label.widthAnchor),
            removeControl.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            removeControl.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -56),
        ])
        controls = [close, countLabel, removeControl]
        controls.forEach { $0.alpha = 0 }
    }

    override func accessibilityPerformEscape() -> Bool {
        dismissToThumbnail()
        return true
    }

    private var sourceFrame: CGRect {
        guard let source, source.window != nil else { return fittedFrame }
        return source.convert(source.bounds, to: view)
    }

    private var fittedFrame: CGRect {
        let size = item.thumbnail.size
        let ratio = size.height > 0 ? size.width / size.height : 1
        let h = min(view.bounds.width / ratio, 640)
        let w = h * ratio
        return CGRect(x: (view.bounds.width - w) / 2, y: (view.bounds.height - h) / 2, width: w, height: h)
    }

    override func viewIsAppearing(_ animated: Bool) {
        super.viewIsAppearing(animated)
        view.layoutIfNeeded()
        if reduceMotion {
            imageView.frame = fittedFrame
            view.alpha = 0
            backdrop.alpha = 1
            controls.forEach { $0.alpha = 1 }
            source?.alpha = 0
            UIView.animate(withDuration: 0.12) { self.view.alpha = 1 }
            UIAccessibility.post(notification: .screenChanged, argument: imageView)
            return
        }
        imageView.frame = sourceFrame
        imageView.layer.cornerRadius = 22
        source?.alpha = 0
        UIView.animate(withDuration: 0.45, delay: 0, usingSpringWithDamping: 0.88, initialSpringVelocity: 0, options: [.allowUserInteraction]) {
            self.imageView.frame = self.fittedFrame
            self.imageView.layer.cornerRadius = 0
        }
        UIView.animate(withDuration: 0.25) { self.backdrop.alpha = 1 }
        UIView.animate(withDuration: 0.2, delay: 0.15, options: [.allowUserInteraction]) { self.controls.forEach { $0.alpha = 1 } }
        UIAccessibility.post(notification: .screenChanged, argument: imageView)
    }

    private func dismissToThumbnail() {
        guard !leaving else { return }
        leaving = true
        let finish = { [weak self] in
            self?.source?.alpha = 1
            self?.dismiss(animated: false)
        }
        if reduceMotion {
            UIView.animate(withDuration: 0.12) { self.view.alpha = 0 } completion: { _ in finish() }
            return
        }
        UIView.animate(withDuration: 0.12) { self.controls.forEach { $0.alpha = 0 } }
        UIView.animate(withDuration: 0.22) { self.backdrop.alpha = 0 }
        UIView.animate(withDuration: 0.4, delay: 0, usingSpringWithDamping: 0.9, initialSpringVelocity: 0, options: []) {
            self.imageView.frame = self.sourceFrame
            self.imageView.layer.cornerRadius = 22
        } completion: { _ in finish() }
    }

    private func removeImage() {
        guard !leaving else { return }
        leaving = true
        let finish = { [weak self] in
            guard let self else { return }
            self.source?.alpha = 1
            self.dismiss(animated: false)
            self.onRemove()
        }
        let duration = reduceMotion ? 0.12 : 0.2
        UIView.animate(withDuration: reduceMotion ? 0.12 : 0.12) { self.controls.forEach { $0.alpha = 0 } }
        UIView.animate(withDuration: reduceMotion ? 0.12 : 0.22) { self.backdrop.alpha = 0 }
        UIView.animate(withDuration: duration) {
            self.imageView.alpha = 0
            if !self.reduceMotion { self.imageView.transform = CGAffineTransform(scaleX: 0.92, y: 0.92) }
        } completion: { _ in finish() }
    }
}
