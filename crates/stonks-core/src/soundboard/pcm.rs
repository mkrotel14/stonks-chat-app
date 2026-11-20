use std::{fs, path::Path};

use rodio::Source;

use super::{Error, MAX_BYTES, MAX_SECONDS};

pub(crate) const SAMPLE_RATE: u32 = 48_000;

pub(crate) fn decode(path: &Path) -> Result<Vec<i16>, Error> {
    let file = fs::File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound,
        _ => Error::Storage,
    })?;
    let metadata = file.metadata().map_err(|_| Error::Storage)?;
    if !metadata.is_file() {
        return Err(Error::InvalidAudio);
    }
    if metadata.len() > MAX_BYTES {
        return Err(Error::TooLarge);
    }
    let mut decoder = rodio::Decoder::try_from(file).map_err(|_| Error::InvalidAudio)?;
    let channels = usize::from(decoder.channels().get());
    let sample_rate = decoder.sample_rate().get();
    let max_frames = sample_rate as usize * MAX_SECONDS;
    let mut mono = Vec::new();
    // Downmix before resampling the whole clip so the resampling phase does not
    // restart at codec packet boundaries. Bound decoded work independently of file size.
    if sample_rate > 384_000 || channels > 32 {
        return Err(Error::InvalidAudio);
    }
    while let Some(first) = decoder.next() {
        if mono.len() == max_frames {
            return Err(Error::TooLong);
        }
        let mut sum = first;
        for _ in 1..channels {
            sum += decoder.next().ok_or(Error::InvalidAudio)?;
        }
        mono.push(sum / channels as f32);
    }
    let resampled = resample_cubic(&mono, sample_rate, SAMPLE_RATE);
    if resampled.is_empty() {
        return Err(Error::InvalidAudio);
    }
    Ok(resampled
        .into_iter()
        .map(|sample| (sample * i16::MAX as f32).round() as i16)
        .collect())
}

fn resample_cubic(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if input.is_empty() || source_rate == 0 || target_rate == 0 {
        return Vec::new();
    }
    if source_rate == target_rate {
        return input.to_vec();
    }

    let output_len = ((input.len() as u64 * target_rate as u64 + source_rate as u64 / 2)
        / source_rate as u64) as usize;
    let step = source_rate as f64 / target_rate as f64;
    let last = input.len().saturating_sub(1) as isize;
    let sample = |index: isize| input[index.clamp(0, last) as usize];

    (0..output_len)
        .map(|output_index| {
            let position = output_index as f64 * step;
            let center = position.floor() as isize;
            let fraction = (position - center as f64) as f32;
            let p0 = sample(center - 1);
            let p1 = sample(center);
            let p2 = sample(center + 1);
            let p3 = sample(center + 2);
            // Catmull-Rom interpolation keeps a continuous first derivative,
            // avoiding the harsh packet-edge transitions of the old path.
            let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
            let b = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
            let c = -0.5 * p0 + 0.5 * p2;
            ((a * fraction + b) * fraction + c) * fraction + p1
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_is_continuous_and_keeps_duration() {
        let source = (0..441)
            .map(|index| ((index as f32 / 441.0) * std::f32::consts::TAU * 5.0).sin())
            .collect::<Vec<_>>();
        let output = resample_cubic(&source, 44_100, 48_000);

        assert_eq!(output.len(), 480);
        assert!(output
            .windows(2)
            .all(|pair| (pair[1] - pair[0]).abs() < 0.1));
    }

    #[test]
    fn equal_rate_pcm_is_not_modified() {
        let source = vec![-1.0, -0.25, 0.0, 0.5, 1.0];
        assert_eq!(resample_cubic(&source, 48_000, 48_000), source);
    }
}
