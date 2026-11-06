//! A tiny JSON writer. Output only; enough for schemas and reports.

use std::fmt::Write;

#[derive(Debug)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(&'static str, Json)>),
}

impl Json {
    pub fn str(s: impl Into<String>) -> Self {
        Json::Str(s.into())
    }

    pub fn num(n: impl Into<f64>) -> Self {
        Json::Num(n.into())
    }

    /// Pretty-print with two-space indentation.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            // JSON has no NaN or infinity.
            Json::Num(n) if !n.is_finite() => out.push_str("null"),
            Json::Num(n) => {
                let _ = write!(out, "{n}");
            }
            Json::Str(s) => write_str(out, s),
            Json::Arr(items) if items.is_empty() => out.push_str("[]"),
            Json::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    pad(out, indent + 1);
                    item.write(out, indent + 1);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push(']');
            }
            Json::Obj(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Obj(fields) => {
                out.push_str("{\n");
                for (i, (k, v)) in fields.iter().enumerate() {
                    pad(out, indent + 1);
                    write_str(out, k);
                    out.push_str(": ");
                    v.write(out, indent + 1);
                    out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push('}');
            }
        }
    }
}

fn pad(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_nests() {
        let j = Json::Obj(vec![
            ("a", Json::str("x\"y\n")),
            (
                "b",
                Json::Arr(vec![Json::num(1.5), Json::Null, Json::num(f64::NAN)]),
            ),
            ("c", Json::Obj(vec![])),
        ]);
        assert_eq!(
            j.render(),
            "{\n  \"a\": \"x\\\"y\\n\",\n  \"b\": [\n    1.5,\n    null,\n    null\n  ],\n  \"c\": {}\n}"
        );
    }
}
