//! OpenAI-format chat JSONL import parser.
//!
//! Parses a user-uploaded JSONL file where each line is an OpenAI chat sample
//! (`{"messages": [...]}`, optionally with a top-level `tools` array) into the
//! platform's internal dataset record shape (`{"messages": [...], "metadata":
//! {...}}`). The parser is pure — no I/O — so it is fully unit tested; the
//! service layer handles buffering, storage, and the dataset row.
//!
//! Tool-calling is preserved on purpose. `role: "tool"` messages and assistant
//! `tool_calls` are validated and carried through verbatim into the stored
//! record, even though the current training path only consumes system/user/
//! assistant text turns. This is deliberate schema groundwork for an
//! agent/tool-calling training track — the import must not silently strip
//! these fields as "unsupported".

use serde_json::{Map, Value, json};

/// The roles accepted in an imported chat sample.
const ALLOWED_ROLES: [&str; 4] = ["system", "user", "assistant", "tool"];

/// Cap on how many per-row errors are returned in the response. The full
/// rejected-row count is always reported; only the detailed list is truncated
/// so a pathological file cannot produce an unbounded response body.
pub const MAX_REPORTED_ERRORS: usize = 100;

/// A single malformed row, reported back to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    /// 1-based line number within the uploaded file.
    pub line: usize,
    /// Human-readable reason the row was rejected.
    pub error: String,
}

/// Outcome of parsing an uploaded JSONL file.
#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    /// Successfully normalized records, in the internal dataset shape.
    pub records: Vec<Value>,
    /// Per-row errors for rejected rows (see [`MAX_REPORTED_ERRORS`]).
    pub errors: Vec<RowError>,
    /// Total non-empty rows seen (accepted + rejected).
    pub total_rows: usize,
    /// Total rejected rows, even if [`Self::errors`] was truncated.
    pub rejected_rows: usize,
    /// Accepted records carrying tool-calling data (a top-level `tools` array
    /// or any message with `tool_calls`).
    pub tool_records: usize,
}

/// Parse an OpenAI-format chat JSONL document.
///
/// Blank lines are skipped and never counted. Every other line is parsed
/// independently: a malformed row becomes a [`RowError`] instead of failing the
/// whole file, and a valid row is normalized into the internal record shape.
pub fn parse_openai_jsonl(content: &str) -> ImportResult {
    let mut result = ImportResult::default();
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        result.total_rows += 1;
        match normalize_row(line) {
            Ok(record) => {
                if record_has_tool_data(&record) {
                    result.tool_records += 1;
                }
                result.records.push(record);
            }
            Err(reason) => {
                result.rejected_rows += 1;
                if result.errors.len() < MAX_REPORTED_ERRORS {
                    result.errors.push(RowError {
                        line: idx + 1,
                        error: reason,
                    });
                }
            }
        }
    }
    result
}

/// Whether a normalized record carries tool-calling data: a top-level `tools`
/// array (only inserted when present in the source row) or any message with
/// `tool_calls`.
fn record_has_tool_data(record: &Value) -> bool {
    if record.get("tools").is_some() {
        return true;
    }
    record
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|msgs| msgs.iter().any(|m| m.get("tool_calls").is_some()))
}

/// Validate and normalize one JSONL line into an internal record. Preserves the
/// original (validated) message objects verbatim, so tool-calling fields
/// survive, and carries a top-level `tools` array through when present.
fn normalize_row(line: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = value.as_object().ok_or("row is not a JSON object")?;

    let messages = obj
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("missing or non-array 'messages'")?;
    if messages.is_empty() {
        return Err("'messages' is empty".to_string());
    }

    let mut has_user = false;
    let mut has_assistant = false;
    let mut normalized = Vec::with_capacity(messages.len());
    for (i, message) in messages.iter().enumerate() {
        let role = validate_message(message, i)?;
        match role {
            "user" => has_user = true,
            "assistant" => has_assistant = true,
            _ => {}
        }
        // Preserve the original message verbatim (tool_calls, tool_call_id,
        // name, ...) now that its structure is validated.
        normalized.push(message.clone());
    }
    if !has_user {
        return Err("no 'user' message — not a trainable exchange".to_string());
    }
    if !has_assistant {
        return Err("no 'assistant' message — not a trainable exchange".to_string());
    }

    let mut out = Map::new();
    out.insert("messages".to_string(), Value::Array(normalized));
    // Carry tool/function definitions through untouched when present.
    if let Some(tools) = obj.get("tools") {
        if !tools.is_array() {
            return Err("'tools' must be an array".to_string());
        }
        out.insert("tools".to_string(), tools.clone());
    }
    out.insert("metadata".to_string(), json!({"source": "openai_import"}));
    Ok(Value::Object(out))
}

