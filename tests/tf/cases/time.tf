;; time.tf - /time itself always shows the actual current wall-clock time
;; (there is no deterministic form of it - see finding B: TF's missing
;; format-string form is Phase 1 work, and Clay's own current /time [/cmd]
;; form necessarily varies run to run too), so it is covered by unit tests
;; instead of a script fixture. The only thing left in this space that is
;; both deterministic and testable is ftime() with a fixed timestamp -
;; functions.tf already exercises that more thoroughly (%Y-%m plus several
;; other functions); this just probes a second, different format string so
;; the case isn't purely a duplicate.
/eval /echo $[ftime("%Y", 1000000000)]
/quit
