//! Local interface byte counters, sampled only by the visible System telemetry request.
use std::collections::HashMap;
#[derive(Clone, Copy)]
pub struct Counter {
    pub id: u64,
    pub received: u64,
    pub sent: u64,
}
#[derive(Default)]
pub struct Sampler {
    previous: HashMap<u64, Counter>,
    at: Option<std::time::Instant>,
}
impl Sampler {
    pub fn update(&mut self, counters: &[Counter], seconds: f64) -> Option<(f64, f64)> {
        let mut down = 0u64;
        let mut up = 0u64;
        let ready = !self.previous.is_empty() && seconds > 0.0 && seconds <= 2.0;
        for c in counters {
            if let Some(p) = self.previous.get(&c.id) {
                if let (Some(d), Some(u)) = (
                    c.received.checked_sub(p.received),
                    c.sent.checked_sub(p.sent),
                ) {
                    down = down.saturating_add(d);
                    up = up.saturating_add(u);
                }
            }
        }
        self.previous = counters.iter().map(|c| (c.id, *c)).collect();
        ready.then_some((down as f64 / seconds, up as f64 / seconds))
    }
    pub fn sample(&mut self) -> Option<(f64, f64)> {
        let now = std::time::Instant::now();
        let elapsed = self
            .at
            .replace(now)
            .map(|at| now.duration_since(at).as_secs_f64())
            .unwrap_or(0.0);
        let counters = counters();
        match counters {
            Some(c) => self.update(&c, elapsed),
            None => {
                self.previous.clear();
                None
            }
        }
    }
}
#[cfg(windows)]
fn counters() -> Option<Vec<Counter>> {
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2};
    unsafe {
        let mut table = std::ptr::null_mut();
        if GetIfTable2(&mut table).0 != 0 || table.is_null() {
            return None;
        }
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let result = rows
            .iter()
            .filter(|r| {
                r.OperStatus.0 == 1
                    && r.Type != 24
                    && r.Type != 131
                    && r.InterfaceAndOperStatusFlags._bitfield & 1 != 0
            })
            .map(|r| Counter {
                id: r.InterfaceLuid.Value,
                received: r.InOctets,
                sent: r.OutOctets,
            })
            .collect();
        FreeMibTable(table.cast());
        Some(result)
    }
}
#[cfg(not(windows))]
fn counters() -> Option<Vec<Counter>> {
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn c(id: u64, n: u64) -> Counter {
        Counter {
            id,
            received: n,
            sent: n / 2,
        }
    }
    #[test]
    fn rates_resets_down_and_idle() {
        let mut s = Sampler::default();
        assert_eq!(s.update(&[c(1, 100)], 1.0), None);
        assert_eq!(s.update(&[c(1, 300)], 1.0), Some((200.0, 100.0)));
        assert_eq!(s.update(&[c(1, 10)], 1.0), Some((0.0, 0.0)));
        assert_eq!(s.update(&[], 1.0), Some((0.0, 0.0)));
        assert_eq!(s.update(&[c(1, 900)], 1.0), None);
        assert_eq!(s.update(&[c(1, 1000)], 4.0), None);
    }
}
