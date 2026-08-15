package com.clay.mudclient;

/**
 * Session-scoped state that must outlive {@link MainActivity}.
 *
 * <p>The bundled local server and the SSH tunnel carry the user's live session: in local mode the
 * server holds all in-memory world state, and in remote+SSH mode the tunnel is the transport.
 * Both used to be Activity fields that {@code onDestroy()} explicitly stopped, which tied the
 * session's lifetime to the UI's. Android destroys and recreates Activities routinely — the Back
 * button, a notification tap, or simply an OEM reclaiming a backgrounded Activity — and each time
 * it did, the server was SIGKILLed and the tunnel torn down. Returning to the app then had to
 * rebuild both from scratch, which is what made a switch-away-and-back look like a cold start
 * even though the foreground service had kept the process alive the whole time.
 *
 * <p>Holding them here instead scopes them to the <em>process</em>, which is precisely the
 * lifetime the foreground service exists to protect. A recreated Activity finds the server and
 * tunnel still running and simply reattaches.
 *
 * <p>Static state is the right shape for this: it lives exactly as long as the process, and dies
 * with it. Both managers already keep only {@code getApplicationContext()}, so there is no
 * Activity leak. They are torn down deliberately — see {@link #shutdown()} — never implicitly by
 * a lifecycle callback.
 */
final class ClaySession {

    private static LocalServerManager localServer;
    private static SshProxyManager sshProxy;

    /**
     * Run mode / SSH config last actually applied to the running managers above. Lives here for
     * the same reason they do: {@code reloadInterfaceRespectingRunMode()} compares the current
     * preferences against these to decide whether anything really changed, and if they reset to
     * null on every Activity recreation that check reads "changed" and forces a needless restart
     * of a perfectly healthy server or tunnel.
     */
    private static String lastAppliedRunMode;
    private static String lastAppliedSshConfigSnapshot;

    private ClaySession() {}

    static synchronized LocalServerManager localServer() {
        return localServer;
    }

    static synchronized void setLocalServer(LocalServerManager m) {
        localServer = m;
    }

    static synchronized SshProxyManager sshProxy() {
        return sshProxy;
    }

    static synchronized void setSshProxy(SshProxyManager m) {
        sshProxy = m;
    }

    static synchronized String lastAppliedRunMode() {
        return lastAppliedRunMode;
    }

    static synchronized void setLastAppliedRunMode(String mode) {
        lastAppliedRunMode = mode;
    }

    static synchronized String lastAppliedSshConfigSnapshot() {
        return lastAppliedSshConfigSnapshot;
    }

    static synchronized void setLastAppliedSshConfigSnapshot(String snapshot) {
        lastAppliedSshConfigSnapshot = snapshot;
    }

    /** True when the bundled server is up, without constructing one if it isn't. */
    static synchronized boolean isLocalServerRunning() {
        return localServer != null && localServer.isRunning();
    }

    /** True when the SSH tunnel is up, without constructing one if it isn't. */
    static synchronized boolean isSshProxyRunning() {
        return sshProxy != null && sshProxy.isRunning();
    }

    /**
     * Tear the session down for real.
     *
     * <p>Called only when the user genuinely ends the session — the notification's "Disconnect"
     * action, the {@code stopBackgroundService()} JS bridge, or a run-mode/SSH-config change that
     * makes the current server or tunnel wrong. Deliberately NOT called from
     * {@code MainActivity.onDestroy()}: an Activity going away says nothing about whether the
     * user is done.
     */
    static synchronized void shutdown() {
        if (localServer != null) {
            localServer.stop();
            localServer = null;
        }
        if (sshProxy != null) {
            sshProxy.stop();
            sshProxy = null;
        }
        lastAppliedRunMode = null;
        lastAppliedSshConfigSnapshot = null;
    }
}
