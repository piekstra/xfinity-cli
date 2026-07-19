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

/// Billing summary from the new experience's `BBDS` object: a concise
/// Balance / Due / Autopay block, else flatten.
pub fn billing_summary(bbds: &Value) {
    let mut printed = false;
    if let Some(bal) = bbds.pointer("/balance/balanceDue").map(scalar) {
        println!("Balance:  ${bal}");
        printed = true;
    }
    if let Some(due) = bbds.get("dueDate").map(scalar).filter(|s| !s.is_empty()) {
        println!("Due:      {}", due.split('T').next().unwrap_or(&due));
        printed = true;
    }
    if let Some(pd) = bbds.pointer("/balance/pastDueBalance").map(scalar) {
        if !matches!(pd.as_str(), "" | "0" | "0.0" | "0.00") {
            println!("Past due: ${pd}");
            printed = true;
        }
    }
    match bbds.pointer("/autopay/status").map(scalar).as_deref() {
        Some("ACTIVE") | Some("ENROLLED") => {
            let d = bbds
                .pointer("/autopay/date")
                .map(scalar)
                .map(|x| x.split('T').next().unwrap_or("").to_string())
                .filter(|x| !x.is_empty());
            println!(
                "Autopay:  on{}",
                d.map(|x| format!(" (draws {x})")).unwrap_or_default()
            );
            printed = true;
        }
        Some(s) if !s.is_empty() => {
            println!("Autopay:  {}", s.to_lowercase());
            printed = true;
        }
        _ => {}
    }
    if bbds
        .pointer("/balance/isDelinquent")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        println!("Status:   delinquent");
    }
    if !printed {
        render(bbds);
    }
}

/// Account profile from the new experience's `accountContext`.
pub fn account(d: &Value) {
    let get = |k: &str| d.get(k).filter(|x| !x.is_null()).map(scalar);

    if let (Some(f), Some(l)) = (get("firstName"), get("lastName")) {
        println!("Name:     {f} {l}");
    }
    if let Some(a) = get("accountNumber") {
        println!("Account:  {a}");
    }
    if let Some(s) = get("status") {
        println!("Status:   {s}");
    }
    if let Some(p) = d.pointer("/contactInfo/homePhone").map(scalar) {
        if !p.is_empty() {
            println!("Phone:    {p}");
        }
    }
    if let Some(addr) = d.get("address") {
        let line = |k: &str| addr.get(k).map(scalar).unwrap_or_default();
        println!(
            "Address:  {} {} {} {}",
            line("line1"),
            line("city"),
            line("state"),
            line("zip")
        );
    }
    if let Some(tier) = d.pointer("/loyalty/loyaltyTier").map(scalar) {
        if !tier.is_empty() {
            println!("Loyalty:  {tier}");
        }
    }
}

/// Equipment/devices from the new experience's `deviceContext.equipment`.
pub fn devices(equipment: &Value) {
    let items = match equipment.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return render(equipment),
    };
    println!("MAKE | MODEL | STATUS | MAC | SERIAL");
    for d in items {
        let g = |k: &str| d.get(k).map(scalar).unwrap_or_default();
        println!(
            "{} | {} | {} | {} | {}",
            g("deviceMake"),
            g("deviceModel"),
            g("deviceStatus"),
            g("macaddress"),
            g("serialNumber"),
        );
    }
}

/// Outage status from the new experience's `outageContext`.
pub fn outages(oc: &Value) {
    let is_outage = oc
        .get("isOutage")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if !is_outage {
        println!("No active outages.");
        // Note upcoming/past if present.
        for (k, label) in [("futureOutage", "future"), ("pastOutage", "recent")] {
            if oc.get(k).and_then(|x| x.as_bool()).unwrap_or(false) {
                println!("({label} outage on record)");
            }
        }
        return;
    }
    println!("OUTAGE — affected services:");
    if let Some(cur) = oc.get("current").and_then(|x| x.as_object()) {
        for (svc, up) in cur {
            if up.as_bool().unwrap_or(false) {
                println!("  {svc}");
            }
        }
    } else {
        render(oc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn billing_summary_reads_bbds_shape() {
        // Smoke test: exercises the field paths without panicking.
        let bbds = json!({
            "dueDate": "2026-07-30",
            "balance": {"balanceDue": 45.33, "pastDueBalance": 0, "isDelinquent": false},
            "autopay": {"status": "ACTIVE", "date": "2026-07-30T00:00:00"}
        });
        billing_summary(&bbds); // prints; must not panic
    }

    #[test]
    fn outages_reports_clear_when_no_outage() {
        outages(&json!({"isOutage": false, "current": {"internet": false}}));
    }
}
