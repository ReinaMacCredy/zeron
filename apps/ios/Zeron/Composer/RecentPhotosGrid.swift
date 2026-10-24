import Photos
import UIKit

/// The attach card's inline library: recent photos in three columns (camera
/// first when there is one), numbered multi-select, and a bottom bar with
/// back and All Photos / Add N.
final class RecentPhotosGrid: UIView, UICollectionViewDataSource, UICollectionViewDelegate, UICollectionViewDataSourcePrefetching {
    var onBack: (() -> Void)?
    var onCamera: (() -> Void)?
    var onAllPhotos: (() -> Void)?
    var onAdd: (([String]) -> Void)?

    private let limit: Int
    private let showsCamera: Bool
    private let assets: PHFetchResult<PHAsset>
    private let imageManager = PHCachingImageManager()
    private let attached: [String]
    private var selected: [String]
    private var thumbSize = CGSize(width: 200, height: 200)
    private let collection: UICollectionView
    private let backButton = UIButton(type: .system)
    private let mainButton = UIButton(type: .system)

    init(limit: Int, showsCamera: Bool, attached: [String]) {
        self.limit = limit
        self.showsCamera = showsCamera
        self.attached = attached
        selected = attached
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.fetchLimit = 240
        assets = PHAsset.fetchAssets(with: .image, options: options)

        let item = NSCollectionLayoutItem(layoutSize: .init(widthDimension: .fractionalWidth(1 / 3), heightDimension: .fractionalHeight(1)))
        item.contentInsets = NSDirectionalEdgeInsets(top: 1, leading: 1, bottom: 1, trailing: 1)
        let group = NSCollectionLayoutGroup.horizontal(layoutSize: .init(widthDimension: .fractionalWidth(1), heightDimension: .fractionalWidth(1 / 3)), subitems: [item])
        let section = NSCollectionLayoutSection(group: group)
        collection = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout(section: section))
        super.init(frame: .zero)
        build()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func build() {
        collection.backgroundColor = .clear
        collection.dataSource = self
        collection.delegate = self
        collection.prefetchDataSource = self
        collection.allowsMultipleSelection = true
        collection.contentInset.bottom = 72
        collection.verticalScrollIndicatorInsets.bottom = 72
        collection.register(PhotoCell.self, forCellWithReuseIdentifier: "photo")
        collection.register(CameraCell.self, forCellWithReuseIdentifier: "camera")
        collection.accessibilityIdentifier = "attach-grid"
        collection.translatesAutoresizingMaskIntoConstraints = false
        addSubview(collection)

        var back = UIButton.Configuration.glass()
        back.image = UIImage(systemName: "chevron.left", withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
        back.baseForegroundColor = Palette.text
        back.cornerStyle = .capsule
        backButton.configuration = back
        backButton.accessibilityLabel = "Back"
        backButton.accessibilityIdentifier = "attach-grid-back"
        backButton.addAction(UIAction { [weak self] _ in self?.onBack?() }, for: .touchUpInside)
        backButton.translatesAutoresizingMaskIntoConstraints = false
        addSubview(backButton)

        mainButton.accessibilityIdentifier = "attach-grid-main"
        mainButton.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            if self.selected == self.attached { self.onAllPhotos?() } else { self.onAdd?(self.selected) }
        }, for: .touchUpInside)
        mainButton.translatesAutoresizingMaskIntoConstraints = false
        addSubview(mainButton)
        refreshMainButton()

