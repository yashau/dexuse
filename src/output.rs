use crate::model::Summary;
use anyhow::Result;

pub fn print_json(summary: &Summary) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(summary)?);
    Ok(())
}

pub fn compact_tokens(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}
