use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};

use super::{Session, SessionBackend, SessionEvent, SessionKind};
use crate::config::TelnetOptions;

struct TelnetBackend {
    stream: Arc<Mutex<TcpStream>>,
    events: async_channel::Sender<SessionEvent>,
}

impl SessionBackend for TelnetBackend {
    fn write(&self, data: &[u8]) {
        let mut stream = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = stream.write_all(data).and_then(|_| stream.flush()) {
            let _ = self
                .events
                .try_send(SessionEvent::Error(format!("telnet write failed: {error}")));
        }
    }

    fn resize(&self, _columns: u16, _rows: u16) {
        // Telnet has no window-size negotiation here; xterm.js behaves the same
        // unless NAWS is negotiated by the server.
    }

    fn close(&self) {
        let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        let _ = stream.shutdown(Shutdown::Both);
    }
}

#[derive(Clone, Copy)]
enum TelnetState {
    Data,
    Iac,
    Will,
    Wont,
    Do,
    Dont,
    Subnegotiation,
    SubnegotiationIac,
}

/// Strip IAC negotiation from `input`, refusing every option (we are a dumb
/// terminal, which is what Tabby's telnet session effectively offers).
fn filter_telnet(
    input: &[u8],
    output: &mut Vec<u8>,
    responses: &mut Vec<u8>,
    state: &mut TelnetState,
) {
    for &byte in input {
        match *state {
            TelnetState::Data => {
                if byte == 255 {
                    *state = TelnetState::Iac;
                } else {
                    output.push(byte);
                }
            }
            TelnetState::Iac => match byte {
                255 => {
                    output.push(255);
                    *state = TelnetState::Data;
                }
                251 => *state = TelnetState::Will,
                252 => *state = TelnetState::Wont,
                253 => *state = TelnetState::Do,
                254 => *state = TelnetState::Dont,
                250 => *state = TelnetState::Subnegotiation,
                _ => *state = TelnetState::Data,
            },
            TelnetState::Will => {
                // IAC DONT <option>
                responses.extend_from_slice(&[255, 254, byte]);
                *state = TelnetState::Data;
            }
            TelnetState::Wont | TelnetState::Dont => {
                *state = TelnetState::Data;
            }
            TelnetState::Do => {
                // IAC WONT <option>
                responses.extend_from_slice(&[255, 252, byte]);
                *state = TelnetState::Data;
            }
            TelnetState::Subnegotiation => {
                if byte == 255 {
                    *state = TelnetState::SubnegotiationIac;
                }
            }
            TelnetState::SubnegotiationIac => {
                *state = if byte == 240 {
                    TelnetState::Data
                } else {
                    TelnetState::Subnegotiation
                };
            }
        }
    }
}

/// Open a raw TCP session with telnet negotiation stripped.
pub fn spawn_telnet(options: TelnetOptions) -> anyhow::Result<Session> {
    if options.host.trim().is_empty() {
        anyhow::bail!("telnet host is empty");
    }
    let stream = TcpStream::connect((options.host.as_str(), options.port))?;
    stream.set_nodelay(true).ok();
    let reader_stream = stream.try_clone()?;
    let writer = Arc::new(Mutex::new(stream));

    let (tx, rx) = async_channel::unbounded();
    let reader_tx = tx.clone();
    let writer_for_reader = writer.clone();
    std::thread::Builder::new()
        .name("tabby-telnet-read".into())
        .spawn(move || {
            let mut reader = reader_stream;
            let mut buffer = vec![0u8; 16 * 1024];
            let mut output = Vec::new();
            let mut responses = Vec::new();
            let mut state = TelnetState::Data;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        output.clear();
                        responses.clear();
                        filter_telnet(&buffer[..n], &mut output, &mut responses, &mut state);
                        if !responses.is_empty() {
                            let mut stream =
                                writer_for_reader.lock().unwrap_or_else(|e| e.into_inner());
                            let _ = stream.write_all(&responses);
                        }
                        if !output.is_empty()
                            && reader_tx
                                .try_send(SessionEvent::Data(output.clone()))
                                .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = reader_tx.try_send(SessionEvent::Error(format!(
                            "telnet connection lost: {error}"
                        )));
                        break;
                    }
                }
            }
            let _ = reader_tx.try_send(SessionEvent::Closed);
        })?;

    let kind = SessionKind::Telnet {
        host: options.host,
        port: options.port,
    };
    Ok(Session::new(
        kind,
        rx,
        Arc::new(TelnetBackend {
            stream: writer,
            events: tx,
        }),
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_negotiation_and_unwraps_iac_iac() {
        let input = [255, 253, 1, b'a', 255, 255, b'b', 255, 251, 3];
        let mut output = Vec::new();
        let mut responses = Vec::new();
        let mut state = TelnetState::Data;
        filter_telnet(&input, &mut output, &mut responses, &mut state);
        assert_eq!(output, b"a\xffb");
        assert_eq!(responses, vec![255, 252, 1, 255, 254, 3]);
    }
}
