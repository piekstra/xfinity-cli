//! `xfin billing` — balance/due summary, statement history (from
//! `billingSummary`), and statement PDF download.

use std::io::Write;
use std::path::{Path, PathBuf};

use pk_cli_core::output as core_output;
use pk_cli_utility::{Paged, RangeArgs, Statement};
use serde_json::{json, Value};

use crate::cli::{BillingCommand, DownloadArgs};
use crate::client::Xfinity;
use crate::commands::Ctx;
use crate::error::AppError;
use crate::output;
use crate::profile;

pub fn run(ctx: &Ctx, cmd: &BillingCommand) -> Result<(), AppError> {
    // Short-circuit the not-yet-mapped command before any network/auth.
    if let BillingCommand::Statement { .. } = cmd {
        return Err(AppError::Other(
            "`billing statement <id>` isn't available yet on the new Xfinity account \
             experience — use `billing statements`, or `billing download <id>` for the \
             PDF — see docs/api.md"
                .into(),
        ));
    }
    // Usage errors (malformed --since/--until, missing id/--all) also come
    // before any network so nothing prompts or touches the keychain first.
    match cmd {
        BillingCommand::Statements(range) => range.validate()?,
        BillingCommand::Download(args) => {
            args.range.validate()?;
            if !args.all && args.id.is_none() {
                return Err(AppError::Usage(
                    "give a statement id (see `xfin billing statements`) or --all".into(),
                ));
            }
            if args.all && args.output.as_deref() == Some("-") {
                return Err(AppError::Usage(
                    "--all can't stream to stdout; give a directory with -o, or omit it".into(),
                ));
            }
        }
        _ => {}
    }
    if let BillingCommand::Download(args) = cmd {
        return download(ctx, args);
    }
    let bbds = ctx.connect()?.bbds()?;
    match cmd {
        BillingCommand::Summary => output::billing_summary(&bbds),
        BillingCommand::DueDates => match bbds.get("dueDate").and_then(|v| v.as_str()) {
            Some(d) => println!("Due: {d}"),
            None => output::render(&bbds),
        },
        BillingCommand::Statements(range) => statements(ctx, &bbds, range),
        BillingCommand::Statement { .. } => unreachable!("handled above"),
        BillingCommand::Download(_) => unreachable!("handled above"),
    }
    Ok(())
}

/// `billing statements` — the utility/v1 `statement-list/v1` envelope with
/// `--json`; the provider-shaped text rendering otherwise (byte-identical to
/// the pre-profile output when no range flag is given).
fn statements(ctx: &Ctx, bbds: &Value, range: &RangeArgs) {
    let unfiltered = range.limit.is_none() && range.since.is_none() && range.until.is_none();
    if !ctx.cli.json && unfiltered {
        // The pre-profile text path, unchanged.
        output::render(bbds.get("statementDetails").unwrap_or(bbds));
        return;
    }
    let records = collect_statements(bbds, range);
    if ctx.cli.json {
        let items: Vec<Statement> = records.into_iter().map(|(_, dto)| dto).collect();
        Paged::new("statement", items).emit(true);
    } else if records.is_empty() {
        println!("(none)");
    } else {
        for (raw, _) in &records {
            output::render(raw);
        }
    }
}

/// Records that match `range`, each carried as (raw provider row, DTO).
fn collect_statements(bbds: &Value, range: &RangeArgs) -> Vec<(Value, Statement)> {
    let mut records: Vec<(Value, Statement)> = profile::statement_values(bbds)
        .into_iter()
        .enumerate()
        .map(|(i, raw)| {
            let dto = profile::statement_dto(&raw, i + 1);
            (raw, dto)
        })
        .collect();
    records.retain(|(_, s)| {
        profile::in_range(
            s.date.as_deref(),
            range.since.as_deref(),
            range.until.as_deref(),
        )
    });
    if let Some(n) = range.limit {
        records.truncate(n as usize);
    }
    records
}

// ---- `billing download` ---------------------------------------------------

