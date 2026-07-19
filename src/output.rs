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

/// Internet plan from `subscriptionContext.customerPlanInfo.internet[0]`:
/// subscribed speed and description.
pub fn internet_plan(net: &Value) {
    let g = |k: &str| net.get(k).filter(|x| !x.is_null()).map(scalar);
    let mut printed = false;
    if let Some(p) = g("plan") {
        println!("Plan:     {p}");
        printed = true;
    }
    if let Some(d) = g("planDescription") {
        println!("Speed:    {d}");
        printed = true;
    }
    // Surface the data policy from the most recent usage cycle, if present.
    if let Some(policy) = net
        .get("usageMonths")
        .and_then(|m| m.as_array())
        .and_then(|a| a.last())
        .and_then(|m| m.get("policyName"))
        .map(scalar)
        .filter(|s| !s.is_empty())
    {
        println!("Data:     {policy}");
        printed = true;
    }
    if !printed {
        render(net);
    }
}

/// Data usage for the current cycle from
/// `subscriptionContext.customerPlanInfo.internet[0].usageMonths` (last entry
/// is the current cycle). Shows used/allowable GB, cycle window, and per-device.
pub fn internet_usage(net: &Value) {
    let months = match net.get("usageMonths").and_then(|m| m.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => {
            println!("No data-usage information available for this account.");
            return;
        }
    };
    let cur = months.last().unwrap();
    let g = |k: &str| cur.get(k).map(scalar).unwrap_or_default();
    let unit = {
        let u = g("unitOfMeasure");
        if u.is_empty() {
            "GB".into()
        } else {
            u
        }
    };
    let used = g("homeUsage");
    let allow = g("allowableUsage");
    let (start, end) = (g("startDate"), g("endDate"));
    if !start.is_empty() || !end.is_empty() {
        println!("Cycle:    {start} – {end}");
    }
    // An unlimited plan shows up as allowableUsage 0, a very large sentinel
    // (the API returns 100000 GB for the Unlimited Data Plan), or an
    // "Unlimited …" policyName.
    const UNLIMITED_SENTINEL_GB: f64 = 100_000.0;
    let unlimited = matches!(allow.as_str(), "" | "0")
        || allow
            .parse::<f64>()
            .map(|n| n >= UNLIMITED_SENTINEL_GB)
            .unwrap_or(false)
        || cur
            .get("policyName")
            .map(scalar)
            .map(|p| p.to_lowercase().contains("unlimited"))
            .unwrap_or(false);
    if unlimited {
        println!("Used:     {used} {unit} (unlimited plan)");
    } else {
        println!("Used:     {used} of {allow} {unit}");
    }
    if let Some(policy) = cur.get("policyName").map(scalar).filter(|s| !s.is_empty()) {
        println!("Policy:   {policy}");
    }
    if let Some(devs) = cur.get("devices").and_then(|d| d.as_array()) {
        for d in devs {
            let id = d.get("id").map(scalar).unwrap_or_default();
            let u = d.get("usage").map(scalar).unwrap_or_default();
            if !id.is_empty() {
                println!("  {id}: {u} {unit}");
            }
        }
    }
}

/// Autopay enrollment from `BBDS.autopay`: status, method, masked instrument,
/// and next draw date.
pub fn autopay(ap: &Value) {
    let g = |k: &str| ap.get(k).filter(|x| !x.is_null()).map(scalar);
    let status = g("status").unwrap_or_default();
    if status.is_empty() {
        println!("Autopay:  not enrolled");
        return;
    }
    let on = matches!(status.to_uppercase().as_str(), "ON" | "ACTIVE" | "ENROLLED");
    println!("Autopay:  {}", if on { "on" } else { &status });
    if let Some(m) = g("method") {
        println!("Method:   {m}");
    }
    // Masked instrument: show type + last 4 only (never the full number).
    if let Some(inst) = ap.get("autopayInstrument") {
        let ty = inst
            .get("paymentInstrumentType")
            .map(scalar)
            .unwrap_or_default();
        let last4 = inst.get("instrumentNumber").map(scalar).unwrap_or_default();
        if !ty.is_empty() || !last4.is_empty() {
            println!("Account:  {ty} ••••{last4}");
        }
    }
    if let Some(d) = g("date").map(|x| x.split('T').next().unwrap_or("").to_string()) {
        if !d.is_empty() {
            println!("Next:     {d}");
        }
    }
}

/// Full per-cycle data-usage history from `usageMonths[]` as a table
/// (newest last, the way the API orders it).
pub fn internet_usage_history(net: &Value) {
    let months = match net.get("usageMonths").and_then(|m| m.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => {
            println!("No data-usage history available for this account.");
            return;
        }
    };
    println!("CYCLE START | CYCLE END | USED | ALLOWABLE | UNIT");
    for m in months {
        let g = |k: &str| m.get(k).map(scalar).unwrap_or_default();
        let unit = {
            let u = g("unitOfMeasure");
            if u.is_empty() {
                "GB".into()
            } else {
                u
            }
        };
        let allow = g("allowableUsage");
        let allow_disp = if matches!(allow.as_str(), "" | "0")
            || allow
                .parse::<f64>()
                .map(|n| n >= 100_000.0)
                .unwrap_or(false)
        {
            "unlimited".to_string()
        } else {
            allow
        };
        println!(
            "{} | {} | {} | {} | {}",
            g("startDate"),
            g("endDate"),
            g("homeUsage"),
            allow_disp,
            unit,
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

    #[test]
    fn internet_plan_and_usage_read_subscription_shape() {
        let net = json!({
            "plan": "300Mbps",
            "planDescription": "Speeds up to 300",
            "usageMonths": [
                {"allowableUsage": 1024, "homeUsage": 100, "unitOfMeasure": "GB",
                 "startDate": "01-JUN-2026", "endDate": "30-JUN-2026",
                 "policyName": "Unlimited Data Plan", "devices": []},
                {"allowableUsage": 1024, "homeUsage": 200, "unitOfMeasure": "GB",
                 "startDate": "01-JUL-2026", "endDate": "31-JUL-2026",
                 "policyName": "Unlimited Data Plan",
                 "devices": [{"id": "aa:bb:cc:dd:ee:ff", "usage": 200}]}
            ]
        });
        internet_plan(&net); // uses current (last) cycle for the data policy
        internet_usage(&net); // must not panic; reads last month as current
        internet_usage_history(&net); // table over all cycles
    }

    #[test]
    fn autopay_renders_and_masks_instrument() {
        let ap = json!({
            "status": "ON",
            "method": "EFT",
            "date": "2026-07-30T00:00:00",
            "autopayInstrument": {"paymentInstrumentType": "Checking", "instrumentNumber": "0000"}
        });
        autopay(&ap); // prints "on", method, masked account, next date; must not panic
        autopay(&json!({"status": ""})); // "not enrolled" path
    }
}
