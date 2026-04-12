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

public class MainActivity extends AppCompatActivity {
    private static final String PREFS_NAME = "ClayPrefs";
    private static final String KEY_SERVER_HOST = "serverHost";
    private static final String KEY_SERVER_PORT = "serverPort";
    private static final String KEY_USE_SECURE = "useSecure";
    private static final String KEY_SAVED_PASSWORD = "savedPassword";
    private static final String KEY_SAVED_USERNAME = "savedUsername";
    private static final String KEY_AUTH_KEY = "authKey";  // Device auth key for passwordless login
    private static final String KEY_ADVANCED_ENABLED = "advancedEnabled";
    private static final String KEY_REMOTE_HOSTNAME = "remoteHostname";
    private static final String KEY_CACHED_THEME_CSS = "cachedThemeCss";

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
    private boolean interfaceLoaded = false;
    private String loadedInterfaceUrl = null;
    // Removed duplicate screenOffWakeLock - ClayForegroundService already holds one
    private Handler keepaliveHandler;
    private Runnable keepaliveRunnable;
    private Handler backgroundShutdownHandler;
    private Runnable backgroundShutdownRunnable;
    private NativeWebSocket nativeWebSocket;
    private int nativeWsFailures = 0;
    private long lastWsConnectTime = 0;
    private Handler heartbeatHandler;
    private Runnable heartbeatRunnable;
    private int missedHeartbeats = 0;
    private static final int HEARTBEAT_INTERVAL_MS = 30000; // 30 seconds

