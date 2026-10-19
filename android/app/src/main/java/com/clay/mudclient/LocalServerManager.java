package com.clay.mudclient;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.Log;

import java.io.File;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.security.SecureRandom;

/**
 * Spawns and monitors the bundled headless Clay server (libclay.so, see --local-server in
 * src/daemon.rs) as a child process, for the app's standalone/on-device run mode. The WebView
 * connects to it over ws://127.0.0.1:&lt;port&gt; exactly as it would to a remote server.
 *
 * start() is synchronous (spawns the process, then polls until the port accepts connections) —
 * callers must invoke it off the main thread.
 */
public class LocalServerManager {
    private static final String TAG = "ClayLocalServer";
    static final int PREFERRED_PORT = 9000;
    // Matches the desktop GUI's own readiness wait (webview_gui::run_master_webgui polls
    // GUI_HTTP_READY for up to 10s). First run on a real device still needs to page in a ~13MB
    // binary, link it, and spin up a tokio thread pool, so keep the same generous margin.
    private static final int READY_TIMEOUT_MS = 10000;
    private static final int READY_POLL_INTERVAL_MS = 150;
    // How long a start with Allow Other Devices waits for its fixed port to come free.
    private static final int PORT_FREE_TIMEOUT_MS = 3000;

    // "Allow Other Devices" (Settings > Clay Server, Run on This Phone): the server listens on
    // the network with a password and port the user chose, instead of on loopback only with a
    // random password nobody sees. Off by default. Kept with the app's other settings, like the
    // remote server's saved password. Other devices get what any Clay server gives them (TLS,
    // the password, the allow list, bans); this app still connects over loopback.
    static final String PREFS_NAME = "ClayPrefs";
    static final String KEY_LAN_ENABLED = "localLanEnabled";
    static final String KEY_LAN_PASSWORD = "localLanPassword";
    static final String KEY_LAN_PORT = "localLanPort";
    static final int MIN_LAN_PASSWORD_LENGTH = 8;

    private final Context appContext;
    private Process process;
    // configSnapshot() of the settings this server was started with, and why the last start
    // failed (for the user; null when it didn't, or there's nothing to say).
    private String startedConfig;
    private String lastError;
    private int port = -1;
    private String password;

    public LocalServerManager(Context context) {
        this.appContext = context.getApplicationContext();
    }

    public synchronized boolean isRunning() {
        return process != null && process.isAlive();
    }

    public synchronized int getPort() {
        return port;
    }

    public synchronized String getPassword() {
        return password;
    }

    /**
     * SHA-256 hex digest of the password — matches Rust's websocket::hash_password() exactly.
     * This, not the raw password, is what the WebView injects as window.AUTO_PASSWORD so app.js
     * can auto-authenticate (see handleSocketOpen() in app.js) the same way the desktop GUI's
     * master mode does.
     */
    public synchronized String getPasswordHash() {
        if (password == null) {
            return null;
        }
        return sha256Hex(password);
    }

    private static String sha256Hex(String text) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            byte[] hash = digest.digest(text.getBytes(java.nio.charset.StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder(hash.length * 2);
            for (byte b : hash) {
                sb.append(String.format("%02x", b));
            }
            return sb.toString();
        } catch (NoSuchAlgorithmException e) {
            // SHA-256 is guaranteed available on every Android API level; unreachable.
            throw new RuntimeException(e);
        }
    }

    static boolean validLanPassword(String pw) {
        return pw != null && pw.length() >= MIN_LAN_PASSWORD_LENGTH;
    }

    /** Above 1023: an app can't listen on a lower port. */
    static boolean validLanPort(int p) {
        return p >= 1024 && p <= 65535;
    }

    /** Allow Other Devices is on, with a password and port the server can use. */
    static boolean lanEnabled(SharedPreferences prefs) {
        return prefs.getBoolean(KEY_LAN_ENABLED, false)
            && validLanPassword(prefs.getString(KEY_LAN_PASSWORD, ""))
            && validLanPort(prefs.getInt(KEY_LAN_PORT, PREFERRED_PORT));
    }

    /** The saved setting in one string, compared with startedConfig() to tell whether a running
     *  server must restart to follow it. The password goes in as its hash. */
    static String configSnapshot(SharedPreferences prefs) {
        if (!lanEnabled(prefs)) return "loopback";
        return "lan:" + prefs.getInt(KEY_LAN_PORT, PREFERRED_PORT) + ":"
            + sha256Hex(prefs.getString(KEY_LAN_PASSWORD, ""));
    }

    public synchronized String startedConfig() {
        return startedConfig;
    }

    public synchronized String lastError() {
        return lastError;
    }

