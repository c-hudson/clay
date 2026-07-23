package com.clay.mudclient;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.Uri;
import android.net.http.SslError;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.provider.Settings;
import android.webkit.ConsoleMessage;
import android.webkit.SslErrorHandler;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.JavascriptInterface;
import android.webkit.WebViewClient;
import android.widget.Toast;

import androidx.appcompat.app.AlertDialog;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.app.ActivityCompat;
import androidx.core.app.NotificationCompat;
import androidx.core.content.ContextCompat;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

public class MainActivity extends AppCompatActivity {
    private static final String PREFS_NAME = "ClayPrefs";
    private static final String KEY_SERVER_HOST = "serverHost";
    private static final String KEY_SERVER_PORT = "serverPort";
    private static final String KEY_USE_SECURE = "useSecure";
    private static final String KEY_SAVED_PASSWORD = "savedPassword";
    private static final String KEY_SAVED_USERNAME = "savedUsername";
    private static final String KEY_AUTH_KEY = "authKey";  // Device auth key for passwordless login
    private static final String KEY_WEB_PATH = "webPath";  // Server's stealth web_path prefix; blank = auto-detect
    private static final String KEY_ADVANCED_ENABLED = "advancedEnabled";
    private static final String KEY_REMOTE_HOSTNAME = "remoteHostname";
    private static final String KEY_CACHED_THEME_CSS = "cachedThemeCss";
    private static final String KEY_SETUP_COMPLETE = "setupComplete";
    // Whether Clay runs its own server on-device ("local") or connects to one elsewhere
    // ("remote"). Unset on first launch until the user picks in the startup chooser; changeable
    // later from Settings. Purely a stored preference until later wiring (LocalServerManager,
    // WebView localhost injection) reads it.
    private static final String KEY_RUN_MODE = "runMode";
    private static final String RUN_MODE_LOCAL = "local";
    private static final String RUN_MODE_REMOTE = "remote";
    // SSH tunnel option for remote mode (see SshProxyManager) — reuses KEY_SERVER_HOST/
    // KEY_SERVER_PORT as the SSH target host and the Clay port reached through the tunnel;
    // no separate "SSH host" field, since the box you SSH into is the box running the daemon.
    private static final String KEY_SSH_ENABLED = "sshEnabled";
    private static final String KEY_SSH_USER = "sshUser";
    private static final String KEY_SSH_PORT = "sshPort";
    private static final String KEY_SSH_PRIVATE_KEY = "sshPrivateKey";
    private static final String KEY_SSH_KEY_PASSPHRASE = "sshKeyPassphrase";
    private static final String KEY_SSH_PASSWORD = "sshPassword";

    // Minimal first-launch page — loads instantly, immediately hands off to the full app
    private static final String FIRST_LAUNCH_HTML =
        "<!DOCTYPE html><html><head>" +
        "<meta charset='UTF-8'>" +
        "<meta name='viewport' content='width=device-width,initial-scale=1'>" +
        "<style>*{margin:0;padding:0}body{background:#131926;display:flex;" +
        "align-items:center;justify-content:center;min-height:100vh}</style>" +
        "</head><body>" +
        "<script>if(window.Android&&typeof Android.loadFullApp==='function')Android.loadFullApp();</script>" +
        "</body></html>";

    // Default dark theme CSS vars used on first launch before server provides real theme
    private static final String DEFAULT_THEME_CSS =
        "--theme-bg: #131926;\n--theme-bg-deep: #131926;\n--theme-bg-surface: #1c1722;\n" +
        "--theme-bg-elevated: #1f1f1f;\n--theme-bg-hover: #2c2535;\n--theme-fg: #e8e4ec;\n" +
        "--theme-fg-secondary: #a89fb4;\n--theme-fg-muted: #6e6479;\n--theme-fg-dim: #4a4255;\n" +
        "--theme-accent: #2657ba;\n--theme-accent-dim: #004080;\n--theme-highlight: #e8c46a;\n" +
        "--theme-success: #7ecf8b;\n--theme-error: #dc2626;\n--theme-error-dim: #5f0000;\n" +
        "--theme-status-bar-bg: #284b63;\n--theme-menu-bar-bg: #152b3a;\n" +
        "--theme-selection-bg: #004080;\n--theme-link: #8cb4e0;\n--theme-prompt: #d4845a;\n" +
        "--theme-border-subtle: #221c2b;\n--theme-border-medium: #2e2738;\n" +
        "--theme-button-selected-bg: #e8e4ec;\n--theme-button-selected-fg: #131926;\n" +
        "--theme-more-indicator-bg: #5f0000;\n--theme-activity-bg: #f5f0d8;\n" +
        "--theme-ansi-0: #000000;\n--theme-ansi-1: #aa0000;\n--theme-ansi-2: #44aa44;\n" +
        "--theme-ansi-3: #aa5500;\n--theme-ansi-4: #0039aa;\n--theme-ansi-5: #aa22aa;\n" +
        "--theme-ansi-6: #1a92aa;\n--theme-ansi-7: #e8e4ec;\n--theme-ansi-8: #777777;\n" +
        "--theme-ansi-9: #ff8787;\n--theme-ansi-10: #4ce64c;\n--theme-ansi-11: #ded82c;\n" +
        "--theme-ansi-12: #295fcc;\n--theme-ansi-13: #cc58cc;\n--theme-ansi-14: #4ccce6;\n" +
        "--theme-ansi-15: #ffffff;\n";

    private static final String CHANNEL_ID_ALERTS = "clay_alerts";
    private static final String CHANNEL_ID_SERVICE = "clay_service";
    private static final int NOTIFICATION_PERMISSION_REQUEST = 1001;
    private static final int BATTERY_OPTIMIZATION_REQUEST = 1002;
    private static final int KEEPALIVE_INTERVAL_MS = 60000; // 60 seconds (reduced from 30s for power savings)
    private static final long BACKGROUND_SHUTDOWN_MS = 60 * 60 * 1000; // 1 hour - auto-disconnect when in background

    private WebView webView;
    private android.widget.LinearLayout connectingOverlay;
    private android.widget.TextView connectingText;
    private android.widget.TextView connectingUrl;
    private android.widget.Button connectingCancelBtn;
    private volatile boolean connectCancelled = false;
    private boolean connectionFailed = false;
    private int notificationId = 1000;
    private boolean isConnected = false;
    private boolean isInitialLoadPending = false;
    private boolean permissionsHandled = false;
    private boolean notificationPermissionDone = false;
    private boolean batteryOptimizationDone = false;
    // Battery optimization exemption is requested lazily (Play Store restricts
    // REQUEST_IGNORE_BATTERY_OPTIMIZATIONS to apps that actually need it, and requesting it
    // unconditionally on every fresh install before any connection is made is the kind of
    // usage Google pushes back on) - it's now asked for the first time a persistent
    // background connection actually starts (see startBackgroundService() /
    // checkBatteryOptimization()), not during startPermissionFlow(). This callback carries
    // the "proceed with starting the connection" continuation across the async
    // startActivityForResult() round-trip.
    private Runnable pendingBatteryOptCallback = null;
    private boolean interfaceLoaded = false;
    private String loadedInterfaceUrl = null;
    private LocalServerManager localServerManager;
    private SshProxyManager sshProxyManager;
    // Snapshot of the SSH settings last applied to a running sshProxyManager (null when SSH
    // wasn't in use), so reloadInterfaceRespectingRunMode() can detect a credential/target
    // change even when KEY_SSH_ENABLED itself didn't flip — see that method.
    private String lastAppliedSshConfigSnapshot = null;
    // Registered in onResume()/unregistered in onPause() (only while in SSH remote mode) so a
    // WiFi<->cellular handoff (or any other default-network change) triggers an unconditional
    // SSH tunnel restart - see restartSshTunnel(). Non-null only while registered.
    private ConnectivityManager.NetworkCallback sshNetworkCallback;
    // Debounces restartSshTunnel() so a resume and a near-simultaneous network-change callback
    // don't both kick off a full restart at once.
    private volatile long lastSshTunnelRestartAt = 0;
    // Removed duplicate screenOffWakeLock - ClayForegroundService already holds one
    private Handler keepaliveHandler;
    private Runnable keepaliveRunnable;
    private Handler backgroundShutdownHandler;
    private Runnable backgroundShutdownRunnable;
    private final java.util.concurrent.ConcurrentHashMap<Integer, NativeWebSocket> nativeWebSockets =
        new java.util.concurrent.ConcurrentHashMap<>();
    private Handler heartbeatHandler;
    private Runnable heartbeatRunnable;
    private int missedHeartbeats = 0;
    private static final int HEARTBEAT_INTERVAL_MS = 30000; // 30 seconds

