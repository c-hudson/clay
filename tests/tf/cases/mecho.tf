;; %mecho: each command a macro runs, and each line a file runs, echoed with
;; %mprefix once per level (echo() is used rather than /echo, which is a
;; library macro in TF).
/def inner = /test echo("in")
/def foo = /test echo("hi")%; /inner%; /let v=1
/def -i hid = /test echo("hidden")
/set mecho=on
/foo
/hid
/set mprefix=>>
/foo
/set mecho=all
/hid
/set mecho=off
/foo
/quit
