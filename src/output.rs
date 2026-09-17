use anyhow::Result;
use serde::Serialize;

pub fn emit_json<T: Serialize>(value: &T, pretty: bool) -> Result<()> {
    if pretty {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", serde_json::to_string(value)?);
    }
    Ok(())
}

pub fn emit_error_json(code: &str, message: &str, pretty: bool) -> Result<()> {
    emit_json(&error_json(code, message), pretty)
}

pub fn error_json(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({ "error": { "code": code, "message": message } })
}

pub fn emit_text(text: impl AsRef<str>) {
    let text = text.as_ref();
    if !text.is_empty() {
        println!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_has_stable_shape() {
        let value = error_json("not_found", "symbol not found: foo");
        assert_eq!(value["error"]["code"], "not_found");
        assert_eq!(value["error"]["message"], "symbol not found: foo");
    }
}
