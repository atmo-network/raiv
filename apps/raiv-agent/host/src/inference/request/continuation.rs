//! One native Run's bounded opaque reasoning carrier. No Debug/Clone/serialization/export.
use super::{
    check_chat_reply_mode, encode_inner, Definition, Limits, ProviderConfig, Request, RequestError,
    Value,
};
use crate::{inference::NormalizedReply, run::RequestTicket};
use raiv::{activation::ModelReply, format::DaslCid};
pub struct ReasoningSession {
    definition: DaslCid,
    limits: Limits,
    records: Vec<NormalizedReply>,
    bytes: usize,
    pending: Option<RequestTicket>,
    last: Option<RequestTicket>,
    failed: bool,
}
impl ReasoningSession {
    /// Requires a separately approved reasoning Definition; construction is not run authority.
    pub fn new(definition: &Definition, limits: Limits) -> Result<Self, RequestError> {
        let profile = guest::ChatProfile::decode(definition.inference_profile_bytes())
            .map_err(|_| RequestError::Binding)?;
        if profile.reasoning_effort.is_none() {
            return Err(RequestError::ReplyMode);
        }
        Ok(Self {
            definition: definition.cid(),
            limits,
            records: Vec::new(),
            bytes: 0,
            pending: None,
            last: None,
            failed: false,
        })
    }
    /// Call only with the originating authorized Run::pending pair. No network; repeated/stale/
    /// cross-run tickets or incomplete native/guest turn joins poison this carrier.
    pub fn encode(
        &mut self,
        definition: &Definition,
        ticket: RequestTicket,
        request: Request<'_>,
        config: &ProviderConfig,
    ) -> Result<Vec<u8>, RequestError> {
        let result = (|| {
            if self.failed
                || self.pending.is_some()
                || definition.cid() != self.definition
                || ticket.request_id() == 0
            {
                return Err(RequestError::Continuation);
            }
            if let Some(last) = self.last {
                if !ticket.same_run(last)
                    || last.request_id().checked_add(1) != Some(ticket.request_id())
                {
                    return Err(RequestError::Continuation);
                }
            }
            let bytes = encode_inner(
                definition,
                request,
                config,
                &self.limits,
                Some(&self.records),
            )?;
            self.pending = Some(ticket);
            Ok(bytes)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    /// Record only after this exact complete reply was successfully routed into the same VM.
    /// Failure requires discarding the live run/carrier, never another exposure or a fake boundary.
    /// Whole normalized reply is retained solely to check text/call/argument joins; reasoning never
    /// enters guest bytes. Both opaque frame+reasoning storage count against the native context cap.
    pub fn record(
        &mut self,
        definition: &Definition,
        ticket: RequestTicket,
        reply: NormalizedReply,
    ) -> Result<(), RequestError> {
        let result = (|| {
            if self.failed || self.pending != Some(ticket) || definition.cid() != self.definition {
                return Err(RequestError::Continuation);
            }
            check_chat_reply_mode(definition, &reply)?;
            let parsed = ModelReply::decode(reply.as_bytes(), &self.limits)
                .map_err(|_| RequestError::Continuation)?;
            // Terminal no-call replies belong to guest Finish, never a replay record.
            if parsed.call_count() == 0 {
                return Err(RequestError::Continuation);
            }
            if parsed.request_id() != ticket.request_id() {
                return Err(RequestError::Continuation);
            }
            if self.records.len() as u64 >= self.limits.max_model_calls_per_activation {
                return Err(RequestError::Limit);
            }
            let record_bytes = reply
                .as_bytes()
                .len()
                .checked_add(reply.reasoning_content().map_or(0, str::len))
                .ok_or(RequestError::Limit)?;
            if record_bytes as u64 > self.limits.max_model_reply_bytes {
                return Err(RequestError::Limit);
            }
            let bytes = self
                .bytes
                .checked_add(record_bytes)
                .ok_or(RequestError::Limit)?;
            if bytes as u64 > self.limits.max_context_bytes {
                return Err(RequestError::Limit);
            }
            self.records
                .try_reserve(1)
                .map_err(|_| RequestError::Allocation)?;
            self.records.push(reply);
            self.bytes = bytes;
            self.pending = None;
            self.last = Some(ticket);
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}
pub(super) fn attach(
    records: &[NormalizedReply],
    messages: &mut [Value],
    limits: &Limits,
) -> Result<(), RequestError> {
    if messages.iter().filter(|m| m["role"] == "assistant").count() != records.len() {
        return Err(RequestError::Continuation);
    }
    for (message, record) in messages
        .iter_mut()
        .filter(|m| m["role"] == "assistant")
        .zip(records)
    {
        let parsed = ModelReply::decode(record.as_bytes(), limits)
            .map_err(|_| RequestError::Continuation)?;
        let content = message["content"].as_str().unwrap_or("");
        if content != parsed.text() {
            return Err(RequestError::Continuation);
        }
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if calls.len() as u64 != parsed.call_count() {
            return Err(RequestError::Continuation);
        }
        let args = match parsed.control_call() {
            Ok(Some(call)) => super::value(call.arguments_bytes(), &limits.decode_limits())?,
            _ => Value::Null,
        };
        for (wire, call) in calls.iter().zip(parsed.calls()) {
            if wire["id"] != call.call_id || wire["function"]["name"] != call.tool_id {
                return Err(RequestError::Continuation);
            }
            let wire_args = wire["function"]["arguments"]
                .as_str()
                .ok_or(RequestError::Continuation)?;
            let wire_args: Value =
                serde_json::from_str(wire_args).map_err(|_| RequestError::Continuation)?;
            if wire_args != args {
                return Err(RequestError::Continuation);
            }
        }
        let reason = record.reasoning_content().ok_or(RequestError::ReplyMode)?;
        let mut text = String::new();
        text.try_reserve_exact(reason.len())
            .map_err(|_| RequestError::Allocation)?;
        text.push_str(reason);
        message["reasoning_content"] = Value::String(text);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