        NSLayoutConstraint.activate([
            collection.topAnchor.constraint(equalTo: topAnchor),
            collection.leadingAnchor.constraint(equalTo: leadingAnchor),
            collection.trailingAnchor.constraint(equalTo: trailingAnchor),
            collection.bottomAnchor.constraint(equalTo: bottomAnchor),
            backButton.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            backButton.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            backButton.widthAnchor.constraint(equalToConstant: 48),
            backButton.heightAnchor.constraint(equalToConstant: 48),
            mainButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            mainButton.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            mainButton.heightAnchor.constraint(equalToConstant: 48),
        ])
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let side = (bounds.width - 4) / 3 * (window?.screen.scale ?? 3)
        if side > 0 { thumbSize = CGSize(width: side, height: side) }
    }

    private func refreshMainButton() {
        let changed = selected != attached
        let added = selected.filter { !attached.contains($0) }.count
        var config: UIButton.Configuration = changed ? .prominentGlass() : .glass()
        config.title = !changed ? "All Photos" : added == 0 ? "Done" : "Add \(added)"
        config.baseForegroundColor = changed ? .white : Palette.text
        if changed { config.baseBackgroundColor = Palette.accent }
        config.cornerStyle = .capsule
        config.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 20)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var a = attrs
            a.font = UIFont.systemFont(ofSize: 17, weight: .semibold)
            return a
        }
        UIView.animate(withDuration: 0.25, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction]) {
            self.mainButton.configuration = config
            self.layoutIfNeeded()
        }
    }

    // MARK: Data

    private var offset: Int { showsCamera ? 1 : 0 }

    private func asset(at indexPath: IndexPath) -> PHAsset? {
        let i = indexPath.item - offset
        return i >= 0 && i < assets.count ? assets[i] : nil
    }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int {
        assets.count + offset
    }

    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        guard let asset = asset(at: indexPath) else {
            return collectionView.dequeueReusableCell(withReuseIdentifier: "camera", for: indexPath)
        }
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "photo", for: indexPath) as! PhotoCell
        cell.assetId = asset.localIdentifier
        cell.badge(selected.firstIndex(of: asset.localIdentifier).map { $0 + 1 }, animated: false)
        let options = PHImageRequestOptions()
        options.isNetworkAccessAllowed = true
        options.deliveryMode = .opportunistic
        imageManager.requestImage(for: asset, targetSize: thumbSize, contentMode: .aspectFill, options: options) { [weak cell] image, _ in
            guard let cell, cell.assetId == asset.localIdentifier else { return }
            cell.imageView.image = image
        }
        return cell
    }

    func collectionView(_ collectionView: UICollectionView, prefetchItemsAt indexPaths: [IndexPath]) {
        imageManager.startCachingImages(for: indexPaths.compactMap(asset(at:)), targetSize: thumbSize, contentMode: .aspectFill, options: nil)
    }

    func collectionView(_ collectionView: UICollectionView, cancelPrefetchingForItemsAt indexPaths: [IndexPath]) {
        imageManager.stopCachingImages(for: indexPaths.compactMap(asset(at:)), targetSize: thumbSize, contentMode: .aspectFill, options: nil)
    }

    // Selection lives in `selected` (ordered), not the collection view's.
    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        guard let asset = asset(at: indexPath) else {
            onCamera?()
            return false
        }
        if let i = selected.firstIndex(of: asset.localIdentifier) {
            selected.remove(at: i)
        } else if selected.count < limit {
            selected.append(asset.localIdentifier)
        } else {
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
            return false
        }
        UISelectionFeedbackGenerator().selectionChanged()
        for case let cell as PhotoCell in collectionView.visibleCells {
            guard let id = cell.assetId else { continue }
            cell.badge(selected.firstIndex(of: id).map { $0 + 1 }, animated: true)
        }
        refreshMainButton()
        return false
    }
}

private final class PhotoCell: UICollectionViewCell {
    let imageView = UIImageView()
    private let badgeLabel = UILabel()
    var assetId: String?

    override init(frame: CGRect) {
        super.init(frame: frame)
        imageView.contentMode = .scaleAspectFill
        imageView.clipsToBounds = true
        imageView.backgroundColor = Palette.controlFill
        imageView.frame = contentView.bounds
        imageView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        contentView.addSubview(imageView)
        badgeLabel.font = UIFont.systemFont(ofSize: 13, weight: .bold)
        badgeLabel.textColor = .white
        badgeLabel.textAlignment = .center
        badgeLabel.layer.cornerRadius = 12
        badgeLabel.layer.borderWidth = 1.5
        badgeLabel.layer.borderColor = UIColor.white.cgColor
        badgeLabel.clipsToBounds = true
        badgeLabel.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(badgeLabel)
        NSLayoutConstraint.activate([
            badgeLabel.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 6),
            badgeLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -6),
            badgeLabel.widthAnchor.constraint(equalToConstant: 24),
            badgeLabel.heightAnchor.constraint(equalToConstant: 24),
        ])
        isAccessibilityElement = true
        accessibilityLabel = "Photo"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func prepareForReuse() {
        super.prepareForReuse()
        imageView.image = nil
        assetId = nil
    }

    func badge(_ number: Int?, animated: Bool) {
        let apply = {
            self.badgeLabel.text = number.map(String.init)
            self.badgeLabel.backgroundColor = number == nil ? UIColor.black.withAlphaComponent(0.15) : Palette.accent
            self.contentView.transform = number == nil ? .identity : CGAffineTransform(scaleX: 0.92, y: 0.92)
        }
        accessibilityTraits = number == nil ? .button : [.button, .selected]
        accessibilityValue = number.map { "Selected, \($0)" }
        guard animated, !UIAccessibility.isReduceMotionEnabled else { return apply() }
        UIView.animate(withDuration: 0.3, delay: 0, usingSpringWithDamping: 0.75, initialSpringVelocity: 0, options: [.beginFromCurrentState, .allowUserInteraction], animations: apply)
    }
}

private final class CameraCell: UICollectionViewCell {
    override init(frame: CGRect) {
        super.init(frame: frame)
        contentView.backgroundColor = UIColor(white: 0.16, alpha: 1)
        let icon = UIImageView(image: UIImage(systemName: "camera.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 24, weight: .medium)))
        icon.tintColor = .white
        icon.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(icon)
        NSLayoutConstraint.activate([
            icon.centerXAnchor.constraint(equalTo: contentView.centerXAnchor),
            icon.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityLabel = "Take Photo"
        accessibilityTraits = .button
    }

    required init?(coder: NSCoder) { fatalError() }
}
