use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tabby_core::config::SshOptions;
use tabby_core::session::{KnownHostsPolicy, SessionEvent, spawn_ssh};
use tokio::net::TcpListener;

const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCa1NtEz1pgpDol3zUC+zOJiqYMuHDG61T++K4/luigvwAAAJiShaiQkoWo
kAAAAAtzc2gtZWQyNTUxOQAAACCa1NtEz1pgpDol3zUC+zOJiqYMuHDG61T++K4/luigvw
AAAEAmPYxfauJ/OkxEFOz/3tgxabhUck9VrFLO1r47fZ3LXprU20TPWmCkOiXfNQL7M4mK
pgy4cMbrVP74rj+W6KC/AAAADmJhbGR3aW5AdWJ1bnR1AQIDBAUGBw==
-----END OPENSSH PRIVATE KEY-----
";

#[derive(Clone)]
struct TestServer;

impl russh::server::Server for TestServer {
    type Handler = Self;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        Self
    }
}

impl russh::server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        session.data(channel, b"ssh-ready\r\n".as_slice())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut echo = b"echo:".to_vec();
        echo.extend_from_slice(data);
        session.data(channel, echo)?;
        Ok(())
    }
}

#[test]
fn ssh_password_auth_opens_a_shell_and_moves_bytes() {
    let (address_tx, address_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();
            address_tx.send(addr).unwrap();
            let key = russh::keys::PrivateKey::from_openssh(HOST_KEY).unwrap();
            let config = Arc::new(russh::server::Config {
                inactivity_timeout: Some(Duration::from_secs(30)),
                auth_rejection_time: Duration::from_millis(10),
                auth_rejection_time_initial: Some(Duration::from_millis(0)),
                keys: vec![key],
                ..Default::default()
            });
            let mut server = TestServer;
            let _ = server.run_on_socket(config, &listener).await;
        });
    });
    let addr = address_rx.recv_timeout(Duration::from_secs(10)).unwrap();

    let options = SshOptions {
        host: "127.0.0.1".into(),
        port: addr.port(),
        user: "demo".into(),
        auth: Some("password".into()),
        password: Some("secret".into()),
        ..Default::default()
    };
    let session = spawn_ssh(
        options,
        Some("secret".into()),
        40,
        10,
        KnownHostsPolicy::AcceptAll,
    )
    .expect("spawn ssh session");
    let events = session.events();
    let mut output = Vec::new();
    let mut wrote = false;
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(SessionEvent::Data(data)) => {
                output.extend_from_slice(&data);
                let text = String::from_utf8_lossy(&output);
                if !wrote && text.contains("ssh-ready") {
                    session.write(b"ping\r\n");
                    wrote = true;
                }
                if wrote && text.contains("echo:ping") {
                    break;
                }
            }
            Ok(SessionEvent::Error(message)) => panic!("ssh error: {message}"),
            Ok(SessionEvent::Exited(_)) | Ok(SessionEvent::Closed) => {
                if wrote {
                    break;
                }
            }
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(async_channel::TryRecvError::Closed) => break,
        }
    }

    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("ssh-ready"), "missing shell banner: {text:?}");
    assert!(text.contains("echo:ping"), "missing echo: {text:?}");
    session.close();
}
