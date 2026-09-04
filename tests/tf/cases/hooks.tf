;; hooks.tf - hook events beyond Clay's original 11 (finding C.10, fixed
;; Job 10: all 31 TF hook events now parse), and the -h"EVENT pattern"
;; combined syntax (also fixed Job 10: fire_hook now matches the pattern
;; against the hook's own argument text the same way a trigger matches a
;; line - see hooks::fire_hook's doc comment).
;;
;; Adapted from the task's original "send a plain `greet bob` line" idea:
;; under real tf, a plain line is a "simple command" that gets sent to the
;; current world, and with no world connected that's a hard, unrecoverable
;; "Invalid command. Aborting." that stops the whole script load (verified -
;; real tf then hangs waiting on stdin instead of continuing to /quit). Using
;; /trigger's own -h<event> option instead exercises the same SEND hook
;; without needing a live connection - it also happens to (verified, stable
;; across repeated runs) echo the simulated text once on its own, which real
;; tf does as local-echo feedback for what would have been sent.
/def -hCONNECT h1 = /echo connect-hook
/trigger -hCONNECT somehost
/def -h"SEND greet*" h2 = /echo send-hook %*
/trigger -hSEND greet bob
/def -hLOADFAIL h3 = /echo lf
/quit
