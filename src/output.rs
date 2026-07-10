//! Output rendering.
//!
//! **Text is the primary format.** Resource reads render token-dense
//! `Key: value` blocks and pipe-delimited tables (`ALL_CAPS` headers). JSON is
//! reserved for control-plane signals — `auth login` / `set-credential`
//! results, `auth status`, `self-update`, and the raw `api` payload — never
//! bolted onto resource reads. Data goes to stdout; diagnostics and
//! confirmations go to stderr.
//!
//! Many Xfinity response shapes aren't pinned down yet, so [`render`] flattens
//! whatever JSON comes back into readable text. As shapes are confirmed, add a
//! purpose-built renderer next to the generic one. For the raw structure, use
//! `xfin api`.

use serde_json::Value;

/// Pretty JSON on stdout. Control-plane only.
pub fn json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

/// Default text renderer for a resource read. Unwraps a `{data: …}` envelope,
/// then renders an object as a key/value block or an array as a pipe-delimited
/// table.
pub fn render(v: &Value) {
    let body = v.get("data").unwrap_or(v);
    match body {
        Value::Array(arr) => render_table(arr),
        Value::Object(_) => render_kv(body, 0),
        Value::Null => println!("(no data)"),
        other => println!("{}", scalar(other)),
    }
}

fn render_kv(obj: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    if let Some(map) = obj.as_object() {
        for (k, val) in map {
            match val {
                Value::Object(_) => {
                    println!("{pad}{k}:");
                    render_kv(val, indent + 2);
                }
                Value::Array(arr) if arr.iter().all(|x| !x.is_object() && !x.is_array()) => {
                    let joined = arr.iter().map(scalar).collect::<Vec<_>>().join(", ");
                    println!("{pad}{k}: {joined}");
                }
                Value::Array(arr) => {
                    println!("{pad}{k}: [{} items]", arr.len());
                    render_table(arr);
                }
                other => println!("{pad}{k}: {}", scalar(other)),
            }
        }
    }
}

/// Render an array of objects as a pipe-delimited table with `ALL_CAPS`
/// headers. Falls back to one value per line for arrays of scalars.
fn render_table(arr: &[Value]) {
    if arr.is_empty() {
        println!("(none)");
        return;
    }
    if arr.iter().all(|x| !x.is_object()) {
        for x in arr {
            println!("{}", scalar(x));
        }
        return;
    }
    let mut cols: Vec<String> = Vec::new();
    for row in arr {
        if let Some(map) = row.as_object() {
            for k in map.keys() {
                if !cols.iter().any(|c| c == k) {
                    cols.push(k.clone());
                }
            }
        }
    }
    println!(
        "{}",
        cols.iter()
            .map(|c| c.to_uppercase())
            .collect::<Vec<_>>()
            .join(" | ")
    );
    for row in arr {
        let cells: Vec<String> = cols
            .iter()
            .map(|c| row.get(c).map(scalar).unwrap_or_default())
            .collect();
        println!("{}", cells.join(" | "));
    }
}

/// Billing summary: a concise Balance / Due / Autopay block, else flatten.
pub fn billing_summary(v: &Value) {
    let d = v.get("data").unwrap_or(v);
    let first = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .filter_map(|k| d.pointer(&format!("/{k}")).or_else(|| d.get(*k)))
            .find(|x| !x.is_null())
            .map(scalar)
    };

    let mut printed = false;
    if let Some(bal) = first(&["balanceDue", "currentBalance", "balance", "amountDue"]) {
        println!("Balance:  {bal}");
        printed = true;
    }
    if let Some(due) = first(&["dueDate", "paymentDueDate", "billDueDate"]) {
        if !due.is_empty() {
            println!("Due:      {due}");
            printed = true;
        }
    }
    if let Some(pd) = first(&["pastDueAmount", "pastDue"]) {
        if !matches!(pd.as_str(), "" | "0" | "0.0" | "0.00" | "$0.00") {
            println!("Past due: {pd}");
            printed = true;
        }
    }
    if let Some(ap) = first(&["autoPayEnabled", "autopay", "isAutoPayEnrolled"]) {
        println!("Autopay:  {ap}");
        printed = true;
    }
    if !printed {
        render(v);
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalar_unwraps_strings() {
        assert_eq!(scalar(&json!("hi")), "hi");
        assert_eq!(scalar(&json!(3)), "3");
        assert_eq!(scalar(&Value::Null), "");
    }
}
