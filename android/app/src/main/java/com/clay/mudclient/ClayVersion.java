package com.clay.mudclient;

import android.content.Context;
import android.os.Build;
import android.util.Log;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.util.concurrent.TimeUnit;

/**
 * This app's build as Clay's {@code /version} names it after "Clay ":
 * {@code v1.7.11 (build 26/10/19 13:16-1d4a8b0a) [android/aarch64]} (src/main.rs
 * {@code version_detail}). It is the bundled Clay's own, asked of libclay.so itself
 * ({@code --version}) once per process, off the UI thread - the app's web page and its
 * server are built from the same tree, which is what the build hash covers. Shown as the
 * {@code Local} line of {@code /version} in Run on Server (app.js
 * {@code attachedLocalVersionDetail}).
 */
final class ClayVersion {
    private static final String TAG = "ClayVersion";
    private static final long PROBE_TIMEOUT_MS = 5000;

    private static volatile String detail;
    private static Thread probe;

    private ClayVersion() {}

    /** Starts asking libclay.so, if nobody has yet. */
    static synchronized void warm(Context context) {
        if (detail != null || probe != null) {
            return;
        }
        final Context app = context.getApplicationContext();
        probe = new Thread(() -> {
            String d = ask(app);
            detail = d != null ? d : fallback(app);
        }, "clay-version");
        probe.setDaemon(true);
        probe.start();
    }

    /** The detail, waiting up to {@code waitMs} for libclay.so's answer; the app's version
     *  number alone if it has none. */
    static String detail(Context context, long waitMs) {
        warm(context);
        Thread t;
        synchronized (ClayVersion.class) {
            t = probe;
        }
        if (detail == null && t != null) {
            try {
                t.join(waitMs);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        String d = detail;
        return d != null ? d : fallback(context.getApplicationContext());
    }

    private static String ask(Context app) {
        String bin = app.getApplicationInfo().nativeLibraryDir + "/libclay.so";
        Process process = null;
        try {
            ProcessBuilder pb = new ProcessBuilder(bin, "--version");
            pb.environment().put("HOME", app.getFilesDir().getAbsolutePath());
            pb.redirectErrorStream(true);
            process = pb.start();
            String first;
            try (BufferedReader r = new BufferedReader(new InputStreamReader(process.getInputStream()))) {
                first = r.readLine();
            }
            if (!process.waitFor(PROBE_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                Log.w(TAG, "libclay.so --version did not exit");
                return null;
            }
            if (first != null && first.startsWith("Clay v")) {
                return first.substring("Clay ".length()).trim();
            }
            Log.w(TAG, "libclay.so --version said: " + first);
        } catch (Exception e) {
            Log.w(TAG, "libclay.so --version failed", e);
        } finally {
            if (process != null) {
                process.destroy();
            }
        }
        return null;
    }

    /** {@code v1.7.11 [android/aarch64]}: what the app knows without libclay.so. */
    private static String fallback(Context app) {
        String version = "";
        try {
            version = app.getPackageManager().getPackageInfo(app.getPackageName(), 0).versionName;
        } catch (Exception ignored) {
        }
        return "v" + version + " [android/" + arch() + "]";
    }

    /** The device's ABI in Rust's names (std::env::consts::ARCH). */
    private static String arch() {
        String abi = Build.SUPPORTED_ABIS.length > 0 ? Build.SUPPORTED_ABIS[0] : "";
        switch (abi) {
            case "arm64-v8a": return "aarch64";
            case "armeabi-v7a": return "arm";
            case "x86_64": return "x86_64";
            case "x86": return "x86";
            default: return abi;
        }
    }
}
