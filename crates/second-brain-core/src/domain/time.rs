//! Utilitários de tempo **std-only** para o domínio.
//!
//! O legado usa `Date.toISOString()` (UTC) para frontmatter/Session e `Date.now()`
//! (epoch ms) para metadata. Reproduzimos a mesma semântica de forma determinística:
//! hora UTC via algoritmo civil (Howard Hinnant), sem dependência de crono.

use std::time::{SystemTime, UNIX_EPOCH};

/// Epoch milliseconds atual (equivalente a `Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `civil_from_days` — algoritmo de Howard Hinnant (dias desde epoch → (ano, mês, dia)).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + i64::from(m <= 2);
    (y, m as u32, d as u32)
}

/// ISO-8601 UTC (`2026-10-06T22:00:00.123Z`) — equivale a `Date.toISOString()`.
pub fn to_iso_utc(ms: i64) -> String {
    let total_secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{millis:03}Z")
}

/// Data UTC `YYYY-MM-DD` — equivale a `Date.toISOString().split("T")[0]`.
pub fn to_date_utc(ms: i64) -> String {
    to_iso_utc(ms).get(0..10).unwrap_or_default().to_string()
}

/// Hora UTC `HH:MM:SS` — determinística (o legado usava `toLocaleTimeString()`,
/// dependente de locale; ver divergence de paridade em migration-log P1).
pub fn to_time_utc(ms: i64) -> String {
    let total_secs = ms.div_euclid(1000);
    let secs_of_day = total_secs.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_matches_known_utc() {
        // 2026-10-06T22:00:00.123Z em epoch ms.
        const MS: i64 = 1_791_324_000_123;
        assert_eq!(to_iso_utc(MS), "2026-10-06T22:00:00.123Z");
        assert_eq!(to_date_utc(MS), "2026-10-06");
    }

    #[test]
    fn iso_handles_leap_day() {
        // 2024-02-29T00:00:00.000Z
        const MS: i64 = 1_709_164_800_000;
        assert_eq!(to_date_utc(MS), "2024-02-29");
    }
}
