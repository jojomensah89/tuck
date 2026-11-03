use std::sync::atomic::{AtomicUsize, Ordering};

const TARGET_RATE: u32 = 16_000;

pub(super) fn try_reserve_audio_frames(
    queued_frames: &AtomicUsize,
    incoming_frames: usize,
    maximum_frames: usize,
) -> bool {
    let mut current = queued_frames.load(Ordering::Acquire);
    loop {
        if incoming_frames == 0 || incoming_frames > maximum_frames.saturating_sub(current) {
            return false;
        }
        match queued_frames.compare_exchange_weak(
            current,
            current + incoming_frames,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

pub(super) fn release_audio_frames(queued_frames: &AtomicUsize, released_frames: usize) {
    let previous = queued_frames.fetch_sub(released_frames, Ordering::AcqRel);
    debug_assert!(previous >= released_frames);
}

pub(super) fn resample_to_16k(input: &[f32], source_rate: u32) -> Vec<f32> {
    if input.is_empty() || source_rate == 0 {
        return Vec::new();
    }
    if source_rate == TARGET_RATE {
        return input.to_vec();
    }
    let output_len = ((input.len() as u64 * TARGET_RATE as u64) / source_rate as u64) as usize;
    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        let start = output_index as f64 * source_rate as f64 / TARGET_RATE as f64;
        let end = (output_index + 1) as f64 * source_rate as f64 / TARGET_RATE as f64;
        let first = start.floor() as usize;
        let last = (end.ceil() as usize).min(input.len());
        let mut total = 0.0f32;
        let mut weight = 0.0f64;
        for index in first..last {
            let overlap = (end.min(index as f64 + 1.0) - start.max(index as f64)).max(0.0);
            total += input[index] * overlap as f32;
            weight += overlap;
        }
        output.push(if weight > 0.0 {
            total / weight as f32
        } else {
            0.0
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_preserves_duration_and_dc_value() {
        let input = vec![0.25; 48_000];
        let output = resample_to_16k(&input, 48_000);
        assert_eq!(output.len(), 16_000);
        assert!(output.iter().all(|sample| (*sample - 0.25).abs() < 0.0001));
    }

    #[test]
    fn identity_rate_copies_samples_without_filtering() {
        let input = [0.0, 0.75, -0.5];
        assert_eq!(resample_to_16k(&input, TARGET_RATE), input);
    }

    #[test]
    fn upsampling_preserves_a_constant_signal_and_duration() {
        let input = vec![-0.2; 8_000];
        let output = resample_to_16k(&input, 8_000);
        assert_eq!(output.len(), 16_000);
        assert!(output.iter().all(|sample| (*sample + 0.2).abs() < 0.0001));
    }

    #[test]
    fn invalid_zero_sample_rate_returns_no_audio() {
        assert!(resample_to_16k(&[1.0], 0).is_empty());
    }

    #[test]
    fn queued_audio_frames_stay_within_the_memory_budget() {
        let queued = AtomicUsize::new(0);
        assert!(try_reserve_audio_frames(&queued, 3, 8));
        assert!(try_reserve_audio_frames(&queued, 5, 8));
        assert!(!try_reserve_audio_frames(&queued, 1, 8));
        assert_eq!(queued.load(Ordering::Acquire), 8);
        release_audio_frames(&queued, 5);
        assert!(try_reserve_audio_frames(&queued, 5, 8));
        assert_eq!(queued.load(Ordering::Acquire), 8);
    }

    #[test]
    fn concurrent_audio_reservations_cannot_exceed_the_limit() {
        let queued = std::sync::Arc::new(AtomicUsize::new(0));
        let start = std::sync::Arc::new(std::sync::Barrier::new(9));
        let workers = (0..8)
            .map(|_| {
                let queued = queued.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    try_reserve_audio_frames(&queued, 7, 50)
                })
            })
            .collect::<Vec<_>>();
        start.wait();
        let accepted = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|accepted| *accepted)
            .count();
        assert_eq!(accepted, 7);
        assert_eq!(queued.load(Ordering::Acquire), 49);
    }
}
