use std::io::{BufReader, BufWriter, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::config::AdapterConfig;
use crate::transport::{self, ClientMessage, ServerMessage};

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] transport::TransportError),
    #[error("Channel disconnected")]
    ChannelClosed,
}

pub struct DapClient {
    tx: Sender<ClientMessage>,
    rx: Receiver<ServerMessage>,
    child: Option<std::process::Child>,
}

impl DapClient {
    pub fn spawn(config: &AdapterConfig, root: &std::path::Path) -> Result<(Self, JoinHandle<()>)> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        let (req_tx, req_rx): (Sender<ClientMessage>, _) = mpsc::channel();
        let (msg_tx, msg_rx): (Sender<ServerMessage>, _) = mpsc::channel();

        let _writer_h = thread::spawn(move || {
            for msg in req_rx {
                let mut buf = BufWriter::new(&mut stdin);
                if let Err(e) = transport::write_message(&mut buf, &msg) {
                    eprintln!("dap write error: {e}");
                    break;
                }
                buf.flush().ok();
            }
        });

        let msg_tx_clone = msg_tx.clone();
        let reader_h = thread::spawn(move || {
            loop {
                match transport::read_message(&mut reader) {
                    Ok(msg) => {
                        if msg_tx_clone.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("dap read error: {e}");
                        break;
                    }
                }
            }
        });

        let client = DapClient {
            tx: req_tx,
            rx: msg_rx,
            child: Some(child),
        };

        Ok((client, reader_h))
    }

    pub fn send_request(&self, req: dap::requests::Request) -> Result<()> {
        self.tx.send(ClientMessage::Request(req)).map_err(|_| ClientError::ChannelClosed)
    }

    pub fn send_response(&self, rsp: dap::responses::Response) -> Result<()> {
        self.tx.send(ClientMessage::Response(rsp)).map_err(|_| ClientError::ChannelClosed)
    }

    pub fn poll(&self) -> Option<ServerMessage> {
        self.rx.try_recv().ok()
    }

    pub fn shutdown(mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            child.kill().ok();
            child.wait().ok();
        }
        Ok(())
    }
}
