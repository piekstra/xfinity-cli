//! utility/v1 profile mapping (cli-common v0.2.0). Converts Xfinity's
//! provider-shaped `BBDS` JSON into the family's shared DTOs
//! (`utility-summary/v1`, `statement/v1`): string-decimal [`Money`], ISO
//! `YYYY-MM-DD` dates, best-effort on every field (Xfinity shapes drift).

use pk_cli_core::output::scalar;
use pk_cli_core::{dates, Money};
use pk_cli_utility::{Statement, UtilitySummary};
use serde_json::Value;

/// Provider money scalar (JSON number `45.33` or text `"$45.33"`) → profile
/// [`Money`] (string-decimal — never floats on the wire).
pub fn money(v: Option<&Value>) -> Option<Money> {
    match v? {
        v @ (Value::Number(_) | Value::String(_)) => Money::parse_usd(&scalar(v)),
        _ => None,
    }
}

/// Provider date (`YYYY-MM-DD[Thh:mm:ss]`, `MM/DD/YYYY`) → ISO `YYYY-MM-DD`
/// per the profile contract; unrecognized text passes through verbatim.
pub fn iso_date(s: &str) -> String {
    let d = s.split('T').next().unwrap_or(s);
    if dates::parse_iso(d).is_ok() {
        return d.to_string();
    }
    if let [m, day, y] = d.split('/').collect::<Vec<_>>()[..] {
        if let (Ok(m), Ok(day), Ok(y)) = (m.parse::<u32>(), day.parse::<u32>(), y.parse::<i64>()) {
            if (1..=12).contains(&m) && (1..=31).contains(&day) && y >= 1000 {
                return format!("{y:04}-{m:02}-{day:02}");
            }
        }
    }
    s.to_string()
}

/// Non-empty scalar at a JSON pointer.
fn str_at(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).map(scalar).filter(|s| !s.is_empty())
}

/// BBDS → `utility-summary/v1`. `account` is the CLI/config override when
/// set — BBDS itself doesn't carry the account number.
pub fn summary_dto(bbds: &Value, account: Option<String>) -> UtilitySummary {
    let mut dto = UtilitySummary::new(
        money(bbds.pointer("/balance/balanceDue")).unwrap_or_else(|| Money::usd("0.00")),
    );
    dto.due_date = str_at(bbds, "/dueDate").map(|d| iso_date(&d));
    dto.account = account.filter(|a| !a.is_empty());
    dto.autopay = match str_at(bbds, "/autopay/status")
        .map(|s| s.to_uppercase())
        .as_deref()
    {
        Some("ON" | "ACTIVE" | "ENROLLED") => Some(true),
        Some(_) => Some(false),
        None => None,
    };
    dto
}

/// The raw statement records under BBDS `statementDetails`. The new account
/// experience reports a single summary object (billStatus, lastStatementDate,
/// statementBalance); tolerate a list in case the shape drifts.
pub fn statement_values(bbds: &Value) -> Vec<Value> {
    match bbds.get("statementDetails") {
        Some(Value::Array(a)) => a.clone(),
        Some(v) if v.is_object() => vec![v.clone()],
        _ => Vec::new(),
    }
}

/// One raw statement object → `statement/v1`, best-effort (absent fields are
/// omitted, never panicked on). `pos` is the record's 1-based position — the
/// id of last resort when the provider gives neither an id nor a date.
pub fn statement_dto(raw: &Value, pos: usize) -> Statement {
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| raw.get(*k).map(scalar).filter(|s| !s.is_empty()))
    };
    let date = field(&["lastStatementDate", "statementDate", "date"]).map(|d| iso_date(&d));
    Statement {
        id: field(&["statementId", "id"])
            .or_else(|| date.clone())
            .unwrap_or_else(|| pos.to_string()),
        date,
        amount: money(raw.get("statementBalance").or_else(|| raw.get("amount")))
            .unwrap_or_else(|| Money::usd("0.00")),
        due_date: field(&["dueDate"]).map(|d| iso_date(&d)),
        paid: match field(&["billStatus", "status"])
            .map(|s| s.to_uppercase())
            .as_deref()
        {
            Some("PAID") => Some(true),
            Some("DUE" | "UNPAID" | "OPEN" | "OVERDUE" | "PAST DUE" | "PAST_DUE") => Some(false),
            _ => None,
        },
    }
}

