use std::time::{Duration, Instant};

use tabby_core::config::LocalOptions;
use tabby_core::session::{SessionEvent, spawn_local};

#[test]
fn local_pty_runs_a_command_and_reports_output() {
    let options = LocalOptions {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf 'hello-from-pty'".into()],
        ..Default::default()
    };
    let session = spawn_local(options, 40, 10).expect("spawn local pty");
    let events = session.events();
    let mut output = Vec::new();
    let mut exited_at: Option<Instant> = None;
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        if Instant::now() >= deadline {
            break;
        }
        if let Some(exited_at) = exited_at {
            if exited_at.elapsed() > Duration::from_millis(300) {
                break;
            }
        }
        match events.try_recv() {
            Ok(SessionEvent::Data(data)) => output.extend_from_slice(&data),
            Ok(SessionEvent::Exited(code)) => {
                assert_eq!(code, Some(0));
                exited_at = Some(Instant::now());
            }
            Ok(SessionEvent::Error(message)) => panic!("session error: {message}"),
            Ok(SessionEvent::Closed) => {}
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(async_channel::TryRecvError::Closed) => break,
        }
    }

    let output = String::from_utf8_lossy(&output);
    assert!(
        output.contains("hello-from-pty"),
        "unexpected PTY output: {output:?}"
    );
    assert!(exited_at.is_some(), "child exit was not reported");
}