fn download(ctx: &Ctx, args: &DownloadArgs) -> Result<(), AppError> {
    let x = ctx.connect()?;
    if args.all {
        download_all(ctx, &x, args)
    } else {
        let id = args
            .id
            .as_deref()
            .expect("caller guards for missing id when --all is unset");
        download_one(ctx, &x, id, args)
    }
}

fn download_one(ctx: &Ctx, x: &Xfinity, id: &str, args: &DownloadArgs) -> Result<(), AppError> {
    let bbds = x.bbds()?;
    let statement = resolve_statement(&bbds, id)?;
    let date = statement.date.as_deref().ok_or_else(|| {
        AppError::NotFound(format!(
            "statement {id} has no date — Xfinity keys statement downloads by date; \
             see `xfin billing statements`"
        ))
    })?;
    let bytes = x.download_statement(date)?;

    // `-o -` streams the PDF to stdout; diagnostics still go to stderr so a
    // pipe stays clean.
    if args.output.as_deref() == Some("-") {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| AppError::Other(format!("writing PDF to stdout: {e}")))?;
        if !ctx.cli.quiet {
            eprintln!(
                "wrote {} bytes (statement {}) to stdout",
                bytes.len(),
                statement.id
            );
        }
        return Ok(());
    }

    let file_name = default_file_name(&statement);
    let path = resolve_output(args.output.as_deref(), &file_name);
    std::fs::write(&path, &bytes)
        .map_err(|e| AppError::Other(format!("writing {}: {e}", path.display())))?;

    let dto = saved_dto(&statement, &path, bytes.len());
    if ctx.cli.json {
        core_output::json(&dto);
    } else {
        println!("{}", saved_line(&dto));
    }
    Ok(())
}

fn download_all(ctx: &Ctx, x: &Xfinity, args: &DownloadArgs) -> Result<(), AppError> {
    let bbds = x.bbds()?;
    let records = collect_statements(&bbds, &args.range);

    let dir = args
        .output
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_default();
    if !dir.as_os_str().is_empty() && !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Other(format!("creating {}: {e}", dir.display())))?;
    }

    // A batch must never lose the record of what it already wrote. Files land
    // on disk one at a time, so bailing out on the first failure would leave
    // real files behind with nothing on stdout naming them — the caller can't
    // tell a partial run from a total one, and re-downloads everything. So:
    // collect per-item outcomes, always emit the report, and let the exit code
    // carry the failure.
    let mut written = Vec::with_capacity(records.len());
    let mut failed: Vec<Value> = Vec::new();
    let mut bytes_total: u64 = 0;
    // An auth failure is not per-item: the token is dead, so every remaining
    // fetch would fail the same way. Stop early rather than hammer the API,
    // but still report what landed.
    let mut fatal: Option<AppError> = None;

    for (_raw, statement) in &records {
        let Some(date) = statement.date.as_deref() else {
            // A statement without a date can't be fetched (date is the handle).
            failed.push(failure_dto(&statement.id, "no date to key the download by"));
            if !ctx.cli.quiet {
                eprintln!(
                    "skipping statement {} — no date to key the download by",
                    statement.id
                );
            }
            continue;
        };
        let bytes = match x.download_statement(date) {
            Ok(b) => b,
            Err(e) => {
                let msg = e.to_string();
                failed.push(failure_dto(&statement.id, &msg));
                if !ctx.cli.quiet {
                    eprintln!("failed statement {}: {msg}", statement.id);
                }
                if matches!(e, AppError::Auth(_)) {
                    fatal = Some(e);
                    break;
                }
                continue;
            }
        };
        let file_name = default_file_name(statement);
        let path = if dir.as_os_str().is_empty() {
            PathBuf::from(&file_name)
        } else {
            dir.join(&file_name)
        };
        if let Err(e) = std::fs::write(&path, &bytes) {
            let msg = format!("writing {}: {e}", path.display());
            failed.push(failure_dto(&statement.id, &msg));
            if !ctx.cli.quiet {
                eprintln!("failed statement {}: {msg}", statement.id);
            }
            continue;
        }
        bytes_total += bytes.len() as u64;
        written.push(saved_dto(statement, &path, bytes.len()));
    }

    if written.is_empty() && failed.is_empty() && !ctx.cli.quiet {
        eprintln!("(no statements to download)");
    }
    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let payload = batch_dto(&where_to, &written, bytes_total, &failed);
    if ctx.cli.json {
        core_output::json(&payload);
    } else {
        for it in written.iter() {
            println!("{}", saved_line(it));
        }
        println!(
            "{} statement(s), {} bytes → {}",
            written.len(),
            bytes_total,
            where_to
        );
        if !failed.is_empty() {
            println!("{} failed", failed.len());
        }
    }

    // The report is out; now let the exit code tell the truth.
    if let Some(e) = fatal {
        return Err(e);
    }
    if !failed.is_empty() {
        return Err(AppError::Upstream(format!(
            "{} of {} statement(s) failed to download (see the report above); \
             the {} that succeeded are on disk and need no re-run",
            failed.len(),
            written.len() + failed.len(),
            written.len()
        )));
    }
    Ok(())
}

