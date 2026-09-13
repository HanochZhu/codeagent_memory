use anyhow::Result;
use serde::Serialize;

pub fn emit_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn emit_text(text: impl AsRef<str>) {
    let text = text.as_ref();
    if !text.is_empty() {
        println!("{text}");
    }
}
