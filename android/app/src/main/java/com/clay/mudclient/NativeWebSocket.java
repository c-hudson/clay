package com.clay.mudclient;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.Socket;
import java.net.SocketAddress;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Arrays;
import java.util.concurrent.TimeUnit;

import javax.net.SocketFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509ExtendedTrustManager;

import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;

/**
 * Native WebSocket client that accepts self-signed certificates.
 * Bridges WebSocket communication between Java and JavaScript.
 *
 * When an auth key is available, connections perform a CLAY-KNOCK v1 preamble
 * (see SECURITY-ROADMAP.md design decision D4) on the raw TCP socket before
 * TLS/HTTP begins. This lets the server hard-drop connections from IPs that
 * aren't in its allow list unless they can prove possession of the auth key.
 */
public class NativeWebSocket {
    private static final String TAG = "NativeWebSocket";

    // Per-process flag: once a knock attempt fails in a way that indicates the
    // server doesn't speak CLAY-KNOCK v1 (old server), skip the knock for the
    // rest of this app session so every subsequent reconnect doesn't pay the
    // knock timeout again. Set ONLY on a knock-PROTOCOL failure (bad/missing
    // challenge magic, bad ack, timeout mid-knock) — never on an ordinary
    // network error (host unreachable, DNS failure, connection refused before
    // the knock even starts), since those say nothing about server support.
    private static volatile boolean knockUnsupported = false;

    private WebSocket webSocket;
    private OkHttpClient client;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private WebSocketCallback callback;
    private boolean isConnected = false;
    private final Context appContext;

    public interface WebSocketCallback {
        void onOpen();
        void onMessage(String message);
        void onClose(int code, String reason);
        void onError(String error);
    }

    public NativeWebSocket(Context context, WebSocketCallback callback) {
        this.appContext = context.getApplicationContext();
        this.callback = callback;
    }

    /** Backward-compatible overload: no auth key means no knock attempt. */
    public void connect(String url) {
        connect(url, null);
    }

    public void connect(String url, String authKey) {
        boolean useKnock = authKey != null && !authKey.isEmpty() && !knockUnsupported;
        connectInternal(url, authKey, useKnock);
    }

