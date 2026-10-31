//! ReplayGain 2.0: how much to turn each track, and the album, up or down so
//! that everything plays at the same loudness. Players apply it from tags;
//! the audio itself is not changed.
//!
//! Loudness is measured as in ITU-R BS.1770 (EBU R128): both channels go
//! through the K-weighting filter (a shelf boosting the treble, as the ear
//! hears it, then a high-pass), their mean square is taken over blocks of
//! 400 ms overlapping by 75 %, and blocks quieter than −70 LUFS, then those
//! more than 10 LU below the average of the rest, are left out. The gain
//! brings the track to −18 LUFS, ReplayGain 2.0's reference. The album is
//! measured as one: the blocks of all its tracks together.

use std::f64::consts::PI;

/// Loudness every track is brought to, in LUFS.
pub const REFERENCE_LUFS: f64 = -18.0;
const RATE: f64 = 44_100.0;
/// Blocks of 400 ms, made of four steps of 100 ms.
const STEP: usize = 4_410;
const STEPS_PER_BLOCK: usize = 4;
const ABSOLUTE_GATE: f64 = -70.0;
const RELATIVE_GATE: f64 = -10.0;

/// The loudness measure of a track (or an album).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Analysis {
    /// Mean square of each 400 ms block, both channels added.
    blocks: Vec<f64>,
    /// Highest sample, from 0 to 1.
    pub peak: f32,
}

impl Analysis {
    /// Measures CD audio (16-bit stereo, little endian).
    pub fn new(audio: &[u8]) -> Self {
        let filter = KWeighting::new(RATE);
        let mut channels = [filter.clone(), filter];
        let mut steps = Vec::new();
        let mut sum = 0.0;
        let mut peak = 0u16;
        for (i, frame) in audio.as_chunks::<4>().0.iter().enumerate() {
            for (c, filter) in channels.iter_mut().enumerate() {
                let sample = i16::from_le_bytes([frame[2 * c], frame[2 * c + 1]]);
                peak = peak.max(sample.unsigned_abs());
                let y = filter.process(f64::from(sample) / 32_768.0);
                sum += y * y;
            }
            if (i + 1) % STEP == 0 {
                steps.push(sum);
                sum = 0.0;
            }
        }
        let block_len = (STEP * STEPS_PER_BLOCK) as f64;
        let blocks = steps.windows(STEPS_PER_BLOCK).map(|w| w.iter().sum::<f64>() / block_len).collect();
        Analysis { blocks, peak: f32::from(peak) / 32_768.0 }
    }

    /// Integrated loudness in LUFS; `None` for silence or less than 400 ms.
    pub fn loudness(&self) -> Option<f64> {
        let loud: Vec<f64> = self.blocks.iter().copied().filter(|&e| lufs(e) > ABSOLUTE_GATE).collect();
        if loud.is_empty() {
            return None;
        }
        let threshold = lufs(mean(&loud)) + RELATIVE_GATE;
        let kept: Vec<f64> = loud.into_iter().filter(|&e| lufs(e) > threshold).collect();
        Some(lufs(mean(&kept)))
    }

    /// Gain to apply, in dB.
    pub fn gain(&self) -> Option<f64> {
        self.loudness().map(|l| REFERENCE_LUFS - l)
    }
}

/// The album as one piece: all the blocks of its tracks, its highest peak.
pub fn album<'a>(tracks: impl IntoIterator<Item = &'a Analysis>) -> Analysis {
    let mut album = Analysis::default();
    for track in tracks {
        album.blocks.extend(&track.blocks);
        album.peak = album.peak.max(track.peak);
    }
    album
}

/// The tags of a track, named as Picard and foobar2000 write them; the
/// album's only for a whole disc.
pub fn tags(track: &Analysis, album: Option<&Analysis>) -> Vec<(&'static str, String)> {
    let mut tags = Vec::new();
    for (analysis, gain_key, peak_key) in [
        (Some(track), "REPLAYGAIN_TRACK_GAIN", "REPLAYGAIN_TRACK_PEAK"),
        (album, "REPLAYGAIN_ALBUM_GAIN", "REPLAYGAIN_ALBUM_PEAK"),
    ] {
        if let Some((analysis, gain)) = analysis.and_then(|a| Some((a, a.gain()?))) {
            tags.push((gain_key, format_gain(gain)));
            tags.push((peak_key, format!("{:.6}", analysis.peak)));
        }
    }
    tags
}

/// `-8.52 dB`
pub fn format_gain(gain: f64) -> String {
    format!("{gain:.2} dB")
}

