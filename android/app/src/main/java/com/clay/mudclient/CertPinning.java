package com.clay.mudclient;

import android.content.Context;
import android.content.SharedPreferences;
import android.util.Log;

import java.net.Socket;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.security.cert.CertificateEncodingException;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.Locale;

import javax.net.ssl.SSLEngine;
import javax.net.ssl.SSLSession;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.X509ExtendedTrustManager;

/**
 * Trust-on-first-use (TOFU) certificate pinning for Android's TLS connections to a
 * user-configured Clay server (local or remote). Mirrors the same TOFU model already used by
 * every other Clay client (the Rust {@code TofuVerifier} — see {@code src/platform.rs},
 * {@code SECURITY-ROADMAP.md} design decision D7): pin the SHA-256 fingerprint of a host's leaf
 * certificate the first time it's seen, and reject any later connection whose certificate
 * doesn't match — rather than silently trusting whatever certificate is presented, which is
 * what an accept-all {@code X509TrustManager}/{@code HostnameVerifier} does. Accept-all trust
 * managers are exactly what Google Play's automated security scanning flags as "Insecure
 * TrustManager"/"Insecure HostnameVerifier" (CWE-295).
 *
 * Pins are stored per {@code "host:port"} in a dedicated SharedPreferences file so they survive
 * app restarts, and can be cleared from {@code SettingsActivity} ("Clear Pinned Certificates")
 * if a server's certificate legitimately rotates.
 *
 * As with the Rust TofuVerifier, this deliberately does not perform standard hostname/CA-chain
 * verification on top of pinning — many Clay servers (and MUDs) use ad hoc self-signed
 * certificates without proper SANs. The per-host-keyed pin itself is what provides the
 * protection: once pinned, a connection only succeeds if it presents the exact same certificate
 * bytes previously seen for that exact host:port.
 */
final class CertPinning {
    private static final String TAG = "CertPinning";
    static final String PREFS_NAME = "clay_known_hosts";

    private CertPinning() {}

    /** Thrown when a pinned host presents a certificate that doesn't match its stored pin. */
    static final class PinMismatchException extends CertificateException {
        final String hostPort;
        PinMismatchException(String hostPort) {
            super("Certificate for " + hostPort + " does not match the previously pinned "
                + "certificate. If you rotated this server's certificate intentionally, clear "
                + "pinned certificates in Settings and reconnect.");
            this.hostPort = hostPort;
        }
    }

    /** Wipes every stored pin (used by SettingsActivity's "Clear Pinned Certificates"). */
    static void clearAllPins(Context context) {
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE).edit().clear().apply();
    }

    /** True if at least one certificate is currently pinned. */
    static boolean hasAnyPins(Context context) {
        SharedPreferences prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
        return !prefs.getAll().isEmpty();
    }

    /**
     * Builds an {@link X509ExtendedTrustManager} that pins the first certificate seen per
     * host:port and rejects any later mismatch. Uses the extended (Socket/SSLEngine-aware)
     * trust manager API specifically so the actual target hostname is available at
     * verification time — a plain {@code X509TrustManager} has no way to know which host the
     * certificate is even for.
     */
    static X509ExtendedTrustManager createTofuTrustManager(Context context) {
        final Context appContext = context.getApplicationContext();
        return new X509ExtendedTrustManager() {
            @Override
            public void checkClientTrusted(X509Certificate[] chain, String authType) {
                // Clay never presents a client certificate - nothing to check.
            }

            @Override
            public void checkClientTrusted(X509Certificate[] chain, String authType, Socket socket) {
            }

            @Override
            public void checkClientTrusted(X509Certificate[] chain, String authType, SSLEngine engine) {
            }

            @Override
            public void checkServerTrusted(X509Certificate[] chain, String authType) throws CertificateException {
                // Called without any host context. Fail closed: pinning can't be applied
                // safely without knowing which host this certificate is for, and every TLS
                // stack Clay actually uses (OkHttp over SSLSocket) calls one of the
                // extended overloads below instead of this one.
                throw new CertificateException("Certificate pinning requires host context");
            }

            @Override
            public void checkServerTrusted(X509Certificate[] chain, String authType, Socket socket) throws CertificateException {
                verify(chain, hostPortFromSocket(socket));
            }

            @Override
            public void checkServerTrusted(X509Certificate[] chain, String authType, SSLEngine engine) throws CertificateException {
                verify(chain, hostPortFromEngine(engine));
            }

            private void verify(X509Certificate[] chain, String hostPort) throws CertificateException {
                if (chain == null || chain.length == 0) {
                    throw new CertificateException("No server certificate presented");
                }
                if (hostPort == null) {
                    throw new CertificateException("Unable to determine host for certificate pinning");
                }
                String fingerprint = sha256Hex(chain[0]);
                SharedPreferences prefs = appContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE);
                String pinned = prefs.getString(hostPort, null);
                if (pinned == null) {
                    // First time seeing this host:port - trust on first use, and pin it.
                    prefs.edit().putString(hostPort, fingerprint).apply();
                    Log.i(TAG, "Pinned new certificate for " + hostPort);
                } else if (!pinned.equals(fingerprint)) {
                    Log.w(TAG, "Certificate mismatch for " + hostPort);
                    throw new PinMismatchException(hostPort);
                }
            }

            @Override
            public X509Certificate[] getAcceptedIssuers() {
                return new X509Certificate[0];
            }
        };
    }

    private static String hostPortFromSocket(Socket socket) {
        if (socket instanceof SSLSocket) {
            SSLSession session = ((SSLSocket) socket).getHandshakeSession();
            if (session == null) {
                session = ((SSLSocket) socket).getSession();
            }
            String hostPort = hostPortFromSession(session);
            if (hostPort != null) {
                return hostPort;
            }
        }
        if (socket != null && socket.getInetAddress() != null) {
            return socket.getInetAddress().getHostName().toLowerCase(Locale.US) + ":" + socket.getPort();
        }
        return null;
    }

    private static String hostPortFromEngine(SSLEngine engine) {
        if (engine == null || engine.getPeerHost() == null) {
            return null;
        }
        return engine.getPeerHost().toLowerCase(Locale.US) + ":" + engine.getPeerPort();
    }

    private static String hostPortFromSession(SSLSession session) {
        if (session == null || session.getPeerHost() == null) {
            return null;
        }
        return session.getPeerHost().toLowerCase(Locale.US) + ":" + session.getPeerPort();
    }

    private static String sha256Hex(X509Certificate cert) throws CertificateException {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            byte[] digest = md.digest(cert.getEncoded());
            StringBuilder sb = new StringBuilder(digest.length * 2);
            for (byte b : digest) {
                sb.append(String.format(Locale.US, "%02x", b));
            }
            return sb.toString();
        } catch (NoSuchAlgorithmException | CertificateEncodingException e) {
            throw new CertificateException("Unable to hash certificate", e);
        }
    }
}
