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
    #[derive(Serialize)]
    struct ErrorBody<'a> {
        code: &'a str,
        message: &'a str,
    }
    #[derive(Serialize)]
    struct ErrorEnvelope<'a> {
        error: ErrorBody<'a>,
    }
    let envelope = ErrorEnvelope {
        error: ErrorBody { code, message },
    };
    emit_json(&envelope, pretty)
}

pub fn emit_text(text: impl AsRef<str>) {
    let text = text.as_ref();
    if !text.is_empty() {
        println!("{text}");
    }
}