    // JavaScript interface for communication between web and Android
    public class AndroidInterface {
        @JavascriptInterface
        public void openServerSettings() {
            runOnUiThread(() -> {
                webView.evaluateJavascript(
                    "if (typeof openSettingsPopup === 'function') openSettingsPopup('clay-server');",
                    null);
            });
        }

        @JavascriptInterface
        public void showFirstLaunchSetup() {
            runOnUiThread(() -> webView.evaluateJavascript(
                "if (typeof openSettingsPopup === 'function') openSettingsPopup('clay-server');", null));
        }

        @JavascriptInterface
        public void loadFullApp() {
            runOnUiThread(() -> loadInterface());
        }

        @JavascriptInterface
        public void showNotification(String title, String message) {
            runOnUiThread(() -> {
                createNotificationChannel();

                // Create intent to open app when notification is tapped
                Intent intent = new Intent(MainActivity.this, MainActivity.class);
                intent.setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP);
                PendingIntent pendingIntent = PendingIntent.getActivity(
                    MainActivity.this, 0, intent,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
                );

                NotificationCompat.Builder builder = new NotificationCompat.Builder(MainActivity.this, CHANNEL_ID_ALERTS)
                    .setSmallIcon(R.drawable.ic_notification)
                    .setContentTitle(title != null ? title : "Clay")
                    .setContentText(message != null ? message : "")
                    .setPriority(NotificationCompat.PRIORITY_HIGH)
                    .setAutoCancel(true)
                    .setContentIntent(pendingIntent);

                NotificationManager manager = getSystemService(NotificationManager.class);
                if (manager != null) {
                    manager.notify(notificationId++, builder.build());
                }
            });
        }

