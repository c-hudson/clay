//! `--native-gui=[user@]host[:clayport[:sshport]] --ssh`: the server reached through SSH,
//! as `--gui`/`--console` reach it (src/ssh.rs). The SSH session opens before any window
//! does, so a key's passphrase can be asked for on the terminal (the remote console's
//! rule); then a loopback port forwards each connection the client makes through a fresh
//! channel of it. The far end is the server's own loopback, so the client speaks plain
//! `ws://` to `127.0.0.1` and logs in as it would on the server's machine.

use std::io::IsTerminal;
use std::sync::Arc;

use crate::ssh::{self, AuthContext, SshCredentials, SshSession, SshTarget};

/// Open the session and the forwarding port; the port, or what went wrong.
pub async fn start(target: SshTarget) -> Result<u16, String> {
    let creds = SshCredentials::desktop_default();
    let ctx = if std::io::stdin().is_terminal() { AuthContext::InteractiveTerminal } else { AuthContext::NonInteractive };
    let session = ssh::establish_session(&target, &creds, ctx).await.map_err(|e| ssh::render_ssh_error(&e))?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let session = Arc::new(tokio::sync::Mutex::new(session));
    tokio::spawn(async move {
        loop {
            let Ok((mut local, _)) = listener.accept().await else { continue };
            let (session, target, creds) = (session.clone(), target.clone(), creds.clone());
            tokio::spawn(async move {
                let Some(mut tunnel) = open(&session, &target, &creds).await else { return };
                let _ = tokio::io::copy_bidirectional(&mut local, &mut tunnel).await;
            });
        }
    });
    Ok(port)
}

/// A channel to the server; if the session has died (a network change, the far end
/// gone), a new session first (no terminal by now: keys and the agent only). `None`
/// drops the client's connection, which retries as from any lost connection.
async fn open(session: &tokio::sync::Mutex<SshSession>, target: &SshTarget, creds: &SshCredentials) -> Option<ssh::SshTunnel> {
    let mut s = session.lock().await;
    if let Ok(t) = s.open_tunnel(target.clay_port).await {
        return Some(t);
    }
    match ssh::establish_session(target, creds, AuthContext::NonInteractive).await {
        Ok(fresh) => {
            *s = fresh;
            s.open_tunnel(target.clay_port).await.ok()
        }
        Err(e) => {
            crate::debug_log(true, &format!("native-gui: SSH to {}: {}", target.host, ssh::render_ssh_error(&e)));
            None
        }
    }
}
