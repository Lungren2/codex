/// Bound accumulated foreground output below the context item's 10K-token cap,
/// leaving room for status headers and terminal metadata.
pub(crate) fn output_budget(requested: Option<usize>) -> usize {
    requested.unwrap_or(8_000).min(8_000)
}