    // JavaScript interface for communication between web and Android
    public class AndroidInterface {
        @JavascriptInterface
        public void openServerSettings() {
            runOnUiThread(() -> {
                // Open settings activity with fromMenu flag (don't clear host/port)
                Intent intent = new Intent(MainActivity.this, SettingsActivity.class);
                intent.putExtra("errorMessage", "Change Clay server connection");
                intent.putExtra("fromMenu", true);
                startActivity(intent);
            });
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
                // Battery optimization should already be handled during startup

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

        @JavascriptInterface
        public void showToast(String message) {
            runOnUiThread(() -> {
                Toast.makeText(MainActivity.this, message, Toast.LENGTH_SHORT).show();
            });
        }

        @JavascriptInterface
        public void connectWebSocket(String url) {
            runOnUiThread(() -> {
                // Close any existing connection
                if (nativeWebSocket != null) {
                    nativeWebSocket.close();
                }

                nativeWebSocket = new NativeWebSocket(new NativeWebSocket.WebSocketCallback() {
                    @Override
                    public void onOpen() {
                        nativeWsFailures = 0;
                        lastWsConnectTime = System.currentTimeMillis();
                        runOnUiThread(() -> {
                            webView.evaluateJavascript("if (typeof onNativeWebSocketOpen === 'function') onNativeWebSocketOpen();", null);
                        });
                    }

                    @Override
                    public void onMessage(String message) {
                        runOnUiThread(() -> {
                            // Use Base64 encoding to safely pass message to JavaScript
                            // This handles all special characters without escaping issues
                            String base64 = android.util.Base64.encodeToString(
                                message.getBytes(java.nio.charset.StandardCharsets.UTF_8),
                                android.util.Base64.NO_WRAP
                            );
                            webView.evaluateJavascript(
                                "if (typeof onNativeWebSocketMessageBase64 === 'function') onNativeWebSocketMessageBase64(\"" + base64 + "\");",
                                null
                            );
                        });
                    }

                    @Override
                    public void onClose(int code, String reason) {
                        nativeWsFailures++;
                        // If we've failed 4+ times and the last successful connect was >30s ago
                        // (or never), the current host is likely unreachable — reload with
                        // host resolution to try the alternate address (local↔remote fallback)
                        boolean staleConnection = lastWsConnectTime == 0
                            || (System.currentTimeMillis() - lastWsConnectTime) > 30000;
                        if (nativeWsFailures >= 4 && staleConnection) {
                            nativeWsFailures = 0;
                            android.util.Log.i("Clay", "Multiple WS failures, reloading with host resolution");
                            runOnUiThread(() -> {
                                resolveHostnameAndLoad();
                            });
                            return;
                        }
                        runOnUiThread(() -> {
                            String escaped = reason != null ? reason.replace("\"", "\\\"") : "";
                            webView.evaluateJavascript("if (typeof onNativeWebSocketClose === 'function') onNativeWebSocketClose(" + code + ", \"" + escaped + "\");", null);
                        });
                    }

                    @Override
                    public void onError(String error) {
                        runOnUiThread(() -> {
                            String escaped = error != null ? error.replace("\"", "\\\"") : "Unknown error";
                            webView.evaluateJavascript("if (typeof onNativeWebSocketError === 'function') onNativeWebSocketError(\"" + escaped + "\");", null);
                        });
                    }
                });

                nativeWebSocket.connect(url);
            });
        }

        @JavascriptInterface
        public void sendWebSocketMessage(String message) {
            if (nativeWebSocket != null) {
                nativeWebSocket.send(message);
            }
        }

        @JavascriptInterface
        public void closeWebSocket() {
            if (nativeWebSocket != null) {
                nativeWebSocket.close();
                nativeWebSocket = null;
            }
        }

        @JavascriptInterface
        public boolean hasNativeWebSocket() {
            return true;
        }

        @JavascriptInterface
        public String getConnectionInfo() {
            SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
            String localHost = prefs.getString(KEY_SERVER_HOST, "");
            String remoteHost = prefs.getString(KEY_REMOTE_HOSTNAME, "");
            int port = prefs.getInt(KEY_SERVER_PORT, 9000);
            boolean advancedEnabled = prefs.getBoolean(KEY_ADVANCED_ENABLED, false);
            boolean useSecure = prefs.getBoolean(KEY_USE_SECURE, false);
            return "{\"localHost\":\"" + localHost.replace("\"", "") +
                   "\",\"remoteHost\":\"" + remoteHost.replace("\"", "") +
                   "\",\"port\":" + port +
                   ",\"advancedEnabled\":" + advancedEnabled +
                   ",\"useSecure\":" + useSecure + "}";
        }

        @JavascriptInterface
        public void saveThemeCss(String cssVars) {
            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit()
                .putString(KEY_CACHED_THEME_CSS, cssVars).apply();
        }

        @JavascriptInterface
        public void reloadPage() {
            runOnUiThread(() -> {
                // Close WebSocket connection
                if (nativeWebSocket != null) {
                    nativeWebSocket.close();
                    nativeWebSocket = null;
                }
                // Clear cache for a true hard refresh
                webView.clearCache(true);
                // Reset interface loaded flag
                interfaceLoaded = false;
                loadedInterfaceUrl = null;
                // Reload from server
                loadWebInterface();
            });
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

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
            openSettings("Connection cancelled");
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
        checkBatteryOptimization();
    }

    private void checkBatteryOptimization() {
        // Step 2: Check battery optimization
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
            String packageName = getPackageName();
            if (pm != null && !pm.isIgnoringBatteryOptimizations(packageName)) {
                // Need to request - will return via onActivityResult
                Intent intent = new Intent();
                intent.setAction(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS);
                intent.setData(Uri.parse("package:" + packageName));
                try {
                    startActivityForResult(intent, BATTERY_OPTIMIZATION_REQUEST);
                    return; // Wait for callback
                } catch (Exception e) {
                    // Device doesn't support this intent
                    Toast.makeText(this,
                        "Please disable battery optimization for Clay in Settings",
                        Toast.LENGTH_LONG).show();
                }
            }
        }
        batteryOptimizationDone = true;
        finishPermissionFlow();
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
            checkBatteryOptimization();
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == BATTERY_OPTIMIZATION_REQUEST) {
            batteryOptimizationDone = true;
            finishPermissionFlow();
        }
    }

    private void proceedAfterPermissions() {
        // Check if we have saved server settings
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, null);
        int port = prefs.getInt(KEY_SERVER_PORT, 0);

        if (host == null || host.isEmpty() || port == 0) {
            // No saved settings, open settings activity
            openSettings(null);
        } else {
            // Load the web interface
            loadWebInterface();
        }
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

                    // Close WebSocket
                    if (nativeWebSocket != null) {
                        nativeWebSocket.close();
                        nativeWebSocket = null;
                    }

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

