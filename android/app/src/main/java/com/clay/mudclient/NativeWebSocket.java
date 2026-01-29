package com.clay.mudclient;

import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.concurrent.TimeUnit;

import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509TrustManager;

import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;

/**
 * Native WebSocket client that accepts self-signed certificates.
 * Bridges WebSocket communication between Java and JavaScript.
 */
public class NativeWebSocket {
    private static final String TAG = "NativeWebSocket";

    private WebSocket webSocket;
    private OkHttpClient client;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private WebSocketCallback callback;
    private boolean isConnected = false;

    public interface WebSocketCallback {
        void onOpen();
        void onMessage(String message);
        void onClose(int code, String reason);
        void onError(String error);
    }

    public NativeWebSocket(WebSocketCallback callback) {
        this.callback = callback;
    }

    public void connect(String url) {
        Log.d(TAG, "Connecting to: " + url);

        try {
            // Create a trust manager that accepts all certificates
            final TrustManager[] trustAllCerts = new TrustManager[]{
                new X509TrustManager() {
                    @Override
                    public void checkClientTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                        // Accept all client certificates
                    }

                    @Override
                    public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                        // Accept all server certificates (including self-signed)
                    }

                    @Override
                    public X509Certificate[] getAcceptedIssuers() {
                        return new X509Certificate[0];
                    }
                }
            };

            // Install the trust manager
            final SSLContext sslContext = SSLContext.getInstance("TLS");
            sslContext.init(null, trustAllCerts, new java.security.SecureRandom());
            final SSLSocketFactory sslSocketFactory = sslContext.getSocketFactory();

            // Build OkHttpClient with custom SSL settings
            client = new OkHttpClient.Builder()
                .sslSocketFactory(sslSocketFactory, (X509TrustManager) trustAllCerts[0])
                .hostnameVerifier((hostname, session) -> true) // Accept all hostnames
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.MILLISECONDS) // No read timeout for WebSocket
                .writeTimeout(10, TimeUnit.SECONDS)
                .pingInterval(30, TimeUnit.SECONDS) // Keep connection alive
                .build();

            Request request = new Request.Builder()
                .url(url)
                .build();

            webSocket = client.newWebSocket(request, new WebSocketListener() {
                @Override
                public void onOpen(WebSocket webSocket, Response response) {
                    Log.d(TAG, "WebSocket connected");
                    isConnected = true;
                    mainHandler.post(() -> {
                        if (callback != null) {
                            callback.onOpen();
                        }
                    });
                }

                @Override
                public void onMessage(WebSocket webSocket, String text) {
                    mainHandler.post(() -> {
                        if (callback != null) {
                            callback.onMessage(text);
                        }
                    });
                }

                @Override
                public void onClosing(WebSocket webSocket, int code, String reason) {
                    Log.d(TAG, "WebSocket closing: " + code + " " + reason);
                    webSocket.close(code, reason);
                }

                @Override
                public void onClosed(WebSocket webSocket, int code, String reason) {
                    Log.d(TAG, "WebSocket closed: " + code + " " + reason);
                    isConnected = false;
                    mainHandler.post(() -> {
                        if (callback != null) {
                            callback.onClose(code, reason);
                        }
                    });
                }

                @Override
                public void onFailure(WebSocket webSocket, Throwable t, Response response) {
                    Log.e(TAG, "WebSocket error: " + t.getMessage(), t);
                    isConnected = false;
                    mainHandler.post(() -> {
                        if (callback != null) {
                            callback.onError(t.getMessage());
                        }
                    });
                }
            });

        } catch (Exception e) {
            Log.e(TAG, "Failed to create WebSocket: " + e.getMessage(), e);
            mainHandler.post(() -> {
                if (callback != null) {
                    callback.onError("Failed to create WebSocket: " + e.getMessage());
                }
            });
        }
    }

    public void send(String message) {
        if (webSocket != null && isConnected) {
            webSocket.send(message);
        } else {
            Log.w(TAG, "Cannot send message: WebSocket not connected");
        }
    }

    public void close() {
        if (webSocket != null) {
            webSocket.close(1000, "Client closing");
            webSocket = null;
        }
        if (client != null) {
            client.dispatcher().executorService().shutdown();
            client = null;
        }
        isConnected = false;
    }

    public boolean isConnected() {
        return isConnected;
    }
}
