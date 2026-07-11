//! Output rendering. The generic text renderer (`Key: value` blocks,
//! pipe-delimited tables) and JSON emission come from `pk-cli-core`;
//! Xfinity-specific views live here.
//!
//! Many Xfinity response shapes aren't pinned down yet, so [`render`]
//! flattens whatever JSON comes back into readable text. As shapes are
//! confirmed, add a purpose-built renderer next to the generic one. For the
//! raw structure, use `xfin api`.

use pk_cli_core::output::scalar;
use serde_json::Value;

pub use pk_cli_core::output::{fail, json};

/// Default text renderer for a resource read: unwrap Xfinity's `{data: …}`
/// envelope, then hand off to the shared renderer.
pub fn render(v: &Value) {
    pk_cli_core::output::render(v.get("data").unwrap_or(v));
}

/// Billing summary: a concise Balance / Due / Autopay block, else flatten.
/// The `/apis/bill/current` payload nests these under a `summary` object.
pub fn billing_summary(v: &Value) {
    let root = v.get("data").unwrap_or(v);
    let s = root.get("summary").unwrap_or(root);
    let first = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .filter_map(|k| s.get(*k))
            .find(|x| !x.is_null())
            .map(scalar)
    };

    let mut printed = false;
    if let Some(bal) = first(&["balanceDue", "currentBalance", "balance", "amountDue"]) {
        println!("Balance:  ${bal}");
        printed = true;
    }
    if let Some(due) = first(&["dueDate", "paymentDueDate", "billDueDate"]) {
        println!("Due:      {}", due.split('T').next().unwrap_or(&due));
        printed = true;
    }
    if let Some(pd) = first(&["pastDueAmount", "pastDue"]) {
        if !matches!(pd.as_str(), "" | "0" | "0.0" | "0.00") {
            println!("Past due: ${pd}");
            printed = true;
        }
    }
    match first(&["autoPayEnabled"]) {
        Some(ap) if ap == "true" => {
            let d = first(&["autoPayDate"])
                .map(|x| x.split('T').next().unwrap_or("").to_string())
                .filter(|x| !x.is_empty());
            println!(
                "Autopay:  on{}",
                d.map(|x| format!(" (draws {x})")).unwrap_or_default()
            );
            printed = true;
        }
        Some(_) => {
            println!("Autopay:  off");
            printed = true;
        }
        None => {}
    }
    if let Some(dq) = root.pointer("/delinquency/delinquencyStatus").map(scalar) {
        if dq != "NOT_DELINQUENT" && !dq.is_empty() {
            println!("Status:   {dq}");
        }
    }
    if !printed {
        render(v);
    }
}

/// Account profile from `/apis/macaroon`: name/contact block plus a one-line
/// account summary. Skips the embedded CSP auth token.
pub fn account(v: &Value) {
    let d = v.get("data").unwrap_or(v);
    let get = |k: &str| d.get(k).filter(|x| !x.is_null()).map(scalar);

    if let (Some(f), Some(l)) = (get("firstName"), get("lastName")) {
        println!("Name:     {f} {l}");
    }
    if let Some(u) = get("uid") {
        println!("Username: {u}");
    }
    if let Some(e) = get("preferredEmail") {
        println!("Email:    {e}");
    }
    if let Some(m) = get("mobileNumber") {
        println!("Mobile:   {m}");
    }
    if let Some(a) = get("mainAccountNumber") {
        println!("Account:  {a}");
    }
    // The `accounts` array carries service address + status (but also a huge
    // CSP token we deliberately omit).
    if let Some(acct) = d
        .get("accounts")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
    {
        if let Some(status) = acct.get("accountStatus").map(scalar) {
            println!("Status:   {status}");
        }
        if let Some(addr) = acct.get("serviceAddress") {
            let line = |k: &str| addr.get(k).map(scalar).unwrap_or_default();
            println!(
                "Address:  {} {} {} {}",
                line("addressLine1"),
                line("city"),
                line("state"),
                line("zip")
            );
        }
    }
}

