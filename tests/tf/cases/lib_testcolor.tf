;; requires-lib
;; preload: stdlib.tf
;; lib_testcolor.tf - loading testcolor.tf itself prints the colour tables
;; (attributes like {C1}/{Cbg2} are stripped to plain text by our filter
;; the same way real terminal color codes would be invisible/absent in a
;; plain-text capture); verified stable across two consecutive oracle runs.
;; testcolor.tf itself calls "/_echo" (stdlib.tf's own "/def -i _echo =
;; /test echo({*})", a thin wrapper Clay's harness needs preloaded the same
;; way stdlib_macros.tf already does - real tf always has the whole stdlib
;; loaded before any script runs, per C.12's fixture convention/README).
/require testcolor.tf
/quit