/// Does `date` (ISO, from a DTO) fall inside the already-validated
/// `--since`/`--until` bounds? With a bound set, records without a comparable
/// date are excluded — a date filter can only match records that have one.
pub fn in_range(date: Option<&str>, since: Option<&str>, until: Option<&str>) -> bool {
    if since.is_none() && until.is_none() {
        return true;
    }
    let Some(d) = date.and_then(|d| dates::parse_iso(d).ok()) else {
        return false;
    };
    if let Some(s) = since.and_then(|s| dates::parse_iso(s).ok()) {
        if d < s {
            return false;
        }
    }
    if let Some(u) = until.and_then(|u| dates::parse_iso(u).ok()) {
        if d > u {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn money_parses_numbers_and_provider_strings() {
        assert_eq!(money(Some(&json!(45.33))).unwrap().amount, "45.33");
        assert_eq!(money(Some(&json!(45.3))).unwrap().amount, "45.30");
        assert_eq!(money(Some(&json!(0))).unwrap().amount, "0.00");
        assert_eq!(money(Some(&json!(-12.34))).unwrap().amount, "-12.34");
        assert_eq!(money(Some(&json!("$1,234.50"))).unwrap().amount, "1234.50");
        assert_eq!(money(Some(&json!(45.33))).unwrap().currency, "USD");
        assert!(money(Some(&json!(null))).is_none());
        assert!(money(Some(&json!("n/a"))).is_none());
        assert!(money(Some(&json!({"nested": 1}))).is_none());
        assert!(money(None).is_none());
    }

    #[test]
    fn iso_date_normalizes_provider_formats() {
        assert_eq!(iso_date("2026-07-30"), "2026-07-30");
        assert_eq!(iso_date("2026-07-30T00:00:00"), "2026-07-30");
        assert_eq!(iso_date("07/30/2026"), "2026-07-30");
        assert_eq!(iso_date("7/5/2026"), "2026-07-05");
        // Unrecognized text passes through verbatim rather than being lost.
        assert_eq!(iso_date("pending"), "pending");
        assert_eq!(iso_date("30-JUN-2026"), "30-JUN-2026");
    }

    #[test]
    fn summary_dto_maps_bbds() {
        let bbds = json!({
            "dueDate": "2026-07-30T00:00:00",
            "balance": {"balanceDue": 45.33, "pastDueBalance": 0},
            "autopay": {"status": "ACTIVE", "date": "2026-07-30T00:00:00"}
        });
        let v = serde_json::to_value(summary_dto(&bbds, Some("1234567890".into()))).unwrap();
        assert_eq!(v["schema"], "utility-summary/v1");
        assert_eq!(v["balance"]["amount"], "45.33");
        assert_eq!(v["balance"]["currency"], "USD");
        assert_eq!(v["due_date"], "2026-07-30");
        assert_eq!(v["account"], "1234567890");
        assert_eq!(v["autopay"], true);
    }

    #[test]
    fn summary_dto_omits_absent_fields() {
        let v = serde_json::to_value(summary_dto(&json!({}), None)).unwrap();
        assert_eq!(v["balance"]["amount"], "0.00");
        assert!(v.get("due_date").is_none());
        assert!(v.get("account").is_none());
        assert!(v.get("autopay").is_none());
    }

    #[test]
    fn summary_dto_autopay_tri_state() {
        let ap = |status: &str| summary_dto(&json!({"autopay": {"status": status}}), None).autopay;
        assert_eq!(ap("ON"), Some(true));
        assert_eq!(ap("enrolled"), Some(true));
        assert_eq!(ap("CANCELLED"), Some(false));
        assert_eq!(ap(""), None);
    }

    #[test]
    fn statement_values_tolerates_object_and_array() {
        let one = json!({"statementDetails": {"billStatus": "PAID"}});
        assert_eq!(statement_values(&one).len(), 1);
        let many = json!({"statementDetails": [{}, {}]});
        assert_eq!(statement_values(&many).len(), 2);
        assert!(statement_values(&json!({})).is_empty());
        assert!(statement_values(&json!({"statementDetails": null})).is_empty());
    }

    #[test]
    fn statement_dto_maps_the_new_experience_shape() {
        let raw = json!({
            "billStatus": "PAID",
            "lastStatementDate": "07/15/2026",
            "statementBalance": 45.33
        });
        let s = statement_dto(&raw, 1);
        // No provider statement id on this surface — the ISO date stands in.
        assert_eq!(s.id, "2026-07-15");
        assert_eq!(s.date.as_deref(), Some("2026-07-15"));
        assert_eq!(s.amount.amount, "45.33");
        assert_eq!(s.paid, Some(true));
        assert_eq!(s.due_date, None);
    }

    #[test]
    fn statement_dto_prefers_a_real_id_with_position_fallback() {
        let s = statement_dto(&json!({"statementId": "ABC123", "billStatus": "DUE"}), 3);
        assert_eq!(s.id, "ABC123");
        assert_eq!(s.paid, Some(false));
        let s = statement_dto(&json!({}), 3);
        assert_eq!(s.id, "3");
        assert_eq!(s.amount.amount, "0.00");
        assert_eq!(s.paid, None);
    }

    #[test]
    fn in_range_bounds() {
        assert!(in_range(Some("2026-07-15"), None, None));
        assert!(in_range(None, None, None));
        assert!(in_range(
            Some("2026-07-15"),
            Some("2026-07-01"),
            Some("2026-07-31")
        ));
        assert!(!in_range(Some("2026-06-30"), Some("2026-07-01"), None));
        assert!(!in_range(Some("2026-08-01"), None, Some("2026-07-31")));
        // A bound is set but the record has no comparable date → excluded.
        assert!(!in_range(None, Some("2026-07-01"), None));
        assert!(!in_range(Some("pending"), Some("2026-07-01"), None));
    }
}
