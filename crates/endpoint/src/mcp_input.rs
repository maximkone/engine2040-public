//! MCP31: the door's `input_required` ask (SEP-2322, multi round-trip).
//!
//! A verb that cannot finish without one client fact answers an
//! [`InputRequiredResult`] instead of a refusal: one request per missing fact,
//! an opaque `requestState`, and the same call retried with `inputResponses`
//! lands it. The first ask is `asset.intake` on a model whose licence is
//! unknown.
//!
//! The ask travels from the verb as an ordinary [`McpToolError`] whose code is
//! [`CODE`] and whose data is the wire result; [`into_response`] turns it back
//! into the typed result at the door, so a verb never touches rmcp's result
//! enum and a caller that cannot take an ask still reads a refusal record.

use rmcp::model::{
    CallToolResponse, ElicitRequest, ElicitRequestParams, ElicitationSchema, InputRequest,
    InputRequests, InputRequiredResult, InputResponses,
};
use rmcp::serde_json::Value;

use super::McpToolError;

/// Refusal code that carries an ask instead of a refusal.
pub const CODE: &str = "input_required";

/// The one request the intake ask carries.
pub const LICENCE_REQUEST_ID: &str = "licence";

/// The field the client answers with.
pub const LICENCE_FIELD: &str = "licence";

/// Opaque state echoed back on the retry. The door keeps no server-side round
/// state: the retry carries the same args plus the answer, so the state only
/// names what the ask was for.
pub const REQUEST_STATE: &str = "asset.intake:licence";

/// One fact the door asks for: the request id and the prompt a client shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskRequest {
    /// Request id, the key the answer comes back under.
    pub id: String,
    /// One sentence naming the fact and its legal shape.
    pub prompt: String,
}

impl AskRequest {
    /// One ask: an id and the prompt that names the fact.
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            prompt: prompt.into(),
        }
    }

    /// The typed elicitation request this ask becomes, or `None` when the
    /// primitive schema is refused (it cannot be: one required string).
    fn to_request(&self) -> Option<InputRequest> {
        let schema = ElicitationSchema::builder()
            .required_string(self.id.clone())
            .build()
            .ok()?;
        let params = ElicitRequestParams::FormElicitationParams {
            meta: None,
            message: self.prompt.clone(),
            requested_schema: schema,
        };
        Some(InputRequest::Elicitation(ElicitRequest::new(params)))
    }
}

/// The typed `input_required` result for one or more asks.
pub fn ask_result(asks: &[AskRequest]) -> Option<InputRequiredResult> {
    let mut requests = InputRequests::new();
    for ask in asks {
        requests.insert(ask.id.clone(), ask.to_request()?);
    }
    (!requests.is_empty())
        .then(|| InputRequiredResult::new(Some(requests), Some(REQUEST_STATE.to_owned())))
}

/// The licence ask: the door cannot cook a model whose licence nobody knows.
pub fn licence_ask(model_id: &str) -> McpToolError {
    let ask = AskRequest::new(
        LICENCE_REQUEST_ID,
        format!(
            "`{model_id}` has no known SPDX licence; answer with the exact identifier its \
             weights ship under (for example MIT, Apache-2.0 or CC-BY-4.0)"
        ),
    );
    let data = ask_result(&[ask])
        .and_then(|result| rmcp::serde_json::to_value(result).ok())
        .unwrap_or(Value::Null);
    // The literal, not [`CODE`], so the refusal scan in `mcp_refusal` sees the
    // code and its kind table cannot drift from what this module emits.
    McpToolError::new(
        "input_required",
        format!(
            "`{model_id}` has no known SPDX licence; answer the `{LICENCE_REQUEST_ID}` request \
             and retry this call with inputResponses"
        ),
    )
    .with_data(data)
}

/// The result shape of an ask, when this refusal is one. A refusal that is not
/// an ask answers `None`, so the door keeps its refusal path untouched.
pub fn into_response(error: &McpToolError) -> Option<CallToolResponse> {
    if error.code != CODE {
        return None;
    }
    let data = error.data()?.clone();
    let result: InputRequiredResult = rmcp::serde_json::from_value(data).ok()?;
    Some(result.into())
}

/// The `requestState` an ask hands the client, when this refusal is one: the
/// ledger keys the call's entry by it so the retry can join that entry.
pub fn asked_state(error: &McpToolError) -> Option<&str> {
    if error.code != CODE {
        return None;
    }
    error.data()?.get("requestState")?.as_str()
}

/// True when the licence text names no real licence: empty, `unknown`, `n/a`,
/// or not SPDX-shaped. A decided licence is never unknown here, even a
/// non-commercial one: that stays the domain owner's refusal.
pub fn licence_is_unknown(licence: &str) -> bool {
    let text = licence.trim();
    if text.is_empty()
        || text.eq_ignore_ascii_case("unknown")
        || text.eq_ignore_ascii_case("n/a")
        || text.eq_ignore_ascii_case("none")
    {
        return true;
    }
    !text.split_whitespace().all(|token| {
        token.eq_ignore_ascii_case("AND")
            || token.eq_ignore_ascii_case("OR")
            || token.eq_ignore_ascii_case("WITH")
            || token
                .chars()
                .all(|lane| lane.is_ascii_alphanumeric() || matches!(lane, '.' | '-' | '+' | ':'))
    })
}

/// The licence a retry answered with, when it accepted the ask and named one.
pub fn answered_licence(answers: &InputResponses) -> Option<String> {
    let answer = answers.get(LICENCE_REQUEST_ID)?;
    if answer.get("action").and_then(Value::as_str) != Some("accept") {
        return None;
    }
    let text = answer
        .get("content")?
        .get(LICENCE_FIELD)?
        .as_str()?
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
#[path = "mcp_input/tests.rs"]
mod tests;
