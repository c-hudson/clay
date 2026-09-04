;; eval.tf - probes /eval (finding B: TF does one extra substitution pass on
;; its argument, then executes it - unlike a plain top-level line, which per
;; finding C.12 is never substituted at all).
;;
;; The middle probe deliberately does NOT write `/eval %cmd` for a variable
;; `cmd` holding a whole command (e.g. "/echo from-var") - under real tf that
;; is dispatched as plain text to send to the world (not a command), because
;; the command/text decision is syntactic on the *unexpanded* argument text
;; ("%cmd" does not itself start with "/"), which fails "Not connected" in a
;; headless run and bakes this fixture's absolute file path into the error
;; text. Instead the leading "/" is written literally in the /eval argument
;; and only the command's *tail* comes from a variable, which both real tf
;; and this probe's intent (can /eval reach into a variable to build and run
;; a command) survive.
/set v=7
/eval /echo v=%v
/set cmdtail=echo from-var
/eval /%cmdtail
/eval /echo nested=$(/echo $(/echo deep))
/quit