/// A sortable `YYYYMMDD` key from an `MM/DD/YYYY` date string, e.g.
/// `"07/30/2026"` -> `20260730`. `None` if it doesn't parse.
fn ymd_key(mmddyyyy: &str) -> Option<i64> {
    let mut it = mmddyyyy.split('/');
    let m: i64 = it.next()?.trim().parse().ok()?;
    let d: i64 = it.next()?.trim().parse().ok()?;
    let y: i64 = it.next()?.trim().parse().ok()?;
    Some(y * 10_000 + m * 100 + d)
}

/// Pick the index of the "current" billing/usage cycle from a list of
/// `{startDate, endDate}` entries: the one whose range contains `today_key`,
/// else the one with the latest `endDate`. Robust to array ordering (Xfinity
/// returns months oldest- or newest-first depending on the endpoint).
fn current_cycle(months: &[Value], today_key: i64) -> Option<usize> {
    let mut best_containing: Option<usize> = None;
    let mut latest: Option<(i64, usize)> = None;
    for (i, m) in months.iter().enumerate() {
        let start = m
            .get("startDate")
            .and_then(|x| x.as_str())
            .and_then(ymd_key);
        let end = m.get("endDate").and_then(|x| x.as_str()).and_then(ymd_key);
        if let (Some(s), Some(e)) = (start, end) {
            if s <= today_key && today_key <= e {
                best_containing = Some(i);
            }
            if latest.map(|(k, _)| e > k).unwrap_or(true) {
                latest = Some((e, i));
            }
        }
    }
    best_containing.or_else(|| latest.map(|(_, i)| i))
}

/// Internet data usage: a concise current-cycle summary
/// (`/apis/csp/account/me/services/internet/usage`). Falls back to the generic
/// renderer if the expected shape isn't present.
pub fn usage(v: &Value) {
    let root = v.get("data").unwrap_or(v);
    let months = match root.get("usageMonths").and_then(|m| m.as_array()) {
        Some(m) if !m.is_empty() => m,
        _ => return render(v),
    };
    let (y, mo, d) = pk_cli_core::dates::today();
    let today_key = y * 10_000 + mo as i64 * 100 + d as i64;
    let cur = match current_cycle(months, today_key).and_then(|i| months.get(i)) {
        Some(c) => c,
        None => return render(v),
    };

    let num = |k: &str| -> Option<f64> {
        cur.get(k).and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
        })
    };
    let text = |k: &str| cur.get(k).map(scalar).unwrap_or_default();

    let start = text("startDate");
    let end = text("endDate");
    if !start.is_empty() {
        println!("Cycle:    {start} – {end}");
    }
    let unit = {
        let u = text("unitOfMeasure");
        if u.is_empty() {
            "GB".to_string()
        } else {
            u
        }
    };
    match (num("totalUsage"), num("allowableUsage")) {
        (Some(u), Some(a)) if a > 0.0 => {
            println!("Used:     {u:.0} of {a:.0} {unit} ({:.0}%)", u / a * 100.0);
        }
        (Some(u), _) => println!("Used:     {u:.0} {unit}"),
        _ => {}
    }
    if let (Some(h), Some(w)) = (num("homeUsage"), num("wifiUsage")) {
        if h + w > 0.0 {
            println!("  home {h:.0} {unit} · wifi {w:.0} {unit}");
        }
    }
    if root
        .get("inPaidOverage")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        let charge = num("overageCharges")
            .filter(|c| *c > 0.0)
            .map(|c| format!(" (${c:.2})"))
            .unwrap_or_default();
        println!("Status:   in paid overage{charge}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ymd_key_parses_mm_dd_yyyy() {
        assert_eq!(ymd_key("07/30/2026"), Some(20_260_730));
        assert_eq!(ymd_key("1/2/2026"), Some(20_260_102));
        assert_eq!(ymd_key("garbage"), None);
    }

    #[test]
    fn current_cycle_prefers_containing_range() {
        let months = json!([
            {"startDate": "05/01/2026", "endDate": "05/31/2026"},
            {"startDate": "07/01/2026", "endDate": "07/31/2026"},
            {"startDate": "06/01/2026", "endDate": "06/30/2026"},
        ]);
        let m = months.as_array().unwrap();
        // Today inside the July range -> index 1, regardless of ordering.
        assert_eq!(current_cycle(m, 20_260_715), Some(1));
        // Today after all ranges -> latest endDate (July, index 1).
        assert_eq!(current_cycle(m, 20_261_231), Some(1));
    }
}
