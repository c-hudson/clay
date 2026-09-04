;; macro_result.tf - a macro used as a function via /result (finding C.5:
;; /result is missing in Clay), invoked both as $[dbl(21)] and via command
;; substitution $(/dbl 5).
/def dbl = /result {1} * 2
/eval /echo dbl=$[dbl(21)] sub=$(/dbl 5)
/quit
