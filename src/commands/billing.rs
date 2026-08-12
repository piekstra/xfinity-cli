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
        let mut env = dto.clone();
        env["schema"] = Value::String("document-download/v1".into());
        core_output::json(&env);
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

    let mut written = Vec::with_capacity(records.len());
    let mut bytes_total: u64 = 0;
    for (_raw, statement) in &records {
        let Some(date) = statement.date.as_deref() else {
            // A statement without a date can't be fetched (date is the handle).
            // Skip rather than crash — the batch keeps going.
            if !ctx.cli.quiet {
                eprintln!(
                    "skipping statement {} — no date to key the download by",
                    statement.id
                );
            }
            continue;
        };
        let bytes = x.download_statement(date)?;
        let file_name = default_file_name(statement);
        let path = if dir.as_os_str().is_empty() {
            PathBuf::from(&file_name)
        } else {
            dir.join(&file_name)
        };
        std::fs::write(&path, &bytes)
            .map_err(|e| AppError::Other(format!("writing {}: {e}", path.display())))?;
        bytes_total += bytes.len() as u64;
        written.push(saved_dto(statement, &path, bytes.len()));
    }

    if written.is_empty() && !ctx.cli.quiet {
        eprintln!("(no statements to download)");
    }
    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let payload = json!({
        "schema": "document-download-batch/v1",
        "count": written.len(),
        "bytes_total": bytes_total,
        "dir": where_to,
        "items": written,
    });
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

fn saved_dto(s: &Statement, path: &Path, bytes: usize) -> Value {
    json!({
        "id": s.id,
        "date": s.date,
        "amount": {"amount": s.amount.amount, "currency": s.amount.currency},
        "file": path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default(),
        "path": path.display().to_string(),
        "bytes": bytes,
    })
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
        assert_eq!(v["id"], "2026-07-15");
        assert_eq!(v["date"], "2026-07-15");
        assert_eq!(v["amount"]["amount"], "45.33");
        assert_eq!(v["amount"]["currency"], "USD");
        assert_eq!(v["bytes"], 1234);
        assert_eq!(v["file"], "xfin-statement-2026-07-15.pdf");
    }
}
