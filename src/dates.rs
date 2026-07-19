//! Date helpers, re-exported from the CLI family's `pk-cli-core`. Retained for
//! upcoming date formatting (e.g. `MM/DD/YYYY` payment dates) as payment
//! commands are remapped to the new account experience; not referenced yet.
#[allow(unused_imports)]
pub use pk_cli_core::dates::{fmt_mm_slash_dd_yyyy as fmt_mm_dd_yyyy, today};