/// Validate a single message object; returns its role on success.
fn validate_message(message: &Value, idx: usize) -> Result<&'static str, String> {
    let obj = message
        .as_object()
        .ok_or_else(|| format!("message {idx} is not an object"))?;

    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("message {idx} missing string 'role'"))?;

    let role = ALLOWED_ROLES
        .into_iter()
        .find(|r| *r == role)
        .ok_or_else(|| format!("message {idx} has unsupported role '{role}'"))?;

    match role {
        "system" | "user" => {
            require_nonempty_content(obj, idx, role)?;
        }
        "assistant" => {
            // An assistant turn is valid with text content OR with tool_calls
            // (a pure tool-call turn carries null/absent content).
            let has_tool_calls = obj
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty());
            let has_text = obj
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty());
            if !has_text && !has_tool_calls {
                return Err(format!(
                    "message {idx} (assistant) must have non-empty content or tool_calls"
                ));
            }
            if let Some(tool_calls) = obj.get("tool_calls")
                && !tool_calls.is_array()
            {
                return Err(format!(
                    "message {idx} (assistant) 'tool_calls' must be an array"
                ));
            }
        }
        "tool" => {
            require_nonempty_content(obj, idx, role)?;
            obj.get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    format!("message {idx} (tool) must have a non-empty 'tool_call_id'")
                })?;
        }
        _ => unreachable!("role already validated against ALLOWED_ROLES"),
    }
    Ok(role)
}