                // Intercept HTTPS requests to handle certificate issues
                if (url.startsWith("https://")) {
                    Exception lastException = null;
                    // Retry up to 3 times for transient connection failures
                    for (int attempt = 1; attempt <= 3; attempt++) {
                        // Create fresh client for each attempt to avoid connection reuse issues
                        okhttp3.OkHttpClient freshClient = activity.createTrustAllClient();
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
                        openSettings("Connection failed to " + request.getUrl().getHost() + ": " + error.getDescription());
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
            public boolean onConsoleMessage(ConsoleMessage consoleMessage) {
                // Check for WebSocket connection errors
                String message = consoleMessage.message();
                if (message != null && message.contains("WebSocket connection") &&
                    (message.contains("failed") || message.contains("error"))) {
                    runOnUiThread(() -> {
                        openSettings("WebSocket connection failed to " + (loadedInterfaceUrl != null ? loadedInterfaceUrl : "server"));
                    });
                }
                return super.onConsoleMessage(consoleMessage);
            }
        });
    }

    /**
     * Resolves which hostname to use.
     * If advanced mode is enabled, tries the local (standard) hostname first with a fast
     * TCP probe. If that fails, falls back to the remote hostname.
     * This avoids unreliable IP/netmask matching and handles NAT hairpinning issues.
     */
    private void resolveHostnameAndLoad() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String standardHost = prefs.getString(KEY_SERVER_HOST, "192.168.2.6");
        int port = prefs.getInt(KEY_SERVER_PORT, 9000);
        boolean useSecure = prefs.getBoolean(KEY_USE_SECURE, false);
        boolean advancedEnabled = prefs.getBoolean(KEY_ADVANCED_ENABLED, false);
        String remoteHostname = prefs.getString(KEY_REMOTE_HOSTNAME, "");

        connectionFailed = false;
        connectCancelled = false;

        if (!advancedEnabled || remoteHostname.isEmpty()) {
            // No advanced mode — just connect to the standard host
            String protocol = useSecure ? "https" : "http";
            String url = protocol + "://" + standardHost + ":" + port;
            showConnectingOverlay(url);
            loadUrl(url, useSecure);
            return;
        }

        // Advanced mode: try local first, fall back to remote
        showConnectingOverlay(standardHost + " ...");

        new Thread(() -> {
            // Quick TCP probe to the local address (2 second timeout)
            boolean localReachable = false;
            try {
                java.net.Socket socket = new java.net.Socket();
                socket.connect(new java.net.InetSocketAddress(standardHost, port), 2000);
                socket.close();
                localReachable = true;
                android.util.Log.i("Clay", "Local host " + standardHost + ":" + port + " is reachable");
            } catch (Exception e) {
                android.util.Log.i("Clay", "Local host " + standardHost + ":" + port + " unreachable: " + e.getMessage());
            }

            if (connectCancelled) return;

            String host = localReachable ? standardHost : remoteHostname;
            String protocol = useSecure ? "https" : "http";
            String url = protocol + "://" + host + ":" + port;

            android.util.Log.i("Clay", "Using hostname: " + host + (localReachable ? " (local)" : " (remote)"));

            runOnUiThread(() -> {
                if (connectCancelled) return;
                connectingUrl.setText(host);
                loadUrl(url, useSecure);
            });
        }).start();
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
     * Load the web interface from a resolved URL.
     * For HTTPS, fetches resources via OkHttp and inlines them.
     * For HTTP, loads directly in the WebView.
     */
    private void loadUrl(String url, boolean useSecure) {
        new Thread(() -> {
            try {
                if (connectCancelled) return;

                // Fast path: load HTML/CSS/JS from APK assets (no network requests needed)
                String html = loadHtmlFromAssets();

                if (html != null) {
                    html = inlineAssetsIntoHtml(html);
                    html = substituteTemplateVars(html, useSecure);
                } else if (useSecure) {
                    // HTTPS fallback: fetch from server (assets not available in APK)
                    okhttp3.OkHttpClient client = createTrustAllClient();

                    okhttp3.Response htmlResp = client.newCall(
                        new okhttp3.Request.Builder().url(url).build()).execute();
                    if (connectCancelled) { htmlResp.close(); return; }
                    if (!htmlResp.isSuccessful()) {
                        final int code = htmlResp.code();
                        htmlResp.close();
                        runOnUiThread(() -> {
                            hideConnectingOverlay();
                            openSettings("HTTP " + code + " loading " + url);
                        });
                        return;
                    }
                    html = htmlResp.body().string();
                    htmlResp.close();

                    try {
                        okhttp3.Response cssResp = client.newCall(
                            new okhttp3.Request.Builder().url(url + "/style.css").build()).execute();
                        if (cssResp.isSuccessful()) {
                            String css = cssResp.body().string();
                            html = html.replace("<link rel=\"stylesheet\" href=\"style.css\">",
                                               "<style>\n" + css + "\n</style>");
                            html = html.replace("<link rel=\"stylesheet\" href=\"/style.css\">",
                                               "<style>\n" + css + "\n</style>");
                        }
                        cssResp.close();
                    } catch (Exception e) { /* ignore */ }

                    try {
                        okhttp3.Response jsResp = client.newCall(
                            new okhttp3.Request.Builder().url(url + "/app.js").build()).execute();
                        if (jsResp.isSuccessful()) {
                            String js = jsResp.body().string();
                            html = html.replace("<script src=\"app.js\"></script>",
                                               "<script>\n" + js + "\n</script>");
                            html = html.replace("<script src=\"/app.js\"></script>",
                                               "<script>\n" + js + "\n</script>");
                        }
                        jsResp.close();
                    } catch (Exception e) { /* ignore */ }
                } else {
                    // HTTP fallback: let WebView fetch normally
                    final String loadUrl = url;
                    runOnUiThread(() -> {
                        webView.loadUrl(loadUrl);
                        interfaceLoaded = true;
                        loadedInterfaceUrl = loadUrl;
                        // WebViewClient.onPageFinished will hide the overlay
                    });
                    return;
                }

                if (connectCancelled) return;
                final String finalHtml = html;
                runOnUiThread(() -> {
                    if (connectCancelled) return;
                    webView.loadDataWithBaseURL(url, finalHtml, "text/html", "UTF-8", null);
                    interfaceLoaded = true;
                    loadedInterfaceUrl = url;
                    hideConnectingOverlay();
                });
            } catch (Exception e) {
                runOnUiThread(() -> {
                    hideConnectingOverlay();
                    openSettings("Failed to connect to " + url + ": " + e.getMessage());
                });
            }
        }).start();
    }

    private void loadWebInterface() {
        resolveHostnameAndLoad();
    }

    /** Read index.html from APK assets, or null if not available. */
    private String loadHtmlFromAssets() {
        try {
            java.io.InputStream is = getAssets().open("web/index.html");
            byte[] buffer = new byte[is.available()];
            is.read(buffer);
            is.close();
            return new String(buffer, "UTF-8");
        } catch (java.io.IOException e) {
            return null;
        }
    }

    /** Inline style.css and app.js from APK assets into the HTML string. */
    private String inlineAssetsIntoHtml(String html) {
        try {
            java.io.InputStream is = getAssets().open("web/style.css");
            byte[] buffer = new byte[is.available()];
            is.read(buffer);
            is.close();
            String css = new String(buffer, "UTF-8");
            html = html.replace("<link rel=\"stylesheet\" href=\"style.css\">",
                                "<style>\n" + css + "\n</style>");
            html = html.replace("<link rel=\"stylesheet\" href=\"/style.css\">",
                                "<style>\n" + css + "\n</style>");
        } catch (java.io.IOException e) { /* fall back to linked CSS */ }

        try {
            java.io.InputStream is = getAssets().open("web/app.js");
            byte[] buffer = new byte[is.available()];
            is.read(buffer);
            is.close();
            String js = new String(buffer, "UTF-8");
            html = html.replace("<script src=\"app.js\"></script>",
                                "<script>\n" + js + "\n</script>");
            html = html.replace("<script src=\"/app.js\"></script>",
                                "<script>\n" + js + "\n</script>");
        } catch (java.io.IOException e) { /* fall back to linked JS */ }

        return html;
    }

    /** Substitute template variables in the HTML using locally-known values. */
    private String substituteTemplateVars(String html, boolean useSecure) {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String cachedTheme = prefs.getString(KEY_CACHED_THEME_CSS, DEFAULT_THEME_CSS);
        return html
            .replace("{{WS_HOST}}", "")   // JS falls back to window.location.hostname
            .replace("{{WS_PORT}}", "0")  // sentinel: JS uses window.location.port
            .replace("{{WS_PROTOCOL}}", useSecure ? "wss" : "ws")
            .replace("{{THEME_CSS_VARS}}", cachedTheme);
    }

    /**
     * Creates an OkHttpClient that accepts all SSL certificates.
     * Used for intercepting HTTPS requests to handle hostname mismatches and expired certs.
     */
    private okhttp3.OkHttpClient createTrustAllClient() {
        try {
            // Create a trust manager that accepts all certificates
            final javax.net.ssl.TrustManager[] trustAllCerts = new javax.net.ssl.TrustManager[]{
                new javax.net.ssl.X509TrustManager() {
                    @Override
                    public void checkClientTrusted(java.security.cert.X509Certificate[] chain, String authType) {
                        // Accept all client certificates
                    }

                    @Override
                    public void checkServerTrusted(java.security.cert.X509Certificate[] chain, String authType) {
                        // Accept all server certificates (including self-signed, expired, hostname mismatch)
                    }

                    @Override
                    public java.security.cert.X509Certificate[] getAcceptedIssuers() {
                        return new java.security.cert.X509Certificate[0];
                    }
                }
            };

            // Install the trust manager
            final javax.net.ssl.SSLContext sslContext = javax.net.ssl.SSLContext.getInstance("TLS");
            sslContext.init(null, trustAllCerts, new java.security.SecureRandom());
            final javax.net.ssl.SSLSocketFactory sslSocketFactory = sslContext.getSocketFactory();

            // Build OkHttpClient with custom SSL settings
            // Disable connection pooling to avoid stale connection issues
            return new okhttp3.OkHttpClient.Builder()
                .sslSocketFactory(sslSocketFactory, (javax.net.ssl.X509TrustManager) trustAllCerts[0])
                .hostnameVerifier((hostname, session) -> true) // Accept all hostnames
                .connectTimeout(15, java.util.concurrent.TimeUnit.SECONDS)
                .readTimeout(30, java.util.concurrent.TimeUnit.SECONDS)
                .writeTimeout(15, java.util.concurrent.TimeUnit.SECONDS)
                .connectionPool(new okhttp3.ConnectionPool(0, 1, java.util.concurrent.TimeUnit.MILLISECONDS)) // No pooling
                .retryOnConnectionFailure(true)
                .build();
        } catch (Exception e) {
            android.util.Log.e("Clay", "Failed to create trust-all client: " + e.getMessage());
            // Return a default client if trust-all setup fails
            return new okhttp3.OkHttpClient.Builder()
                .connectTimeout(10, java.util.concurrent.TimeUnit.SECONDS)
                .readTimeout(30, java.util.concurrent.TimeUnit.SECONDS)
                .build();
        }
    }

    private void openSettings(String errorMessage) {
        Intent intent = new Intent(this, SettingsActivity.class);
        if (errorMessage != null) {
            intent.putExtra("errorMessage", errorMessage);
        }
        startActivity(intent);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // Called when activity is brought to front via FLAG_ACTIVITY_CLEAR_TOP
        // Force a check to load the web interface if settings are now available
        android.util.Log.i("Clay", "onNewIntent called, checking if interface needs loading");
        checkAndLoadInterface();
    }

    @Override
    protected void onResume() {
        super.onResume();

        // Cancel background shutdown timer since user is back
        cancelBackgroundShutdownTimer();

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
        // Check if we have saved server settings and need to load interface
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, null);
        int port = prefs.getInt(KEY_SERVER_PORT, 0);
        boolean useSecure = prefs.getBoolean(KEY_USE_SECURE, false);

        if (host != null && !host.isEmpty() && port > 0) {
            if (!interfaceLoaded) {
                android.util.Log.i("Clay", "Loading interface: not yet loaded");
                loadWebInterface();
            } else if (webView != null) {
                // Interface loaded - always verify connection health on resume.
                // Java's isConnected flag can be stale if the WS died silently,
                // so let JS check the actual socket state and reconnect if needed.
                // This also handles the visibilitychange event not firing on some Android versions.
                missedHeartbeats = 0;
                webView.evaluateJavascript(
                    "if (typeof checkConnectionOnResume === 'function') checkConnectionOnResume();",
                    null
                );
            }
        }
    }

    @Override
    protected void onPause() {
        super.onPause();
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
