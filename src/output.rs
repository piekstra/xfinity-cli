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