/// Look up a statement by id. Since the new experience surfaces at most one
/// statement (see `docs/api.md`), the id often IS the ISO date; tolerate both
/// the provider id and the date form so scripts don't have to know the
/// difference.
fn resolve_statement(bbds: &Value, id: &str) -> Result<Statement, AppError> {
    let dtos: Vec<Statement> = profile::statement_values(bbds)
        .into_iter()
        .enumerate()
        .map(|(i, raw)| profile::statement_dto(&raw, i + 1))
        .collect();
    dtos.into_iter()
        .find(|s| s.id == id || s.date.as_deref() == Some(id))
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "statement {id} not found — see `xfin billing statements`"
            ))
        })
}

/// Default file name for a saved statement: `xfin-statement-<date>.pdf`, or
/// the id when there's no date.
fn default_file_name(s: &Statement) -> String {
    let handle = s.date.as_deref().unwrap_or(&s.id);
    // Keep the handle filesystem-safe; the id/date is normally alphanumeric
    // (an ISO date or a provider id), but a stray '/' would be a path escape.
    let safe: String = handle
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("xfin-statement-{safe}.pdf")
}

/// A file path as given, a filename joined onto a directory, or the default
/// filename in the current directory when nothing was asked for.
fn resolve_output(output: Option<&str>, file_name: &str) -> PathBuf {
    match output {
        None => PathBuf::from(file_name),
        Some(o) => {
            let p = PathBuf::from(o);
            if p.is_dir() {
                p.join(file_name)
            } else {
                p
            }
        }
    }
}

/// One saved statement as `document-download/v1`.
///
/// This mirrors `SavedDocument` in cli-common's `documents/v1` profile
/// (DESIGN.md): `schema`, `id`, `name`, optional `category`/`date`/`file`,
/// `path`, `bytes`. Two details are load-bearing rather than stylistic:
///
/// - **No `amount`.** The spec is explicit that a document DTO carries no
///   financial fields — a statement's balance belongs to `utility/v1`
///   (`statement-list/v1`, which `billing statements` already emits), not to
///   the file that happens to contain it. Putting money here would give
///   consumers two disagreeing sources for the same number.
/// - **`name` is required**, so a caller listing a download directory has a
///   human title without re-deriving one from the filename.
///
/// It is hand-built rather than imported because `pk-cli-documents` is not in
/// any released cli-common tag yet (it lives on the unmerged
/// `documents-profile` branch); see the follow-up note in `docs/api.md`.
fn saved_dto(s: &Statement, path: &Path, bytes: usize) -> Value {
    json!({
        "schema": "document-download/v1",
        "id": s.id,
        "name": statement_name(s),
        "category": "statement",
        "date": s.date,
        "file": path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default(),
        "path": path.display().to_string(),
        "bytes": bytes,
    })
}

/// Human title for a statement — `document/v1` requires a `name`, and Xfinity
/// doesn't supply one, so derive it from the handle we do have.
fn statement_name(s: &Statement) -> String {
    match s.date.as_deref() {
        Some(d) => format!("Statement {d}"),
        None => format!("Statement {}", s.id),
    }
}

