//! Assembling clips from synthesised segments.
//!
//! Everything here is arithmetic on samples: no synthesiser, no network. The
//! synthesiser supplies words; this decides where they sit in time, which is
//! what the evaluation set is actually measuring.
//!
//! Noise is deterministic. Regenerating the set on another machine has to
//! produce the same audio, or two reports are not comparable.

/// 20 ms at 16 kHz — the frame size the pipeline delivers.
pub const SAMPLE_RATE_HZ: u32 = 16_000;

/// The level of the noise floor that stands in for silence. Real calls always
/// carry one; a digitally silent gap behaves in ways no call does.
pub const NOISE_FLOOR_DBFS: f32 = -60.0;

/// Anything quieter than this counts as the synthesiser's own padding when
/// trimming. Above the noise floor, below any speech.
const TRIM_THRESHOLD_DBFS: f32 = -45.0;

pub fn dbfs_to_amplitude(dbfs: f32) -> f32 {
    10_f32.powf(dbfs / 20.0) * f32::from(i16::MAX)
}

pub fn ms_to_samples(ms: u32) -> usize {
    (u64::from(ms) * u64::from(SAMPLE_RATE_HZ) / 1000) as usize
}

pub fn samples_to_ms(samples: usize) -> u32 {
    (samples as u64 * 1000 / u64::from(SAMPLE_RATE_HZ)) as u32
}

/// Deterministic noise, so the set is reproducible. A seeded xorshift is ample
/// for a noise floor and costs nothing to carry.
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        // To roughly -1.0..1.0
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// `ms` of noise floor.
    pub fn floor(&mut self, ms: u32) -> Vec<i16> {
        let amplitude = dbfs_to_amplitude(NOISE_FLOOR_DBFS);
        (0..ms_to_samples(ms))
            .map(|_| (self.next() * amplitude) as i16)
            .collect()
    }

    /// A short burst at speech level, standing in for a cough. A synthesiser
    /// cannot say one, and the clip needs something that is loud, brief, and not
    /// words.
    pub fn burst(&mut self, ms: u32) -> Vec<i16> {
        let peak = dbfs_to_amplitude(-12.0);
        let total = ms_to_samples(ms);
        (0..total)
            .map(|index| {
                // Sharp attack, exponential decay — the shape of an impulse
                // rather than a tone.
                let progress = index as f32 / total as f32;
                let envelope = (-6.0 * progress).exp();
                (self.next() * peak * envelope) as i16
            })
            .collect()
    }
}

/// Drops the synthesiser's own leading and trailing near-silence.
///
/// Without this a gap labelled 600 ms is 600 ms plus whatever padding the
/// synthesiser attached, and every conclusion drawn from the clip is wrong by
/// an unknown amount.
pub fn trim(samples: &[i16]) -> &[i16] {
    let threshold = dbfs_to_amplitude(TRIM_THRESHOLD_DBFS) as i32;
    let loud = |sample: &i16| i32::from(sample.abs()) >= threshold;

    let Some(first) = samples.iter().position(loud) else {
        return &[];
    };
    let last = samples.iter().rposition(loud).unwrap_or(first);
    &samples[first..=last]
}

/// A linear fade over the final `ms`, ending at `floor_dbfs` of its original
/// level. Audio before the fade is untouched.
pub fn apply_decay(samples: &mut [i16], ms: u32, floor_dbfs: f32) {
    let span = ms_to_samples(ms).min(samples.len());
    if span == 0 {
        return;
    }
    let start = samples.len() - span;
    let floor = 10_f32.powf(floor_dbfs / 20.0);
    for (offset, sample) in samples[start..].iter_mut().enumerate() {
        let progress = offset as f32 / span as f32;
        let gain = 1.0 - (1.0 - floor) * progress;
        *sample = (f32::from(*sample) * gain) as i16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech(ms: u32) -> Vec<i16> {
        vec![8_000; ms_to_samples(ms)]
    }

    /// Spec 58.
    #[test]
    fn a_gap_is_exactly_as_long_as_it_says() {
        let mut noise = Noise::new(1);
        let gap = noise.floor(600);

        assert_eq!(gap.len(), 9_600, "600 ms at 16 kHz");
        assert_eq!(samples_to_ms(gap.len()), 600);
    }

    /// Spec 59. The reason trimming exists.
    #[test]
    fn trimming_removes_the_synthesisers_padding() {
        let mut noise = Noise::new(2);
        let mut padded = noise.floor(200);
        padded.extend(speech(500));
        padded.extend(noise.floor(200));

        let trimmed = trim(&padded);

        assert_eq!(
            samples_to_ms(trimmed.len()),
            500,
            "only the speech survives, so a labelled gap is the gap"
        );
    }

    /// Spec 60.
    #[test]
    fn trimming_silence_yields_nothing_rather_than_everything() {
        let mut noise = Noise::new(3);
        let quiet = noise.floor(500);

        assert!(trim(&quiet).is_empty());
        assert!(trim(&[]).is_empty());
    }

    /// Spec 61.
    #[test]
    fn decay_falls_monotonically_and_leaves_the_rest_alone() {
        let mut samples = speech(1_000);
        let before: Vec<i16> = samples[..ms_to_samples(200)].to_vec();

        apply_decay(&mut samples, 500, -30.0);

        assert_eq!(
            &samples[..ms_to_samples(200)],
            &before[..],
            "audio before the fade is untouched"
        );

        let tail = &samples[samples.len() - ms_to_samples(500)..];
        for pair in tail.windows(2) {
            assert!(pair[1] <= pair[0], "the fade never rises");
        }
        let expected_floor = (8_000.0 * 10_f32.powf(-30.0 / 20.0)) as i16;
        let last = *tail.last().expect("a non-empty tail");
        assert!(
            (last - expected_floor).abs() <= 2,
            "reaches the target level: got {last}, expected about {expected_floor}"
        );
    }

    /// The set has to regenerate byte for byte, or two reports measure two
    /// different sets of recordings.
    #[test]
    fn noise_is_reproducible() {
        assert_eq!(Noise::new(7).floor(100), Noise::new(7).floor(100));
        assert_ne!(Noise::new(7).floor(100), Noise::new(8).floor(100));
    }

    /// A cough has to be loud enough to be mistaken for speech, or the clip
    /// tests nothing.
    #[test]
    fn a_burst_is_loud_and_brief() {
        let burst = Noise::new(11).burst(150);

        assert_eq!(samples_to_ms(burst.len()), 150);
        let peak = burst.iter().map(|s| s.abs()).max().expect("samples");
        assert!(
            i32::from(peak) > dbfs_to_amplitude(-24.0) as i32,
            "a burst nobody could mistake for silence"
        );
    }
}
