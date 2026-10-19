/// Rational-phase interpolation for 16 -> 24 kHz mono speech. The phase and
/// one lookbehind sample survive packet boundaries, so splitting the input
/// never changes the PCM output and no packet tail is lost or repeated.
#[derive(Default)]
pub(super) struct Resampler16To24 {
    received: u64,
    emitted: u64,
    last: Option<i16>,
}
impl Resampler16To24 {
    pub fn push(&mut self, pcm: &[i16]) -> Vec<i16> {
        if pcm.is_empty() {
            return Vec::new();
        }
        let start = self.received;
        self.received += pcm.len() as u64;
        let mut result = Vec::with_capacity(pcm.len() * 3 / 2 + 2);
        loop {
            let numerator = self.emitted * 2;
            let left = numerator / 3;
            let fraction = numerator % 3;
            if left >= self.received || (fraction != 0 && left + 1 >= self.received) {
                break;
            }
            let sample = |index: u64| {
                if index < start {
                    self.last.expect("one retained boundary sample")
                } else {
                    pcm[(index - start) as usize]
                }
            };
            let a = i32::from(sample(left));
            let b = if fraction == 0 {
                a
            } else {
                i32::from(sample(left + 1))
            };
            result.push(
                ((a * (3 - fraction as i32) + b * fraction as i32) as f64 / 3.0).round() as i16,
            );
            self.emitted += 1;
        }
        self.last = pcm.last().copied();
        result
    }
    pub fn finish(&mut self) -> Vec<i16> {
        let count = self.received * 3 / 2 - self.emitted;
        self.emitted += count;
        vec![self.last.unwrap_or(0); count as usize]
    }
}
