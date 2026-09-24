//! The response to a request (RFC 8620 §3.4): one answer per call, found by call id.

use super::MethodError;
use super::field::malformed;
use crate::ProtoError;
use serde_json::Value;

/// Every answer in one response, in the order the server sent them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Responses {
    answers: Vec<(String, Value, String)>,
    /// The session's state as of this response. A value unlike the one the session was fetched
    /// with means the session changed and should be fetched again.
    pub session_state: Option<String>,
}

impl Responses {
    /// Parse a response body.
    pub fn parse(bytes: &[u8]) -> Result<Responses, ProtoError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| malformed(format!("the response is not JSON: {e}")))?;
        let calls = value
            .get("methodResponses")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("the response has no methodResponses"))?;
        let answers = calls
            .iter()
            .map(|call| match call.as_array().map(Vec::as_slice) {
                Some([Value::String(name), args, Value::String(id)]) => {
                    Ok((name.clone(), args.clone(), id.clone()))
                }
                _ => Err(malformed("a method response is not [name, arguments, id]")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Responses {
            answers,
            session_state: value
                .get("sessionState")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// The answer to the call with this id, named `name`.
    ///
    /// A call can produce more than one response — `EmailSubmission/set` with
    /// `onSuccessUpdateEmail` adds an implicit `Email/set` under the same id — so the name is
    /// part of the question. An `error` answer to the id is returned as the error it is.
    pub fn answer(&self, id: &str, name: &str) -> Result<&Value, MethodError> {
        let mine = self.answers.iter().filter(|(_, _, i)| i == id);
        let mut error = None;
        for (answered, args, _) in mine {
            if answered == name {
                return Ok(args);
            }
            if answered == "error" {
                error = Some(MethodError::from_args(args));
            }
        }
        Err(error.unwrap_or_else(|| MethodError::Missing(format!("no {name} answer to {id}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_answer_is_the_error_it_names() {
        let body = br#"{"methodResponses":[
            ["error",{"type":"cannotCalculateChanges"},"c"],
            ["Mailbox/get",{"state":"1","list":[]},"m"]
        ],"sessionState":"s1"}"#;
        let responses = Responses::parse(body).unwrap();
        assert_eq!(
            responses.answer("c", "Email/changes"),
            Err(MethodError::CannotCalculateChanges)
        );
        assert!(responses.answer("m", "Mailbox/get").is_ok());
        assert!(matches!(
            responses.answer("x", "Email/get"),
            Err(MethodError::Missing(_))
        ));
        assert_eq!(responses.session_state.as_deref(), Some("s1"));
    }

    #[test]
    fn a_body_of_the_wrong_shape_is_malformed_not_a_panic() {
        for body in [
            &b"not json"[..],
            br#"{"methodResponses":7}"#,
            br#"{"methodResponses":[["Email/get",{}]]}"#,
        ] {
            assert!(matches!(
                Responses::parse(body),
                Err(ProtoError::Malformed(_))
            ));
        }
    }
}
