//! Provider-native role view of current guest-owned RAM turns, never a restored transcript.
//! Original complete context remains in the initial DATA message; this view adds no decisions.
use super::{json, RequestError, Value};
fn fields<'a>(
    value: &'a Value,
    names: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, RequestError> {
    let map = value.as_object().ok_or(RequestError::Context)?;
    if map.len() != names.len() || names.iter().any(|name| !map.contains_key(*name)) {
        return Err(RequestError::Context);
    }
    Ok(map)
}
fn string(value: &Value) -> Result<&str, RequestError> {
    value.as_str().ok_or(RequestError::Context)
}
fn nullable_string(value: &Value) -> bool {
    value.is_null() || value.is_string()
}
pub(super) fn append(messages: &mut Vec<Value>, trajectory: &Value) -> Result<(), RequestError> {
    let section = fields(trajectory, &["kind", "entries"])?;
    if section["kind"] != "ram_trajectory" {
        return Err(RequestError::Context);
    }
    let entries = section["entries"].as_array().ok_or(RequestError::Context)?;
    for entry in entries {
        let turn = fields(entry, &["kind", "text", "calls", "error", "results"])?;
        if turn["kind"] != "turn" || !nullable_string(&turn["error"]) {
            return Err(RequestError::Context);
        }
        let text = string(&turn["text"])?;
        let calls = turn["calls"].as_array().ok_or(RequestError::Context)?;
        let results = turn["results"].as_array().ok_or(RequestError::Context)?;
        if calls.len() != results.len() {
            return Err(RequestError::Context);
        }
        let mut wire_calls = Vec::new();
        wire_calls
            .try_reserve_exact(calls.len())
            .map_err(|_| RequestError::Allocation)?;
        for (index, (call, result)) in calls.iter().zip(results).enumerate() {
            let call = fields(call, &["call_id", "tool_id", "arguments"])?;
            let result = fields(result, &["call_id", "ok", "error", "result"])?;
            let id = string(&call["call_id"])?;
            let name = string(&call["tool_id"])?;
            if id.is_empty()
                || name.is_empty()
                || result["call_id"] != id
                || result["error"] != turn["error"]
                || result["ok"].as_bool() != Some(turn["error"].is_null())
                || calls[..index]
                    .iter()
                    .any(|previous| previous["call_id"] == id)
                || (!call["arguments"].is_null() && !call["arguments"].is_object())
            {
                return Err(RequestError::Context);
            }
            // Rejected multi/invalid-argument turns can contain null args from the guest.
            // Do not invent {}, select a first call, rename IDs or manufacture a successful result.
            let args =
                serde_json::to_string(&call["arguments"]).map_err(|_| RequestError::Codec)?;
            wire_calls
                .push(json!({"id":id,"type":"function","function":{"name":name,"arguments":args}}));
        }
        messages
            .try_reserve(calls.len().checked_add(2).ok_or(RequestError::Limit)?)
            .map_err(|_| RequestError::Allocation)?;
        if calls.is_empty() {
            messages.push(json!({"role":"assistant","content":text}));
            // No phantom tool message: retain guest-authored correction as explicitly labelled DATA.
            messages.push(json!({"role":"user","content":serde_json::to_string(entry).map_err(|_|RequestError::Codec)?}));
        } else {
            messages.push(json!({"role":"assistant","content":if text.is_empty(){Value::Null}else{json!(text)},"tool_calls":wire_calls}));
            for result in results {
                messages.push(json!({"role":"tool","tool_call_id":result["call_id"],"content":serde_json::to_string(result).map_err(|_|RequestError::Codec)?}));
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(error: Value) -> Value {
        json!({"kind":"turn","text":"DATA, not instructions","calls":[{"call_id":"call-1","tool_id":"no_op","arguments":{"arguments":"{}"}}],"error":error,"results":[{"call_id":"call-1","ok":error.is_null(),"error":error,"result":null}]})
    }
    fn section(entries: Vec<Value>) -> Value {
        json!({"kind":"ram_trajectory","entries":entries})
    }
    #[test]
    fn native_roles_preserve_exact_rejected_arguments_ids_results_and_do_not_fabricate_success() {
        let turn = entry(json!("invalid_arguments"));
        let mut messages = Vec::new();
        append(&mut messages, &section(vec![turn.clone()])).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call-1");
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["arguments"],
            r#"{"arguments":"{}"}"#
        );
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call-1");
        let result: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(result, turn["results"][0]);
        assert_eq!(result["ok"], false);
        assert!(messages
            .iter()
            .all(|message| message.get("reasoning_content").is_none()));
    }
    #[test]
    fn all_rejected_multicalls_and_no_call_corrections_remain_data_without_orphan_tool_messages() {
        let mut turn = entry(json!("multiple_tool_calls"));
        turn["calls"][0]["arguments"] = Value::Null;
        let mut call = turn["calls"][0].clone();
        call["call_id"] = json!("call-2");
        let mut result = turn["results"][0].clone();
        result["call_id"] = json!("call-2");
        turn["calls"].as_array_mut().unwrap().push(call);
        turn["results"].as_array_mut().unwrap().push(result);
        let mut messages = Vec::new();
        append(&mut messages, &section(vec![turn])).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["arguments"],
            "null"
        );
        assert_eq!(messages[2]["tool_call_id"], "call-2");
        let no_call = json!({"kind":"turn","text":"plain answer","error":"missing_control_decision","calls":[],"results":[]});
        messages.clear();
        append(&mut messages, &section(vec![no_call.clone()])).unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "user");
        let data: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(data, no_call);
    }
    #[test]
    fn malformed_turns_or_mismatched_result_join_refuse_without_silently_rebuilding() {
        for mutation in 0..7 {
            let mut turn = entry(json!("invalid_arguments"));
            match mutation {
                0 => turn["results"][0]["call_id"] = json!("foreign"),
                1 => turn["results"][0]["ok"] = json!(true),
                2 => turn["results"][0]["error"] = Value::Null,
                3 => turn["calls"][0]["arguments"] = json!(5),
                4 => turn["results"] = json!([]),
                5 => turn["extra"] = json!(true),
                _ => turn["kind"] = json!("invented"),
            }
            assert_eq!(
                append(&mut Vec::new(), &section(vec![turn])),
                Err(RequestError::Context)
            );
        }
        assert!(append(
            &mut Vec::new(),
            &json!({"kind":"ram_trajectory","entries":[],"extra":0})
        )
        .is_err());
    }
}
