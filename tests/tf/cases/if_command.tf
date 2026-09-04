;; if_command.tf - /if /command%; /then ... (finding C.8: Clay requires the
;; condition to be enclosed in parentheses and rejects a bare /command
;; condition).
/if /test 1%; /then /echo yes%; /endif
/if /test 0%; /then /echo no%; /else /echo else-branch%; /endif
/quit
