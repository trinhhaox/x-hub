// Captures the screen of a USB-connected iPhone (the CoreMediaIO "screen capture device",
// the same source QuickTime uses), encodes it to H.264 with VideoToolbox and writes it to
// stdout using scrcpy's framing, so xhub reuses its Android stream pipeline:
//   "h264" codec id, then 12-byte headers:
//   session meta: u32 flags (MSB set) + u32 width + u32 height
//   packet:       u64 pts|flags (bit62 config, bit61 key frame) + u32 size, then Annex-B data
// Usage: ios-screen-capture <udid> [device name]. Exits when stdin closes (parent gone).
import AVFoundation
import CoreMediaIO
import Foundation
import VideoToolbox

setvbuf(stdout, nil, _IONBF, 0)
signal(SIGPIPE, SIG_IGN)

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

let writeLock = NSLock()

func writeAll(_ data: Data) {
    writeLock.lock()
    defer { writeLock.unlock() }
    data.withUnsafeBytes { raw in
        var offset = 0
        while offset < raw.count {
            let n = write(STDOUT_FILENO, raw.baseAddress! + offset, raw.count - offset)
            if n <= 0 { exit(0) } // xhub closed the stream
            offset += n
        }
    }
}

func be32(_ v: UInt32) -> Data { withUnsafeBytes(of: v.bigEndian) { Data($0) } }
func be64(_ v: UInt64) -> Data { withUnsafeBytes(of: v.bigEndian) { Data($0) } }

let flagConfig: UInt64 = 1 << 62
let flagKey: UInt64 = 1 << 61
let startCode = Data([0, 0, 0, 1])

func writePacket(_ payload: Data, ptsFlags: UInt64) {
    writeAll(be64(ptsFlags) + be32(UInt32(payload.count)) + payload)
}

// MARK: - Find the iPhone screen device

func enableScreenCaptureDevices() {
    var prop = CMIOObjectPropertyAddress(
        mSelector: CMIOObjectPropertySelector(kCMIOHardwarePropertyAllowScreenCaptureDevices),
        mScope: CMIOObjectPropertyScope(kCMIOObjectPropertyScopeGlobal),
        mElement: CMIOObjectPropertyElement(kCMIOObjectPropertyElementMain))
    var allow: UInt32 = 1
    CMIOObjectSetPropertyData(CMIOObjectID(kCMIOObjectSystemObject), &prop, 0, nil,
                              UInt32(MemoryLayout<UInt32>.size), &allow)
}

let normalize = { (s: String) in s.replacingOccurrences(of: "-", with: "").lowercased() }

