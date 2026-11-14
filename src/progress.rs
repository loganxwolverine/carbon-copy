//! Vitesse et temps restant d'une copie, à partir des fractions successives
//! (0.0 – 1.0) et de la taille totale à lire.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Fenêtre glissante : la vitesse reflète les dernières secondes, pas la
/// moyenne depuis le début (le démarrage du lecteur fausserait tout).
const WINDOW: Duration = Duration::from_secs(30);
/// Durée minimale d'observation avant d'afficher une estimation.
const WARMUP: Duration = Duration::from_secs(5);

pub struct Rate {
    total_bytes: u64,
    samples: VecDeque<(Instant, f64)>,
}

/// Estimation courante.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// Octets par seconde (`None` si la taille totale est inconnue).
    pub bytes_per_sec: Option<f64>,
    pub remaining: Duration,
}

impl Rate {
    pub fn new(total_bytes: u64) -> Self {
        Self { total_bytes, samples: VecDeque::new() }
    }

    pub fn update(&mut self, fraction: f64) -> Option<Estimate> {
        self.update_at(Instant::now(), fraction)
    }

    fn update_at(&mut self, now: Instant, fraction: f64) -> Option<Estimate> {
        // Une fraction qui recule (nouvelle phase) repart de zéro.
        if self.samples.back().is_some_and(|(_, f)| fraction + 1e-9 < *f) {
            self.samples.clear();
        }
        self.samples.push_back((now, fraction));
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
        let (t0, f0) = *self.samples.front()?;
        let dt = now.duration_since(t0);
        let df = fraction - f0;
        if dt < WARMUP || df <= 0.0 {
            return None;
        }
        let per_sec = df / dt.as_secs_f64();
        let remaining = Duration::from_secs_f64(((1.0 - fraction).max(0.0) / per_sec).min(360_000.0));
        let bytes_per_sec = (self.total_bytes > 0).then(|| per_sec * self.total_bytes as f64);
        Some(Estimate { bytes_per_sec, remaining })
    }
}

/// « 8,4 Mo/s · reste 12 min ».
pub fn describe(e: &Estimate) -> String {
    let mut parts = Vec::new();
    if let Some(b) = e.bytes_per_sec {
        parts.push(format!("{} Mo/s", format!("{:.1}", b / 1e6).replace('.', ",")));
    }
    parts.push(format!("reste {}", short_duration(e.remaining)));
    parts.join(" · ")
}

/// « 1 h 05 », « 12 min », « moins d'une minute ».
pub fn short_duration(d: Duration) -> String {
    let mins = (d.as_secs() + 30) / 60;
    match mins {
        0 => "moins d'une minute".into(),
        m if m < 60 => format!("{m} min"),
        m => format!("{} h {:02}", m / 60, m % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_speed_and_remaining_time() {
        let mut r = Rate::new(6_000_000_000);
        let t = Instant::now();
        assert_eq!(r.update_at(t, 0.0), None);
        // 1 % toutes les 10 s → 60 Mo / 10 s = 6 Mo/s ; 99 % restants ≈ 990 s.
        let e = r.update_at(t + Duration::from_secs(10), 0.01).unwrap();
        assert!((e.bytes_per_sec.unwrap() - 6e6).abs() < 1.0);
        assert_eq!(e.remaining.as_secs(), 990);
        assert_eq!(describe(&e), "6,0 Mo/s · reste 17 min");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(short_duration(Duration::from_secs(20)), "moins d'une minute");
        assert_eq!(short_duration(Duration::from_secs(65 * 60)), "1 h 05");
    }
}
