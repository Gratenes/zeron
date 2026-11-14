//! On-device dictation: Parakeet Unified (EN, INT8), streaming and offline.
//!
//! A [`Model`] is loaded once (about 2 s, 0.7-0.9 GB) and shared. Each
//! utterance is a [`Session`]: audio goes in as it is spoken, the partial text
//! comes back at 0.56 s steps (with ~2 s of left context: the encoder
//! re-reads it every step), and [`Session::finish`] re-decodes the whole
//! utterance offline for the final text. The offline pass is both more
//! accurate and the same model, so there is one download.

mod jni_bridge;

use parakeet_rs::{
    ExecutionConfig, ExecutionProvider, ParakeetUnified, ParakeetUnifiedHandle,
    UnifiedStreamingConfig,
};
use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<parakeet_rs::Error> for Error {
    fn from(e: parakeet_rs::Error) -> Self {
        Error(e.to_string())
    }
}

/// Audio is 16 kHz mono; a longer utterance is cut (the offline pass can't take minutes).
pub const SAMPLE_RATE: usize = 16_000;
pub const MAX_SECONDS: usize = 60;
/// Less than this (0.3 s) is a click, not speech: the encoder can't take under a frame or two.
pub const MIN_SAMPLES: usize = SAMPLE_RATE * 3 / 10;

/// The streaming profile: 0.56 s chunks, 0.56 s of lookahead, 2 s of left context.
/// (5.6 s of left context cost twice the CPU and began some utterances with a stray word.)
pub fn streaming_config() -> UnifiedStreamingConfig {
    UnifiedStreamingConfig { left_context_secs: 2.0, chunk_secs: 0.56, right_context_secs: 0.56 }
}

pub struct Model {
    handle: ParakeetUnifiedHandle,
}

impl Model {
    /// Load from a directory with `encoder.int8.onnx` (+ `.data`), `decoder_joint.int8.onnx`
    /// and `tokenizer.model`.
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        if !dir.join("tokenizer.model").exists() {
            return Err(Error(format!("no model in {}", dir.display())));
        }
        let config = ExecutionConfig::default()
            .with_intra_threads(threads.max(1))
            .with_inter_threads(1)
            .with_execution_provider(ExecutionProvider::Cpu);
        Ok(Self { handle: ParakeetUnifiedHandle::from_pretrained(dir, Some(config))? })
    }

    pub fn session(&self) -> Result<Session> {
        Ok(Session {
            stream: ParakeetUnified::from_shared_with_streaming_config(&self.handle, streaming_config())?,
            offline: ParakeetUnified::from_shared(&self.handle)?,
            audio: Vec::new(),
            partial: String::new(),
        })
    }
}

pub struct Session {
    stream: ParakeetUnified,
    offline: ParakeetUnified,
    audio: Vec<f32>,
    partial: String,
}

/// 16-bit little-endian PCM to samples in [-1, 1).
pub fn pcm16_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect()
}

impl Session {
    /// Feed audio; the whole partial text so far when it changed.
    pub fn feed(&mut self, pcm16: &[u8]) -> Result<Option<String>> {
        let samples = pcm16_to_f32(pcm16);
        if samples.is_empty() {
            return Ok(None);
        }
        let room = (MAX_SECONDS * SAMPLE_RATE).saturating_sub(self.audio.len());
        let samples = &samples[..samples.len().min(room)];
        if samples.is_empty() {
            return Ok(None);
        }
        self.audio.extend_from_slice(samples);
        self.stream.transcribe_chunk(samples)?;
        let text = self.stream.get_transcript().trim().to_string();
        if text != self.partial {
            self.partial = text.clone();
            return Ok(Some(text));
        }
        Ok(None)
    }

    /// The final text: the whole utterance decoded offline. If that comes back empty
    /// while streaming heard words, the streaming text stands.
    pub fn finish(&mut self) -> Result<String> {
        if self.audio.len() < MIN_SAMPLES {
            return Ok(String::new());
        }
        let audio = std::mem::take(&mut self.audio);
        let text = self.offline.transcribe_audio(audio, SAMPLE_RATE as u32, 1)?;
        let text = text.trim();
        if text.is_empty() {
            let _ = self.stream.flush();
            return Ok(self.stream.get_transcript().trim().to_string());
        }
        Ok(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_is_little_endian_and_scaled() {
        let out = pcm16_to_f32(&[0x00, 0x40, 0x00, 0xC0, 0xFF]);
        assert_eq!(out, vec![0.5, -0.5]); // the odd trailing byte is dropped
    }

    #[test]
    fn under_a_third_of_a_second_is_too_short_to_decode() {
        assert_eq!(MIN_SAMPLES, 4_800);
    }

    #[test]
    fn streaming_profile_is_the_benchmarked_one() {
        let c = streaming_config();
        assert_eq!((c.left_context_secs, c.chunk_secs, c.right_context_secs), (2.0, 0.56, 0.56));
        assert!(c.validate().is_ok());
    }

    #[test]
    fn a_missing_model_is_an_error_not_a_crash() {
        let err = Model::load(Path::new("/nonexistent/zeron-model"), 4).err().expect("must fail");
        assert!(err.0.contains("no model"));
    }

    /// Real model: `ZERON_ASR_MODEL=<dir> ZERON_ASR_WAV=<16 kHz mono wav> ORT_DYLIB_PATH=<libonnxruntime.so>
    /// cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn the_real_model_hears_words_streaming_and_offline() {
        let dir = std::env::var("ZERON_ASR_MODEL").expect("ZERON_ASR_MODEL");
        let wav = std::env::var("ZERON_ASR_WAV").expect("ZERON_ASR_WAV");
        let model = Model::load(Path::new(&dir), 4).unwrap();
        let mut reader = hound::WavReader::open(wav).unwrap();
        let pcm: Vec<u8> = reader.samples::<i16>().flat_map(|s| s.unwrap().to_le_bytes()).collect();
        let mut session = model.session().unwrap();
        let mut partials = 0;
        for chunk in pcm.chunks(5120) {
            if session.feed(chunk).unwrap().is_some() {
                partials += 1;
            }
        }
        let text = session.finish().unwrap().to_lowercase();
        assert!(partials > 0, "no live partials");
        assert!(text.contains("main branch"), "final text: {text}");
    }
}
