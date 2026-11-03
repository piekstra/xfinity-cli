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

    fn subscription(&self) -> Result<Value, AppError> {
        self.read(&json!({"customerPlanInfo": {"internet": [self.net]}}))
    }
}

fn tools_over(portal: Fixtures) -> UtilityTools<XfinitySource<Fixtures>> {
    with_source(XfinitySource { portal }, false)
}

fn ok(t: &UtilityTools<XfinitySource<Fixtures>>, tool: &str, args: Value) -> Value {
    let result = call(t, tool, args);
    assert_fits(t, tool, &result).clone()
}

#[test]
fn the_definitions_conform_and_there_is_no_payments_tool_or_account_argument() {
    let portal = Fixtures::new();
    let reads = portal.reads.clone();
    let t = tools_over(portal);
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
    let t = tools_over(Fixtures::new());
    assert_eq!(
        ok(&t, "xfinity_balance", json!({})),
        json!({"balance": "45.33", "due_date": "2026-08-01", "autopay": true})
    );
}

#[test]
fn no_account_number_reaches_the_balance_in_either_format() {
    let t = tools_over(Fixtures::new());
    let out = ok(
        &t,
        "xfinity_balance",
        json!({"response_format": "detailed"}),
    );
    assert!(out.get("account_id").is_none(), "{out}");
    assert!(out.get("account").is_none(), "{out}");
}

#[test]
fn an_answer_xfinity_did_not_give_is_an_error_not_zero_or_empty() {
    for (tool, bbds) in [
        ("xfinity_balance", Value::Null),
        ("xfinity_balance", json!({"balance": {"renamed": 1}})),
        ("xfinity_bills", Value::Null),
        ("xfinity_bills", json!({"balance": {"balanceDue": 1}})),
        (
            "xfinity_bills",
            json!({"statementDetails": {"renamedAmount": 1}}),
        ),
    ] {
        let mut portal = Fixtures::new();
        portal.bbds = bbds;
        let result = call(&tools_over(portal), tool, json!({}));
        assert!(result.text().starts_with("upstream:"), "{}", result.text());
    }
    for net in [
        json!({"plan": "300Mbps"}),
        json!({"usageMonths": [{"renamedStart": "2026-07-01"}]}),
    ] {
        let mut portal = Fixtures::new();
        portal.net = net;
        let result = call(&tools_over(portal), "xfinity_usage", json!({}));
        assert!(result.text().starts_with("upstream:"), "{}", result.text());
    }
}

/// A portal answering the subscription with exactly this.
struct Subscription(Value);

impl Portal for Subscription {
    fn bbds(&self) -> Result<Value, AppError> {
        Ok(Value::Null)
    }

    fn subscription(&self) -> Result<Value, AppError> {
        Ok(self.0.clone())
    }
}

#[test]
fn no_internet_line_is_an_empty_list_and_an_answer_that_moved_is_an_error() {
    for answer in [
        json!({"customerPlanInfo": {"internet": null}}),
        json!({"customerPlanInfo": {"internet": []}}),
    ] {
        let t = with_source(
            XfinitySource {
                portal: Subscription(answer),
            },
            false,
        );
        let page = call(&t, "xfinity_usage", json!({}));
        let page = assert_fits(&t, "xfinity_usage", &page);
        assert_eq!(page["returned"], 0);
    }
    for answer in [
        Value::Null,
        json!({"renamedPlanInfo": {"internet": []}}),
        json!({"customerPlanInfo": {"renamed": []}}),
    ] {
        let t = with_source(
            XfinitySource {
                portal: Subscription(answer),
            },
            false,
        );
        let result = call(&t, "xfinity_usage", json!({}));
        assert!(result.text().starts_with("upstream:"), "{}", result.text());
    }
}

#[test]
fn no_statement_yet_is_an_empty_list() {
    let mut portal = Fixtures::new();
    portal.bbds = json!({"statementDetails": null});
    let t = tools_over(portal);
    assert_eq!(ok(&t, "xfinity_bills", json!({}))["returned"], 0);
}

