use std::io::{BufRead, Read, Write};

#[derive(Debug)]
pub enum ServerMessage {
    Response(dap::responses::Response),
    Event(dap::events::Event),
    Request(dap::requests::Request),
}

#[derive(Debug)]
pub enum ClientMessage {
    Request(dap::requests::Request),
    Response(dap::responses::Response),
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, TransportError>;

pub fn read_message<R: Read>(reader: &mut R) -> Result<ServerMessage> {
    let mut buf_reader = std::io::BufReader::new(reader);
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = buf_reader.read_line(&mut line)?;
        if n == 0 {
            return Err(TransportError::Protocol("Connection closed".into()));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let len = content_length.ok_or_else(|| TransportError::Protocol("Missing Content-Length".into()))?;
    let mut buf = vec![0u8; len];
    buf_reader.read_exact(&mut buf)?;
    let val: serde_json::Value = serde_json::from_slice(&buf)?;

    let type_field = val["type"].as_str().unwrap_or("");
    match type_field {
        "request" => {
            let req: dap::requests::Request = serde_json::from_value(val)?;
            Ok(ServerMessage::Request(req))
        }
        "response" => {
            let rsp: dap::responses::Response = serde_json::from_value(val)?;
            Ok(ServerMessage::Response(rsp))
        }
        "event" => {
            let evt: dap::events::Event = serde_json::from_value(val)?;
            Ok(ServerMessage::Event(evt))
        }
        _ => Err(TransportError::Protocol(format!("Unknown message type: {type_field}"))),
    }
}

/// Drop every `null`-valued key, recursively.
///
/// The `dap` crate serializes an absent optional as an explicit `null`, but in
/// DAP "optional" means *omitted*, and a strict adapter reads `null` as the
/// wrong type rather than as nothing: lldb-dap rejects the whole `initialize`
/// with "expected bool at arguments.supportsMemoryReferences" over a
/// capability we never claimed to have an opinion about.
fn strip_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_nulls(v);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

pub fn write_message<W: Write>(writer: &mut W, msg: &ClientMessage) -> Result<()> {
    let (mut json, kind) = match msg {
        ClientMessage::Request(req) => (serde_json::to_value(req)?, "request"),
        ClientMessage::Response(rsp) => (serde_json::to_value(rsp)?, "response"),
    };
    // Every DAP message must carry a `type` discriminator — `read_message`
    // above dispatches on exactly that field — but the `dap` crate does not
    // serialize one. An adapter that validates it simply drops the frame:
    // lldb-dap answered our `initialize` with nothing at all, no error on any
    // stream, and the session sat in RUNNING forever with no thread to stop.
    strip_nulls(&mut json);
    if let Some(obj) = json.as_object_mut() {
        obj.insert("type".to_string(), serde_json::Value::from(kind));
    }
    let body = serde_json::to_string(&json)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(msg: &ClientMessage) -> serde_json::Value {
        let mut out = Vec::new();
        write_message(&mut out, msg).unwrap();
        let text = String::from_utf8(out).unwrap();
        let (header, body) = text.split_once("\r\n\r\n").expect("framed");
        assert_eq!(
            header,
            format!("Content-Length: {}", body.len()),
            "the length header must count the body's bytes"
        );
        serde_json::from_str(body).unwrap()
    }

    /// The `dap` crate leaves `type` off, and an adapter that validates it
    /// drops the frame in silence — no response, no error, a session that
    /// never starts. Every outgoing message has to carry it.
    #[test]
    fn every_outgoing_message_declares_its_type() {
        let req = dap::requests::Request {
            seq: 1,
            command: dap::requests::Command::ConfigurationDone,
        };
        assert_eq!(framed(&ClientMessage::Request(req))["type"], "request");

        let rsp = dap::responses::Response {
            request_seq: 1,
            success: true,
            message: None,
            body: None,
            error: None,
        };
        assert_eq!(framed(&ClientMessage::Response(rsp))["type"], "response");
    }

    /// An unset optional has to be absent, not `null`. lldb-dap failed
    /// `initialize` outright over a `null` capability the editor never set.
    #[test]
    fn unset_optionals_are_omitted_rather_than_sent_as_null() {
        let req = dap::requests::Request {
            seq: 1,
            command: dap::requests::Command::Initialize(dap::requests::InitializeArguments {
                client_id: Some("ruster".into()),
                lines_start_at1: Some(true),
                ..Default::default()
            }),
        };
        let json = framed(&ClientMessage::Request(req));
        let args = json["arguments"].as_object().expect("arguments object");
        assert!(
            args.values().all(|v| !v.is_null()),
            "no null may survive into the wire form: {args:?}"
        );
        // The fields that *were* set still arrive.
        assert_eq!(args["clientID"], "ruster");
        assert_eq!(args["linesStartAt1"], true);
        // And one we left unset is simply gone.
        assert!(!args.contains_key("supportsMemoryReferences"));
    }
}
