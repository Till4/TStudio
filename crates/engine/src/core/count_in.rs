//! Count-in before playback (recording): N metronome clicks, then the transport starts.
//! The clicks are placed sample-accurately; `EngineCore::process` splits its chunks so the
//! song starts exactly on the frame after the last count-in beat.

/// Running count-in. Frames are counted from the moment `Play` (with count-in) arrived.
#[derive(Clone, Debug)]
pub(super) struct CountIn {
    beats: u32,
    beat_frames: f64,
    beats_per_bar: u32,
    done: usize,
}

impl CountIn {
    pub(super) fn new(beats: u32, beat_frames: f64, beats_per_bar: u32) -> Self {
        Self { beats, beat_frames: beat_frames.max(1.0), beats_per_bar: beats_per_bar.max(1), done: 0 }
    }

    /// Total length in frames.
    pub(super) fn total(&self) -> usize {
        (self.beats as f64 * self.beat_frames).round() as usize
    }

    /// Frames left until the transport starts.
    pub(super) fn remaining(&self) -> usize {
        self.total().saturating_sub(self.done)
    }

    /// Advances by `n` frames and reports every click inside them as
    /// (frame offset inside the block, downbeat).
    pub(super) fn advance(&mut self, n: usize, mut click: impl FnMut(usize, bool)) {
        let (a, b) = (self.done, self.done + n);
        for k in 0..self.beats {
            let f = (k as f64 * self.beat_frames).round() as usize;
            if f >= a && f < b {
                click(f - a, k % self.beats_per_bar == 0);
            }
        }
        self.done = b.min(self.total());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_on_every_beat_with_downbeats() {
        // 120 bpm at 48 kHz: 24000 frames per beat, 4/4, one bar
        let mut c = CountIn::new(4, 24000.0, 4);
        assert_eq!(c.total(), 96000);
        let mut clicks = Vec::new();
        let mut base = 0;
        while c.remaining() > 0 {
            let n = c.remaining().min(1000);
            c.advance(n, |f, down| clicks.push((base + f, down)));
            base += n;
        }
        assert_eq!(clicks, vec![(0, true), (24000, false), (48000, false), (72000, false)]);
        assert_eq!(base, 96000);
    }

    #[test]
    fn two_bars_have_two_downbeats() {
        let mut c = CountIn::new(8, 100.0, 4);
        let mut downs = 0;
        c.advance(10_000, |_, d| downs += d as usize);
        assert_eq!(downs, 2);
        assert_eq!(c.remaining(), 0);
    }
}
