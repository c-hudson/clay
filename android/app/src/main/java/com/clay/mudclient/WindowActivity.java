package com.clay.mudclient;

import android.os.Bundle;

import java.util.Collections;
import java.util.Set;
import java.util.WeakHashMap;

/**
 * A second Clay window: `/window [world]`, `/window --grep ...` and the menu's New Window, as
 * the desktop GUI and the browser open one (user ruling 2026-10-16: the app implements
 * /window too). Its own task (FLAG_ACTIVITY_NEW_DOCUMENT | MULTIPLE_TASK), so it sits beside
 * the main window in split screen and has its own entry in recents. It is the same page as
 * the main window, given the browser's window-mode query (`?world=`, `?grep=`), connecting to
 * the session the main window started (ClaySession's local server or SSH tunnel, or the
 * saved remote server) - it starts, restarts and stops none of it: closing it closes only it.
 */
public class WindowActivity extends MainActivity {
    /** The page's query string (`?world=...` / `?grep=...`), empty for a plain window. */
    static final String EXTRA_QUERY = "clay_window_query";

    /** The open windows, so the main window's /quit can close them with the app. */
    private static final Set<WindowActivity> OPEN = Collections.newSetFromMap(new WeakHashMap<>());

    @Override
    protected boolean isWindowActivity() {
        return true;
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        synchronized (OPEN) {
            OPEN.add(this);
        }
    }

    @Override
    protected void onDestroy() {
        synchronized (OPEN) {
            OPEN.remove(this);
        }
        super.onDestroy();
    }

    /** /quit in the main window: every window goes with the app. */
    static void finishAll() {
        WindowActivity[] open;
        synchronized (OPEN) {
            open = OPEN.toArray(new WindowActivity[0]);
        }
        for (WindowActivity w : open) {
            w.finishAndRemoveTask();
        }
    }
}
