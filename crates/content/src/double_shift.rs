//! Two bare Shift taps in quick succession, as IntelliJ's Search Everywhere opens: shared by Lenz Code (Search
//! Everywhere) and Lenz Diff (the command palette), in their windows and terminal apps.
use std::time::{Duration, Instant};

/// Detect two bare Shift taps; shifted typing, selection, repeats and long holds do not count.
#[derive(Default)]
pub struct DoubleShift { down: Option<Instant>, previous: Option<Instant>, used: bool }
impl DoubleShift {
    pub fn update(&mut self, shift: bool, other: bool, now: Instant) -> bool {
        if other { self.interrupt(); }
        if shift {
            if self.down.is_none() { self.down = Some(now); self.used = other; }
            return false;
        }
        let Some(down) = self.down.take() else { return false };
        let short = !self.used && !other && now.saturating_duration_since(down) <= Duration::from_millis(400);
        self.used = false;
        if !short { self.previous = None; return false; }
        if self.previous.take().is_some_and(|last| now.saturating_duration_since(last) <= Duration::from_millis(400)) { return true; }
        self.previous = Some(now);
        false
    }
    pub fn interrupt(&mut self) { self.previous = None; self.used = true; }
    /// A Shift release from a terminal that may not report its press (foot): a tap unless a key was typed with
    /// Shift held. Only an actual Shift release counts this way; other modifiers' releases go through `update`.
    pub fn release(&mut self, other: bool, now: Instant) -> bool {
        if self.down.is_none() { self.down = Some(now); }
        self.update(false, other, now)
    }
    /// A Shift press was seen and its release has not been.
    pub fn held(&self) -> bool { self.down.is_some() }
    /// Another key: it ends a first tap, and one typed with Shift held means that Shift was not a tap.
    pub fn key(&mut self, shifted: bool) {
        self.previous = None;
        if shifted || self.down.is_some() { self.used = true; }
    }
    pub fn reset(&mut self) { *self = Self::default(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_shift_requires_two_short_uninterrupted_taps() {
        let start = Instant::now();
        let mut d = DoubleShift::default();
        assert!(!d.update(true,false,start));
        assert!(!d.update(false,false,start+Duration::from_millis(30)));
        assert!(!d.update(true,false,start+Duration::from_millis(120)));
        assert!(d.update(false,false,start+Duration::from_millis(150)));
        d.update(true,false,start+Duration::from_millis(180)); d.interrupt();
        assert!(!d.update(false,false,start+Duration::from_millis(210)));
        d.update(true,false,start+Duration::from_millis(230));
        assert!(!d.update(false,false,start+Duration::from_millis(260)));
        d.reset(); d.update(true,false,start);
        assert!(!d.update(true,false,start+Duration::from_millis(50)), "repeat");
        assert!(!d.update(false,false,start+Duration::from_secs(1)), "long hold");
        d.update(true,true,start+Duration::from_secs(2));
        assert!(!d.update(false,false,start+Duration::from_millis(2050)), "chord");
    }

    #[test]
    fn double_shift_counts_release_only_reports() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut d = DoubleShift::default();
        // foot reports a bare Shift's release only.
        assert!(!d.release(false,at(0)));
        assert!(d.release(false,at(150)));
        // A plain key between the taps ends the first; the next two taps count again.
        assert!(!d.release(false,at(1000)));
        d.key(false);
        assert!(!d.release(false,at(1100)));
        assert!(d.release(false,at(1200)));
        // Shifted typing: its Shift release is not a tap.
        d.key(true);
        assert!(!d.release(false,at(2000)));
        assert!(!d.release(false,at(2100)), "only one tap since the shifted key");
        assert!(d.release(false,at(2200)));
        // Taps too far apart do not count.
        assert!(!d.release(false,at(3000)));
        assert!(!d.release(false,at(3600)));
        // The desktop reports every modifier change through `update`: a Ctrl release is not a Shift tap.
        let mut d = DoubleShift::default();
        assert!(!d.update(false,true,at(4000)));
        assert!(!d.update(false,false,at(4050)), "Ctrl released");
        assert!(!d.update(true,false,at(4100)));
        assert!(!d.update(false,false,at(4150)), "one Shift tap after a Ctrl tap");
    }
}