#[test]
fn a_token_in_the_environment_points_at_the_variable_not_the_keychain() {
    let mut portal = Fixtures::new();
    portal.expired = true;
    let t = with_source(XfinitySource { portal }, true);
    let result = call(&t, "xfinity_balance", json!({}));
    assert!(
        result.text().contains("$XFINITY_TOKEN"),
        "{}",
        result.text()
    );
    assert!(
        !result.text().contains("xfin auth login"),
        "{}",
        result.text()
    );
}

#[test]
fn bills_are_the_one_statement_the_summary_carries() {
    let t = tools_over(Fixtures::new());
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
    let t = tools_over(Fixtures::new());
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
    let t = tools_over(portal);
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
    let t = tools_over(portal);
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

/// A keychain that counts its reads and holds one token for any account.
struct StubKeychain {
    token: Option<&'static str>,
    reads: Cell<u32>,
}

impl StubKeychain {
    fn holding(token: Option<&'static str>) -> Self {
        Self {
            token,
            reads: Cell::new(0),
        }
    }
}

impl KeychainRead for StubKeychain {
    fn service(&self) -> &str {
        "stub"
    }

    fn get(&self, _account: &str) -> Result<Option<Secret>, AppError> {
        self.reads.set(self.reads.get() + 1);
        Ok(self.token.map(Secret::new))
    }
}

// Each test names its own variable: the tests share one process's environment.

#[test]
fn the_environment_token_comes_before_the_keychain() {
    const VAR: &str = "XFIN_TEST_TOKEN_SET";
    std::env::set_var(VAR, "from-env");
    let keychain = StubKeychain::holding(Some("stored"));
    let token = resolve_token(&keychain, VAR, Some("user")).unwrap();
    assert_eq!(token.secret.expose(), "from-env");
    assert_eq!(token.source, SourceKind::Env);
    assert_eq!(keychain.reads.get(), 0);
}

#[test]
fn an_empty_environment_token_is_an_error_not_a_keychain_read() {
    const VAR: &str = "XFIN_TEST_TOKEN_EMPTY";
    std::env::set_var(VAR, "");
    let keychain = StubKeychain::holding(Some("stored"));
    let err = resolve_token(&keychain, VAR, Some("user")).unwrap_err();
    assert!(err.to_string().contains(VAR), "{err}");
    // A credential problem, which no tool argument can fix.
    assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    assert_eq!(keychain.reads.get(), 0);
}

#[test]
fn an_unset_environment_token_falls_to_the_keychain() {
    let keychain = StubKeychain::holding(Some("stored"));
    let token = resolve_token(&keychain, "XFIN_TEST_TOKEN_UNSET", Some("user")).unwrap();
    assert_eq!(token.secret.expose(), "stored");
    assert_eq!(token.source, SourceKind::Keychain);
}

#[test]
fn no_username_reads_no_keychain() {
    let keychain = StubKeychain::holding(Some("stored"));
    for username in [None, Some("")] {
        let err = resolve_token(&keychain, "XFIN_TEST_TOKEN_NONE", username).unwrap_err();
        assert!(matches!(err, AppError::Auth(_)), "{err:?}");
    }
    assert_eq!(keychain.reads.get(), 0);
}

#[test]
fn an_environment_token_needs_no_username() {
    const VAR: &str = "XFIN_TEST_TOKEN_ALONE";
    std::env::set_var(VAR, "from-env");
    let token = resolve_token(&StubKeychain::holding(None), VAR, None).unwrap();
    assert_eq!(token.secret.expose(), "from-env");
}

#[test]
fn nothing_stored_says_so() {
    let keychain = StubKeychain::holding(None);
    let err = resolve_token(&keychain, "XFIN_TEST_TOKEN_NOTHING", Some("user")).unwrap_err();
    assert!(err.to_string().contains("no stored token"), "{err}");
}
