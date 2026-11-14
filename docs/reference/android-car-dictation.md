# Android Auto dictation: on-device Parakeet

The car app can dictate with NVIDIA **Parakeet Unified 0.6B (EN, INT8)** on the phone, instead of Android's speech recognizer. Settings → Android Auto → *Dictation* picks the engine (**Local Parakeet** / **Android**). Choosing Local downloads the model once (about 663 MB); until it is on the phone, and whenever the local engine can't run or errors, the Android recognizer answers.

## What it does

- Streams the car microphone (16 kHz mono PCM, from `CarAudioRecord`, or the phone's microphone when the car has none) through the model at 0.56 s chunks with 0.56 s of lookahead and 2 s of left context. The partial text so far is delivered to the listening screen as it changes.
- When `Endpointer` ends the utterance (2.2 s of quiet after speech, 8 s without any), the whole utterance is decoded again offline with the same model. That text is the final one: it has punctuation and casing and keeps fillers ("um", "uh", "hmm"). If it comes back empty the streaming text stands.
- Laughter is not tagged: it comes out as syllables ("he-he-he"). Utterances under 0.3 s, and anything past 60 s, are not decoded.

Measured on a Nothing Phone 2 (Snapdragon 8+ Gen 1, 4 threads, TTS clips): streaming runs at 0.2-0.4 × real time (a chunk takes 0.2-0.3 s), the offline pass at 0.04-0.07 ×; memory is 0.8-0.9 GB while loaded (the model is released after 5 idle minutes); loading takes 1-2 s (a car connecting warms it).

## Pieces

| | |
| --- | --- |
| `apps/android/local-asr` | Rust crate (`zeron-local-asr`, its own workspace): `parakeet-rs` 0.3.8 behind JNI (`LocalAsrNative`). ONNX Runtime is **loaded by path** (`ort` `load-dynamic`) from the `libonnxruntime.so` that the `onnxruntime-android` AAR puts in the APK, so the root workspace's linked ONNX Runtime and this one never mix. Built by `scripts/android/build-local-asr.sh` (the `buildLocalAsr` Gradle task), beside `libzeron_mobile.so`. |
| `car/LocalModel.kt` | Pinned revision, size and SHA-256 per file; staged `.part` files (resumed), hashed, then moved into place; a `.verified` marker names the revision. A mismatch deletes the part and fails. |
| `car/LocalAsr.kt` | Loads the library and model lazily, keeps them for 5 idle minutes, hands out one `AsrSession` per utterance. |
| `car/LocalListening.kt` | The pump: audio into the session in 160 ms steps, `Endpointer`, partials out, offline final. A stall guard in `CarVoice` stops a microphone that delivers nothing for 10 s. |
| `car/CarVoice.kt` | `listen()` chooses the engine (`engineFor`), and falls back to `SpeechRecognizer` when the local one can't start or fails before any words. |

**ONNX Runtime teardown:** the benchmark binary aborted at *process exit* ("mutex destroyed") when it unloaded ONNX Runtime. In the app the library is never unloaded (`ort` keeps it for the life of the process) and releasing a model only drops its sessions; load/release/load cycles were run on an x86_64 emulator without a crash.

## Licences

- **Weights:** NVIDIA's `nvidia/parakeet-unified-en-0.6b` is under the **NVIDIA Open Model License** (`license: other` on its card; https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/). Summary of that licence as read on 2026-11-14: models are commercially usable (perpetual, worldwide, royalty-free); converted or quantized versions are "Derivative Models" that may be created and distributed; whoever distributes the weights must give recipients the licence and a Notice file with "Licensed by NVIDIA Corporation under the NVIDIA Open Model License."; rights end if its guardrails are bypassed (this model has none) or on a patent/copyright suit over the model; the model is as-is and use must follow NVIDIA's Trustworthy AI terms.
- **Conversion:** the ONNX files are `bobNight/parakeet-unified-en-0.6b-onnx`, whose card says **CC-BY-4.0** and credits the NVIDIA model. That label does not replace NVIDIA's licence for the weights: both attributions are kept (below). The conversion's INT8 files are what is pinned; their provenance beyond the card is not published.
- **Zeron does not bundle or redistribute the weights.** The phone downloads them from Hugging Face on the user's request, as desktop dictation does; the attribution is in `THIRD_PARTY_NOTICES.md` and in this document. Not legal advice: a release for other users should have the NOML notice text reviewed.
- **Runtime:** parakeet-rs 0.3.8 (MIT OR Apache-2.0), ort 2.0.0-rc.13 (MIT OR Apache-2.0), jni 0.21 (MIT OR Apache-2.0), ONNX Runtime 1.32.0 from `com.microsoft.onnxruntime:onnxruntime-android` (MIT). The AAR also contains ONNX Runtime's Java API and its telemetry class, unused; a harmless `HttpClient.finalize` exception (`Receiver not registered`) is logged when the garbage collector finalizes it.

## Testing

- Unit tests (JVM): `LocalModelTest` (a local HTTP server: download, progress, corrupt file refused, resume, 404, revision/truncation/removal), `LocalListeningTest` (partials, endpointing, cancel, failure keeps words, limit, stall clock), `EngineChoiceTest`.
- Rust: `cargo test --manifest-path apps/android/local-asr/Cargo.toml --lib`; with a model, `ZERON_ASR_MODEL=<dir> ZERON_ASR_WAV=<16 kHz mono wav> ORT_DYLIB_PATH=<libonnxruntime.so> cargo test --release --lib -- --ignored`.
- Device (`LocalAsrDeviceTest`, `am instrument`): the real library and ONNX Runtime from the APK with the real model on an x86_64 emulator: partials, offline final, fillers, load/release cycles, short audio, the small pinned files from Hugging Face and (with `-e full true`) the whole 663 MB download, verified and decoded. See the file for how to push the model.
- DHU: with `Local` chosen the car microphone (`CarAudioRecord`) fed the local engine and the text was sent. The DHU's file-backed microphone delivers audio slower than real time, so listening takes much longer than the speech.
