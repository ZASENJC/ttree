// CI-only comparison against Apple's minimal Vision request; synthetic text only.
import Foundation
import ImageIO
import Vision
import CoreML
import CoreImage

let file = URL(fileURLWithPath: CommandLine.arguments[1])
let data = try Data(contentsOf: file)
let source = CGImageSourceCreateWithData(data as CFData, nil)!
let image = CGImageSourceCreateImageAtIndex(source, 0, nil)!
print("Vision default revision: \(VNRecognizeTextRequest.defaultRevision)")
print("Vision supported revisions: \(VNRecognizeTextRequest.supportedRevisions)")

func assignCPU(_ request: VNRecognizeTextRequest) throws {
    if #available(macOS 14.0, *) {
        let stages = try request.supportedComputeStageDevices
        for (stage, devices) in stages {
            print("compute stage \(stage): \(devices)")
            if let cpu = devices.first(where: {
                if case .cpu(_) = $0 { return true }
                return false
            }) {
                request.setComputeDevice(cpu, for: stage)
            }
        }
    }
}

let profiles: [(String, (VNRecognizeTextRequest) throws -> Void)] = [
    ("apple-default", { _ in }),
    ("default-explicit-cpu", { try assignCPU($0) }),
    ("rev3-english-cpu", { request in
        request.revision = 3
        request.recognitionLanguages = ["en-US"]
        try assignCPU(request)
    }),
    ("rev3-chinese-english-cpu", { request in
        request.revision = 3
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        try assignCPU(request)
    }),
    ("rev2-chinese-english-cpu", { request in
        request.revision = 2
        request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"]
        try assignCPU(request)
    }),
    ("fast-english-cpu", { request in
        request.recognitionLevel = .fast
        request.recognitionLanguages = ["en-US"]
        try assignCPU(request)
    }),
]
for (name, configure) in profiles {
    for software in [false, true] {
    do {
        let request = VNRecognizeTextRequest()
        try configure(request)
        let options: [VNImageOption: Any] = software
            ? [.ciContext: CIContext(options: [.useSoftwareRenderer: true])]
            : [:]
        let handler = VNImageRequestHandler(cgImage: image, options: options)
        try handler.perform([request])
        let text = request.results?.compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n") ?? ""
        print("\(name) software=\(software): OK \(text)")
    } catch {
        let error = error as NSError
        print("\(name) software=\(software): ERROR domain=\(error.domain) code=\(error.code) details=\(error.userInfo)")
    }
    }
}
