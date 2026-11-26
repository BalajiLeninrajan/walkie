//! The four beeps, and a small WAV reader for them and for test clips.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beep {
    Start,
    Stop,
    Lock,
    Error,
}

impl Beep {
    fn file_name(self) -> &'static str {
        match self {
            Self::Start => "start.wav",
            Self::Stop => "stop.wav",
            Self::Lock => "lock.wav",
            Self::Error => "error.wav",
        }
    }

    fn builtin(self) -> &'static [u8] {
        match self {
            Self::Start => include_bytes!("../sounds/start.wav"),
            Self::Stop => include_bytes!("../sounds/stop.wav"),
            Self::Lock => include_bytes!("../sounds/lock.wav"),
            Self::Error => include_bytes!("../sounds/error.wav"),
        }
    }

    /// `SOUND_DIR/<name>.wav` if it exists and parses, else the built-in one,
    /// scaled by `volume`.
    pub fn load(self, dir: &Path, volume: f32) -> Pcm {
        let mut pcm = std::fs::read(dir.join(self.file_name()))
            .ok()
            .and_then(|b| parse_wav(&b))
            .or_else(|| parse_wav(self.builtin()))
            .expect("built-in beeps are valid 16-bit WAV");
        for s in &mut pcm.samples {
            *s = (f32::from(*s) * volume) as i16;
        }
        pcm
    }
}

/// Mono 16-bit PCM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// Parses a RIFF/WAVE file with 16-bit integer PCM. Multi-channel audio is
/// averaged down to mono. Returns `None` for anything else.
pub fn parse_wav(b: &[u8]) -> Option<Pcm> {
    if b.get(0..4)? != b"RIFF" || b.get(8..12)? != b"WAVE" {
        return None;
    }
    let u16_at = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let (mut pos, mut fmt) = (12, None);
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32_at(pos + 4)? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            let (format, channels, rate, bits) =
                (u16_at(body)?, u16_at(body + 2)?, u32_at(body + 4)?, u16_at(body + 14)?);
            // 1 = PCM, 0xFFFE = WAVE_FORMAT_EXTENSIBLE (assume PCM subformat).
            if !(format == 1 || format == 0xFFFE) || bits != 16 || channels == 0 {
                return None;
            }
            fmt = Some((channels, rate));
        } else if id == b"data" {
            let (channels_u16, rate) = fmt?;
            let channels = usize::from(channels_u16);
            let data = b.get(body..(body + len).min(b.len()))?;
            let samples = data
                .chunks_exact(2 * channels)
                .map(|frame| {
                    let sum: i32 = frame.chunks_exact(2).map(|s| i32::from(i16::from_le_bytes([s[0], s[1]]))).sum();
                    (sum / i32::from(channels_u16)) as i16
                })
                .collect();
            return Some(Pcm { rate, samples });
        }
        pos = body + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        b.extend_from_slice(&(channels * 2).to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    #[test]
    fn mono() {
        let p = parse_wav(&wav(1, 16000, &[1, -2, 3])).unwrap();
        assert_eq!(p, Pcm { rate: 16000, samples: vec![1, -2, 3] });
    }

    #[test]
    fn stereo_is_averaged() {
        let p = parse_wav(&wav(2, 48000, &[100, 300, -10, -30])).unwrap();
        assert_eq!(p.samples, [200, -20]);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_wav(b"not a wav"), None);
        let mut w = wav(1, 16000, &[1]);
        w[34] = 8; // 8-bit
        assert_eq!(parse_wav(&w), None);
    }

    #[test]
    fn builtin_beeps_parse() {
        for b in [Beep::Start, Beep::Stop, Beep::Lock, Beep::Error] {
            let p = b.load(Path::new("/nonexistent"), 0.5);
            assert_eq!(p.rate, 48000);
            assert_ne!(p.samples, [] as [i16; 0]);
        }
    }
}
