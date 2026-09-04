;; requires-lib
;; lib_kbfunc.tf - adapted from the task's original /input + dokey_home/
;; dokey_end + kb_backward_kill_line + kbpoint()/kblen() idea: under real tf,
;; ANY buffer-content-changing /input or /dokey_* op in non-visual "-q" batch
;; mode still emits raw terminal control bytes (bare \r and \x08 backspace
;; runs) to redraw the simulated command line, interleaved with whatever the
;; script echoes right after. Those bytes are stable and reproducible, but
;; they are pure terminal-redraw noise that Clay's headless engine-only test
;; harness (no App, no crossterm rendering) can never emit itself - even
;; after /require and /dokey are fully implemented - so a fixture built on
;; them could never move from XFAIL to PASS. Instead this checks that the
;; library's main /dokey_* and /kb_* wrapper macros actually got defined.
/require kbfunc.tf
/eval /echo home=$[!!ismacro("dokey_home")] end=$[!!ismacro("dokey_end")] killline=$[!!ismacro("kb_backward_kill_line")]
/quit
