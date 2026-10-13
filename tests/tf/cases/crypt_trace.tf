;; crypt_trace.tf - the pieces of crypt.tf's listen_mush trigger, each made visible:
;; which branch a /trigger'd say line takes, what a gagged /echo inside a trigger
;; shows (nothing), and what /substitute inside a /trigger'd line shows.
/def -t"gagme*" -mglob gag_in_trigger = /echo -ag -- hidden %1%;/echo shown %1
/trigger gagme one
/def -t"subme*" -mglob subst_in_trigger = /substitute -aCred -- replaced %1
/trigger subme two
/def -t"both*" -mglob both_in_trigger = /echo -ag -- %*%;/substitute -aCred -- replaced %1
/trigger both three
/def -mregexp -t'^Bob (say|says|says,) "(.*)"$$' trace = \
   /echo P1=%P1 P2=%P2 first=$[substr({P2},0,1)]%;\
   /if (substr({P2},0,1) =~ "\\") \
      /echo branch=backslash%;\
   /else \
      /echo branch=plain%;\
   /endif%;\
   /let d=abc3.14%;\
   /if (d =/ "*3.14") /echo glob-match-ok%; /else /echo glob-match-FAILED%; /endif%;\
   /let g=garbage%;\
   /if (g =/ "*3.14") /echo glob-nomatch-FAILED%; /else /echo glob-nomatch-ok%; /endif%;\
   /if (d =/ "\:*") /echo colon-match-FAILED%; /else /echo colon-nomatch-ok%; /endif
/trigger Bob says, "plain text"
/eval /trigger Bob says, "\\coded"
/quit
