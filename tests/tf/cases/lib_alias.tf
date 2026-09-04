;; requires-lib
;; lib_alias.tf - adapted from the task's original "plain `greet bob` line"
;; idea: same hard-abort problem as hooks.tf (a plain unconnected line is an
;; unrecoverable "Invalid command. Aborting." under real tf - see hooks.tf's
;; note). /alias's own mechanism is a SEND hook, so /trigger -hSEND exercises
;; it the same safe way hooks.tf does, without needing a live connection.
/require alias.tf
/alias greet /echo greetings %1
/trigger -hSEND greet bob
/quit
