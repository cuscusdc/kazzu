//! Optional desktop Discord Rich Presence over the local RPC socket.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, Sender};
use serde_json::{Value, json};

type Activity = Option<(String, String)>;

pub(crate) struct DiscordPresence {
    current: Arc<Mutex<Activity>>,
    notify: Sender<()>,
}

impl DiscordPresence {
    pub(crate) fn from_env() -> Option<Self> {
        // Supplying a real Discord application ID explicitly enables sharing
        // song and artist names with the locally running Discord client.
        let id = std::env::var("KAZZU_DISCORD_APPLICATION_ID").ok()?;
        if !(16..=20).contains(&id.len()) || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            log::warn!("ID de aplicativo Discord inválido");
            return None;
        }

        let current = Arc::new(Mutex::new(None));
        let state = current.clone();
        let (notify, changed) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("kazzu-discord".into())
            .spawn(move || {
                let mut socket = None;
                let mut next_connect = Instant::now();
                let mut published = None;
                let mut nonce = 0u64;
                loop {
                    match changed.recv_timeout(Duration::from_secs(1)) {
                        Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            if let Some(mut socket) = socket {
                                let _ = publish(&mut socket, None, nonce);
                            }
                            break;
                        }
                    }
                    let activity = state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .clone();
                    let mut force = false;
                    if socket.is_none() && activity.is_some() && Instant::now() >= next_connect {
                        socket = connect(&id).ok();
                        next_connect = Instant::now() + Duration::from_secs(10);
                        force = socket.is_some();
                    }
                    if let Some(connection) = socket.as_mut() {
                        let result = if force || activity != published {
                            nonce = nonce.wrapping_add(1);
                            publish(connection, activity.as_ref(), nonce)
                        } else {
                            Ok(())
                        }
                        .and_then(|()| maintain(connection));
                        if result.is_err() {
                            socket = None;
                        } else {
                            published = activity;
                        }
                    }
                }
            })
            .ok()?;

        Some(Self { current, notify })
    }

    pub(crate) fn update(&self, track: Option<&crate::vm::TrackVm>) {
        let activity = track.map(|track| {
            (
                track.title.chars().take(128).collect(),
                track.artist.chars().take(128).collect(),
            )
        });
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *current != activity {
            *current = activity;
            let _ = self.notify.try_send(());
        }
    }
}

fn connect(id: &str) -> io::Result<UnixStream> {
    let candidates = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .chain(std::iter::once(PathBuf::from("/tmp")));
    for folder in candidates {
        for index in 0..10 {
            let Ok(mut connection) =
                UnixStream::connect(folder.join(format!("discord-ipc-{index}")))
            else {
                continue;
            };
            connection.set_write_timeout(Some(Duration::from_millis(500)))?;
            connection.set_read_timeout(Some(Duration::from_millis(500)))?;
            frame(
                &mut connection,
                0,
                &serde_json::to_vec(&json!({"v": 1, "client_id": id}))?,
            )?;
            let Some((1, payload)) = read_frame(&mut connection)? else {
                continue;
            };
            let ready: Value = serde_json::from_slice(&payload)?;
            if ready["evt"] == "READY" {
                return Ok(connection);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotConnected,
        "Discord IPC not available",
    ))
}

fn publish(
    connection: &mut UnixStream,
    activity: Option<&(String, String)>,
    nonce: u64,
) -> io::Result<()> {
    let payload = activity.map(|(title, artist)| {
        json!({
            "type": 2, "details": title, "state": artist
        })
    });
    frame(
        connection,
        1,
        &serde_json::to_vec(&json!({
            "cmd": "SET_ACTIVITY", "args": {"pid": std::process::id(), "activity": payload},
            "nonce": nonce.to_string(),
        }))?,
    )
}

fn maintain(connection: &mut UnixStream) -> io::Result<()> {
    match read_frame(connection)? {
        Some((2, _)) => Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Discord closed IPC",
        )),
        Some((3, bytes)) => frame(connection, 4, &bytes),
        _ => Ok(()),
    }
}

fn frame(connection: &mut UnixStream, op: u32, payload: &[u8]) -> io::Result<()> {
    connection.write_all(&op.to_le_bytes())?;
    connection.write_all(&(payload.len() as u32).to_le_bytes())?;
    connection.write_all(payload)
}

fn read_frame(connection: &mut UnixStream) -> io::Result<Option<(u32, Vec<u8>)>> {
    let mut header = [0u8; 8];
    match connection.read(&mut header) {
        Ok(0) => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Discord IPC EOF",
            ));
        }
        Ok(size) => connection.read_exact(&mut header[size..])?,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }
    let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if size > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Discord IPC frame too large",
        ));
    }
    let mut data = vec![0u8; size];
    connection.read_exact(&mut data)?;
    Ok(Some((op, data)))
}
