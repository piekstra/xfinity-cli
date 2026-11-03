//! The `xfin mcp` tools, offline: `tests/fixtures/` behind the [`Portal`] seam,
//! mapped by the commands' own mappers, checked by cli-common's conformance
//! helper. `tests/mcp_stdio.rs` and `tests/mcp_http.rs` drive the real binary.

use std::cell::Cell;
use std::rc::Rc;

use pk_cli_mcp::conformance::{assert_conforms, assert_fits, call, definitions_size};
use pk_cli_mcp::Toolset;
use serde_json::json;

use super::*;

struct Fixtures {
    bbds: Value,
    net: Value,
    reads: Rc<Cell<u32>>,
    expired: bool,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            bbds: serde_json::from_str(include_str!("../../tests/fixtures/bbds.json")).unwrap(),
            net: serde_json::from_str(include_str!("../../tests/fixtures/internet_plan.json"))
                .unwrap(),
            reads: Rc::default(),
            expired: false,
        }
    }

    fn read(&self, value: &Value) -> Result<Value, AppError> {
        self.reads.set(self.reads.get() + 1);
        if self.expired {
            return Err(without_capture_steps(AppError::Auth(
                "Xfinity returned 401 for BillingInfo/getBBDS — the stored token is expired or \
                 invalid. Capture a fresh `Authorization: Bearer …` in your browser and re-run \
                 `xfin auth login --overwrite`."
                    .into(),
            )));
        }
        Ok(value.clone())
    }
}

impl Portal for Fixtures {
    fn bbds(&self) -> Result<Value, AppError> {
        self.read(&self.bbds)
    }

    fn internet_plan(&self) -> Result<Value, AppError> {
        self.read(&self.net)
    }
}

fn tools_over(portal: Fixtures, account: Option<&str>) -> UtilityTools<XfinitySource<Fixtures>> {
    with_source(XfinitySource {
        portal,
        account: account.map(str::to_string),
    })
}

fn ok(t: &UtilityTools<XfinitySource<Fixtures>>, tool: &str, args: Value) -> Value {
    let result = call(t, tool, args);
    assert_fits(t, tool, &result).clone()
}

#[test]
fn the_definitions_conform_and_there_is_no_payments_tool_or_account_argument() {
    let portal = Fixtures::new();
    let reads = portal.reads.clone();
    let t = tools_over(portal, None);
    assert_conforms(&t);
    assert!(definitions_size(&t) < 8_000);
    let tools = t.tools();
    let names: Vec<&str> = tools.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["xfinity_balance", "xfinity_bills", "xfinity_usage"]);
    for tool in &tools {
        assert!(
            !tool.input_properties().contains(&"account"),
            "{}",
            tool.name
        );
    }
    assert_eq!(reads.get(), 0);
}

#[test]
fn balance_maps_the_billing_summary() {
    let t = tools_over(Fixtures::new(), None);
    assert_eq!(
        ok(&t, "xfinity_balance", json!({})),
        json!({"balance": "45.33", "due_date": "2026-08-01", "autopay": true})
    );
}

#[test]
fn the_configured_account_shows_only_its_end_unless_detailed() {
    let t = tools_over(Fixtures::new(), Some("1234567890"));
    let out = ok(&t, "xfinity_balance", json!({}));
    assert_eq!(out["account"], "ending 7890");
    assert!(out.get("account_id").is_none(), "{out}");
    let out = ok(
        &t,
        "xfinity_balance",
        json!({"response_format": "detailed"}),
    );
    assert_eq!(out["account_id"], "1234567890");
}

#[test]
fn bills_are_the_one_statement_the_summary_carries() {
    let t = tools_over(Fixtures::new(), None);
    let page = ok(&t, "xfinity_bills", json!({}));
    assert_eq!(
        page["items"],
        json!([{"date": "2026-07-15", "amount": "45.33", "paid": false}])
    );
    let page = ok(&t, "xfinity_bills", json!({"until": "2026-01-01"}));
    assert_eq!(page["returned"], 0);
}

#[test]
fn usage_comes_newest_first_in_the_stated_unit() {
    let t = tools_over(Fixtures::new(), None);
    let page = ok(&t, "xfinity_usage", json!({}));
    assert_eq!(
        page["items"],
        json!([
            {"period": "2026-07-01", "quantity": 222.5, "unit": "GB"},
            {"period": "2026-06-01", "quantity": 111.0, "unit": "GB"},
            {"period": "2026-05-01", "quantity": 333.0, "unit": "TB"}
        ])
    );
    let page = ok(&t, "xfinity_usage", json!({"since": "2026-06-15"}));
    assert_eq!(page["returned"], 1);
}

#[test]
fn a_lapsed_token_asks_for_the_one_sign_in_command() {
    let mut portal = Fixtures::new();
    portal.expired = true;
    let t = tools_over(portal, None);
    let result = call(&t, "xfinity_balance", json!({}));
    let text = result.text();
    assert!(text.starts_with("auth:"), "{text}");
    assert!(
        text.contains("the stored token is expired or invalid"),
        "{text}"
    );
    assert!(text.contains(&format!("`{SIGN_IN}`")), "{text}");
    assert!(!text.contains("Capture a fresh"), "{text}");
    assert!(!text.contains("`xfin auth login --overwrite`"), "{text}");
}

#[test]
fn a_bad_argument_reaches_no_portal() {
    let portal = Fixtures::new();
    let reads = portal.reads.clone();
    let t = tools_over(portal, None);
    for (tool, args) in [
        ("xfinity_bills", json!({"since": "yesterday"})),
        ("xfinity_usage", json!({"limit": 0})),
        ("xfinity_usage", json!({"account": "1234567890"})),
        ("xfinity_balance", json!({"nope": 1})),
    ] {
        let result = call(&t, tool, args);
        assert!(result.text().starts_with("usage:"), "{}", result.text());
    }
    assert_eq!(reads.get(), 0);
}

#[test]
fn the_environment_token_comes_before_the_keychain_and_empty_counts_as_unset() {
    let never = || -> Result<Secret, AppError> { panic!("the keychain was read") };
    assert_eq!(
        token_from(Some("from-env".into()), never).unwrap().expose(),
        "from-env"
    );
    let stored = token_from(Some(String::new()), || Ok(Secret::new("stored"))).unwrap();
    assert_eq!(stored.expose(), "stored");
}

#[test]
fn no_username_is_an_auth_error_before_any_keychain_read() {
    let err = read_keychain(None).unwrap_err();
    assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    let err = read_keychain(Some("")).unwrap_err();
    assert!(matches!(err, AppError::Auth(_)), "{err:?}");
}
