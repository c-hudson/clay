;; The status area's fields as real tf keeps them: status_fields() after
;; /status_add, /status_rm, /status_edit, /clock, /status_save, /status_restore,
;; /status_defaults and a (deprecated) /set status_fields.
/def sf = /echo [$[status_fields()]]
/def sf1 = /echo 1:[$[status_fields(1)]]
/def sv = /echo var:[%{status_fields}]
/sf
/status_add -A@world hp:4
/sf
/status_add -B foo:3
/sf
/status_add -x hp:4
/status_add hp:4
/sf
/status_rm @mail
/status_rm hp
/sf
/status_edit @log:1
/sf
/sv
/status_add -r1 bar:5
/status_add -r1 bar:5
/sf1
/status_add -c "lit" :2 'q' `bq`
/sf
/status_add -B -s0 first:2
/sf
/status_add -c a:-5 b:-0 :2:r "lit":4:B "es\"c" @world:5:Cred,u x:03
/sf
/status_add -c :1 a:2 :3
/status_rm a
/sf
/status_add -c @world
/clock
/sf
/clock %I:%M:%S
/sf
/clock off
/sf
/status_save mine
/status_defaults
/sf
/status_restore mine
/sf
/set warn_status=off
/set status_fields=y:2 :1 @world
/sf
/sv
/quit