fn require_nonempty_content(
    obj: &Map<String, Value>,
    idx: usize,
    role: &str,
) -> Result<(), String> {
    obj.get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|_| ())
        .ok_or_else(|| format!("message {idx} ({role}) must have non-empty string content"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_simple_valid_sample() {
        let content = r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#;
        let r = parse_openai_jsonl(content);
        assert_eq!(r.total_rows, 1);
        assert_eq!(r.rejected_rows, 0);
        assert_eq!(r.records.len(), 1);
        let rec = &r.records[0];
        assert_eq!(rec["messages"].as_array().unwrap().len(), 2);
        assert_eq!(rec["metadata"]["source"], "openai_import");
    }

    #[test]
    fn blank_lines_are_skipped_and_not_counted() {
        let content = "\n\n{\"messages\":[{\"role\":\"user\",\"content\":\"a\"},{\"role\":\"assistant\",\"content\":\"b\"}]}\n\n";
        let r = parse_openai_jsonl(content);
        assert_eq!(r.total_rows, 1);
        assert_eq!(r.records.len(), 1);
    }

    #[test]
    fn preserves_system_prompt_turn() {
        let content = r#"{"messages":[{"role":"system","content":"You are X"},{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#;
        let r = parse_openai_jsonl(content);
        let msgs = r.records[0]["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "system");
    }

    #[test]
    fn preserves_assistant_tool_calls_and_tool_turn() {
        // Assistant makes a tool call (null content), then a tool result turn,
        // then a final assistant answer. Tool fields must survive verbatim.
        let content = r#"{"tools":[{"type":"function","function":{"name":"get_weather"}}],"messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_1","content":"sunny"},{"role":"assistant","content":"It is sunny."}]}"#;
        let r = parse_openai_jsonl(content);
        assert_eq!(r.rejected_rows, 0, "errors: {:?}", r.errors);
        assert_eq!(r.tool_records, 1);
        let rec = &r.records[0];
        // Top-level tools preserved.
        assert!(rec["tools"].is_array());
        let msgs = rec["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        // Assistant tool_calls preserved.
        assert_eq!(msgs[1]["tool_calls"][0]["id"], "call_1");
        // Tool turn preserved with its tool_call_id.
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn counts_tool_records() {
        // One record with top-level tools only, one with tool_calls only,
        // one plain — tool_records must count the first two.
        let with_tools = r#"{"tools":[{"type":"function","function":{"name":"f"}}],"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#;
        let with_tool_calls = r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b","tool_calls":[{"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#;
        let plain =
            r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#;
        let r = parse_openai_jsonl(&format!("{with_tools}\n{with_tool_calls}\n{plain}"));
        assert_eq!(r.records.len(), 3);
        assert_eq!(r.tool_records, 2);
    }

    #[test]
    fn plain_records_count_no_tool_records() {
        let content = r#"{"messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#;
        assert_eq!(parse_openai_jsonl(content).tool_records, 0);
    }

    #[test]
    fn malformed_json_is_a_row_error_not_a_file_failure() {
        let good =
            r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"}]}"#;
        let content = format!("{good}\n{{not json\n{good}");
        let r = parse_openai_jsonl(&content);
        assert_eq!(r.total_rows, 3);
        assert_eq!(r.records.len(), 2);
        assert_eq!(r.rejected_rows, 1);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 2);
        assert!(r.errors[0].error.contains("invalid JSON"));
    }

    #[test]
    fn rejects_row_without_messages() {
        let r = parse_openai_jsonl(r#"{"prompt":"x","completion":"y"}"#);
        assert_eq!(r.records.len(), 0);
        assert!(r.errors[0].error.contains("messages"));
    }

    #[test]
    fn rejects_json_array_line() {
        let r = parse_openai_jsonl(r#"[{"role":"user","content":"a"}]"#);
        assert!(r.errors[0].error.contains("not a JSON object"));
    }

    #[test]
    fn rejects_unsupported_role() {
        let r = parse_openai_jsonl(
            r#"{"messages":[{"role":"user","content":"a"},{"role":"robot","content":"b"}]}"#,
        );
        assert!(r.errors[0].error.contains("unsupported role"));
    }

    #[test]
    fn rejects_empty_user_content() {
        let r = parse_openai_jsonl(
            r#"{"messages":[{"role":"user","content":"  "},{"role":"assistant","content":"b"}]}"#,
        );
        assert!(r.errors[0].error.contains("non-empty string content"));
    }

    #[test]
    fn rejects_exchange_without_user_or_assistant() {
        let only_user = parse_openai_jsonl(r#"{"messages":[{"role":"user","content":"a"}]}"#);
        assert!(only_user.errors[0].error.contains("no 'assistant'"));

        let only_assistant =
            parse_openai_jsonl(r#"{"messages":[{"role":"assistant","content":"a"}]}"#);
        assert!(only_assistant.errors[0].error.contains("no 'user'"));
    }

    #[test]
    fn tool_turn_requires_tool_call_id() {
        let r = parse_openai_jsonl(
            r#"{"messages":[{"role":"user","content":"a"},{"role":"assistant","content":"b"},{"role":"tool","content":"result"}]}"#,
        );
        assert!(r.errors[0].error.contains("tool_call_id"));
    }

    #[test]
    fn error_list_is_capped_but_count_is_not() {
        // Build more malformed rows than the reported-errors cap.
        let bad_rows = "not json\n".repeat(MAX_REPORTED_ERRORS + 50);
        let r = parse_openai_jsonl(&bad_rows);
        assert_eq!(r.rejected_rows, MAX_REPORTED_ERRORS + 50);
        assert_eq!(r.errors.len(), MAX_REPORTED_ERRORS);
    }
}
