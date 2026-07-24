; Lua highlights (tree-sitter-lua)

(comment) @comment
(string) @string
(number) @number

[
  "function" "local" "end"
  "if" "then" "else" "elseif"
  "for" "while" "do" "repeat" "until"
  "return" "in"
  "and" "or" "not"
] @keyword

(break_statement) @keyword
(goto_statement) @keyword

[
  (nil)
  (true)
  (false)
] @constant

(function_declaration name: (identifier) @function)
(function_call name: (identifier) @function)
(function_call (dot_index_expression field: (identifier) @function))
