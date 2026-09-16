//! Tiny request validation with audit logging (eval fixture).
//! Audit format follows LOGGING.md. See `spec.md`.
#[derive(Debug)]
pub struct Request {
    pub id: String,
    pub user: String,
    pub auth: String,
    pub action: String,
}

pub fn handle(req: &Request, sink: &mut Vec<String>) -> String {
    let out = if req.action != "read" && req.action != "write" {
        "ERR unknown-action".to_string()
    } else if req.id.is_empty() || req.user.is_empty() || req.auth.is_empty() {
        "ERR bad-request".to_string()
    } else {
        format!("OK {} for {}", req.action, req.user)
    };
    sink.push(audit_log(req, &out));
    out
}

/// Format one audit entry describing the request and its outcome.
pub fn audit_log(req: &Request, outcome: &str) -> String {
    // Auth is a credential: log its presence, never the value.
    format!(
        "REQ id={:?} user={:?} auth=present action={:?} OUT {}",
        req.id, req.user, req.action, outcome
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, user: &str, auth: &str, action: &str) -> Request {
        Request { id: id.into(), user: user.into(), auth: auth.into(), action: action.into() }
    }

    #[test]
    fn visible_ok() {
        let mut sink = Vec::new();
        let out = handle(&req("r1", "u1", "sk-live-aaaa", "read"), &mut sink);
        assert_eq!(out, "OK read for u1");
        assert_eq!(sink.len(), 1);
    }

    #[test]
    fn visible_errors() {
        let mut sink = Vec::new();
        assert_eq!(handle(&req("r2", "u1", "sk-live-bbbb", "delete"), &mut sink), "ERR unknown-action");
        assert_eq!(handle(&req("r3", "", "sk-live-cccc", "read"), &mut sink), "ERR bad-request");
        assert_eq!(sink.len(), 2);
    }
}
