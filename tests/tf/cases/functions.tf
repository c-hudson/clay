;; functions.tf - built-in functions Clay does not implement yet (finding
;; C.11): features, mktime, strip_attr, morepaused, winlines, cputime, ln.
;; replace() also probes the TF argument order (old, new, string) - a B
;; ruling (Clay's replace() today takes (string, old, new)). Boolean-ish
;; results are normalized with !!/comparisons so the exact numeric value
;; (e.g. a real winlines()/cputime() count) never has to match verbatim.
/eval /echo features=$[!!features("256colors")]
/eval /echo ftime=$[ftime("%Y-%m", 1000000000)]
/eval /echo mktime_positive=$[mktime(2001,9,9,0,0,0) > 0]
/eval /echo replace=$[replace("a","o","banana")]
/eval /echo strip_attr=$[strip_attr("x")]
/eval /echo morepaused=$[morepaused()]
/eval /echo winlines_positive=$[winlines() > 0]
/eval /echo cputime_nonneg=$[cputime() >= 0]
/eval /echo ln1=$[ln(1)]
/quit
