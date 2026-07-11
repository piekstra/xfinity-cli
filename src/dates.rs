//! Date helpers, shared with the CLI family via `pk-cli-core`. Xfinity's
//! payment endpoints take `MM/DD/YYYY`.

pub use pk_cli_core::dates::{fmt_mm_slash_dd_yyyy as fmt_mm_dd_yyyy, today};