/// Screen devices are muxed (audio+video); this excludes Continuity Camera, which is video-only.
func findScreenDevice(udid: String, name: String?) -> AVCaptureDevice? {
    let types: [AVCaptureDevice.DeviceType]
    if #available(macOS 14.0, *) { types = [.external] } else { types = [.externalUnknown] }
    let devices = AVCaptureDevice.DiscoverySession(
        deviceTypes: types, mediaType: .muxed, position: .unspecified).devices
    return devices.first { normalize($0.uniqueID).contains(normalize(udid)) }
        ?? devices.first { name != nil && $0.localizedName == name }
        ?? (devices.count == 1 ? devices.first : nil)
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: ios-screen-capture <udid> [device name]") }
let udid = args[1]
let deviceName = args.count >= 3 ? args[2] : nil

enableScreenCaptureDevices()
// The device shows up a moment after enabling; notifications need the run loop to spin
var device: AVCaptureDevice?
let deadline = Date().addingTimeInterval(6)
while device == nil && Date() < deadline {
    RunLoop.current.run(until: Date().addingTimeInterval(0.25))
    device = findScreenDevice(udid: udid, name: deviceName)
}
guard let device else {
    fail("NOT_FOUND: Không thấy màn hình iPhone qua cáp. Hãy cắm cáp, mở khoá iPhone và chọn Tin cậy máy tính này.")
}

// MARK: - Encoder

final class Encoder {
    private var session: VTCompressionSession?
    private var width: Int32 = 0
    private var height: Int32 = 0
    private var lastFormat: CMFormatDescription?
    private var started = false

    func encode(_ pixelBuffer: CVPixelBuffer, pts: CMTime) {
        let w = Int32(CVPixelBufferGetWidth(pixelBuffer))
        let h = Int32(CVPixelBufferGetHeight(pixelBuffer))
        if session == nil || w != width || h != height {
            createSession(width: w, height: h)
        }
        guard let session else { return }
        VTCompressionSessionEncodeFrame(session, imageBuffer: pixelBuffer, presentationTimeStamp: pts,
                                        duration: .invalid, frameProperties: nil, infoFlagsOut: nil) {
            [weak self] status, _, sample in
            guard status == noErr, let sample else { return }
            self?.emit(sample)
        }
    }

    private func createSession(width w: Int32, height h: Int32) {
        if let session { VTCompressionSessionInvalidate(session) }
        width = w
        height = h
        lastFormat = nil
        var created: VTCompressionSession?
        let status = VTCompressionSessionCreate(
            allocator: nil, width: w, height: h, codecType: kCMVideoCodecType_H264,
            encoderSpecification: nil, imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &created)
        guard status == noErr, let created else { fail("Không tạo được bộ mã hoá H.264 (\(status))") }
        let props: [CFString: Any] = [
            kVTCompressionPropertyKey_RealTime: true,
            kVTCompressionPropertyKey_ProfileLevel: kVTProfileLevel_H264_High_AutoLevel,
            // No B-frames: the webview decodes frames in arrival order
            kVTCompressionPropertyKey_AllowFrameReordering: false,
            kVTCompressionPropertyKey_AverageBitRate: 12_000_000,
            kVTCompressionPropertyKey_ExpectedFrameRate: 60,
            kVTCompressionPropertyKey_MaxKeyFrameInterval: 120,
        ]
        VTSessionSetProperties(created, propertyDictionary: props as CFDictionary)
        VTCompressionSessionPrepareToEncodeFrames(created)
        session = created
        // Session meta tells xhub the new video size (also after rotation)
        writeAll(be32(0x8000_0000) + be32(UInt32(w)) + be32(UInt32(h)))
    }

    private func emit(_ sample: CMSampleBuffer) {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false)
            as? [[CFString: Any]]
        let isKey = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)

        if isKey, let format = CMSampleBufferGetFormatDescription(sample),
           lastFormat == nil || !CMFormatDescriptionEqual(format, otherFormatDescription: lastFormat) {
            lastFormat = format
            writePacket(parameterSets(format), ptsFlags: flagConfig)
        }

        guard let block = CMSampleBufferGetDataBuffer(sample) else { return }
        var length = 0
        var pointer: UnsafeMutablePointer<CChar>?
        guard CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: nil,
                                          totalLengthOut: &length, dataPointerOut: &pointer) == noErr,
              let pointer else { return }
        // VideoToolbox emits AVCC (4-byte lengths); swap each length for an Annex-B start code
        var data = Data(bytes: pointer, count: length)
        var offset = 0
        while offset + 4 <= data.count {
            let nalLength = data[offset..<offset + 4].reduce(0) { ($0 << 8) | Int($1) }
            data.replaceSubrange(offset..<offset + 4, with: startCode)
            offset += 4 + nalLength
        }
        let pts = UInt64(max(0, CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sample))) * 1_000_000)
        writePacket(data, ptsFlags: (pts & (flagKey - 1)) | (isKey ? flagKey : 0))
    }

    private func parameterSets(_ format: CMFormatDescription) -> Data {
        var count = 0
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            format, parameterSetIndex: 0, parameterSetPointerOut: nil, parameterSetSizeOut: nil,
            parameterSetCountOut: &count, nalUnitHeaderLengthOut: nil)
        var out = Data()
        for i in 0..<count {
            var ptr: UnsafePointer<UInt8>?
            var size = 0
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format, parameterSetIndex: i, parameterSetPointerOut: &ptr, parameterSetSizeOut: &size,
                parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr, let ptr {
                out += startCode + Data(bytes: ptr, count: size)
            }
        }
        return out
    }
}

// MARK: - Capture

final class Capture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    let encoder = Encoder()

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer,
                       from connection: AVCaptureConnection) {
        guard let pixels = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        encoder.encode(pixels, pts: CMSampleBufferGetPresentationTimeStamp(sampleBuffer))
    }
}

let captureSession = AVCaptureSession()
let delegate = Capture()
do {
    let input = try AVCaptureDeviceInput(device: device)
    guard captureSession.canAddInput(input) else { fail("Không mở được màn hình iPhone (thiết bị đang bận)") }
    captureSession.addInput(input)
} catch {
    fail("Không mở được màn hình iPhone: \(error.localizedDescription)")
}
let output = AVCaptureVideoDataOutput()
output.alwaysDiscardsLateVideoFrames = true
output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]
output.setSampleBufferDelegate(delegate, queue: DispatchQueue(label: "xhub.ios-capture"))
guard captureSession.canAddOutput(output) else { fail("Không nhận được hình từ iPhone") }
captureSession.addOutput(output)

writeAll("h264".data(using: .ascii)!)
captureSession.startRunning()

// Stop when xhub closes our stdin (stop button, device switch or app quit)
DispatchQueue.global().async {
    while FileHandle.standardInput.availableData.count > 0 {}
    captureSession.stopRunning()
    exit(0)
}
RunLoop.main.run()
