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
    let len =
        content_length.ok_or_else(|| TransportError::Protocol("Missing Content-Length".into()))?;
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
        _ => Err(TransportError::Protocol(format!(
            "Unknown message type: {type_field}"
        ))),
    }
}

pub fn write_message<W: Write>(writer: &mut W, msg: &ClientMessage) -> Result<()> {
    let json = match msg {
        ClientMessage::Request(req) => serde_json::to_value(req)?,
        ClientMessage::Response(rsp) => serde_json::to_value(rsp)?,
    };
    let body = serde_json::to_string(&json)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()?;
    Ok(())
}
