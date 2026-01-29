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
    private static final String KEY_LAST_LOADED_URL = "lastLoadedUrl";
    private static final String KEY_SAVED_PASSWORD = "savedPassword";

    private static final String CHANNEL_ID_ALERTS = "clay_alerts";
    private static final String CHANNEL_ID_SERVICE = "clay_service";
    private static final int NOTIFICATION_PERMISSION_REQUEST = 1001;
    private static final int KEEPALIVE_INTERVAL_MS = 30000; // 30 seconds

    private WebView webView;
    private boolean connectionFailed = false;
    private int notificationId = 1000;
    private boolean isConnected = false;
    private PowerManager.WakeLock screenOffWakeLock;
    private Handler keepaliveHandler;
    private Runnable keepaliveRunnable;
    private NativeWebSocket nativeWebSocket;

    // JavaScript interface for communication between web and Android
    public class AndroidInterface {
        @JavascriptInterface
        public void openServerSettings() {
            runOnUiThread(() -> {
                // Clear saved host/port to force settings screen to show
                SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
                SharedPreferences.Editor editor = prefs.edit();
                editor.remove(KEY_SERVER_HOST);
                editor.remove(KEY_SERVER_PORT);
                editor.apply();

                // Open settings activity
                openSettings("Change Clay server connection");
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
                // Request battery optimization exemption first
                requestBatteryOptimizationExemption();

                Intent serviceIntent = new Intent(MainActivity.this, ClayForegroundService.class);
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    startForegroundService(serviceIntent);
                } else {
                    startService(serviceIntent);
                }

                // Mark as connected and start keepalive
                isConnected = true;
                acquireScreenOffWakeLock();
                startKeepalive();
            });
        }

        @JavascriptInterface
        public void stopBackgroundService() {
            runOnUiThread(() -> {
                isConnected = false;
                stopKeepalive();
                releaseScreenOffWakeLock();

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
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        // Request notification permission for Android 13+
        requestNotificationPermission();

        // Request battery optimization exemption to prevent Doze from killing the service
        requestBatteryOptimizationExemption();

        // Create notification channels
        createNotificationChannel();
        createServiceNotificationChannel();

        webView = findViewById(R.id.webView);
        setupWebView();

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

    private void requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                    != PackageManager.PERMISSION_GRANTED) {
                ActivityCompat.requestPermissions(this,
                    new String[]{Manifest.permission.POST_NOTIFICATIONS},
                    NOTIFICATION_PERMISSION_REQUEST);
            }
        }
    }

    private void requestBatteryOptimizationExemption() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
            String packageName = getPackageName();
            if (pm != null && !pm.isIgnoringBatteryOptimizations(packageName)) {
                // Request exemption from battery optimization
                Intent intent = new Intent();
                intent.setAction(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS);
                intent.setData(Uri.parse("package:" + packageName));
                try {
                    startActivity(intent);
                } catch (Exception e) {
                    // Some devices may not support this intent
                    Toast.makeText(this,
                        "Please disable battery optimization for Clay in Settings",
                        Toast.LENGTH_LONG).show();
                }
            }
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

            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.createNotificationChannel(channel);
            }
        }
    }

    private void acquireScreenOffWakeLock() {
        if (screenOffWakeLock == null) {
            PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
            if (pm != null) {
                screenOffWakeLock = pm.newWakeLock(
                    PowerManager.PARTIAL_WAKE_LOCK,
                    "Clay::ScreenOffWakeLock"
                );
                screenOffWakeLock.acquire();
            }
        }
    }

    private void releaseScreenOffWakeLock() {
        if (screenOffWakeLock != null && screenOffWakeLock.isHeld()) {
            screenOffWakeLock.release();
            screenOffWakeLock = null;
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

    private void setupWebView() {
        WebSettings webSettings = webView.getSettings();
        webSettings.setJavaScriptEnabled(true);
        webSettings.setDomStorageEnabled(true);
        webSettings.setMixedContentMode(WebSettings.MIXED_CONTENT_ALWAYS_ALLOW);

        // Add JavaScript interface for Android communication
        webView.addJavascriptInterface(new AndroidInterface(), "Android");

        webView.setWebViewClient(new WebViewClient() {
            @Override
            public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                super.onReceivedError(view, request, error);
                android.util.Log.w("Clay", "WebView error " + error.getErrorCode() + " on " + request.getUrl() + " main=" + request.isForMainFrame());
                // Only handle errors for the main frame
                if (request.isForMainFrame()) {
                    connectionFailed = true;
                    runOnUiThread(() -> {
                        openSettings("Connection failed: " + error.getDescription());
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
                android.util.Log.i("Clay", "Page loaded: " + url);
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
                        openSettings("WebSocket connection failed");
                    });
                }
                return super.onConsoleMessage(consoleMessage);
            }
        });
    }

    private void loadWebInterface() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, "192.168.2.6");
        int port = prefs.getInt(KEY_SERVER_PORT, 9000);
        boolean useSecure = prefs.getBoolean(KEY_USE_SECURE, false);

        String protocol = useSecure ? "https" : "http";
        String url = protocol + "://" + host + ":" + port;

        connectionFailed = false;

        // Persist URL to survive Activity recreation
        prefs.edit().putString(KEY_LAST_LOADED_URL, url).apply();

        webView.loadUrl(url);
    }

    private void openSettings(String errorMessage) {
        Intent intent = new Intent(this, SettingsActivity.class);
        if (errorMessage != null) {
            intent.putExtra("errorMessage", errorMessage);
        }
        startActivity(intent);
    }

    @Override
    protected void onResume() {
        super.onResume();
        // Only reload if settings changed or WebView was destroyed
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, null);
        int port = prefs.getInt(KEY_SERVER_PORT, 0);
        boolean useSecure = prefs.getBoolean(KEY_USE_SECURE, false);
        String savedUrl = prefs.getString(KEY_LAST_LOADED_URL, null);

        if (host != null && !host.isEmpty() && port > 0) {
            String protocol = useSecure ? "https" : "http";
            String expectedUrl = protocol + "://" + host + ":" + port;

            // Check if WebView already has content loaded
            String webViewUrl = webView.getUrl();
            boolean webViewHasContent = webViewUrl != null && webViewUrl.startsWith(expectedUrl);

            // Only reload if:
            // 1. WebView was destroyed (no URL or wrong URL)
            // 2. Settings changed (expected URL differs from saved URL)
            if (!webViewHasContent || (savedUrl != null && !expectedUrl.equals(savedUrl))) {
                loadWebInterface();
            }
            // Don't reload if just returning from background with WebView intact
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
    }

    @Override
    protected void onDestroy() {
        stopKeepalive();
        releaseScreenOffWakeLock();
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
