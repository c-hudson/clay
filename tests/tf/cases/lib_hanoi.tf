;; requires-lib
;; lib_hanoi.tf - hanoi's moves go to the MUD (a "simple command" line, per
;; do_hanoi's :moves-a-disk text), not the screen, so about all that's
;; testable headlessly is that the library defines the macro.
/require hanoi.tf
/eval /echo loaded=$[!!ismacro("hanoi")]
/quit