    /** Starts the server if not already running. Returns true once it's accepting connections. */
    public synchronized boolean start() {
        if (isRunning()) {
            return true;
        }

        String binPath = appContext.getApplicationInfo().nativeLibraryDir + "/libclay.so";
        File bin = new File(binPath);
        if (!bin.exists() || !bin.canExecute()) {
            Log.e(TAG, "libclay.so not found or not executable at " + binPath);
            return false;
        }

        lastError = null;
        SharedPreferences prefs = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
        boolean lan = lanEnabled(prefs);
        if (lan) {
            // Other devices are told this port, so never fall back to another one.
            port = prefs.getInt(KEY_LAN_PORT, PREFERRED_PORT);
            if (!waitForPortFree(port, PORT_FREE_TIMEOUT_MS)) {
                lastError = "Port " + port + " is in use by another app, so Clay's server could not start."
                    + " Choose another port in Settings > Clay Server.";
                Log.e(TAG, lastError);
                port = -1;
                return false;
            }
            password = prefs.getString(KEY_LAN_PASSWORD, "");
        } else {
            port = pickPort();
            password = generatePassword();
        }
        startedConfig = configSnapshot(prefs);

        File home = appContext.getFilesDir();
        File logFile = new File(appContext.getCacheDir(), "clay-local-server.log");

        try {
            ProcessBuilder pb = new ProcessBuilder(binPath, "--local-server", "--port=" + port);
            pb.environment().put("HOME", home.getAbsolutePath());
            pb.environment().put("CLAY_WS_PASSWORD", password);
            if (lan) {
                pb.environment().put("CLAY_LOCAL_SERVER_LAN", "1");
            }
            pb.redirectErrorStream(true);
            pb.redirectOutput(logFile);
            process = pb.start();
        } catch (IOException e) {
            Log.e(TAG, "Failed to start local server", e);
            process = null;
            port = -1;
            return false;
        }

        boolean ready = waitForReady(port, READY_TIMEOUT_MS);
        if (!ready) {
            Log.e(TAG, "Local server on port " + port + " did not become ready within "
                + READY_TIMEOUT_MS + "ms — killing it so a retry doesn't see a stale process");
            // Without this, a slow-but-eventually-successful start would leak as an orphaned
            // process: isRunning() would report true forever, so a later start() call would
            // just return true without ever re-checking readiness.
            stop();
        } else {
            Log.i(TAG, "Local server ready on " + (lan ? "all interfaces" : "127.0.0.1") + ":" + port);
        }
        return ready;
    }

    /** Stops the server if running. Safe to call even if never started. */
    public synchronized void stop() {
        if (process != null) {
            // Forcibly (SIGKILL), not just destroy() (SIGTERM) — this must reliably tear down
            // a process that may be in an unknown state (e.g. still starting up).
            process.destroyForcibly();
            process = null;
        }
        port = -1;
    }

    // Prefer the default port; fall back to any free loopback port if it's taken (e.g. a
    // previous instance still shutting down). Mirrors the same bind-then-drop probe
    // webview_gui::run_master_webgui uses on the desktop side to detect port availability.
    private int pickPort() {
        if (isPortFree(PREFERRED_PORT)) {
            return PREFERRED_PORT;
        }
        try (ServerSocket s = new ServerSocket(0, 0, InetAddress.getByName("127.0.0.1"))) {
            return s.getLocalPort();
        } catch (IOException e) {
            Log.w(TAG, "Could not find a free fallback port, defaulting to " + PREFERRED_PORT, e);
            return PREFERRED_PORT;
        }
    }

    private boolean isPortFree(int candidatePort) {
        return isPortFree(candidatePort, "127.0.0.1");
    }

    /** True once the port is free on loopback and on every interface. A restart stops the old
     *  server just before this start, and the dying process holds the port for a moment - the
     *  check came 4 ms after the kill and read "in use" against Clay itself. */
    private boolean waitForPortFree(int candidatePort, int timeoutMs) {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (true) {
            if (isPortFree(candidatePort) && isPortFree(candidatePort, "0.0.0.0")) return true;
            if (System.currentTimeMillis() >= deadline) return false;
            try {
                Thread.sleep(READY_POLL_INTERVAL_MS);
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
    }

    private boolean isPortFree(int candidatePort, String host) {
        try (ServerSocket s = new ServerSocket(candidatePort, 0, InetAddress.getByName(host))) {
            return true;
        } catch (IOException e) {
            return false;
        }
    }

    private boolean waitForReady(int targetPort, int timeoutMs) {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (!isRunning()) {
                Log.e(TAG, "Local server process exited before becoming ready");
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

    private String generatePassword() {
        byte[] bytes = new byte[32];
        new SecureRandom().nextBytes(bytes);
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(String.format("%02x", b));
        }
        return sb.toString();
    }
}