    private void connectInternal(String url, String authKey, boolean useKnock) {
        Log.d(TAG, "Connecting to: " + url + (useKnock ? " (with CLAY-KNOCK)" : ""));

        try {
            // Trust-on-first-use certificate pinning (see CertPinning) instead of accepting
            // every certificate unconditionally - mirrors the Rust TofuVerifier used by every
            // other Clay client. Hostname verification is intentionally left permissive here
            // (like the Rust side): the per-host-keyed pin itself is what provides the
            // protection, and many self-hosted Clay servers use ad hoc self-signed certs
            // without proper SANs.
            final X509ExtendedTrustManager tofuTrustManager = CertPinning.createTofuTrustManager(appContext);

            final SSLContext sslContext = SSLContext.getInstance("TLS");
            sslContext.init(null, new TrustManager[]{tofuTrustManager}, new java.security.SecureRandom());
            final SSLSocketFactory sslSocketFactory = sslContext.getSocketFactory();

            // Build OkHttpClient with custom SSL settings
            OkHttpClient.Builder builder = new OkHttpClient.Builder()
                .sslSocketFactory(sslSocketFactory, tofuTrustManager)
                .hostnameVerifier((hostname, session) -> true) // see comment above
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.MILLISECONDS) // No read timeout for WebSocket
                .writeTimeout(10, TimeUnit.SECONDS)
                .pingInterval(30, TimeUnit.SECONDS); // Keep connection alive

            if (useKnock) {
                // OkHttp creates the raw socket via this factory and calls
                // socket.connect(address, timeout) to establish the TCP
                // connection BEFORE layering sslSocketFactory (wss) or writing
                // the HTTP upgrade request (ws). KnockSocket.connect() runs the
                // CLAY-KNOCK v1 handshake synchronously right after the TCP
                // handshake completes, so the ordering required by D4 falls out
                // automatically for both schemes.
                builder.socketFactory(new KnockSocketFactory(authKey));
            }

            client = builder.build();

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
                    if (useKnock && isKnockRejection(t)) {
                        // Old-server fallback (SECURITY-ROADMAP.md Phase 4 point 3):
                        // the TCP connection succeeded but the knock handshake
                        // itself failed (bad/missing challenge magic, bad ack, or
                        // a timeout mid-handshake) — a strong signal the server on
                        // the other end doesn't speak CLAY-KNOCK v1. Remember that
                        // for the rest of this session and silently retry once
                        // without the knock, so the UX is unaffected.
                        Log.w(TAG, "Knock rejected/unsupported, falling back without knock: " + t.getMessage());
                        knockUnsupported = true;
                        isConnected = false;
                        connectInternal(url, authKey, false);
                        return;
                    }

                    // Build detailed error message
                    String errorMsg = t.getClass().getSimpleName();
                    if (t.getMessage() != null && !t.getMessage().isEmpty()) {
                        errorMsg += ": " + t.getMessage();
                    }
                    if (t.getCause() != null) {
                        errorMsg += " (caused by: " + t.getCause().getClass().getSimpleName();
                        if (t.getCause().getMessage() != null) {
                            errorMsg += ": " + t.getCause().getMessage();
                        }
                        errorMsg += ")";
                    }
                    Log.e(TAG, "WebSocket error: " + errorMsg, t);
                    isConnected = false;
                    final String finalError = errorMsg;
                    mainHandler.post(() -> {
                        if (callback != null) {
                            callback.onError(finalError);
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

    /**
     * True if the throwable chain contains the sentinel "knock rejected"
     * IOException thrown by KnockSocket.connect() when the CLAY-KNOCK v1
     * handshake itself fails. This is distinct from ordinary connection
     * errors (host unreachable, DNS failure, refused) that happen before
     * KnockSocket ever gets a chance to run the handshake — those propagate
     * with their own message/type and are never mistaken for a knock
     * rejection.
     */
    private static boolean isKnockRejection(Throwable t) {
        Throwable cur = t;
        int depth = 0;
        while (cur != null && depth < 8) {
            if (cur instanceof IOException && "knock rejected".equals(cur.getMessage())) {
                return true;
            }
            cur = cur.getCause();
            depth++;
        }
        return false;
    }

    // Null the callback so subsequent async close/error events don't reach JS.
    // Call this before close() when replacing an old socket with a new one.
    public void clearCallback() {
        this.callback = null;
    }

    public void send(String message) {
        if (webSocket != null && isConnected) {
            webSocket.send(message);
        } else {
            Log.w(TAG, "Cannot send message: WebSocket not connected");
        }
    }

    public void close() {
        Log.d(TAG, "Closing WebSocket connection");
        isConnected = false;
        if (webSocket != null) {
            try {
                webSocket.close(1000, "Client closing");
            } catch (Exception e) {
                Log.w(TAG, "Error closing WebSocket: " + e.getMessage());
            }
            webSocket = null;
        }
        if (client != null) {
            try {
                // Cancel all pending calls
                client.dispatcher().cancelAll();
                // Don't shutdown executor - just let it be garbage collected
                // Shutting down can cause issues with rapid reconnect
            } catch (Exception e) {
                Log.w(TAG, "Error cleaning up client: " + e.getMessage());
            }
            client = null;
        }
    }

    public boolean isConnected() {
        return isConnected;
    }

    /**
     * SocketFactory that hands OkHttp a {@link KnockSocket} for every raw
     * socket it creates, so the CLAY-KNOCK v1 preamble runs as part of the
     * TCP connect step, before OkHttp layers TLS or writes the HTTP upgrade.
     */
    private static class KnockSocketFactory extends SocketFactory {
        private final String authKey;

        KnockSocketFactory(String authKey) {
            this.authKey = authKey;
        }

        @Override
        public Socket createSocket() throws IOException {
            return new KnockSocket(authKey);
        }

        @Override
        public Socket createSocket(String host, int port) throws IOException {
            return new KnockSocket(authKey);
        }

        @Override
        public Socket createSocket(String host, int port, InetAddress localHost, int localPort) throws IOException {
            return new KnockSocket(authKey);
        }

        @Override
        public Socket createSocket(InetAddress host, int port) throws IOException {
            return new KnockSocket(authKey);
        }

        @Override
        public Socket createSocket(InetAddress address, int port, InetAddress localAddress, int localPort) throws IOException {
            return new KnockSocket(authKey);
        }
    }

    /**
     * A plain TCP socket that runs the CLAY-KNOCK v1 client handshake
     * (SECURITY-ROADMAP.md design decision D4) immediately after the TCP
     * connection is established, before returning control to the caller
     * (OkHttp). OkHttp then layers TLS (wss) or writes the HTTP upgrade
     * request (ws) on this same, already-knocked socket.
     *
     * Wire protocol (all sizes fixed, all reads exact):
     *   C->S HELLO (6 bytes):      C7 4C 41 59 01 00
     *   S->C CHALLENGE (34 bytes): C7 4B + 32 random bytes
     *   C->S RESPONSE (32 bytes):  raw SHA256(auth_key_utf8 || challenge) —
     *                              NOT the hex-string convention used by the
     *                              WS-level challenge-response in app.js's
     *                              tryAuthWithKey/hash_with_challenge; this is
     *                              a separate transport-gating proof using the
     *                              raw digest bytes.
     *   S->C ACK (2 bytes):        C7 06
     */
    private static class KnockSocket extends Socket {
        private static final byte[] KNOCK_HELLO = {(byte) 0xC7, 0x4C, 0x41, 0x59, 0x01, 0x00};
        private static final int KNOCK_TIMEOUT_MS = 10000;

        private final String authKey;

        KnockSocket(String authKey) {
            this.authKey = authKey;
        }

        @Override
        public void connect(SocketAddress endpoint, int timeout) throws IOException {
            super.connect(endpoint, timeout);

            int originalTimeout = getSoTimeout();
            try {
                setSoTimeout(KNOCK_TIMEOUT_MS);
                try {
                    performKnock();
                } catch (IOException e) {
                    // Normalize every knock-handshake failure (bad magic, short
                    // read/EOF, timeout, digest mismatch reported by server as a
                    // closed connection) to a single distinguishable message so
                    // NativeWebSocket.isKnockRejection() can reliably tell "old
                    // server, fall back" apart from an ordinary network error —
                    // the TCP connect() above already succeeded at this point.
                    throw new IOException("knock rejected", e);
                }
            } finally {
                // Restore whatever timeout the caller (OkHttp) had configured;
                // 0 means "no timeout", which matches the socket default.
                setSoTimeout(originalTimeout);
            }
        }

        private void performKnock() throws IOException {
            OutputStream out = getOutputStream();
            DataInputStream in = new DataInputStream(getInputStream());

            // C->S HELLO
            out.write(KNOCK_HELLO);
            out.flush();

            // S->C CHALLENGE: C7 4B + 32 random bytes
            byte[] challengeMsg = new byte[34];
            in.readFully(challengeMsg);
            if (challengeMsg[0] != (byte) 0xC7 || challengeMsg[1] != 0x4B) {
                throw new IOException("bad knock challenge magic");
            }
            byte[] challenge = Arrays.copyOfRange(challengeMsg, 2, 34);

            // C->S RESPONSE: raw SHA256(auth_key_utf8 || challenge)
            byte[] digest;
            try {
                MessageDigest md = MessageDigest.getInstance("SHA-256");
                md.update(authKey.getBytes(StandardCharsets.UTF_8));
                md.update(challenge);
                digest = md.digest();
            } catch (NoSuchAlgorithmException e) {
                throw new IOException("SHA-256 unavailable", e);
            }
            out.write(digest);
            out.flush();

            // S->C ACK: C7 06
            byte[] ack = new byte[2];
            in.readFully(ack);
            if (ack[0] != (byte) 0xC7 || ack[1] != 0x06) {
                throw new IOException("bad knock ack");
            }
        }
    }
}
