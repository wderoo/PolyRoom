//! Human-readable formatting of numbers, sizes and dates.

use polyroom_engine::MeshStats;

/// `1234567` → `1,234,567`.
pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 1 model, 2 models, 1,234 models.
pub fn models(n: u64) -> String {
    if n == 1 {
        "1 model".to_owned()
    } else {
        format!("{} models", count(n))
    }
}

/// What a folder holds: `3 models · 4 images · 1 ZIP`, leaving out kinds it has none of.
pub fn contents(models_n: u64, images: u64, archives: u64) -> String {
    let mut parts = Vec::new();
    if models_n > 0 || (images == 0 && archives == 0) {
        parts.push(models(models_n));
    }
    if images > 0 {
        parts.push(match images {
            1 => "1 image".to_owned(),
            n => format!("{} images", count(n)),
        });
    }
    if archives > 0 {
        parts.push(format!("{} ZIP", count(archives)));
    }
    parts.join(" · ")
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{n} {name}")
    } else {
        format!("{v:.1} {name}")
    }
}

/// `120.0 × 80.5 × 42.0 mm`.
pub fn dimensions(stats: &MeshStats) -> String {
    let s = stats.bounds.size();
    format!("{} × {} × {} mm", mm(s.x), mm(s.y), mm(s.z))
}

fn mm(v: f32) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Volume in cm³ (mm³ / 1000).
pub fn volume(mm3: f64) -> String {
    format!("{:.1} cm³", mm3 / 1000.0)
}

pub fn area(mm2: f64) -> String {
    format!("{:.1} cm²", mm2 / 100.0)
}

/// Seconds since the Unix epoch → `YYYY-MM-DD` (UTC).
pub fn date(secs: u64) -> String {
    // Howard Hinnant's civil-from-days algorithm.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(count(0), "0");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_760_572_800), "2025-10-16");
        assert_eq!(volume(1000.0), "1.0 cm³");
        assert_eq!(models(1), "1 model");
        assert_eq!(contents(3, 4, 1), "3 models · 4 images · 1 ZIP");
        assert_eq!(contents(0, 1, 0), "1 image");
        assert_eq!(contents(0, 0, 0), "0 models");
        assert_eq!(models(1500), "1,500 models");
    }
}
