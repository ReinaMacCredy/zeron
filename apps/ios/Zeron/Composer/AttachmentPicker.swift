import PhotosUI
import UIKit
import UniformTypeIdentifiers

/// The attach card's actions (Camera / Photos / Files / Paste, plus the
/// inline grid's picks) turning into `StagedImage`s: HEIC→JPEG, long side
/// ≤ 2560px, ≤ 24 MB.
final class AttachmentPicker: NSObject, PHPickerViewControllerDelegate, UIImagePickerControllerDelegate, UINavigationControllerDelegate, UIDocumentPickerDelegate {
    private weak var host: UIViewController?
    let limit: Int
    let attached: [String]
    let gridLimit: Int
    var onDrop: (([String]) -> Void)?
    private let completion: ([StagedImage]) -> Void
    private var retainSelf: AttachmentPicker?

    init(host: UIViewController, limit: Int, attached: [String] = [], completion: @escaping ([StagedImage]) -> Void) {
        self.host = host
        self.limit = max(1, limit)
        self.attached = attached
        gridLimit = max(1, limit + attached.count)
        self.completion = completion
    }

    var canTakePhoto: Bool { UIImagePickerController.isSourceTypeAvailable(.camera) }
    var canPaste: Bool { UIPasteboard.general.hasImages }

    func pasteImages() {
        completion(UIPasteboard.general.images?.compactMap { AttachmentPicker.stage(image: $0) } ?? [])
    }

    func presentPhotos() {
        var config = PHPickerConfiguration(photoLibrary: .shared())
        config.filter = .images
        config.selectionLimit = limit
        config.preferredAssetRepresentationMode = .current
        let picker = PHPickerViewController(configuration: config)
        picker.delegate = self
        retainSelf = self
        host?.present(picker, animated: true)
    }

    /// The inline recent-photos grid's selection: photos it no longer holds
    /// leave the composer, new ones are attached in selection order.
    func attach(_ ids: [String]) {
        let dropped = attached.filter { !ids.contains($0) }
        if !dropped.isEmpty { onDrop?(dropped) }
        let new = ids.filter { !attached.contains($0) }
        var found: [String: PHAsset] = [:]
        PHAsset.fetchAssets(withLocalIdentifiers: new, options: nil).enumerateObjects { asset, _, _ in found[asset.localIdentifier] = asset }
        let assets = new.compactMap { found[$0] }
        let options = PHImageRequestOptions()
        options.isNetworkAccessAllowed = true
        options.deliveryMode = .highQualityFormat
        options.version = .current
        var staged = [StagedImage?](repeating: nil, count: assets.count)
        let group = DispatchGroup()
        for (i, asset) in assets.enumerated() {
            group.enter()
            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) { data, _, _, _ in
                DispatchQueue.global(qos: .userInitiated).async {
                    let s = data.flatMap(UIImage.init(data:)).flatMap { AttachmentPicker.stage(image: $0, assetId: asset.localIdentifier) }
                    DispatchQueue.main.async {
                        staged[i] = s
                        group.leave()
                    }
                }
            }
        }
        group.notify(queue: .main) { [completion] in completion(staged.compactMap { $0 }) }
    }

    func presentCamera() {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = self
        retainSelf = self
        host?.present(picker, animated: true)
    }

    func presentFiles() {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.image], asCopy: true)
        picker.allowsMultipleSelection = true
        picker.delegate = self
        retainSelf = self
        host?.present(picker, animated: true)
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        let group = DispatchGroup()
        var staged: [(Int, StagedImage)] = []
        let lock = NSLock()
        for (i, result) in results.enumerated() {
            group.enter()
            result.itemProvider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) { data, _ in
                defer { group.leave() }
                guard let data, let image = UIImage(data: data), let s = AttachmentPicker.stage(image: image, assetId: result.assetIdentifier) else { return }
                lock.lock()
                staged.append((i, s))
                lock.unlock()
            }
        }
        group.notify(queue: .main) { [self] in
            completion(staged.sorted { $0.0 < $1.0 }.map(\.1))
            retainSelf = nil
        }
    }

    func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
        picker.dismiss(animated: true)
        if let image = info[.originalImage] as? UIImage, let s = AttachmentPicker.stage(image: image) { completion([s]) }
        retainSelf = nil
    }

    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
        picker.dismiss(animated: true)
        retainSelf = nil
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        let staged = urls.compactMap { url -> StagedImage? in
            guard let data = try? Data(contentsOf: url), let image = UIImage(data: data) else { return nil }
            return AttachmentPicker.stage(image: image, name: url.deletingPathExtension().lastPathComponent)
        }
        completion(staged)
        retainSelf = nil
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { retainSelf = nil }

    static func stage(image: UIImage, name: String? = nil, assetId: String? = nil) -> StagedImage? {
        let maxSide: CGFloat = 2560
        let size = image.size
        let scale = min(1, maxSide / max(size.width, size.height))
        let target = CGSize(width: (size.width * scale).rounded(), height: (size.height * scale).rounded())
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let resized = scale < 1 ? UIGraphicsImageRenderer(size: target, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: target)) } : image
        guard let data = resized.jpegData(compressionQuality: 0.86), data.count <= 24 * 1024 * 1024 else { return nil }
        let fill = min(1, 312 / max(1, min(target.width, target.height)))
        let thumb = resized.preparingThumbnail(of: CGSize(width: (target.width * fill).rounded(), height: (target.height * fill).rounded())) ?? resized
        let id = UUID().uuidString.lowercased()
        return StagedImage(id: id, name: "\(name ?? "image-\(id.prefix(8))").jpg", data: data, thumbnail: thumb, assetId: assetId)
    }
}
