;; requires-lib
/require kbregion.tf
/eval /echo mark=$[!!ismacro("kb_set_mark")] cut=$[!!ismacro("kb_cut_region")] copy=$[!!ismacro("kb_copy_region")] paste=$[!!ismacro("kb_paste_buffer")]
/quit
