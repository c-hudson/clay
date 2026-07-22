package com.clay.mudclient;

import android.content.Context;
import android.util.Log;

import java.io.File;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Spawns and monitors the bundled Clay binary (libclay.so) in --ssh-proxy mode (see
 * run_ssh_proxy_mode in src/ssh.rs) as a child process, for the Connection Settings "SSH"
 * option. Establishes one SSH session to the configured [user@]host:clayport:sshport target,
 * then forwards 127.0.0.1:&lt;local port&gt; to the remote clay port through it as a transparent
 * raw-TCP tunnel - carrying the CLAY-KNOCK preamble and any TLS bytes through untouched, so
 * NativeWebSocket/MainActivity need no changes of their own: they just connect to
 * 127.0.0.1:&lt;port&gt; instead of the real remote host, exactly as LocalServerManager already
 * does for the standalone/local run mode.
 *
 * start() is synchronous (spawns the process, then polls until the local port accepts
 * connections) — callers must invoke it off the main thread. Mirrors LocalServerManager's
 * structure closely; see its class doc for the shared "exec the app's own bundled .so"
 * rationale (Android's sandbox otherwise has no ssh binary reachable to shell out to).
 */
public class SshProxyManager {
    private static final String TAG = "ClaySshProxy";
    // Same generous margin as LocalServerManager - an SSH handshake + key/agent auth adds
    // real network round-trips on top of the same binary-paging/tokio-startup cost.
    private static final int READY_TIMEOUT_MS = 10000;
    private static final int READY_POLL_INTERVAL_MS = 150;
    // Every instance gets its own log file (see logFile below) so two candidates racing in
    // MainActivity.raceSshProxyStart() - or back-to-back retries in
    // MainActivity.startSshProxyThenLoadInterface() - never clobber each other's diagnostics.
    private static final AtomicInteger INSTANCE_COUNTER = new AtomicInteger();

    private final Context appContext;
    private final int instanceId = INSTANCE_COUNTER.incrementAndGet();
    private Process process;
    private int localPort = -1;
    // Set by cancel() to bail out of an in-progress start() early - e.g. when this instance
    // lost a MainActivity.raceSshProxyStart() race against another candidate host. Deliberately
    // NOT synchronized: start()/stop() share this instance's intrinsic lock and start() holds it
    // for the whole blocking spawn+poll, so a synchronized cancel() would just block until
    // start() finishes on its own - defeating the point. A plain volatile flag lets another
    // thread signal cancellation immediately; waitForReady()'s poll loop notices it within one
    // READY_POLL_INTERVAL_MS.
    private volatile boolean cancelRequested;
    // Human-readable reason the last start() call failed, for MainActivity to surface in its
    // SSH-failed dialog. Empty until a failure occurs; not meaningful after a successful start().
    private String lastError = "";

    public SshProxyManager(Context context) {
        this.appContext = context.getApplicationContext();
    }

    /** Reason the last start() attempt failed, or "" if it hasn't failed (yet). */
    public synchronized String getLastError() {
        return lastError;
    }

    public synchronized boolean isRunning() {
        return process != null && process.isAlive();
    }

    public synchronized int getLocalPort() {
        return localPort;
    }

    /**
     * Starts the proxy if not already running. Returns true once it's accepting connections.
     *
     * @param sshUser        SSH username (required)
     * @param sshHost        Remote host to SSH into (required)
     * @param sshPort        Remote SSH port (22 if &lt;= 0)
     * @param clayPort       The Clay daemon's port on the remote host, reached via the tunnel
     *                       (9000 if &lt;= 0)
     * @param privateKeyPem  PEM-encoded private key text, or null/empty if not using a key
     * @param keyPassphrase  Passphrase for privateKeyPem, or null/empty if the key isn't
     *                       encrypted (or no key is set)
     * @param password       SSH password, or null/empty if not using password auth
     */
    public synchronized boolean start(String sshUser, String sshHost, int sshPort, int clayPort,
                                       String privateKeyPem, String keyPassphrase, String password) {
        cancelRequested = false;
        if (isRunning()) {
            return true;
        }

        if (sshUser == null || sshUser.isEmpty() || sshHost == null || sshHost.isEmpty()) {
            lastError = "SSH user and host are required";
            Log.e(TAG, lastError);
            return false;
        }
        boolean hasKey = privateKeyPem != null && !privateKeyPem.isEmpty();
        boolean hasPassword = password != null && !password.isEmpty();
        if (!hasKey && !hasPassword) {
            lastError = "At least one of private key or password is required";
            Log.e(TAG, lastError);
            return false;
        }

        String binPath = appContext.getApplicationInfo().nativeLibraryDir + "/libclay.so";
        File bin = new File(binPath);
        if (!bin.exists() || !bin.canExecute()) {
            lastError = "Clay binary not found or not executable";
            Log.e(TAG, lastError + " at " + binPath);
            return false;
        }

        int resolvedSshPort = sshPort > 0 ? sshPort : 22;
        int resolvedClayPort = clayPort > 0 ? clayPort : 9000;
        localPort = pickPort();

        String target = sshUser + "@" + sshHost + ":" + resolvedClayPort + ":" + resolvedSshPort;

        File home = appContext.getFilesDir();
        File logFile = new File(appContext.getCacheDir(), "clay-ssh-proxy-" + instanceId + ".log");

        try {
            ProcessBuilder pb = new ProcessBuilder(binPath, "--ssh-proxy",
                "--target=" + target, "--listen-port=" + localPort);
            pb.environment().put("HOME", home.getAbsolutePath());
            if (hasKey) {
                pb.environment().put("CLAY_SSH_KEY", privateKeyPem);
                if (keyPassphrase != null && !keyPassphrase.isEmpty()) {
                    pb.environment().put("CLAY_SSH_KEY_PASSPHRASE", keyPassphrase);
                }
            }
            if (hasPassword) {
                pb.environment().put("CLAY_SSH_PASSWORD", password);
            }
            pb.redirectErrorStream(true);
            pb.redirectOutput(logFile);
            process = pb.start();
        } catch (IOException e) {
            lastError = "Failed to start SSH proxy process: " + e.getMessage();
            Log.e(TAG, "Failed to start SSH proxy", e);
            process = null;
            localPort = -1;
            return false;
        }

        boolean ready = waitForReady(localPort, READY_TIMEOUT_MS);
        if (!ready) {
            if (cancelRequested) {
                // Informational only - a cancelled candidate never contributes to a user-facing
                // failure message (cancellation only happens on the losing side of an otherwise
                // successful race), so this never needs to be more specific than this.
                lastError = "Cancelled (lost race to another candidate)";
                Log.i(TAG, "SSH proxy on port " + localPort + " cancelled (lost a race against "
                    + "another candidate host) — killing it");
            } else {
                lastError = readLastErrorFromLog(logFile);
                Log.e(TAG, "SSH proxy on port " + localPort + " did not become ready within "
                    + READY_TIMEOUT_MS + "ms (see " + logFile + ") — killing it so a retry doesn't see a stale process");
            }
            stop();
        } else {
            Log.i(TAG, "SSH proxy ready on 127.0.0.1:" + localPort + " -> " + target);
        }
        return ready;
    }

    // Reads the proxy's own stdout/stderr (see pb.redirectOutput above) for a real failure
    // reason - e.g. "clay: SSH auth failed" / "clay: could not resolve host" from ssh.rs's
    // render_ssh_error - rather than surfacing only a generic timeout to the user. The file is
    // small (one connection attempt's worth of output), so it's read in full; only the last few
    // lines are kept since those are the actual error, not the initial "connecting to..." line.
    private String readLastErrorFromLog(File logFile) {
        try {
            String content = new String(Files.readAllBytes(logFile.toPath()), StandardCharsets.UTF_8).trim();
            if (content.isEmpty()) {
                return "SSH connection timed out";
            }
            String[] lines = content.split("\n");
            int keep = Math.min(lines.length, 5);
            StringBuilder sb = new StringBuilder();
            for (int i = lines.length - keep; i < lines.length; i++) {
                if (sb.length() > 0) sb.append('\n');
                sb.append(lines[i]);
            }
            return sb.toString();
        } catch (IOException e) {
            return "SSH connection timed out (no log available)";
        }
    }

    /**
     * Signals an in-progress start() to bail out early rather than run its full readiness
     * timeout - used by MainActivity.raceSshProxyStart() to give up on the losing candidate as
     * soon as another one wins. Safe to call from any thread, including while start() is
     * blocked; a no-op if start() isn't currently running (or already finished).
     */
    public void cancel() {
        cancelRequested = true;
    }

    /** Stops the proxy if running. Safe to call even if never started. */
    public synchronized void stop() {
        if (process != null) {
            // Forcibly (SIGKILL), not just destroy() (SIGTERM) — matches LocalServerManager,
            // must reliably tear down a process that may be mid SSH-handshake.
            process.destroyForcibly();
            process = null;
        }
        localPort = -1;
    }

    // Always an ephemeral loopback port - unlike LocalServerManager there's no shared default
    // (9000) to prefer, since this port is purely local plumbing the WebView is told about via
    // buildVarInjectionScript, never a fixed port a user would type in.
    private int pickPort() {
        try (ServerSocket s = new ServerSocket(0, 0, InetAddress.getByName("127.0.0.1"))) {
            return s.getLocalPort();
        } catch (IOException e) {
            Log.w(TAG, "Could not find a free local port, defaulting to 9000", e);
            return 9000;
        }
    }

    private boolean waitForReady(int targetPort, int timeoutMs) {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cancelRequested) {
                return false; // caller (start()) logs + cleans up
            }
            if (!isRunning()) {
                Log.e(TAG, "SSH proxy process exited before becoming ready");
                return false;
            }
            try (Socket sock = new Socket()) {
                sock.connect(new InetSocketAddress("127.0.0.1", targetPort), 300);
                return true;
            } catch (IOException e) {
                // Not ready yet
            }
            try {
                Thread.sleep(READY_POLL_INTERVAL_MS);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
        return false;
    }
}
