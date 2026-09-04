;; strings.tf - probes TF's built-in string functions.
;;
;; Finding C.12: TinyFugue does NOT expand %var/$[...]/$(...) on a top-level
;; line read from a file - only inside a macro body, or after /eval performs
;; its own extra substitution pass. Every probe below is wrapped in /eval so
;; both TinyFugue and Clay expand it the same way.
/eval /echo strlen=$[strlen("hello")]
/eval /echo substr1=$[substr("hello world",6)]
/eval /echo substr2=$[substr("hello world",0,5)]
/eval /echo strchr=$[strchr("hello","l")]
/eval /echo strcat=$[strcat("foo","bar","baz")]
/eval /echo toupper=$[toupper("hello")]
/eval /echo tolower=$[tolower("HELLO")]
/eval /echo pad=[$[pad("hi",5)]]
/quit
