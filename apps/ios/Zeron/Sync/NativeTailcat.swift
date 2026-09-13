// Required gomobile bridge. TestFlight and the local wrapper both generate
// target/tailcat/KratosTailcat.xcframework; there is no network fallback.

import Foundation
#if canImport(KratosTailcat)
import KratosTailcat
#else
#error("KratosTailcat.xcframework is required; run Native/build-xcframework.sh")
#endif

enum NativeTailcatError: LocalizedError {
    case invalidProxyURL
    case startFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidProxyURL: return "The native Tailcat adapter returned an invalid proxy URL."
        case .startFailed(let message): return "Tailcat could not start: \(message)"
        }
    }
}

/// Owns the embedded Tailcat client and its loopback HTTP/WebSocket proxy.
/// The gomobile contract is `StartClient(address,stateDir,derpMap)` and a
/// returned handle with `URL()` and `Close()`.
protocol TailcatClient: Sendable {
    var url: URL { get }
    func close()
}

final class NativeTailcatClient: TailcatClient, @unchecked Sendable {
    private let lock = NSLock()
    private var handle: TailcatnativeClient?
    let url: URL

    init(address: String, stateDirectory: URL, derpMap: String?) throws {
        try FileManager.default.createDirectory(at: stateDirectory,
                                                withIntermediateDirectories: true)
        do {
            let started = try TailcatnativeStartClient(address, stateDirectory.path, derpMap ?? "")
            guard let url = URL(string: started.url()), url.isLoopbackHTTP else {
                started.close()
                throw NativeTailcatError.invalidProxyURL
            }
            handle = started
            self.url = url
        } catch let error as NativeTailcatError {
            throw error
        } catch {
            throw NativeTailcatError.startFailed(error.localizedDescription)
        }
    }

    func close() {
        let old = lock.withLock {
            let old = handle
            handle = nil
            return old
        }
        old?.close()
    }

    deinit { close() }
}

private extension URL {
    var isLoopbackHTTP: Bool {
        (scheme == "http" || scheme == "https")
            && (host == "127.0.0.1" || host == "localhost" || host == "::1")
    }
}
