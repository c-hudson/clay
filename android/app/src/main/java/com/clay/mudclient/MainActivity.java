package com.clay.mudclient;

import android.content.Intent;
import android.content.SharedPreferences;
import android.net.http.SslError;
import android.os.Bundle;
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

import androidx.appcompat.app.AppCompatActivity;

public class MainActivity extends AppCompatActivity {
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
    }

    private static final String PREFS_NAME = "ClayPrefs";
    private static final String KEY_SERVER_HOST = "serverHost";
    private static final String KEY_SERVER_PORT = "serverPort";
    private static final String KEY_USE_SECURE = "useSecure";

    private WebView webView;
    private boolean connectionFailed = false;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

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
                // For self-signed certificates, allow proceeding
                // In production, you might want to show a dialog
                handler.proceed();
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                connectionFailed = false;
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
        // Reload if returning from settings
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String host = prefs.getString(KEY_SERVER_HOST, null);
        int port = prefs.getInt(KEY_SERVER_PORT, 0);

        if (host != null && !host.isEmpty() && port > 0) {
            // Check if we need to reload (settings may have changed)
            if (!connectionFailed) {
                loadWebInterface();
            }
        }
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
