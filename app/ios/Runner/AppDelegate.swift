import AVFoundation
import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    // The phone's video decoder for the core's sampling plan (lib/src/video.dart).
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "BragiVideoDecoder") {
      let channel = FlutterMethodChannel(name: "org.weirdbrains.bragi/video", binaryMessenger: registrar.messenger())
      channel.setMethodCallHandler(VideoDecoder.handle)
    }
  }
}

/// The phone's video decoder for bragi-core's sampling plan (LF-43, V3): `probe` reads a clip's
/// length, size (upright), frame rate and whether it has sound; `frames` decodes the frames at the
/// times the core planned, scaled to fit its size, as JPEG. AVFoundation only: no ffmpeg on phones.
enum VideoDecoder {
  static let work = DispatchQueue(label: "org.weirdbrains.bragi.video", qos: .userInitiated)

  static func handle(_ call: FlutterMethodCall, _ result: @escaping FlutterResult) {
    guard let args = call.arguments as? [String: Any], let path = args["path"] as? String else {
      result(FlutterError(code: "bad_request", message: "no clip given", details: nil))
      return
    }
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    switch call.method {
    case "probe":
      run(result) { try probe(asset) }
    case "frames":
      let times = (args["times_ms"] as? [NSNumber] ?? []).map { $0.int64Value }
      let width = (args["width"] as? NSNumber)?.intValue ?? 0
      let height = (args["height"] as? NSNumber)?.intValue ?? 0
      let quality = min(max((args["quality"] as? NSNumber)?.doubleValue ?? 85, 1), 100) / 100
      run(result) { try frames(asset, times, width, height, quality) }
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  static func run(_ result: @escaping FlutterResult, _ job: @escaping () throws -> Any) {
    work.async {
      do {
        let out = try job()
        DispatchQueue.main.async { result(out) }
      } catch {
        DispatchQueue.main.async {
          result(FlutterError(code: "video", message: error.localizedDescription, details: nil))
        }
      }
    }
  }

  struct Unreadable: LocalizedError {
    let errorDescription: String?
  }

  static func probe(_ asset: AVURLAsset) throws -> [String: Any] {
    guard let track = asset.tracks(withMediaType: .video).first else {
      throw Unreadable(errorDescription: "the file has no video track")
    }
    let size = track.naturalSize.applying(track.preferredTransform)
    let fps = track.nominalFrameRate
    return [
      "duration_ms": Int64((CMTimeGetSeconds(asset.duration) * 1000).rounded()),
      "width": Int(abs(size.width)),
      "height": Int(abs(size.height)),
      "fps": fps > 0 ? Double(fps) : NSNull(),
      "has_audio": !asset.tracks(withMediaType: .audio).isEmpty,
    ]
  }

  static func frames(_ asset: AVURLAsset, _ times: [Int64], _ width: Int, _ height: Int, _ quality: Double) throws -> [[String: Any]] {
    let g = AVAssetImageGenerator(asset: asset)
    g.appliesPreferredTrackTransform = true
    g.requestedTimeToleranceBefore = .zero
    g.requestedTimeToleranceAfter = .zero
    if width > 0 && height > 0 {
      g.maximumSize = CGSize(width: width, height: height)
    }
    return try times.map { t in
      let image = try g.copyCGImage(at: CMTime(value: t, timescale: 1000), actualTime: nil)
      guard let jpeg = UIImage(cgImage: image).jpegData(compressionQuality: quality) else {
        throw Unreadable(errorDescription: "no frame at \(t) ms")
      }
      return ["at_ms": t, "bytes": FlutterStandardTypedData(bytes: jpeg)]
    }
  }
}
