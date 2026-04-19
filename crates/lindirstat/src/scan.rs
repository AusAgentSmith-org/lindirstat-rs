use anyhow::{Context, Result};
use lindirstat_wire::{read_frame, Entry, Frame, Summary};
use std::io::{BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

pub enum Msg {
    Header { root: String },
    Batch(Vec<Entry>),
    Done(Summary),
    Error(String),
}

pub struct ScanHandle {
    pub rx: mpsc::Receiver<Msg>,
    _child: Option<Child>,
}

const BATCH_SIZE: usize = 512;

pub struct PasswordAuth<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
}

pub fn spawn_ssh(
    ctx: egui::Context,
    host: &str,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
) -> Result<ScanHandle> {
    let cmd = build_cmd(scanner_path, remote_path, sudo);
    let mut child = Command::new("ssh")
        .arg(host)
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn ssh")?;

    let stdout = child.stdout.take().unwrap();
    let rx = run_reader(ctx, stdout);
    Ok(ScanHandle {
        rx,
        _child: Some(child),
    })
}

pub fn spawn_password(
    ctx: egui::Context,
    auth: PasswordAuth<'_>,
    remote_path: &str,
    scanner_path: &str,
    sudo: bool,
) -> Result<ScanHandle> {
    let tcp = std::net::TcpStream::connect((auth.host, auth.port)).context("tcp connect")?;
    let mut sess = ssh2::Session::new().context("create session")?;
    sess.set_tcp_stream(tcp);
    sess.handshake().context("ssh handshake")?;
    sess.userauth_password(auth.username, auth.password)
        .context("password auth")?;

    let cmd = build_cmd(scanner_path, remote_path, sudo);
    let mut channel = sess.channel_session().context("open channel")?;
    channel.exec(&cmd).context("exec")?;

    let rx = run_reader(ctx, channel);
    Ok(ScanHandle { rx, _child: None })
}

fn run_reader(ctx: egui::Context, reader: impl Read + Send + 'static) -> mpsc::Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut r = BufReader::new(reader);
        let mut batch: Vec<Entry> = Vec::with_capacity(BATCH_SIZE);
        loop {
            match read_frame(&mut r) {
                Ok(Some(Frame::Header(h))) => {
                    let _ = tx.send(Msg::Header { root: h.root });
                    ctx.request_repaint();
                }
                Ok(Some(Frame::Entry(e))) => {
                    batch.push(e);
                    if batch.len() >= BATCH_SIZE {
                        let _ = tx.send(Msg::Batch(std::mem::take(&mut batch)));
                        batch = Vec::with_capacity(BATCH_SIZE);
                        ctx.request_repaint();
                    }
                }
                Ok(Some(Frame::Summary(s))) => {
                    if !batch.is_empty() {
                        let _ = tx.send(Msg::Batch(std::mem::take(&mut batch)));
                    }
                    let _ = tx.send(Msg::Done(s));
                    ctx.request_repaint();
                    break;
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Msg::Error(e.to_string()));
                    ctx.request_repaint();
                    break;
                }
            }
        }
    });
    rx
}

fn build_cmd(scanner_path: &str, remote_path: &str, sudo: bool) -> String {
    if sudo {
        format!(
            "sudo {} {}",
            shell_escape(scanner_path),
            shell_escape(remote_path)
        )
    } else {
        format!(
            "{} {}",
            shell_escape(scanner_path),
            shell_escape(remote_path)
        )
    }
}

fn shell_escape(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_.-~=:".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
