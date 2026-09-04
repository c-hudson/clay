;; requires-lib
/require complete.tf
/eval /echo complete=$[!!ismacro("complete")] ctx=$[!!ismacro("complete_context")] vars=$[!!ismacro("complete_variable")]
/quit