        @JavascriptInterface
        public void startBackgroundService() {
            runOnUiThread(() -> {
                // Request battery optimization exemption here, lazily, the first time a
                // persistent background connection actually starts - not unconditionally at
                // app startup (see checkBatteryOptimization()'s doc comment). No-ops
                // immediately if already granted/handled.
                checkBatteryOptimization(() -> {
                    Intent serviceIntent = new Intent(MainActivity.this, ClayForegroundService.class);
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        startForegroundService(serviceIntent);
                    } else {
                        startService(serviceIntent);
                    }

                    // Mark as connected and start keepalive + heartbeat
                    isConnected = true;
                    startKeepalive();
                    startHeartbeat();
                });
            });
        }

        @JavascriptInterface
        public void stopBackgroundService() {
            runOnUiThread(() -> {
                isConnected = false;
                stopKeepalive();
                stopHeartbeat();

                Intent serviceIntent = new Intent(MainActivity.this, ClayForegroundService.class);
                stopService(serviceIntent);
            });
        }

        @JavascriptInterface
        public void keepaliveAck() {
            // Called by JavaScript to acknowledge keepalive ping
            // This helps detect if the WebView is actually responsive
        }

        @JavascriptInterface
        public void savePassword(String password) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().putString(KEY_SAVED_PASSWORD, password).apply();
        }

        @JavascriptInterface
        public String getSavedPassword() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SAVED_PASSWORD, "");
        }

        @JavascriptInterface
        public void clearSavedPassword() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().remove(KEY_SAVED_PASSWORD).apply();
        }

        @JavascriptInterface
        public void saveUsername(String username) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().putString(KEY_SAVED_USERNAME, username).apply();
        }

        @JavascriptInterface
        public String getSavedUsername() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SAVED_USERNAME, "");
        }

        @JavascriptInterface
        public void clearSavedUsername() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().remove(KEY_SAVED_USERNAME).apply();
        }

        // Device auth key methods for passwordless authentication
        @JavascriptInterface
        public void saveAuthKey(String key) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().putString(KEY_AUTH_KEY, key).apply();
        }

        @JavascriptInterface
        public String getAuthKey() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_AUTH_KEY, "");
        }

        @JavascriptInterface
        public void clearAuthKey() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().remove(KEY_AUTH_KEY).apply();
        }

        // Server's stealth web_path prefix. Blank = auto-detect (probe /clay/ws then /ws).
        // Learned automatically from the server's settings payload (see app.js), and/or
        // set explicitly in Settings.
        @JavascriptInterface
        public void saveWebPath(String path) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().putString(KEY_WEB_PATH, path == null ? "" : path).apply();
        }

        @JavascriptInterface
        public String getWebPath() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_WEB_PATH, "");
        }

        @JavascriptInterface
        public String getConnectionMode() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString("connectionMode", "auto");
        }

        @JavascriptInterface
        public void saveConnectionMode(String mode) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            prefs.edit().putString("connectionMode", mode).apply();
        }

        // Run mode ("local" | "remote") — used by the web settings popup's Clay Server tab to
        // show/hide the remote fields and to persist a mode switch before calling reloadPage().
        @JavascriptInterface
        public String getRunMode() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE);
        }

        @JavascriptInterface
        public void setRunMode(String mode) {
            String sanitized = RUN_MODE_LOCAL.equals(mode) ? RUN_MODE_LOCAL : RUN_MODE_REMOTE;
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_RUN_MODE, sanitized).apply();
        }

        // SSH tunnel option (remote mode only) — see SshProxyManager and
        // MainActivity#buildVarInjectionScript's SSH branch. Credentials (key/passphrase/
        // password) are stored in the same plain SharedPreferences as the existing saved
        // Clay password/auth key (KEY_SAVED_PASSWORD/KEY_AUTH_KEY) — consistent with current
        // practice for this app, not a new weakness.
        @JavascriptInterface
        public boolean getSshEnabled() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getBoolean(KEY_SSH_ENABLED, false);
        }

        @JavascriptInterface
        public void saveSshEnabled(boolean enabled) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putBoolean(KEY_SSH_ENABLED, enabled).apply();
        }

        @JavascriptInterface
        public String getSshUser() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SSH_USER, "");
        }

        @JavascriptInterface
        public void saveSshUser(String user) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_SSH_USER, user != null ? user.trim() : "").apply();
        }

        @JavascriptInterface
        public int getSshPort() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getInt(KEY_SSH_PORT, 22);
        }

        @JavascriptInterface
        public void saveSshPort(int port) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putInt(KEY_SSH_PORT, port > 0 ? port : 22).apply();
        }

        @JavascriptInterface
        public String getSshPrivateKey() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SSH_PRIVATE_KEY, "");
        }

        @JavascriptInterface
        public void saveSshPrivateKey(String key) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_SSH_PRIVATE_KEY, key != null ? key : "").apply();
        }

        @JavascriptInterface
        public void clearSshPrivateKey() {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .remove(KEY_SSH_PRIVATE_KEY).apply();
        }

        @JavascriptInterface
        public String getSshKeyPassphrase() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SSH_KEY_PASSPHRASE, "");
        }

        @JavascriptInterface
        public void saveSshKeyPassphrase(String passphrase) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_SSH_KEY_PASSPHRASE, passphrase != null ? passphrase : "").apply();
        }

        @JavascriptInterface
        public void clearSshKeyPassphrase() {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .remove(KEY_SSH_KEY_PASSPHRASE).apply();
        }

        @JavascriptInterface
        public String getSshPassword() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            return prefs.getString(KEY_SSH_PASSWORD, "");
        }

        @JavascriptInterface
        public void saveSshPassword(String password) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_SSH_PASSWORD, password != null ? password : "").apply();
        }

        @JavascriptInterface
        public void clearSshPassword() {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .remove(KEY_SSH_PASSWORD).apply();
        }

        @JavascriptInterface
        public void showToast(String message) {
            runOnUiThread(() -> {
                Toast.makeText(MainActivity.this, message, Toast.LENGTH_SHORT).show();
            });
        }

        @JavascriptInterface
        public void showErrorBanner(String message) {
            runOnUiThread(() -> {
                Toast.makeText(MainActivity.this, "JS ERROR: " + message, Toast.LENGTH_LONG).show();
            });
        }

        @JavascriptInterface
        public void connectWebSocket(int id, String url) {
            runOnUiThread(() -> {
                // Block connections until the user has configured a remote server. Local mode
                // needs no such configuration — choosing it in the run-mode chooser is itself
                // "configured" (buildVarInjectionScript already points the WebView at the
                // locally-running server by the time this fires).
                SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
                boolean isLocalMode = RUN_MODE_LOCAL.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE));
                if (!isLocalMode && !prefs.getBoolean(KEY_SETUP_COMPLETE, false)) {
                    webView.evaluateJavascript(
                        "if (typeof openSettingsPopup === 'function') openSettingsPopup('clay-server');",
                        null);
                    return;
                }

                NativeWebSocket ws = new NativeWebSocket(MainActivity.this, new NativeWebSocket.WebSocketCallback() {
                    @Override
                    public void onOpen() {
                        runOnUiThread(() -> {
                            webView.evaluateJavascript(
                                "if (typeof onNativeWebSocketOpen === 'function') onNativeWebSocketOpen(" + id + ");", null);
                        });
                    }

                    @Override
                    public void onMessage(String message) {
                        runOnUiThread(() -> {
                            String base64 = android.util.Base64.encodeToString(
                                message.getBytes(java.nio.charset.StandardCharsets.UTF_8),
                                android.util.Base64.NO_WRAP
                            );
                            webView.evaluateJavascript(
                                "if (typeof onNativeWebSocketMessageBase64 === 'function') onNativeWebSocketMessageBase64(" + id + ", \"" + base64 + "\");",
                                null
                            );
                        });
                    }

                    @Override
                    public void onClose(int code, String reason) {
                        runOnUiThread(() -> {
                            String escaped = reason != null ? reason.replace("\"", "\\\"") : "";
                            webView.evaluateJavascript(
                                "if (typeof onNativeWebSocketClose === 'function') onNativeWebSocketClose(" + id + ", " + code + ", \"" + escaped + "\");", null);
                        });
                    }

                    @Override
                    public void onError(String error) {
                        runOnUiThread(() -> {
                            String escaped = error != null ? error.replace("\"", "\\\"") : "Unknown error";
                            webView.evaluateJavascript(
                                "if (typeof onNativeWebSocketError === 'function') onNativeWebSocketError(" + id + ", \"" + escaped + "\");", null);
                        });
                    }
                });

                nativeWebSockets.put(id, ws);
                String authKey = prefs.getString(KEY_AUTH_KEY, "");
                ws.connect(url, authKey);
                android.util.Log.i("Clay", "connectWebSocket [" + id + "] " + url);
            });
        }

        @JavascriptInterface
        public void sendWebSocketMessage(int id, String message) {
            NativeWebSocket ws = nativeWebSockets.get(id);
            if (ws != null) ws.send(message);
        }

        @JavascriptInterface
        public void closeWebSocket(int id) {
            NativeWebSocket ws = nativeWebSockets.remove(id);
            if (ws != null) {
                ws.clearCallback();
                ws.close();
                android.util.Log.i("Clay", "closeWebSocket [" + id + "]");
            }
        }

        @JavascriptInterface
        public void closeOtherWebSockets(int winnerId) {
            for (java.util.Map.Entry<Integer, NativeWebSocket> entry : nativeWebSockets.entrySet()) {
                if (entry.getKey() != winnerId) {
                    entry.getValue().clearCallback();
                    entry.getValue().close();
                    android.util.Log.i("Clay", "closeWebSocket [" + entry.getKey() + "] (lost race to " + winnerId + ")");
                }
            }
            nativeWebSockets.entrySet().removeIf(e -> e.getKey() != winnerId);
        }

        @JavascriptInterface
        public boolean hasNativeWebSocket() {
            return true;
        }

        // Called by app.js's connect() before every SSH-mode WS dial (see window.SSH_MODE,
        // injected by buildVarInjectionScript()). Returns true only if the tunnel process is
        // actually alive right now; if it's dead, kicks off restartSshTunnel() (fresh
        // ephemeral port pushed back via updateSshTunnelPort() -> forceReconnect()) and
        // returns false so app.js defers this cycle instead of dialing a dead port. Must stay
        // cheap/non-blocking — isRunning() is just a process.isAlive() check, no TCP probe
        // here (that would risk NetworkOnMainThread / adding latency to every reconnect
        // attempt). restartSshTunnel() is already safe to call off the UI thread (see its
        // other callers: the network-change callback above calls it directly too), so no
        // runOnUiThread hop is needed.
        @JavascriptInterface
        public boolean ensureSshTunnelReady() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            if (!RUN_MODE_REMOTE.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))
                || !prefs.getBoolean(KEY_SSH_ENABLED, false)) {
                return true; // not in SSH mode - nothing to gate
            }
            if (sshProxyManager != null && sshProxyManager.isRunning()) {
                return true;
            }
            android.util.Log.i("Clay", "ensureSshTunnelReady: tunnel down, triggering restart");
            restartSshTunnel(true);
            return false;
        }

        @JavascriptInterface
        public boolean isSettingsConfigured() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            // app.js's connect() gates its entire flow on this before even attempting a
            // WebSocket — local mode needs no remote host/port configuration, so it always
            // counts as configured (same reasoning as the connectWebSocket() gate below).
            if (RUN_MODE_LOCAL.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))) {
                return true;
            }
            return prefs.getBoolean(KEY_SETUP_COMPLETE, false);
        }

        @JavascriptInterface
        public String getConnectionInfo() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            String localHost = prefs.getString(KEY_SERVER_HOST, "192.168.2.6");
            String remoteHost = prefs.getString(KEY_REMOTE_HOSTNAME, "teenymush.dynu.net");
            int port = prefs.getInt(KEY_SERVER_PORT, 9000);
            return "{\"localHost\":\"" + localHost.replace("\"", "") +
                   "\",\"remoteHost\":\"" + remoteHost.replace("\"", "") +
                   "\",\"port\":" + port + "}";
        }

        @JavascriptInterface
        public void saveConnectionSettings(String host, String port, String remoteHostname) {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            SharedPreferences.Editor editor = prefs.edit();
            editor.putString(KEY_SERVER_HOST, host != null ? host : "");
            try { editor.putInt(KEY_SERVER_PORT, Integer.parseInt(port != null ? port.trim() : "9000")); }
            catch (NumberFormatException e) { editor.putInt(KEY_SERVER_PORT, 9000); }
            String remote = remoteHostname != null ? remoteHostname.trim() : "";
            editor.putString(KEY_REMOTE_HOSTNAME, remote);
            editor.putBoolean(KEY_ADVANCED_ENABLED, !remote.isEmpty());
            editor.putBoolean(KEY_SETUP_COMPLETE, true);
            editor.apply();
        }

        @JavascriptInterface
        public void saveThemeCss(String cssVars) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_CACHED_THEME_CSS, cssVars).apply();
        }

        @JavascriptInterface
        public void openExternalUrl(String url) {
            runOnUiThread(() -> {
                try {
                    Intent intent = new Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url));
                    startActivity(intent);
                } catch (Exception e) {
                    android.util.Log.w("Clay", "openExternalUrl failed: " + e.getMessage());
                }
            });
        }

        @JavascriptInterface
        public void reloadPage() {
            runOnUiThread(() -> {
                // Close all WebSocket connections
                for (NativeWebSocket ws : nativeWebSockets.values()) { ws.clearCallback(); ws.close(); }
                nativeWebSockets.clear();
                // Clear cache for a true hard refresh
                webView.clearCache(true);
                // Reload — restarts the local server first if the run mode just changed (see
                // saveSettingsAll()'s 'clay-server' tab in app.js, which calls this after saving),
                // otherwise just a normal reload/resync.
                reloadInterfaceRespectingRunMode();
            });
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        // getNoBackupFilesDir() is never included in any backup (Auto Backup, ADB, OEM).
        // If this flag is absent it is a true fresh install — clear any restored prefs so
        // the first-launch setup page always appears when no real configuration has been done.
        java.io.File installFlag = new java.io.File(getNoBackupFilesDir(), "installed.v1");
        if (!installFlag.exists()) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit().clear().apply();
            try { installFlag.createNewFile(); } catch (java.io.IOException ignored) {}
        }

        // Migrate existing users from old host-presence heuristic to the explicit setup flag.
        // Runs after the install-flag clear, so fresh installs skip this (prefs are empty).
        SharedPreferences migratePrefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        if (!migratePrefs.getBoolean(KEY_SETUP_COMPLETE, false)) {
            String existingHost = migratePrefs.getString(KEY_SERVER_HOST, "");
            if (existingHost != null && !existingHost.isEmpty()) {
                migratePrefs.edit().putBoolean(KEY_SETUP_COMPLETE, true).apply();
            }
        }

        // Create notification channels first
        createNotificationChannel();
        createServiceNotificationChannel();

        webView = findViewById(R.id.webView);
        connectingOverlay = findViewById(R.id.connectingOverlay);
        connectingText = findViewById(R.id.connectingText);
        connectingUrl = findViewById(R.id.connectingUrl);
        connectingCancelBtn = findViewById(R.id.connectingCancelBtn);
        connectingCancelBtn.setOnClickListener(v -> {
            connectCancelled = true;
            hideConnectingOverlay();
            webView.evaluateJavascript(
                "if (typeof openSettingsPopup === 'function') openSettingsPopup('clay-server');",
                null);
        });
        setupWebView();

        // Start permission flow - will call proceedAfterPermissions when done
        isInitialLoadPending = true;
        startPermissionFlow();
    }

    private void startPermissionFlow() {
        // Step 1: Check notification permission
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                    != PackageManager.PERMISSION_GRANTED) {
                // Need to request - callback will continue the flow
                ActivityCompat.requestPermissions(this,
                    new String[]{Manifest.permission.POST_NOTIFICATIONS},
                    NOTIFICATION_PERMISSION_REQUEST);
                return; // Wait for callback
            }
        }
        notificationPermissionDone = true;
        // Battery optimization exemption is no longer requested here - see
        // checkBatteryOptimization()'s doc comment. Startup proceeds straight to loading the
        // interface once notification permission is resolved.
        finishPermissionFlow();
    }

    // Requests exemption from battery optimization (Doze mode), lazily: only called the first
    // time the user actually starts a persistent background connection (from
    // startBackgroundService()), not unconditionally at app startup. `onDone` runs once the
    // exemption is either already granted, just granted/denied by the user, or unsupported on
    // this device - i.e. it's always eventually invoked exactly once per call.
    private void checkBatteryOptimization(Runnable onDone) {
        if (batteryOptimizationDone) {
            onDone.run();
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
            String packageName = getPackageName();
            if (pm != null && !pm.isIgnoringBatteryOptimizations(packageName)) {
                // Need to request - will return via onActivityResult
                Intent intent = new Intent();
                intent.setAction(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS);
                intent.setData(Uri.parse("package:" + packageName));
                try {
                    pendingBatteryOptCallback = onDone;
                    startActivityForResult(intent, BATTERY_OPTIMIZATION_REQUEST);
                    return; // Wait for callback via onActivityResult
                } catch (Exception e) {
                    // Device doesn't support this intent
                    Toast.makeText(this,
                        "Please disable battery optimization for Clay in Settings",
                        Toast.LENGTH_LONG).show();
                }
            }
        }
        batteryOptimizationDone = true;
        onDone.run();
    }

    private void finishPermissionFlow() {
        if (!permissionsHandled) {
            permissionsHandled = true;
            isInitialLoadPending = false;
            proceedAfterPermissions();
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST) {
            notificationPermissionDone = true;
            finishPermissionFlow();
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == BATTERY_OPTIMIZATION_REQUEST) {
            batteryOptimizationDone = true;
            Runnable callback = pendingBatteryOptCallback;
            pendingBatteryOptCallback = null;
            if (callback != null) {
                callback.run();
            }
        }
    }

    // Guards against proceedAfterPermissions() running twice: onResume()/onNewIntent() can fire
    // while it's already in flight (e.g. the notification/battery permission dialogs themselves
    // cycle the Activity through onPause/onResume), which used to race a plain loadInterface()
    // call against the run-mode chooser — loading with stale/default vars and no mode decided
    // yet. checkAndLoadInterface() now routes through this method instead of loadInterface()
    // directly, and this flag makes repeat entry a safe no-op.
    private boolean runModeFlowStarted = false;
    // The run mode actually applied by the last completed proceedAfterPermissions() /
    // startLocalServerThenLoadInterface() / loadInterfaceForRemoteMode(). Compared against the
    // live pref in onNewIntent() (see reloadIfRunModeChanged()) to detect a mode switch made from
    // Settings, since that path reuses this Activity instance rather than a fresh onCreate().
    private String lastAppliedRunMode = null;

    private void proceedAfterPermissions() {
        if (runModeFlowStarted) {
            return;
        }
        runModeFlowStarted = true;
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        if (!prefs.contains(KEY_RUN_MODE)) {
            showRunModeChooser();
        } else if (RUN_MODE_LOCAL.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))) {
            startLocalServerThenLoadInterface();
        } else {
            loadInterfaceForRemoteMode();
        }
    }

    // First-launch chooser: run Clay's own server on this phone, or connect to one running
    // elsewhere. Not cancelable — the app needs an explicit choice before it can proceed.
    // Also reachable later from Settings to change the choice (SettingsActivity).
    private void showRunModeChooser() {
        new AlertDialog.Builder(this)
            .setTitle("How do you want to run Clay?")
            .setMessage("Clay can run entirely on this phone, with no separate server needed, " +
                "or connect to a Clay server running elsewhere. You can change this later in Settings.")
            .setCancelable(false)
            .setPositiveButton("Run on This Phone", (dialog, which) -> {
                getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                    .putString(KEY_RUN_MODE, RUN_MODE_LOCAL).apply();
                startLocalServerThenLoadInterface();
            })
            .setNegativeButton("Connect to a Server", (dialog, which) -> {
                getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                    .putString(KEY_RUN_MODE, RUN_MODE_REMOTE).apply();
                loadInterfaceForRemoteMode();
            })
            .show();
    }

    private void loadInterfaceForRemoteMode() {
        lastAppliedRunMode = RUN_MODE_REMOTE;
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        if (prefs.getBoolean(KEY_SSH_ENABLED, false)) {
            startSshProxyThenLoadInterface();
        } else {
            lastAppliedSshConfigSnapshot = null;
            loadInterface();
        }
    }

    // How many total SSH connect attempts (each attempt = one raceSshProxyStart() call, itself
    // possibly racing two candidate hosts) before giving up and showing showSshFailedDialog().
    // No added delay between attempts - each attempt already has its own internal readiness
    // timeout (SshProxyManager.READY_TIMEOUT_MS), so a genuinely unreachable host still takes a
    // while overall, but nothing artificial is added on top of that.
    private static final int SSH_MAX_ATTEMPTS = 3;

    // Establishes the SSH tunnel (spawn + readiness poll, both blocking) on a worker thread,
    // then loads the WebView pointed at the local proxy port — mirrors
    // startLocalServerThenLoadInterface()'s structure exactly; buildVarInjectionScript() reads
    // sshProxyManager's local port once it's running, so the WebView must not load before this
    // completes. When "Remote Hostname" (the same Advanced field the direct/non-SSH path races
    // against "Server Host") is set and differs from "Server Host", races an SSH tunnel to BOTH
    // simultaneously via raceSshProxyStart() and keeps whichever comes up first — mirroring the
    // direct path's parallel host race in app.js (buildCandidates()/connect()/handleAttemptWin()).
    // Retries up to SSH_MAX_ATTEMPTS times; on total failure, does NOT fall back to a direct
    // connection (SSH being enabled means only SSH is ever attempted) - instead shows
    // showSshFailedDialog() with a real error and Retry/Cancel actions.
    private void startSshProxyThenLoadInterface() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        lastAppliedSshConfigSnapshot = sshConfigSnapshot(prefs);
        final String sshUser = prefs.getString(KEY_SSH_USER, "");
        final String sshHost = prefs.getString(KEY_SERVER_HOST, "");
        final String sshRemoteHost = prefs.getString(KEY_REMOTE_HOSTNAME, "");
        final int sshPort = prefs.getInt(KEY_SSH_PORT, 22);
        final int clayPort = prefs.getInt(KEY_SERVER_PORT, 9000);
        final String privateKeyPem = prefs.getString(KEY_SSH_PRIVATE_KEY, "");
        final String keyPassphrase = prefs.getString(KEY_SSH_KEY_PASSPHRASE, "");
        final String password = prefs.getString(KEY_SSH_PASSWORD, "");
        runOnUiThread(() -> showConnectingOverlay("Establishing SSH tunnel..."));
        new Thread(() -> {
            SshProxyManager winner = null;
            SshRaceResult lastResult = null;
            for (int attempt = 1; attempt <= SSH_MAX_ATTEMPTS && winner == null; attempt++) {
                lastResult = raceSshProxyStart(sshUser, sshHost, sshRemoteHost, sshPort,
                    clayPort, privateKeyPem, keyPassphrase, password);
                winner = lastResult.winner;
                if (winner == null) {
                    android.util.Log.w("Clay", "SSH connect attempt " + attempt + "/"
                        + SSH_MAX_ATTEMPTS + " failed: " + lastResult.errors);
                }
            }
            if (winner != null) {
                sshProxyManager = winner;
                android.util.Log.i("Clay", "SSH proxy ready on port " + winner.getLocalPort());
                runOnUiThread(this::loadInterface);
            } else {
                sshProxyManager = null;
                final String message = summarizeSshErrors(lastResult.errors);
                runOnUiThread(() -> {
                    hideConnectingOverlay();
                    showSshFailedDialog(message);
                });
            }
        }, "ClaySshProxyStart").start();
    }

    /** Winning manager (or null) plus per-candidate error strings from a raceSshProxyStart() call. */
    private static class SshRaceResult {
        final SshProxyManager winner;
        final java.util.List<String> errors;
        SshRaceResult(SshProxyManager winner, java.util.List<String> errors) {
            this.winner = winner;
            this.errors = errors;
        }
    }

    /**
     * Races SSH tunnel startup against up to two candidate hosts - "Server Host" and, when it's
     * set and differs, "Remote Hostname" - and returns whichever SshProxyManager comes up first,
     * having told the other to cancel(); on total failure, includes each candidate's
     * SshProxyManager.getLastError() for showSshFailedDialog(). Blocking - must be called off the
     * main thread (each SshProxyManager.start() call blocks internally).
     */
    private SshRaceResult raceSshProxyStart(String user, String hostA, String hostB, int sshPort,
                                             int clayPort, String key, String keyPass, String password) {
        boolean hasSecondCandidate = hostB != null && !hostB.isEmpty() && !hostB.equals(hostA);
        if (!hasSecondCandidate) {
            // No race needed - reuse the existing manager exactly as before this change (a
            // still-running instance with a matching target is a no-op inside start()).
            if (sshProxyManager == null) {
                sshProxyManager = new SshProxyManager(this);
            }
            SshProxyManager manager = sshProxyManager;
            boolean ok = manager.start(user, hostA, sshPort, clayPort, key, keyPass, password);
            if (ok) {
                return new SshRaceResult(manager, null);
            }
            return new SshRaceResult(null,
                java.util.Collections.singletonList(hostA + ": " + manager.getLastError()));
        }

        SshProxyManager mgrA = new SshProxyManager(this);
        SshProxyManager mgrB = new SshProxyManager(this);
        AtomicReference<SshProxyManager> winnerRef = new AtomicReference<>();
        CountDownLatch latch = new CountDownLatch(2);
        final String[] errorA = new String[1];
        final String[] errorB = new String[1];

        Runnable attemptA = () -> {
            try {
                if (mgrA.start(user, hostA, sshPort, clayPort, key, keyPass, password)) {
                    if (winnerRef.compareAndSet(null, mgrA)) {
                        mgrB.cancel();
                    } else {
                        mgrA.stop(); // lost a near-simultaneous tie, don't leak it
                    }
                } else {
                    errorA[0] = mgrA.getLastError();
                }
            } finally {
                latch.countDown();
            }
        };
        Runnable attemptB = () -> {
            try {
                if (mgrB.start(user, hostB, sshPort, clayPort, key, keyPass, password)) {
                    if (winnerRef.compareAndSet(null, mgrB)) {
                        mgrA.cancel();
                    } else {
                        mgrB.stop();
                    }
                } else {
                    errorB[0] = mgrB.getLastError();
                }
            } finally {
                latch.countDown();
            }
        };
        new Thread(attemptA, "ClaySshProxyRaceA").start();
        new Thread(attemptB, "ClaySshProxyRaceB").start();
        try {
            // Defensive bound only - start() itself can't block longer than its own
            // READY_TIMEOUT_MS per candidate (a hung/unreachable connect just times out and
            // self-stops), so both threads should always finish well within this.
            latch.await(15, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
        SshProxyManager winner = winnerRef.get();
        if (winner != null) {
            return new SshRaceResult(winner, null);
        }
        java.util.List<String> errors = new java.util.ArrayList<>();
        errors.add(hostA + ": " + (errorA[0] != null ? errorA[0] : "unknown"));
        errors.add(hostB + ": " + (errorB[0] != null ? errorB[0] : "unknown"));
        return new SshRaceResult(null, errors);
    }

    /** Joins a raceSshProxyStart() failure's per-candidate errors into one dialog message. */
    private String summarizeSshErrors(java.util.List<String> errors) {
        if (errors == null || errors.isEmpty()) {
            return "SSH connection failed.";
        }
        return "Could not establish an SSH connection after " + SSH_MAX_ATTEMPTS + " attempts:\n\n"
            + String.join("\n\n", errors);
    }

    // Shown after every SSH attempt (all candidates, all SSH_MAX_ATTEMPTS retries) has failed.
    // Deliberately does NOT offer a "connect directly instead" option - SSH being enabled means
    // only SSH is ever attempted, matching the rest of this method's callers. Not cancelable
    // (back button) so the app doesn't end up in a half-connected, silently-blank state.
    private void showSshFailedDialog(String message) {
        new AlertDialog.Builder(this)
            .setTitle("SSH Connection Failed")
            .setMessage(message)
            .setCancelable(false)
            .setPositiveButton("Retry", (dialog, which) -> startSshProxyThenLoadInterface())
            .setNegativeButton("Cancel", (dialog, which) ->
                startActivity(new Intent(this, SettingsActivity.class)))
            .show();
    }

    // --- SSH tunnel self-heal (network change / resume watchdog) ---
    //
    // The initial-connect flow above (startSshProxyThenLoadInterface/raceSshProxyStart) only
    // ever runs once, at launch or after a settings change. Nothing previously re-checked the
    // tunnel afterward, so a network change, sleep, or the remote SSH session simply dying left
    // the WebView redialing a dead 127.0.0.1:<port> forever with no way to notice or recover -
    // see ssh.rs's run_ssh_proxy_mode, which now exits the proxy process when a forward attempt
    // fails, making SshProxyManager.isRunning() an honest signal for the checks below.

    // Registered in onResume(), unregistered in onPause() - only while in SSH remote mode.
    // onAvailable() fires when the system's default network changes (e.g. WiFi->cellular
    // handoff, or reconnecting after being fully offline) and is itself strong enough evidence
    // that the old tunnel is stale to restart unconditionally, without waiting to see whether
    // isRunning() has caught up yet (it might not have: nothing has necessarily tried to use the
    // tunnel since the network changed, so the proxy process may not have noticed and exited).
    private void registerSshNetworkCallbackIfNeeded() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        boolean sshActive = RUN_MODE_REMOTE.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))
            && prefs.getBoolean(KEY_SSH_ENABLED, false);
        if (!sshActive) {
            return;
        }
        ConnectivityManager cm = (ConnectivityManager) getSystemService(Context.CONNECTIVITY_SERVICE);
        if (cm == null) {
            return;
        }
        sshNetworkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                android.util.Log.i("Clay", "Default network changed - restarting SSH tunnel");
                restartSshTunnel(true);
            }
            // onLost() deliberately does nothing - there's nothing useful to reconnect to until
            // some network becomes the default again, which is what onAvailable() is for.
        };
        try {
            cm.registerDefaultNetworkCallback(sshNetworkCallback);
        } catch (RuntimeException e) {
            android.util.Log.w("Clay", "Could not register network callback", e);
            sshNetworkCallback = null;
        }
    }

    private void unregisterSshNetworkCallback() {
        if (sshNetworkCallback == null) {
            return;
        }
        ConnectivityManager cm = (ConnectivityManager) getSystemService(Context.CONNECTIVITY_SERVICE);
        if (cm != null) {
            try {
                cm.unregisterNetworkCallback(sshNetworkCallback);
            } catch (RuntimeException e) {
                // Already unregistered, or registration itself never actually succeeded - fine.
            }
        }
        sshNetworkCallback = null;
    }

    /**
     * Checks/restarts the SSH tunnel outside the normal user-initiated connect flow.
     * unconditional=true (network change): restart regardless of isRunning() - a network change
     * is strong enough evidence the old tunnel is stale even if the proxy process hasn't
     * technically exited yet. unconditional=false (app resume): only restart if isRunning() is
     * already false - avoids unnecessarily tearing down a tunnel that's still perfectly fine
     * just because the app came back to the foreground.
     *
     * Deliberately a single attempt, not the 3x retry loop startSshProxyThenLoadInterface() uses
     * for the user-visible initial connect - this is a background self-heal; if it fails, the
     * next network-change or resume event tries again, avoiding a tight retry loop against a
     * still-unreachable remote. Silent either way (Toast on success only, log-only on failure)
     * per the initial-connect dialog being reserved for that user-visible flow.
     */
    private void restartSshTunnel(boolean unconditional) {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        if (!RUN_MODE_REMOTE.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))
            || !prefs.getBoolean(KEY_SSH_ENABLED, false)) {
            return; // not in SSH mode - nothing to watch/restart
        }
        if (sshProxyManager == null) {
            return; // never started (still on the first-connect flow) - nothing to restart
        }
        if (!unconditional && sshProxyManager.isRunning()) {
            return; // still healthy
        }
        long now = System.currentTimeMillis();
        if (now - lastSshTunnelRestartAt < 2000) {
            android.util.Log.i("Clay", "SSH tunnel restart debounced");
            return;
        }
        lastSshTunnelRestartAt = now;

        final String sshUser = prefs.getString(KEY_SSH_USER, "");
        final String sshHost = prefs.getString(KEY_SERVER_HOST, "");
        final String sshRemoteHost = prefs.getString(KEY_REMOTE_HOSTNAME, "");
        final int sshPort = prefs.getInt(KEY_SSH_PORT, 22);
        final int clayPort = prefs.getInt(KEY_SERVER_PORT, 9000);
        final String privateKeyPem = prefs.getString(KEY_SSH_PRIVATE_KEY, "");
        final String keyPassphrase = prefs.getString(KEY_SSH_KEY_PASSPHRASE, "");
        final String password = prefs.getString(KEY_SSH_PASSWORD, "");

        android.util.Log.i("Clay", "SSH tunnel watchdog: restarting (unconditional=" + unconditional + ")");
        // Tear down whatever's there first - required for the unconditional case (the manager
        // may still report isRunning()==true, and raceSshProxyStart()'s single-candidate path
        // would otherwise short-circuit via SshProxyManager.start()'s own "already running"
        // check and skip restarting entirely) and harmless/idempotent otherwise.
        sshProxyManager.stop();

        new Thread(() -> {
            SshRaceResult result = raceSshProxyStart(sshUser, sshHost, sshRemoteHost, sshPort,
                clayPort, privateKeyPem, keyPassphrase, password);
            if (result.winner != null) {
                sshProxyManager = result.winner;
                final int newPort = result.winner.getLocalPort();
                android.util.Log.i("Clay", "SSH tunnel watchdog: restarted OK on port " + newPort);
                runOnUiThread(() -> {
                    Toast.makeText(this, "SSH tunnel reconnected", Toast.LENGTH_SHORT).show();
                    if (webView != null) {
                        webView.evaluateJavascript(
                            "if (typeof updateSshTunnelPort === 'function') updateSshTunnelPort(" + newPort + ");",
                            null);
                    }
                });
            } else {
                android.util.Log.w("Clay", "SSH tunnel watchdog: restart failed: " + result.errors);
                // Silent - no dialog/toast for a background self-heal failure (that's reserved
                // for the initial-connect flow); the next network-change or resume event retries.
            }
        }, "ClaySshWatchdogRestart").start();
    }

    // Fingerprint of everything that determines the running SshProxyManager's target/creds —
    // used by reloadInterfaceRespectingRunMode() to detect a settings change that requires
    // killing and restarting the tunnel (unlike plain remote mode, where a host/port change
    // just needs a normal WS reconnect with fresh vars — the SSH tunnel is a subprocess that
    // can't be redirected without a fresh --target=/env). Includes KEY_REMOTE_HOSTNAME since
    // raceSshProxyStart() now races it as a second SSH candidate alongside KEY_SERVER_HOST.
    private String sshConfigSnapshot(SharedPreferences prefs) {
        return prefs.getString(KEY_SSH_USER, "") + "|" + prefs.getString(KEY_SERVER_HOST, "") + "|"
            + prefs.getString(KEY_REMOTE_HOSTNAME, "") + "|"
            + prefs.getInt(KEY_SSH_PORT, 22) + "|" + prefs.getInt(KEY_SERVER_PORT, 9000) + "|"
            + prefs.getString(KEY_SSH_PRIVATE_KEY, "") + "|" + prefs.getString(KEY_SSH_KEY_PASSPHRASE, "") + "|"
            + prefs.getString(KEY_SSH_PASSWORD, "");
    }

    // Starts the bundled Clay server (spawn + readiness poll, both blocking) on a worker thread,
    // then loads the WebView — buildVarInjectionScript() reads localServerManager's port/password
    // once it's running, so the WebView must not load (and race to connect) before this
    // completes. Proceeds to loadInterface() either way; if the server failed to start, app.js
    // will simply fail to connect, the same UX as an unreachable remote host.
    private void startLocalServerThenLoadInterface() {
        lastAppliedRunMode = RUN_MODE_LOCAL;
        if (localServerManager == null) {
            localServerManager = new LocalServerManager(this);
        }
        final LocalServerManager manager = localServerManager;
        runOnUiThread(() -> showConnectingOverlay("Starting local server..."));
        new Thread(() -> {
            boolean ready = manager.start();
            android.util.Log.i("Clay", "Local server " + (ready ? "ready" : "FAILED to start")
                + " on port " + manager.getPort());
            runOnUiThread(this::loadInterface);
        }, "ClayLocalServerStart").start();
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID_ALERTS,
                "Clay Alerts",
                NotificationManager.IMPORTANCE_HIGH
            );
            channel.setDescription("Notifications from Clay MUD client");

            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    private void createServiceNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID_SERVICE,
                "Clay Service",
                NotificationManager.IMPORTANCE_LOW
            );
            channel.setDescription("Keeps Clay connected in the background");
            channel.setShowBadge(false);  // Don't show badge for service notification

            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    private void startKeepalive() {
        if (keepaliveHandler == null) {
            keepaliveHandler = new Handler(Looper.getMainLooper());
        }

        keepaliveRunnable = new Runnable() {
            @Override
            public void run() {
                if (isConnected && webView != null) {
                    // Ping the JavaScript to keep the WebSocket alive
                    // This also wakes up the WebView if it was suspended
                    webView.evaluateJavascript(
                        "if (typeof keepalivePing === 'function') { keepalivePing(); Android.keepaliveAck(); }",
                        null
                    );
                    keepaliveHandler.postDelayed(this, KEEPALIVE_INTERVAL_MS);
                }
            }
        };

        keepaliveHandler.postDelayed(keepaliveRunnable, KEEPALIVE_INTERVAL_MS);
    }

    private void stopKeepalive() {
        if (keepaliveHandler != null && keepaliveRunnable != null) {
            keepaliveHandler.removeCallbacks(keepaliveRunnable);
            keepaliveRunnable = null;
        }
    }

    private void startHeartbeat() {
        if (heartbeatHandler == null) {
            heartbeatHandler = new Handler(Looper.getMainLooper());
        }

        missedHeartbeats = 0;
        heartbeatRunnable = new Runnable() {
            @Override
            public void run() {
                if (isConnected && webView != null) {
                    webView.evaluateJavascript(
                        "typeof heartbeatAck === 'function' && heartbeatAck()",
                        value -> {
                            if (value != null && value.contains("ok")) {
                                missedHeartbeats = 0;
                            } else {
                                missedHeartbeats++;
                                if (missedHeartbeats >= 2) {
                                    android.util.Log.w("Clay", "WebView unresponsive (" + missedHeartbeats + " missed heartbeats), triggering resync");
                                    missedHeartbeats = 0;
                                    webView.evaluateJavascript(
                                        "if (typeof triggerResync === 'function') triggerResync();",
                                        null
                                    );
                                }
                            }
                        }
                    );
                    heartbeatHandler.postDelayed(this, HEARTBEAT_INTERVAL_MS);
                }
            }
        };

        heartbeatHandler.postDelayed(heartbeatRunnable, HEARTBEAT_INTERVAL_MS);
    }

    private void stopHeartbeat() {
        if (heartbeatHandler != null && heartbeatRunnable != null) {
            heartbeatHandler.removeCallbacks(heartbeatRunnable);
            heartbeatRunnable = null;
        }
    }

    private void startBackgroundShutdownTimer() {
        // Only start timer if connected - no point otherwise
        if (!isConnected) {
            return;
        }

        if (backgroundShutdownHandler == null) {
            backgroundShutdownHandler = new Handler(Looper.getMainLooper());
        }

        // Cancel any existing timer
        cancelBackgroundShutdownTimer();

        backgroundShutdownRunnable = new Runnable() {
            @Override
            public void run() {
                if (isConnected) {
                    android.util.Log.i("Clay", "Background timeout reached (1 hour), disconnecting to save power");
                    // Stop the foreground service and disconnect
                    isConnected = false;
                    stopKeepalive();
                    stopHeartbeat();

                    Intent serviceIntent = new Intent(MainActivity.this, ClayForegroundService.class);
                    stopService(serviceIntent);

                    // Close all WebSocket connections
                    for (NativeWebSocket ws : nativeWebSockets.values()) { ws.clearCallback(); ws.close(); }
                    nativeWebSockets.clear();

                    // Notify JavaScript that we disconnected due to timeout
                    if (webView != null) {
                        webView.evaluateJavascript(
                            "if (typeof onBackgroundTimeout === 'function') onBackgroundTimeout();",
                            null
                        );
                    }
                }
            }
        };

        backgroundShutdownHandler.postDelayed(backgroundShutdownRunnable, BACKGROUND_SHUTDOWN_MS);
        android.util.Log.i("Clay", "Background shutdown timer started (1 hour)");
    }

    private void cancelBackgroundShutdownTimer() {
        if (backgroundShutdownHandler != null && backgroundShutdownRunnable != null) {
            backgroundShutdownHandler.removeCallbacks(backgroundShutdownRunnable);
            backgroundShutdownRunnable = null;
            android.util.Log.i("Clay", "Background shutdown timer cancelled");
        }
    }

    private void setupWebView() {
        WebSettings webSettings = webView.getSettings();
        webSettings.setJavaScriptEnabled(true);
        webSettings.setDomStorageEnabled(true);
        webSettings.setMixedContentMode(WebSettings.MIXED_CONTENT_ALWAYS_ALLOW);

        // Add JavaScript interface for Android communication
        webView.addJavascriptInterface(new AndroidInterface(), "Android");

        final MainActivity activity = this;

        webView.setWebViewClient(new WebViewClient() {
            @Override
            public android.webkit.WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                String url = request.getUrl().toString();

                // Intercept HTTPS requests to handle certificate issues.
                // Skip well-known CDNs (fonts, etc.) — let WebView handle them natively.
                if (url.startsWith("https://") &&
                    !url.contains("fonts.googleapis.com") &&
                    !url.contains("fonts.gstatic.com")) {
                    Exception lastException = null;
                    // Retry up to 3 times for transient connection failures
                    for (int attempt = 1; attempt <= 3; attempt++) {
                        // Create fresh client for each attempt to avoid connection reuse issues
                        okhttp3.OkHttpClient freshClient = activity.createHttpsInterceptClient();
                        try {
                            okhttp3.Request okRequest = new okhttp3.Request.Builder()
                                .url(url)
                                .build();
                            okhttp3.Response response = freshClient.newCall(okRequest).execute();

                            if (!response.isSuccessful()) {
                                final String errMsg = "HTTP " + response.code() + ": " + url;
                                android.util.Log.e("Clay", errMsg);
                                response.close();
                                // Return error response instead of falling back
                                return new android.webkit.WebResourceResponse(
                                    "text/plain", "UTF-8",
                                    response.code(), response.message(),
                                    new java.util.HashMap<>(),
                                    new java.io.ByteArrayInputStream(errMsg.getBytes())
                                );
                            }

                            String contentType = response.header("Content-Type", "text/html");
                            String mimeType = contentType.split(";")[0].trim();
                            String encoding = "UTF-8";

                            if (contentType.contains("charset=")) {
                                encoding = contentType.split("charset=")[1].trim();
                            }

                            byte[] bodyBytes = response.body().bytes();
                            response.close();
                            String shortUrl = url.length() > 40 ? "..." + url.substring(url.length() - 37) : url;
                            android.util.Log.d("Clay", "OK " + shortUrl + " (" + bodyBytes.length + "b, attempt " + attempt + ")");

                            return new android.webkit.WebResourceResponse(
                                mimeType,
                                encoding,
                                200, "OK",
                                new java.util.HashMap<>(),
                                new java.io.ByteArrayInputStream(bodyBytes)
                            );
                        } catch (Exception e) {
                            lastException = e;
                            android.util.Log.w("Clay", "Attempt " + attempt + " failed for " + url + ": " + e.getMessage());
                            if (attempt < 3) {
                                try { Thread.sleep(500 * attempt); } catch (InterruptedException ie) { }
                            }
                        }
                    }
                    // All retries failed
                    final String errMsg = "Failed after 3 attempts: " + (lastException != null ? lastException.getMessage() : "unknown");
                    android.util.Log.e("Clay", errMsg);
                    return new android.webkit.WebResourceResponse(
                        "text/plain", "UTF-8",
                        500, "Error",
                        new java.util.HashMap<>(),
                        new java.io.ByteArrayInputStream(errMsg.getBytes())
                    );
                }
                return super.shouldInterceptRequest(view, request);
            }

            @Override
            public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                super.onReceivedError(view, request, error);
                String msg = "Error " + error.getErrorCode() + " on " + request.getUrl();
                android.util.Log.w("Clay", msg + " main=" + request.isForMainFrame());
                if (request.isForMainFrame()) {
                    connectionFailed = true;
                    runOnUiThread(() -> {
                        hideConnectingOverlay();
                        loadInterface();
                    });
                }
            }

            @Override
            public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) {
                // Accept all SSL errors (expired, hostname mismatch, untrusted, etc.)
                // This allows the app to work even with certificate issues
                android.util.Log.w("Clay", "SSL error " + error.getPrimaryError() + " on " + error.getUrl() + " - proceeding");
                handler.proceed();
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                connectionFailed = false;
                hideConnectingOverlay();
                android.util.Log.i("Clay", "Page loaded: " + url);
                // Inject template variables that were NOT substituted at load time (loadUrl path)
                String varScript = buildVarInjectionScript();
                view.evaluateJavascript(varScript, unused -> {
                    // connect() AFTER vars are injected so buildCandidates() sees them
                    view.postDelayed(() -> view.evaluateJavascript(
                        "if (typeof connect === 'function') connect();", null), 300);
                });
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                String url = request.getUrl().toString();

                // Check if this is the Clay server URL
                if (loadedInterfaceUrl != null) {
                    try {
                        java.net.URI serverUri = new java.net.URI(loadedInterfaceUrl);
                        java.net.URI requestUri = new java.net.URI(url);

                        // If same host and port, let WebView handle it
                        if (serverUri.getHost().equals(requestUri.getHost()) &&
                            serverUri.getPort() == requestUri.getPort()) {
                            return false;
                        }
                    } catch (Exception e) {
                        android.util.Log.w("Clay", "Error parsing URL: " + e.getMessage());
                    }
                }

                // External URL - open in default browser
                android.util.Log.i("Clay", "Opening external URL: " + url);
                Intent intent = new Intent(Intent.ACTION_VIEW, android.net.Uri.parse(url));
                startActivity(intent);
                return true;
            }
        });

        webView.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(ConsoleMessage cm) {
                android.util.Log.i("ClayJS",
                    cm.message() + " @ " + cm.sourceId() + ":" + cm.lineNumber());
                return true;
            }
        });
    }

    /** Load the web interface from bundled APK assets. Template vars injected via JS in onPageFinished. */
    private void loadInterface() {
        connectionFailed = false;
        runOnUiThread(() -> {
            webView.loadUrl("file:///android_asset/web/index.html");
            interfaceLoaded = true;
            loadedInterfaceUrl = "file:///android_asset/web/index.html";
        });
    }

    /** Build a JS snippet that sets window.WS_* vars and applies the saved theme. */
    private String buildVarInjectionScript() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        boolean isLocalMode = RUN_MODE_LOCAL.equals(prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE))
            && localServerManager != null && localServerManager.isRunning();
        boolean isSshProxyMode = !isLocalMode && prefs.getBoolean(KEY_SSH_ENABLED, false)
            && sshProxyManager != null && sshProxyManager.isRunning();

        String localHost;
        String remoteHost;
        int port;
        String mode;
        String autoPasswordScript;
        String webPathScript;
        if (isLocalMode) {
            localHost = jsStr("127.0.0.1");
            remoteHost = jsStr("");  // no remote candidate to race against in local mode
            port = localServerManager.getPort();
            mode = jsStr("non_secure");  // plain ws only — local server is loopback, unencrypted
            autoPasswordScript = "window.AUTO_PASSWORD=" + jsStr(localServerManager.getPasswordHash()) + ";";
            // Leave window.WEB_PATH unset (not the possibly-stale value saved from a previous
            // remote server) — the local server always uses the default "clay" web_path, and
            // wsPathCandidates() already probes /clay/ws then /ws when WEB_PATH is unset.
            webPathScript = "";
        } else if (isSshProxyMode) {
            // The WebView connects to the local SSH-forwarding proxy, not the real remote host
            // directly — identical shape to local mode (loopback, unencrypted locally, since SSH
            // already encrypts everything up to that point; no remote candidate to race
            // against). Unlike local mode, NO auto-password: Clay's own WS password/auth-key is
            // still required over the tunnel (nothing about that authentication changes), so the
            // normal saved-password/auth-key flow in connectWebSocket() applies exactly as it
            // would for a direct remote connection.
            localHost = jsStr("127.0.0.1");
            remoteHost = jsStr("");
            port = sshProxyManager.getLocalPort();
            mode = jsStr("non_secure");
            autoPasswordScript = "";
            webPathScript = "";
        } else {
            localHost = jsStr(prefs.getString(KEY_SERVER_HOST, ""));
            remoteHost = jsStr(prefs.getString(KEY_REMOTE_HOSTNAME, ""));
            port = prefs.getInt(KEY_SERVER_PORT, 9000);
            mode = jsStr(prefs.getString("connectionMode", "auto"));
            autoPasswordScript = "";
            String webPath = prefs.getString(KEY_WEB_PATH, "");
            // Only inject window.WEB_PATH when we have a known non-empty value. Emitting an
            // empty string would mean "legacy mode" and wrongly disable the Android probe
            // path in wsPathCandidates(); leaving it unset (the bundled asset's raw
            // '{{WEB_PATH}}' placeholder) is correctly treated as "unset" by injectedWebPath()
            // in app.js, which engages the /clay/ws then /ws probe.
            webPathScript = webPath.isEmpty() ? "" : "window.WEB_PATH=" + jsStr(webPath) + ";";
        }
        String theme = prefs.getString(KEY_CACHED_THEME_CSS, DEFAULT_THEME_CSS)
                            .replace("\\", "\\\\").replace("'", "\\'")
                            .replace("\r", "").replace("\n", " ");
        return "window.WS_HOST=" + localHost + ";" +
               "window.WS_LOCAL_HOST=" + localHost + ";" +
               "window.WS_REMOTE_HOST=" + remoteHost + ";" +
               "window.WS_PORT=" + port + ";" +
               "window.WS_PROTOCOL='wss';" +
               "window.CONNECTION_MODE=" + mode + ";" +
               // Tells app.js's connect() to gate each dial on Android.ensureSshTunnelReady()
               // before opening a socket to the tunneled loopback port — explicit boolean
               // (not just inferred from CONNECTION_MODE/host) so a stale true never survives
               // a switch back to local/remote mode across reloads.
               "window.SSH_MODE=" + isSshProxyMode + ";" +
               "window.SHOW_CONNECTION_WINDOW=true;" +
               webPathScript +
               autoPasswordScript +
               "(function(){var s=document.getElementById('theme-vars');" +
               "if(s)s.textContent=':root{" + theme + "}';}());";
    }

    /** Wrap a string as a JS single-quoted literal with minimal escaping. */
    private static String jsStr(String s) {
        if (s == null) s = "";
        return "'" + s.replace("\\", "\\\\").replace("'", "\\'") + "'";
    }

    private void showConnectingOverlay(String urlText) {
        connectingOverlay.setVisibility(android.view.View.VISIBLE);
        connectingText.setText("Connecting...");
        connectingUrl.setText(urlText);
        webView.setVisibility(android.view.View.INVISIBLE);
    }

    private void hideConnectingOverlay() {
        connectingOverlay.setVisibility(android.view.View.GONE);
        webView.setVisibility(android.view.View.VISIBLE);
    }

    /**
     * Creates an OkHttpClient for intercepting arbitrary third-party HTTPS requests the
     * WebView's JS makes (e.g. GMCP media URLs a MUD server sends — see
     * shouldInterceptRequest() below). Unlike NativeWebSocket's connection to a
     * user-configured Clay server, this fetches arbitrary internet content picked by
     * whatever server the user connected to, so it deliberately uses standard platform CA-chain
     * + hostname validation rather than trust-on-first-use pinning or accept-all: there is no
     * single "known host" relationship here to pin against, and blindly trusting self-signed
     * certificates for arbitrary third-party URLs would let a malicious server point this at an
     * internal/spoofed HTTPS endpoint. Normal websites' CDNs/image hosts already use CA-signed
     * certificates, so this should not affect legitimate content.
     */
    private okhttp3.OkHttpClient createHttpsInterceptClient() {
        // Disable connection pooling to avoid stale connection issues across retries.
        return new okhttp3.OkHttpClient.Builder()
            .connectTimeout(15, java.util.concurrent.TimeUnit.SECONDS)
            .readTimeout(30, java.util.concurrent.TimeUnit.SECONDS)
            .writeTimeout(15, java.util.concurrent.TimeUnit.SECONDS)
            .connectionPool(new okhttp3.ConnectionPool(0, 1, java.util.concurrent.TimeUnit.MILLISECONDS)) // No pooling
            .retryOnConnectionFailure(true)
            .build();
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // Called when activity is brought to front via FLAG_ACTIVITY_CLEAR_TOP — notably this is
        // how SettingsActivity returns after saveAndConnect(), reusing this same instance rather
        // than a fresh onCreate().
        android.util.Log.i("Clay", "onNewIntent called, checking if interface needs loading");
        if (interfaceLoaded) {
            reloadInterfaceRespectingRunMode();
        } else {
            checkAndLoadInterface();
        }
    }

    // Reloads the WebView. If the run mode changed since it was last applied, or the SSH tunnel
    // target/creds changed while still in use (from either SettingsActivity or the web settings
    // popup's "clay-server" tab — see reloadPage() below), tears down the old local server/SSH
    // proxy/WebSockets first and re-enters the run-mode decision fresh via
    // proceedAfterPermissions(). Otherwise this is just a normal reload (e.g. a manual resync,
    // or an unrelated remote-settings change) — the fast path, since restarting an
    // already-running local server or SSH tunnel would lose its in-memory world state / drop
    // the session for no reason.
    private void reloadInterfaceRespectingRunMode() {
        interfaceLoaded = false;
        loadedInterfaceUrl = null;
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String currentMode = prefs.getString(KEY_RUN_MODE, RUN_MODE_REMOTE);
        boolean modeChanged = lastAppliedRunMode != null && !currentMode.equals(lastAppliedRunMode);

        boolean sshEnabledNow = RUN_MODE_REMOTE.equals(currentMode) && prefs.getBoolean(KEY_SSH_ENABLED, false);
        String currentSshSnapshot = sshEnabledNow ? sshConfigSnapshot(prefs) : null;
        boolean sshChanged = lastAppliedSshConfigSnapshot != null
            ? !lastAppliedSshConfigSnapshot.equals(currentSshSnapshot)
            : currentSshSnapshot != null;

        if (modeChanged || sshChanged) {
            android.util.Log.i("Clay", "Run mode or SSH config changed, reloading");
            if (localServerManager != null) {
                localServerManager.stop();
            }
            if (sshProxyManager != null) {
                sshProxyManager.stop();
            }
            runModeFlowStarted = false;
            proceedAfterPermissions();
        } else {
            loadInterface();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();

        // Cancel background shutdown timer since user is back
        cancelBackgroundShutdownTimer();

        // Watch for network changes (WiFi<->cellular etc.) while in the foreground, and check
        // whether the SSH tunnel survived whatever happened while we were paused/asleep - both
        // feed into restartSshTunnel(). See that method and the field doc on sshNetworkCallback.
        registerSshNetworkCallbackIfNeeded();
        restartSshTunnel(false);

        // Always resume WebView timers/JS execution (may have been paused in onPause)
        if (webView != null) {
            webView.onResume();
        }

        // Don't interfere if initial delayed load is pending
        if (isInitialLoadPending) {
            return;
        }

        checkAndLoadInterface();
    }

    private void checkAndLoadInterface() {
        if (!interfaceLoaded) {
            android.util.Log.i("Clay", "Loading interface: not yet loaded");
            // Route through the run-mode decision (chooser / local / remote), not loadInterface()
            // directly — runModeFlowStarted makes this a no-op if that's already in flight, and
            // a real fallback if this is somehow the first thing to reach it.
            proceedAfterPermissions();
        } else if (webView != null) {
            // Interface loaded - always verify connection health on resume.
            missedHeartbeats = 0;
            webView.evaluateJavascript(
                "if (typeof checkConnectionOnResume === 'function') checkConnectionOnResume();",
                null
            );
        }
    }

    @Override
    protected void onPause() {
        super.onPause();
        unregisterSshNetworkCallback();
        // Don't pause WebView if connected - we want to keep receiving notifications
        // The WebView will continue running in the background with the foreground service
        if (!isConnected && webView != null) {
            webView.onPause();
        }
    }

    @Override
    protected void onStop() {
        super.onStop();
        // WebView continues running if connected (foreground service keeps process alive)
        // Start background shutdown timer to save power after 1 hour
        startBackgroundShutdownTimer();
    }

    @Override
    protected void onDestroy() {
        stopKeepalive();
        stopHeartbeat();
        cancelBackgroundShutdownTimer();
        unregisterSshNetworkCallback(); // normally already done in onPause(); safe/idempotent
        if (localServerManager != null) {
            localServerManager.stop();
        }
        if (sshProxyManager != null) {
            sshProxyManager.stop();
        }
        super.onDestroy();
    }

    @Override
    public void onBackPressed() {
        if (webView.canGoBack()) {
            webView.goBack();
        } else {
            super.onBackPressed();
        }
    }
}