/// One statement the batch could not save.
fn failure_dto(id: &str, reason: &str) -> Value {
    json!({"id": id, "error": reason})
}

/// The batch envelope as `document-download-batch/v1` (cli-common
/// `DownloadBatch`: `schema`, `count`, `bytes_total`, `dir`, `items`).
///
/// `failed` is an xfin-local addition and is **omitted entirely when nothing
/// failed**, so a clean run is byte-identical to the family shape. The spec
/// has no representation for a partially-completed batch yet; dropping the
/// information instead would leave a caller unable to tell which statements
/// actually reached disk. Revisit when `pk-cli-documents` is released.
fn batch_dto(dir: &str, written: &[Value], bytes_total: u64, failed: &[Value]) -> Value {
    let mut payload = json!({
        "schema": "document-download-batch/v1",
        "count": written.len(),
        "bytes_total": bytes_total,
        "dir": dir,
        "items": written,
    });
    if !failed.is_empty() {
        payload["failed"] = json!(failed);
    }
    payload
}

fn saved_line(v: &Value) -> String {
    format!(
        "Saved statement {} → {} ({} bytes)",
        v.get("id").and_then(Value::as_str).unwrap_or("?"),
        v.get("path").and_then(Value::as_str).unwrap_or("?"),
        v.get("bytes").and_then(Value::as_u64).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_output_defaults_to_the_generated_filename() {
        assert_eq!(
            resolve_output(None, "xfin-statement-2026-07-15.pdf"),
            PathBuf::from("xfin-statement-2026-07-15.pdf")
        );
    }

    #[test]
    fn resolve_output_uses_an_explicit_path_verbatim() {
        assert_eq!(
            resolve_output(Some("/tmp/x.pdf"), "xfin-statement-2026-07-15.pdf"),
            PathBuf::from("/tmp/x.pdf")
        );
    }

    #[test]
    fn resolve_output_joins_the_filename_onto_a_directory() {
        // The current directory always exists, so it's a stable "is a dir" case.
        assert_eq!(
            resolve_output(Some("."), "xfin-statement-2026-07-15.pdf"),
            PathBuf::from("./xfin-statement-2026-07-15.pdf")
        );
    }

    #[test]
    fn default_file_name_uses_the_iso_date_when_present() {
        let s = Statement {
            id: "2026-07-15".into(),
            date: Some("2026-07-15".into()),
            amount: pk_cli_core::Money::usd("45.33"),
            due_date: None,
            paid: Some(true),
        };
        assert_eq!(default_file_name(&s), "xfin-statement-2026-07-15.pdf");
    }

    #[test]
    fn default_file_name_falls_back_to_the_id_and_sanitizes_slashes() {
        let s = Statement {
            id: "abc/def".into(),
            date: None,
            amount: pk_cli_core::Money::usd("0.00"),
            due_date: None,
            paid: None,
        };
        // A stray '/' would be a path escape; the sanitizer replaces it.
        assert_eq!(default_file_name(&s), "xfin-statement-abc-def.pdf");
    }

    #[test]
    fn resolve_statement_matches_by_id_or_date_and_misses_return_not_found() {
        let bbds = json!({
            "statementDetails": {
                "billStatus": "PAID",
                "lastStatementDate": "07/15/2026",
                "statementBalance": 45.33
            }
        });
        // The new-experience shape has no provider id — the ISO date is the id.
        let s = resolve_statement(&bbds, "2026-07-15").unwrap();
        assert_eq!(s.date.as_deref(), Some("2026-07-15"));
        assert_eq!(s.id, "2026-07-15");
        // A miss returns NotFound (exit code 4), not a crash.
        let err = resolve_statement(&bbds, "1999-01-01").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn saved_dto_carries_the_fields_the_download_dto_promises() {
        let s = Statement {
            id: "2026-07-15".into(),
            date: Some("2026-07-15".into()),
            amount: pk_cli_core::Money::usd("45.33"),
            due_date: None,
            paid: Some(true),
        };
        let v = saved_dto(&s, Path::new("/tmp/xfin-statement-2026-07-15.pdf"), 1234);
        assert_eq!(v["schema"], "document-download/v1");
        assert_eq!(v["id"], "2026-07-15");
        assert_eq!(v["name"], "Statement 2026-07-15");
        assert_eq!(v["category"], "statement");
        assert_eq!(v["date"], "2026-07-15");
        assert_eq!(v["bytes"], 1234);
        assert_eq!(v["file"], "xfin-statement-2026-07-15.pdf");
        assert_eq!(v["path"], "/tmp/xfin-statement-2026-07-15.pdf");
    }

    #[test]
    fn saved_dto_carries_no_financial_fields() {
        // cli-common DESIGN.md, documents/v1: a document DTO has "**no**
        // financial fields — a statement's amount belongs to utility/v1, not
        // the file". `billing statements` already publishes the money as
        // statement-list/v1; repeating it here would give consumers two
        // sources for one number, free to disagree.
        let s = Statement {
            id: "2026-07-15".into(),
            date: Some("2026-07-15".into()),
            amount: pk_cli_core::Money::usd("45.33"),
            due_date: None,
            paid: Some(true),
        };
        let v = saved_dto(&s, Path::new("/tmp/x.pdf"), 1);
        for banned in ["amount", "currency", "balance", "due_date", "paid"] {
            assert!(
                v.get(banned).is_none(),
                "document-download/v1 must not carry `{banned}`"
            );
        }
        assert!(
            !v.to_string().contains("45.33"),
            "the statement amount must not appear anywhere in the download DTO"
        );
    }

    #[test]
    fn saved_dto_field_set_matches_the_spec_exactly() {
        // Guards against drift in either direction: a field the family shape
        // doesn't have is as much a conformance break as a missing one.
        let s = Statement {
            id: "2026-07-15".into(),
            date: Some("2026-07-15".into()),
            amount: pk_cli_core::Money::usd("45.33"),
            due_date: None,
            paid: None,
        };
        let v = saved_dto(&s, Path::new("/tmp/x.pdf"), 1);
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["bytes", "category", "date", "file", "id", "name", "path", "schema"],
            "SavedDocument is schema/id/name/category?/date?/file?/path/bytes"
        );
    }

    #[test]
    fn statement_name_falls_back_to_the_id_without_a_date() {
        let s = Statement {
            id: "abc".into(),
            date: None,
            amount: pk_cli_core::Money::usd("0.00"),
            due_date: None,
            paid: None,
        };
        assert_eq!(statement_name(&s), "Statement abc");
    }

    // ---- partial-batch reporting

    #[test]
    fn batch_dto_omits_failed_entirely_on_a_clean_run() {
        // A clean batch must be byte-identical to the family shape — the
        // partial-failure extension may not leak into the happy path.
        let v = batch_dto(".", &[json!({"id": "a"})], 10, &[]);
        assert!(v.get("failed").is_none());
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["bytes_total", "count", "dir", "items", "schema"]);
        assert_eq!(v["schema"], "document-download-batch/v1");
        assert_eq!(v["count"], 1);
    }

    #[test]
    fn batch_dto_reports_what_succeeded_alongside_what_failed() {
        // The point of the partial report: a caller can see exactly which
        // statements are already on disk and must not re-download them.
        let written = vec![json!({"id": "2026-06-15", "path": "/tmp/a.pdf"})];
        let failed = vec![failure_dto("2026-07-15", "boom")];
        let v = batch_dto("/tmp", &written, 42, &failed);
        assert_eq!(v["count"], 1, "count covers what actually landed");
        assert_eq!(v["bytes_total"], 42);
        assert_eq!(v["items"][0]["id"], "2026-06-15");
        assert_eq!(v["failed"][0]["id"], "2026-07-15");
        assert_eq!(v["failed"][0]["error"], "boom");
    }

    #[test]
    fn failure_dto_names_the_statement_and_the_reason() {
        let v = failure_dto("2026-07-15", "no date to key the download by");
        assert_eq!(v["id"], "2026-07-15");
        assert_eq!(v["error"], "no date to key the download by");
    }
}
