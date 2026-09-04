;; control_flow.tf - single-line /if...endif, with and without a space before
;; the closing %; (Clay's line-continuation handling loses track of the block
;; boundary when there is no space, per finding C.3).
/if (1) /echo spaced%; /endif
/if (1) /echo nospace%;/endif
/echo after
/quit