fn lufs(mean_square: f64) -> f64 {
    -0.691 + 10.0 * mean_square.log10()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// The K-weighting filter of BS.1770: two biquads, designed for any sample
/// rate as libebur128 does (the standard gives coefficients for 48 kHz only).
#[derive(Debug, Clone)]
struct KWeighting {
    stages: [Biquad; 2],
}

impl KWeighting {
    fn new(rate: f64) -> Self {
        // High shelf: +4 dB above about 1.7 kHz.
        let (f0, gain_db, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
        let k = (PI * f0 / rate).tan();
        let vh = 10f64.powf(gain_db / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        let shelf = Biquad::new(
            [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
            [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        );
        // High-pass at about 38 Hz.
        let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
        let k = (PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        let high_pass = Biquad::new([1.0, -2.0, 1.0], [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0]);
        KWeighting { stages: [shelf, high_pass] }
    }

    fn process(&mut self, x: f64) -> f64 {
        self.stages.iter_mut().fold(x, |x, stage| stage.process(x))
    }
}

/// A second-order filter, `b` over `1 + a1·z⁻¹ + a2·z⁻²`.
#[derive(Debug, Clone)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    /// Direct form II transposed state.
    z: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Biquad { b, a, z: [0.0; 2] }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of a stereo sine of `frequency` Hz at `dbfs`.
    fn sine(frequency: f64, dbfs: f64, seconds: f64) -> Vec<u8> {
        let amplitude = 10f64.powf(dbfs / 20.0) * 32_767.0;
        (0..(seconds * RATE) as usize)
            .flat_map(|i| {
                let s = ((2.0 * PI * frequency * i as f64 / RATE).sin() * amplitude).round() as i16;
                [s.to_le_bytes(), s.to_le_bytes()].concat()
            })
            .collect()
    }

    #[test]
    fn filter_matches_the_standard_at_48_khz() {
        // ITU-R BS.1770-4, table 1 and 2.
        let k = KWeighting::new(48_000.0);
        let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-8);
        assert!(close(&k.stages[0].b, &[1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85]));
        assert!(close(&k.stages[0].a, &[-1.690_659_293_182_41, 0.732_480_774_215_85]));
        assert!(close(&k.stages[1].a, &[-1.990_047_454_833_98, 0.990_072_250_366_21]));
    }

    #[test]
    fn sine_at_minus_23_dbfs_is_minus_23_lufs() {
        // EBU Tech 3341, test 1: 1 kHz, −23 dBFS on both channels.
        let loudness = Analysis::new(&sine(1_000.0, -23.0, 20.0)).loudness().unwrap();
        assert!((loudness + 23.0).abs() < 0.05, "{loudness}");
        let gain = Analysis::new(&sine(1_000.0, -23.0, 20.0)).gain().unwrap();
        assert!((gain - 5.0).abs() < 0.05, "{gain}");
    }

    #[test]
    fn gates_silence_and_quiet_parts() {
        // EBU Tech 3341, test 3 in spirit: a loud part, a much quieter one
        // (below the relative gate) and silence measure as the loud part.
        let mut audio = sine(1_000.0, -20.0, 10.0);
        audio.extend(sine(1_000.0, -40.0, 10.0));
        audio.extend(vec![0; 4 * 44_100 * 10]);
        // The few blocks straddling the change of level count a little.
        let loudness = Analysis::new(&audio).loudness().unwrap();
        assert!((loudness + 20.0).abs() < 0.1, "{loudness}");
        assert_eq!(Analysis::new(&vec![0; 4 * 44_100 * 5]).loudness(), None);
        assert_eq!(Analysis::new(&sine(1_000.0, -20.0, 0.3)).loudness(), None);
    }

    #[test]
    fn album_measures_all_tracks_together() {
        let loud = Analysis::new(&sine(1_000.0, -14.0, 10.0));
        let quiet = Analysis::new(&sine(1_000.0, -20.0, 30.0));
        let album = album([&loud, &quiet]);
        // Energy-weighted: (10 s × 10^-1.4 + 30 s × 10^-2) / 40 s.
        let expected = 10.0 * ((10.0 * 10f64.powf(-1.4) + 30.0 * 10f64.powf(-2.0)) / 40.0).log10();
        assert!((album.loudness().unwrap() - expected).abs() < 0.05);
        assert_eq!(album.peak, loud.peak);

        let tags = tags(&quiet, Some(&album));
        assert!(matches!(&tags[0], ("REPLAYGAIN_TRACK_GAIN", gain) if gain == "1.99 dB" || gain == "2.00 dB"));
        assert_eq!(tags[1].0, "REPLAYGAIN_TRACK_PEAK");
        assert_eq!(tags.iter().map(|t| t.0).collect::<Vec<_>>()[2..], ["REPLAYGAIN_ALBUM_GAIN", "REPLAYGAIN_ALBUM_PEAK"]);
    }
}
