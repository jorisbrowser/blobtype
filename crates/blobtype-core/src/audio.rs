//! Pure audio helpers: resampling, PCM16 conversion, WAV encoding, level.

pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Linear-interpolation resampler; plenty for speech recognition.
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() || from_rate == 0 || to_rate == 0 {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

pub fn to_pcm16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect()
}

pub fn pcm16_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Root-mean-square level of a block, 0.0..=1.0 for normalised samples.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Map an RMS value to a perceptually reasonable 0..1 meter value.
pub fn meter_level(rms: f32) -> f32 {
    // Speech RMS is typically 0.01..0.2; a sqrt curve keeps quiet voices visible.
    (rms * 6.0).sqrt().clamp(0.0, 1.0)
}

/// Encode mono PCM16 as a canonical 44-byte-header WAV file.
pub fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Accumulates 16 kHz mono PCM16 with a hard length cap.
pub struct Recording {
    samples: Vec<i16>,
    max_samples: usize,
}

impl Recording {
    pub fn new(max_secs: u32) -> Self {
        Self {
            samples: Vec::new(),
            max_samples: max_secs as usize * STT_SAMPLE_RATE as usize,
        }
    }

    /// Append a block; returns `true` once the cap is reached (extra audio is dropped).
    pub fn push(&mut self, block: &[i16]) -> bool {
        let room = self.max_samples.saturating_sub(self.samples.len());
        self.samples
            .extend_from_slice(&block[..block.len().min(room)]);
        self.is_full()
    }

    pub fn is_full(&self) -> bool {
        self.samples.len() >= self.max_samples
    }

    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / STT_SAMPLE_RATE as f32
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    pub fn to_wav(&self) -> Vec<u8> {
        encode_wav(&self.samples, STT_SAMPLE_RATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_same_rate_is_identity() {
        let x = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&x, 16000, 16000), x);
    }

    #[test]
    fn resample_length_scales() {
        let x = vec![0.0; 48000];
        assert_eq!(resample_linear(&x, 48000, 16000).len(), 16000);
        assert_eq!(resample_linear(&x, 16000, 24000).len(), 72000);
    }

    #[test]
    fn resample_interpolates_ramp() {
        let x: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let y = resample_linear(&x, 100, 50);
        assert_eq!(y.len(), 50);
        assert!((y[10] - 0.20).abs() < 1e-4);
    }

    #[test]
    fn pcm16_clamps() {
        assert_eq!(to_pcm16(&[2.0, -2.0, 0.0]), vec![i16::MAX, -i16::MAX, 0]);
    }

    #[test]
    fn rms_and_meter() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(meter_level(0.0), 0.0);
        assert_eq!(meter_level(10.0), 1.0);
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav(&[1, -1, 2], 16000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes(wav[44..46].try_into().unwrap()), 1);
    }

    #[test]
    fn recording_caps_length() {
        let mut r = Recording::new(1);
        assert!(!r.push(&vec![0; 10_000]));
        assert!(r.push(&vec![0; 10_000]));
        assert_eq!(r.samples().len(), 16_000);
        assert!((r.duration_secs() - 1.0).abs() < 1e-6);
    }
}
